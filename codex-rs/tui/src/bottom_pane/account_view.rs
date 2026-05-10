use std::path::PathBuf;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::render::renderable::Renderable;
use crate::tui::FrameRequester;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;

/// The steps of the ATA email-OTP login flow.
#[allow(dead_code)]
enum AtaLoginStep {
    /// Already signed in — show status with option to sign out.
    AlreadySigned {
        email: String,
    },
    EmailInput {
        email: String,
        error: Option<String>,
    },
    SendingOtp {
        email: String,
    },
    OtpInput {
        email: String,
        otp: String,
        error: Option<String>,
    },
    VerifyingOtp {
        email: String,
    },
    Success {
        email: String,
    },
    Error(String),
}

/// A `BottomPaneView` overlay that implements the `/account` slash command.
///
/// If already signed in, shows account status with sign-out option.
/// Otherwise walks the user through email OTP sign-in.
pub(crate) struct AccountView {
    step: Arc<RwLock<AtaLoginStep>>,
    frame_requester: FrameRequester,
    codex_home: PathBuf,
    complete: bool,
}

impl AccountView {
    pub(crate) fn new(frame_requester: FrameRequester, codex_home: PathBuf) -> Self {
        // Check for session-expired marker written by the daemon when the
        // refresh token is rejected by Supabase (HTTP 400/401/403).
        let marker_path = codex_home.join("session_expired");
        let session_was_expired = marker_path.exists();
        if session_was_expired {
            // Remove the marker so we don't show this message again.
            let _ = std::fs::remove_file(&marker_path);
        }

        let initial_step = if session_was_expired {
            // The daemon detected an expired refresh token — force re-login.
            AtaLoginStep::EmailInput {
                email: String::new(),
                error: Some("Your ATA session has expired. Please sign in again.".to_string()),
            }
        } else {
            match crate::legacy_core::supabase::load_ata_session(&codex_home) {
                Ok(Some(session)) if !crate::legacy_core::supabase::is_session_expired(&session) => {
                    AtaLoginStep::AlreadySigned {
                        email: session.user.email,
                    }
                }
                _ => AtaLoginStep::EmailInput {
                    email: String::new(),
                    error: None,
                },
            }
        };
        Self {
            step: Arc::new(RwLock::new(initial_step)),
            frame_requester,
            codex_home,
            complete: false,
        }
    }

    /// Spawn an async task that sends the OTP to the given email address.
    fn spawn_send_otp(&self, email: String) {
        let step = self.step.clone();
        let frame_requester = self.frame_requester.clone();
        tokio::spawn(async move {
            let ata_config = crate::legacy_core::config::types::AtaAccountConfig::default();
            let client = crate::legacy_core::supabase::SupabaseClient::new(
                ata_config.supabase_url,
                ata_config.supabase_anon_key,
            );
            let auth = crate::legacy_core::supabase::SupabaseAuth::new(client);
            match auth.sign_in_with_otp(&email).await {
                Ok(()) => {
                    *step.write().unwrap_or_else(PoisonError::into_inner) =
                        AtaLoginStep::OtpInput {
                            email,
                            otp: String::new(),
                            error: None,
                        };
                }
                Err(e) => {
                    *step.write().unwrap_or_else(PoisonError::into_inner) =
                        AtaLoginStep::EmailInput {
                            email,
                            error: Some(format!("Failed to send code: {e}")),
                        };
                }
            }
            frame_requester.schedule_frame();
        });
    }

    /// Spawn an async task that verifies the OTP code against the email.
    fn spawn_verify_otp(&self, email: String, otp: String) {
        let step = self.step.clone();
        let frame_requester = self.frame_requester.clone();
        let codex_home = self.codex_home.clone();
        tokio::spawn(async move {
            let ata_config = crate::legacy_core::config::types::AtaAccountConfig::default();
            let client = crate::legacy_core::supabase::SupabaseClient::new(
                ata_config.supabase_url,
                ata_config.supabase_anon_key,
            );
            let auth = crate::legacy_core::supabase::SupabaseAuth::new(client);
            match auth.exchange_code_for_session(&email, &otp).await {
                Ok(session) => {
                    if let Err(e) = crate::legacy_core::supabase::save_ata_session(&codex_home, &session) {
                        *step.write().unwrap_or_else(PoisonError::into_inner) =
                            AtaLoginStep::Error(format!("Failed to save session: {e}"));
                    } else {
                        *step.write().unwrap_or_else(PoisonError::into_inner) =
                            AtaLoginStep::Success { email };
                    }
                }
                Err(e) => {
                    *step.write().unwrap_or_else(PoisonError::into_inner) =
                        AtaLoginStep::OtpInput {
                            email,
                            otp: String::new(),
                            error: Some(format!("Verification failed: {e}")),
                        };
                }
            }
            frame_requester.schedule_frame();
        });
    }
}

