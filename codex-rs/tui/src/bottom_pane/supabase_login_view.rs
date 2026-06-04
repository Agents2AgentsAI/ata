//! Bottom-pane view that runs the ATA Supabase OTP sign-in flow from inside
//! an active session.
//!
//! State machine:
//!   EnterEmail → SendingOtp → EnterOtp → VerifyingOtp → Success | Cancelled
//!
//! All app-server calls (`AtaSendOtp`, `AtaVerifyOtp`) are dispatched via
//! `AppEvent`s so the underlying transport stays in `event_dispatch.rs`. The
//! view holds an `Arc<RwLock<SupabaseLoginState>>` that the event dispatcher
//! also writes to when a result comes back — both sides see the same shared
//! state without threading channels through the view.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use std::sync::Arc;
use std::sync::RwLock;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::render::renderable::Renderable;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::bottom_pane_view::ViewCompletion;
use super::popup_consts::standard_popup_hint_line;

/// Shared state machine between the view and the event dispatcher.
///
/// The view edits `email` / `otp` text on key events. The dispatcher
/// rewrites the entire `phase` field when an app-server response lands.
#[derive(Debug, Clone)]
pub(crate) enum SupabaseLoginPhase {
    EnterEmail {
        email: String,
        error: Option<String>,
    },
    SendingOtp {
        email: String,
    },
    EnterOtp {
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
    Cancelled,
}

#[derive(Debug)]
pub(crate) struct SupabaseLoginState {
    pub(crate) phase: SupabaseLoginPhase,
}

impl SupabaseLoginState {
    pub(crate) fn new() -> Self {
        Self {
            phase: SupabaseLoginPhase::EnterEmail {
                email: String::new(),
                error: None,
            },
        }
    }
}

pub(crate) struct SupabaseLoginView {
    state: Arc<RwLock<SupabaseLoginState>>,
    app_event_tx: AppEventSender,
    completion: Option<ViewCompletion>,
}

impl SupabaseLoginView {
    pub(crate) fn new(
        state: Arc<RwLock<SupabaseLoginState>>,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self {
            state,
            app_event_tx,
            completion: None,
        }
    }

    fn handle_key_in_email(&mut self, key_event: KeyEvent) {
        let mut guard = match self.state.write() {
            Ok(g) => g,
            Err(_) => return,
        };
        let SupabaseLoginPhase::EnterEmail { email, error } = &mut guard.phase else {
            return;
        };
        match key_event.code {
            KeyCode::Char(c) if !key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                email.push(c);
                *error = None;
            }
            KeyCode::Backspace => {
                email.pop();
            }
            KeyCode::Enter => {
                let trimmed = email.trim().to_string();
                if trimmed.is_empty() {
                    *error = Some("Email is required.".to_string());
                    return;
                }
                if !trimmed.contains('@') {
                    *error = Some("Enter a valid email address.".to_string());
                    return;
                }
                guard.phase = SupabaseLoginPhase::SendingOtp {
                    email: trimmed.clone(),
                };
                drop(guard);
                self.app_event_tx
                    .send(AppEvent::SupabaseSendOtp { email: trimmed });
            }
            _ => {}
        }
    }

    fn handle_key_in_otp(&mut self, key_event: KeyEvent) {
        let mut guard = match self.state.write() {
            Ok(g) => g,
            Err(_) => return,
        };
        let SupabaseLoginPhase::EnterOtp { email, otp, error } = &mut guard.phase else {
            return;
        };
        match key_event.code {
            KeyCode::Char(c) if c.is_ascii_alphanumeric() => {
                otp.push(c);
                *error = None;
            }
            KeyCode::Backspace => {
                otp.pop();
            }
            KeyCode::Enter => {
                let trimmed = otp.trim().to_string();
                if trimmed.is_empty() {
                    *error = Some("Enter the OTP from your email.".to_string());
                    return;
                }
                let email_for_call = email.clone();
                guard.phase = SupabaseLoginPhase::VerifyingOtp {
                    email: email_for_call.clone(),
                };
                drop(guard);
                self.app_event_tx.send(AppEvent::SupabaseVerifyOtp {
                    email: email_for_call,
                    otp: trimmed,
                });
            }
            _ => {}
        }
    }

    fn current_phase(&self) -> SupabaseLoginPhase {
        self.state
            .read()
            .ok()
            .map(|g| g.phase.clone())
            .unwrap_or(SupabaseLoginPhase::Cancelled)
    }
}