impl BottomPaneView for AccountView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        let mut guard = self.step.write().unwrap_or_else(PoisonError::into_inner);
        match &mut *guard {
            AtaLoginStep::EmailInput { email, error } => match key_event {
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => {
                    drop(guard);
                    self.complete = true;
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } => {
                    let trimmed = email.trim().to_string();
                    if trimmed.is_empty() || !trimmed.contains('@') {
                        *error = Some("Please enter a valid email address.".to_string());
                    } else {
                        let email_owned = trimmed.clone();
                        *guard = AtaLoginStep::SendingOtp { email: trimmed };
                        drop(guard);
                        self.spawn_send_otp(email_owned);
                    }
                }
                KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                } => {
                    email.pop();
                    *error = None;
                }
                KeyEvent {
                    code: KeyCode::Char(c),
                    kind: KeyEventKind::Press,
                    modifiers,
                    ..
                } if !modifiers.contains(KeyModifiers::SUPER)
                    && !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    email.push(c);
                    *error = None;
                }
                _ => {}
            },
            AtaLoginStep::OtpInput { email, otp, error } => match key_event {
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => {
                    drop(guard);
                    self.complete = true;
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    ..
                } => {
                    if otp.len() != 6 {
                        *error = Some("Please enter the 6-digit code.".to_string());
                    } else {
                        let email_owned = email.clone();
                        let otp_owned = otp.clone();
                        *guard = AtaLoginStep::VerifyingOtp {
                            email: email_owned.clone(),
                        };
                        drop(guard);
                        self.spawn_verify_otp(email_owned, otp_owned);
                    }
                }
                KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                } => {
                    otp.pop();
                    *error = None;
                }
                KeyEvent {
                    code: KeyCode::Char(c),
                    kind: KeyEventKind::Press,
                    modifiers,
                    ..
                } if c.is_ascii_digit()
                    && !modifiers.contains(KeyModifiers::SUPER)
                    && !modifiers.contains(KeyModifiers::CONTROL)
                    && !modifiers.contains(KeyModifiers::ALT) =>
                {
                    if otp.len() < 6 {
                        otp.push(c);
                    }
                    *error = None;
                }
                _ => {}
            },
            AtaLoginStep::AlreadySigned { .. } => match key_event.code {
                KeyCode::Esc | KeyCode::Enter => {
                    drop(guard);
                    self.complete = true;
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    let _ = crate::legacy_core::supabase::delete_ata_session(&self.codex_home);
                    *guard = AtaLoginStep::EmailInput {
                        email: String::new(),
                        error: None,
                    };
                }
                _ => {}
            },
            AtaLoginStep::SendingOtp { .. } | AtaLoginStep::VerifyingOtp { .. } => {
                if matches!(key_event.code, KeyCode::Esc) {
                    drop(guard);
                    self.complete = true;
                }
            }
            AtaLoginStep::Success { .. } | AtaLoginStep::Error(_) => {
                if matches!(key_event.code, KeyCode::Enter | KeyCode::Esc) {
                    drop(guard);
                    self.complete = true;
                }
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        true
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if pasted.is_empty() {
            return false;
        }
        let mut guard = self.step.write().unwrap_or_else(PoisonError::into_inner);
        match &mut *guard {
            AtaLoginStep::EmailInput { email, error } => {
                email.push_str(pasted.trim());
                *error = None;
                true
            }
            AtaLoginStep::OtpInput { otp, error, .. } => {
                // Extract only digits from the pasted text, up to 6 total.
                for c in pasted.chars() {
                    if c.is_ascii_digit() && otp.len() < 6 {
                        otp.push(c);
                    }
                }
                *error = None;
                true
            }
            _ => false,
        }
    }
}