impl BottomPaneView for SupabaseLoginView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if matches!(key_event.code, KeyCode::Esc) {
            self.on_ctrl_c();
            return;
        }
        match self.current_phase() {
            SupabaseLoginPhase::EnterEmail { .. } => self.handle_key_in_email(key_event),
            SupabaseLoginPhase::EnterOtp { .. } => self.handle_key_in_otp(key_event),
            SupabaseLoginPhase::Success { .. } => {
                // Any key dismisses the success screen.
                self.completion = Some(ViewCompletion::Accepted);
            }
            SupabaseLoginPhase::SendingOtp { .. } | SupabaseLoginPhase::VerifyingOtp { .. } => {
                // Network calls in flight — swallow input apart from Esc.
            }
            SupabaseLoginPhase::Cancelled => {
                self.completion = Some(ViewCompletion::Cancelled);
            }
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if let Ok(mut guard) = self.state.write() {
            guard.phase = SupabaseLoginPhase::Cancelled;
        }
        self.completion = Some(ViewCompletion::Cancelled);
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        if matches!(self.current_phase(), SupabaseLoginPhase::Cancelled) {
            return true;
        }
        self.completion.is_some()
    }

    fn completion(&self) -> Option<ViewCompletion> {
        self.completion
    }
}

impl Renderable for SupabaseLoginView {
    fn desired_height(&self, _width: u16) -> u16 {
        // Title + body lines (max ~4) + blank + hint
        7
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let phase = self.current_phase();
        let title = "Sign in to ATA (Supabase)".to_string();
        let title_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        Paragraph::new(Line::from(vec![gutter(), title.bold()])).render(title_area, buf);

        let body_y = area.y.saturating_add(1);
        let body_lines = render_body_lines(&phase);
        let mut y = body_y;
        for line in body_lines {
            if y >= area.y.saturating_add(area.height) {
                break;
            }
            Paragraph::new(line).render(
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            y = y.saturating_add(1);
        }

        let hint_y = area.y.saturating_add(area.height.saturating_sub(1));
        if hint_y > body_y {
            Paragraph::new(standard_popup_hint_line()).render(
                Rect {
                    x: area.x,
                    y: hint_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }

    fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
        None
    }
}

fn render_body_lines(phase: &SupabaseLoginPhase) -> Vec<Line<'static>> {
    match phase {
        SupabaseLoginPhase::EnterEmail { email, error } => {
            let mut lines = vec![
                Line::from(vec![
                    gutter(),
                    "Email: ".dim(),
                    email.clone().into(),
                    "▌".cyan(),
                ]),
                Line::from(vec![
                    gutter(),
                    "Enter sends a one-time code to your inbox.".dim(),
                ]),
            ];
            if let Some(err) = error {
                lines.push(Line::from(vec![gutter(), err.clone().red()]));
            }
            lines
        }
        SupabaseLoginPhase::SendingOtp { email } => vec![Line::from(vec![
            gutter(),
            "Sending OTP to ".dim(),
            email.clone().into(),
            "…".dim(),
        ])],
        SupabaseLoginPhase::EnterOtp { email, otp, error } => {
            let mut lines = vec![
                Line::from(vec![
                    gutter(),
                    "Code from ".dim(),
                    email.clone().into(),
                    ": ".dim(),
                    otp.clone().into(),
                    "▌".cyan(),
                ]),
                Line::from(vec![
                    gutter(),
                    "Enter verifies. Check your inbox for the 6-digit code.".dim(),
                ]),
            ];
            if let Some(err) = error {
                lines.push(Line::from(vec![gutter(), err.clone().red()]));
            }
            lines
        }
        SupabaseLoginPhase::VerifyingOtp { email } => vec![Line::from(vec![
            gutter(),
            "Verifying code for ".dim(),
            email.clone().into(),
            "…".dim(),
        ])],
        SupabaseLoginPhase::Success { email } => vec![
            Line::from(vec![gutter(), "Signed in as ".dim(), email.clone().green()]),
            Line::from(vec![gutter(), "Press any key to close.".dim()]),
        ],
        SupabaseLoginPhase::Cancelled => {
            vec![Line::from(vec![gutter(), "Sign-in cancelled.".dim()])]
        }
    }
}

fn gutter() -> Span<'static> {
    "▌ ".cyan()
}