impl Renderable for AccountView {
    fn desired_height(&self, _width: u16) -> u16 {
        8
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.height < 4 || area.width < 6 {
            return None;
        }
        let guard = self.step.read().unwrap_or_else(PoisonError::into_inner);
        match &*guard {
            AtaLoginStep::EmailInput { email, .. } => {
                // The input box starts at row 3 (title + subtitle + spacing) inside the
                // layout.  The bordered block adds 1 for the top border and 1 for the left
                // border, so the text content is at (area.x + 2, area.y + 3).
                let text_x = area.x + 2 + email.len() as u16;
                let text_y = area.y + 3;
                Some((text_x, text_y))
            }
            AtaLoginStep::OtpInput { otp, .. } => {
                let text_x = area.x + 2 + otp.len() as u16;
                let text_y = area.y + 3;
                Some((text_x, text_y))
            }
            _ => None,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let guard = self.step.read().unwrap_or_else(PoisonError::into_inner);

        match &*guard {
            AtaLoginStep::AlreadySigned { email } => {
                let [title_area, status_area, hint_area] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Min(1),
                ])
                .areas(area);

                Paragraph::new(Line::from("A2A Account".bold())).render(title_area, buf);
                let display_email = if email.is_empty() {
                    "unknown"
                } else {
                    email.as_str()
                };
                Paragraph::new(Line::from(format!("Signed in as {display_email}")))
                    .style(Style::default().fg(Color::Green))
                    .render(status_area, buf);
                Paragraph::new(Line::from("S to sign out · Esc to close".dim()))
                    .render(hint_area, buf);
            }
            AtaLoginStep::EmailInput { email, error } => {
                let [title_area, subtitle_area, input_area, footer_area] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                ])
                .areas(area);

                Paragraph::new(Line::from("Sign in with A2A account".bold()))
                    .render(title_area, buf);
                Paragraph::new(Line::from("Enter your email address".dim()))
                    .render(subtitle_area, buf);

                let input_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray));
                let inner = input_block.inner(input_area);
                input_block.render(input_area, buf);

                if email.is_empty() {
                    Paragraph::new(Line::from("your@email.com".dim())).render(inner, buf);
                } else {
                    Paragraph::new(Line::from(email.as_str())).render(inner, buf);
                }

                let mut footer_lines: Vec<Line<'_>> = Vec::new();
                if let Some(err) = error {
                    footer_lines
                        .push(Line::from(err.as_str()).style(Style::default().fg(Color::Red)));
                }
                footer_lines.push(Line::from("Enter to continue \u{00b7} Esc to cancel".dim()));
                Paragraph::new(footer_lines).render(footer_area, buf);
            }

            AtaLoginStep::SendingOtp { email } => {
                let [title_area, body_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

                Paragraph::new(Line::from("Sign in with A2A account".bold()))
                    .render(title_area, buf);
                Paragraph::new(Line::from(format!("Sending sign-in code to {email}...")))
                    .wrap(Wrap { trim: false })
                    .render(body_area, buf);
            }

            AtaLoginStep::OtpInput { email, otp, error } => {
                let [title_area, subtitle_area, input_area, footer_area] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                ])
                .areas(area);

                Paragraph::new(Line::from("Check your email".bold())).render(title_area, buf);
                Paragraph::new(Line::from(
                    format!("Enter the 6-digit code sent to {email}").dim(),
                ))
                .render(subtitle_area, buf);

                let input_block = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray));
                let inner = input_block.inner(input_area);
                input_block.render(input_area, buf);

                if otp.is_empty() {
                    Paragraph::new(Line::from("000000".dim())).render(inner, buf);
                } else {
                    Paragraph::new(Line::from(otp.as_str())).render(inner, buf);
                }

                let mut footer_lines: Vec<Line<'_>> = Vec::new();
                if let Some(err) = error {
                    footer_lines
                        .push(Line::from(err.as_str()).style(Style::default().fg(Color::Red)));
                }
                footer_lines.push(Line::from("Enter to verify \u{00b7} Esc to cancel".dim()));
                Paragraph::new(footer_lines).render(footer_area, buf);
            }

            AtaLoginStep::VerifyingOtp { .. } => {
                let [title_area, body_area] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

                Paragraph::new(Line::from("Sign in with A2A account".bold()))
                    .render(title_area, buf);
                Paragraph::new(Line::from("Verifying...")).render(body_area, buf);
            }

            AtaLoginStep::Success { email } => {
                let [title_area, message_area, hint_area] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Min(1),
                ])
                .areas(area);

                Paragraph::new(Line::from("Signed in!".bold())).render(title_area, buf);
                Paragraph::new(Line::from(format!("Successfully signed in as {email}")))
                    .style(Style::default().fg(Color::Green))
                    .wrap(Wrap { trim: false })
                    .render(message_area, buf);
                Paragraph::new(Line::from("Press Enter to close".dim())).render(hint_area, buf);
            }

            AtaLoginStep::Error(msg) => {
                let [title_area, message_area, hint_area] = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Min(1),
                ])
                .areas(area);

                Paragraph::new(Line::from("Sign-in failed".bold())).render(title_area, buf);
                Paragraph::new(Line::from(msg.as_str()))
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: false })
                    .render(message_area, buf);
                Paragraph::new(Line::from("Press Enter to close".dim())).render(hint_area, buf);
            }
        }
    }
}
