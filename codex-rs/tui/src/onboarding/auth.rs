//! Authentication step UI and state transitions used by onboarding.
//!
//! This module owns the auth-step state machine (ChatGPT login/device-code/API
//! key), renders the corresponding UI, and handles auth-scoped keyboard input.
//! It intentionally does not decide onboarding flow completion; the enclosing
//! onboarding screen coordinates step progression.

#![allow(clippy::unwrap_used)]

use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::AccountLoginCompletedNotification;
use codex_app_server_protocol::AccountUpdatedNotification;
use codex_app_server_protocol::AuthMode as AppServerAuthMode;
use codex_app_server_protocol::CancelLoginAccountParams;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::LoginAccountParams;
use codex_app_server_protocol::LoginAccountResponse;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use codex_protocol::config_types::ForcedLoginMethod;
use std::cell::Cell;
use std::sync::Arc;
use std::sync::RwLock;
use uuid::Uuid;

use crate::LoginStatus;
use crate::key_hint::KeyBinding;
use crate::key_hint::KeyBindingListExt;
use crate::motion::MotionMode;
use crate::motion::shimmer_text;
use crate::onboarding::keys;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::tui::FrameRequester;

/// Marks buffer cells that have cyan+underlined style as an OSC 8 hyperlink.
///
/// Terminal emulators recognise the OSC 8 escape sequence and treat the entire
/// marked region as a single clickable link, regardless of row wrapping.  This
/// is necessary because ratatui's cell-based rendering emits `MoveTo` at every
/// row boundary, which breaks normal terminal URL detection for long URLs that
/// wrap across multiple rows.
pub(crate) fn mark_url_hyperlink(buf: &mut Buffer, area: Rect, url: &str) {
    mark_hyperlink_cells(buf, area, url, |cell| {
        cell.fg == Color::Cyan && cell.modifier.contains(Modifier::UNDERLINED)
    });
}

/// Marks any underlined buffer cells as an OSC 8 hyperlink.
pub(crate) fn mark_underlined_hyperlink(buf: &mut Buffer, area: Rect, url: &str) {
    mark_hyperlink_cells(buf, area, url, |cell| {
        cell.modifier.contains(Modifier::UNDERLINED)
    });
}

fn mark_hyperlink_cells(
    buf: &mut Buffer,
    area: Rect,
    url: &str,
    should_mark: impl Fn(&ratatui::buffer::Cell) -> bool,
) {
    // Sanitize: strip any characters that could break out of the OSC 8
    // sequence (ESC or BEL) to prevent terminal escape injection from a
    // malformed or compromised upstream URL.
    let safe_url: String = url
        .chars()
        .filter(|&c| c != '\x1B' && c != '\x07')
        .collect();
    if safe_url.is_empty() {
        return;
    }

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            if !should_mark(cell) {
                continue;
            }
            let sym = cell.symbol().to_string();
            if sym.trim().is_empty() {
                continue;
            }
            cell.set_symbol(&format!("\x1B]8;;{safe_url}\x07{sym}\x1B]8;;\x07"));
        }
    }
}

use super::onboarding_screen::StepState;

mod copilot_login;
mod headless_chatgpt_login;

pub(crate) use super::provider_picker::ProviderOption;
pub(crate) use copilot_login::start_copilot_login;

#[derive(Clone)]
pub(crate) enum SignInState {
    PickMode,
    ChatGptContinueInBrowser(ContinueInBrowserState),
    #[allow(dead_code)]
    ChatGptDeviceCode(ContinueWithDeviceCodeState),
    ChatGptSuccessMessage,
    ChatGptSuccess,
    /// GitHub Copilot OAuth device-code flow. Reuses
    /// `ContinueWithDeviceCodeState` since the UX is identical to ChatGPT
    /// device code; differentiation is by enum variant so completion
    /// notifications (matched by `login_id`) can route to the right path.
    CopilotDeviceCode(ContinueWithDeviceCodeState),
    /// Terminal "you're signed in" screen for the Copilot flow, mirroring
    /// `ChatGptSuccessMessage`/`ChatGptSuccess`.
    CopilotSuccessMessage,
    CopilotSuccess,
    ApiKeyEntry(ApiKeyInputState),
    ApiKeyConfigured,
    // === ATA: provider-picker + email-OTP states ===
    // Appended at the end of the enum so future upstream additions land
    // ahead of our overlay states and merges stay clean.
    /// "Configure model providers" sub-screen (OpenAI / Anthropic / Gemini /
    /// GitHub Copilot). Highlights `AuthModeWidget::highlighted_provider`.
    PickProvider,
    /// Per-provider auth method picker. Used only by providers that support
    /// both API key and OAuth (currently Gemini).
    PickProviderAuthMethod(ProviderOption),
    /// "Configured providers" view. Wired in the key handler but never
    /// pushed by the picker yet — kept in the state machine so the
    /// L-shortcut can be re-enabled without an enum change.
    #[allow(dead_code)]
    ProviderList,
    /// Browser-based OAuth login in progress (Gemini Code Assist). Mirrors
    /// `ChatGptContinueInBrowser` but scoped to a per-provider flow.
    ProviderOauthContinueInBrowser(ProviderOauthContinueInBrowserState),
    /// Provider OAuth completed — terminal success screen.
    ProviderOauthSuccessMessage(ProviderOption),
    ProviderConfigured,
    /// "Sign in with ATA account" email step.
    AtaEmailInput(AtaEmailInputState),
    AtaSendingOtp {
        email: String,
    },
    AtaOtpInput(AtaOtpInputState),
    AtaVerifyingOtp {
        _email: String,
    },
    AtaSuccessMessage,
    AtaSuccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignInOption {
    ChatGpt,
    DeviceCode,
    // === ATA: top-level picker entries that replace the legacy Copilot/ApiKey
    // direct options — the same flows still exist but are reached via the
    // provider sub-picker (Copilot, OpenAI API key) below. ===
    ConfigureProviders,
    AtaAccount,
}

const API_KEY_DISABLED_MESSAGE: &str = "API key login is disabled.";
pub(super) fn onboarding_request_id() -> codex_app_server_protocol::RequestId {
    codex_app_server_protocol::RequestId::String(Uuid::new_v4().to_string())
}

pub(super) async fn cancel_login_attempt(
    request_handle: &AppServerRequestHandle,
    login_id: String,
) {
    let _ = request_handle
        .request_typed::<codex_app_server_protocol::CancelLoginAccountResponse>(
            ClientRequest::CancelLoginAccount {
                request_id: onboarding_request_id(),
                params: CancelLoginAccountParams { login_id },
            },
        )
        .await;
}

#[derive(Clone, Default)]
pub(crate) struct ApiKeyInputState {
    value: String,
    prepopulated_from_env: bool,
    /// Which provider this API key is being saved for. `None` preserves the
    /// historic "top-level Provide your own API key" path that always wrote
    /// to OpenAI. The provider picker sets this to a concrete provider.
    provider: Option<ProviderOption>,
}

#[derive(Clone, Default)]
pub(crate) struct AtaEmailInputState {
    pub email: String,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct AtaOtpInputState {
    pub email: String,
    pub otp: String,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ProviderOauthContinueInBrowserState {
    provider: ProviderOption,
    auth_url: String,
    login_id: Option<String>,
}

impl ProviderOauthContinueInBrowserState {
    pub(super) fn new(
        provider: ProviderOption,
        auth_url: String,
        login_id: Option<String>,
    ) -> Self {
        Self {
            provider,
            auth_url,
            login_id,
        }
    }

    pub(crate) fn provider(&self) -> ProviderOption {
        self.provider
    }

    pub(super) fn auth_url(&self) -> &str {
        self.auth_url.as_str()
    }

    pub(crate) fn login_id(&self) -> Option<&str> {
        self.login_id.as_deref()
    }
}

impl ApiKeyInputState {
    pub(super) fn new(
        value: String,
        prepopulated_from_env: bool,
        provider: Option<ProviderOption>,
    ) -> Self {
        Self {
            value,
            prepopulated_from_env,
            provider,
        }
    }

    pub(crate) fn provider(&self) -> Option<ProviderOption> {
        self.provider
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderAuthMethod {
    ApiKey,
    Oauth,
}

#[derive(Clone)]
/// Used to manage the lifecycle of SpawnedLogin and ensure it gets cleaned up.
pub(crate) struct ContinueInBrowserState {
    login_id: String,
    auth_url: String,
}

#[derive(Clone)]
pub(crate) struct ContinueWithDeviceCodeState {
    request_id: String,
    login_id: Option<String>,
    verification_url: Option<String>,
    user_code: Option<String>,
}

impl ContinueWithDeviceCodeState {
    pub(crate) fn pending(request_id: String) -> Self {
        Self {
            request_id,
            login_id: None,
            verification_url: None,
            user_code: None,
        }
    }

    pub(crate) fn ready(
        request_id: String,
        login_id: String,
        verification_url: String,
        user_code: String,
    ) -> Self {
        Self {
            request_id,
            login_id: Some(login_id),
            verification_url: Some(verification_url),
            user_code: Some(user_code),
        }
    }

    pub(crate) fn login_id(&self) -> Option<&str> {
        self.login_id.as_deref()
    }

    pub(crate) fn is_showing_copyable_auth(&self) -> bool {
        self.verification_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
            && self
                .user_code
                .as_deref()
                .is_some_and(|user_code| !user_code.is_empty())
    }
}

impl KeyboardHandler for AuthModeWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.handle_api_key_entry_key_event(&key_event) {
            return;
        }
        // === ATA: dispatch into provider sub-picker / email-OTP first ===
        {
            let sign_in_state = { (*self.sign_in_state.read().unwrap()).clone() };
            if self.handle_provider_picker_key_event(&key_event, &sign_in_state) {
                return;
            }
        }
        if self.handle_ata_input_key_event(&key_event) {
            return;
        }

        if keys::MOVE_UP.is_pressed(key_event) {
            self.move_highlight(/*delta*/ -1);
            return;
        }
        if keys::MOVE_DOWN.is_pressed(key_event) {
            self.move_highlight(/*delta*/ 1);
            return;
        }
        if keys::SELECT_FIRST.is_pressed(key_event) {
            self.select_option_by_index(/*index*/ 0);
            return;
        }
        if keys::SELECT_SECOND.is_pressed(key_event) {
            self.select_option_by_index(/*index*/ 1);
            return;
        }
        if keys::SELECT_THIRD.is_pressed(key_event) {
            self.select_option_by_index(/*index*/ 2);
            return;
        }
        if keys::CONFIRM.is_pressed(key_event) {
            let sign_in_state = { (*self.sign_in_state.read().unwrap()).clone() };
            match sign_in_state {
                SignInState::PickMode => {
                    self.handle_sign_in_option(self.highlighted_mode);
                }
                SignInState::ChatGptSuccessMessage => {
                    *self.sign_in_state.write().unwrap() = SignInState::ChatGptSuccess;
                }
                SignInState::CopilotSuccessMessage => {
                    *self.sign_in_state.write().unwrap() = SignInState::CopilotSuccess;
                }
                // === ATA: success transitions ===
                SignInState::AtaSuccessMessage => {
                    *self.sign_in_state.write().unwrap() = SignInState::AtaSuccess;
                }
                _ => {}
            }
            return;
        }
        if keys::CANCEL.is_pressed(key_event) {
            tracing::info!("Cancel onboarding auth step");
            self.cancel_active_attempt();
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        let _ = self.handle_api_key_entry_paste(pasted);
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct AuthModeWidget {
    pub request_frame: FrameRequester,
    pub highlighted_mode: SignInOption,
    pub error: Arc<RwLock<Option<String>>>,
    pub sign_in_state: Arc<RwLock<SignInState>>,
    pub login_status: LoginStatus,
    pub app_server_request_handle: AppServerRequestHandle,
    pub forced_login_method: Option<ForcedLoginMethod>,
    pub animations_enabled: bool,
    pub animations_suppressed: Cell<bool>,
    // === ATA: provider-picker + ATA-account state ===
    /// Highlighted row in `SignInState::PickProvider`.
    pub highlighted_provider: ProviderOption,
    /// Highlighted row in `SignInState::PickProviderAuthMethod`.
    pub highlighted_provider_auth_method: ProviderAuthMethod,
}

impl AuthModeWidget {
    // === ATA: helpers used by provider_picker.rs ===
    pub(super) fn set_error_message(&self, message: Option<String>) {
        *self.error.write().unwrap() = message;
    }

    pub(super) fn error_message_snapshot(&self) -> Option<String> {
        self.error.read().ok().and_then(|guard| guard.clone())
    }

    pub(super) fn error_arc(&self) -> Arc<RwLock<Option<String>>> {
        self.error.clone()
    }

    pub(crate) fn set_animations_suppressed(&self, suppressed: bool) {
        self.animations_suppressed.set(suppressed);
    }

    pub(crate) fn should_suppress_animations(&self) -> bool {
        matches!(
            &*self.sign_in_state.read().unwrap(),
            SignInState::ChatGptContinueInBrowser(_)
                | SignInState::ChatGptDeviceCode(_)
                | SignInState::CopilotDeviceCode(_)
        )
    }

    pub(crate) fn cancel_active_attempt(&self) {
        let mut sign_in_state = self.sign_in_state.write().unwrap();
        match &*sign_in_state {
            SignInState::ChatGptContinueInBrowser(state) => {
                let request_handle = self.app_server_request_handle.clone();
                let login_id = state.login_id.clone();
                tokio::spawn(async move {
                    cancel_login_attempt(&request_handle, login_id).await;
                });
            }
            SignInState::ChatGptDeviceCode(state) | SignInState::CopilotDeviceCode(state) => {
                if let Some(login_id) = state.login_id().map(str::to_owned) {
                    let request_handle = self.app_server_request_handle.clone();
                    tokio::spawn(async move {
                        cancel_login_attempt(&request_handle, login_id).await;
                    });
                }
            }
            SignInState::ProviderOauthContinueInBrowser(state) => {
                if let Some(login_id) = state.login_id().map(str::to_owned) {
                    let request_handle = self.app_server_request_handle.clone();
                    tokio::spawn(async move {
                        cancel_login_attempt(&request_handle, login_id).await;
                    });
                }
            }
            _ => return,
        }
        *sign_in_state = SignInState::PickMode;
        drop(sign_in_state);
        self.set_error(/*message*/ None);
        self.request_frame.schedule_frame();
    }

    fn set_error(&self, message: Option<String>) {
        *self.error.write().unwrap() = message;
    }

    fn error_message(&self) -> Option<String> {
        self.error.read().unwrap().clone()
    }

    /// Returns whether the auth flow is currently in API-key entry mode.
    pub(crate) fn is_api_key_entry_active(&self) -> bool {
        self.sign_in_state
            .read()
            .is_ok_and(|guard| matches!(&*guard, SignInState::ApiKeyEntry(_)))
    }

    /// Returns whether the API-key entry field currently contains any text.
    pub(crate) fn api_key_entry_has_text(&self) -> bool {
        self.sign_in_state.read().is_ok_and(
            |guard| matches!(&*guard, SignInState::ApiKeyEntry(state) if !state.value.is_empty()),
        )
    }

    fn confirm_binding(&self) -> KeyBinding {
        keys::CONFIRM[0]
    }

    fn cancel_binding(&self) -> KeyBinding {
        keys::CANCEL[0]
    }

    fn is_api_login_allowed(&self) -> bool {
        !matches!(self.forced_login_method, Some(ForcedLoginMethod::Chatgpt))
    }

    fn is_chatgpt_login_allowed(&self) -> bool {
        !matches!(self.forced_login_method, Some(ForcedLoginMethod::Api))
    }

    fn displayed_sign_in_options(&self) -> Vec<SignInOption> {
        let mut options = vec![SignInOption::ChatGpt];
        if self.is_chatgpt_login_allowed() {
            options.push(SignInOption::DeviceCode);
        }
        if self.is_api_login_allowed() {
            options.push(SignInOption::ConfigureProviders);
        }
        options.push(SignInOption::AtaAccount);
        options
    }

    fn selectable_sign_in_options(&self) -> Vec<SignInOption> {
        let mut options = Vec::new();
        if self.is_chatgpt_login_allowed() {
            options.push(SignInOption::ChatGpt);
            options.push(SignInOption::DeviceCode);
        }
        if self.is_api_login_allowed() {
            options.push(SignInOption::ConfigureProviders);
        }
        options.push(SignInOption::AtaAccount);
        options
    }

    fn move_highlight(&mut self, delta: isize) {
        let options = self.selectable_sign_in_options();
        if options.is_empty() {
            return;
        }

        let current_index = options
            .iter()
            .position(|option| *option == self.highlighted_mode)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_mode = options[next_index];
    }

    fn select_option_by_index(&mut self, index: usize) {
        let options = self.displayed_sign_in_options();
        if let Some(option) = options.get(index).copied() {
            self.handle_sign_in_option(option);
        }
    }

    fn handle_sign_in_option(&mut self, option: SignInOption) {
        match option {
            SignInOption::ChatGpt => {
                if self.is_chatgpt_login_allowed() {
                    self.start_chatgpt_login();
                }
            }
            SignInOption::DeviceCode => {
                if self.is_chatgpt_login_allowed() {
                    self.start_device_code_login();
                }
            }
            SignInOption::ConfigureProviders => {
                if self.is_api_login_allowed() {
                    self.start_provider_selection();
                } else {
                    self.disallow_api_login();
                }
            }
            SignInOption::AtaAccount => {
                self.start_ata_email_input();
            }
        }
    }

    fn disallow_api_login(&mut self) {
        self.highlighted_mode = SignInOption::ChatGpt;
        self.set_error(Some(API_KEY_DISABLED_MESSAGE.to_string()));
        *self.sign_in_state.write().unwrap() = SignInState::PickMode;
        self.request_frame.schedule_frame();
    }

    fn render_pick_mode(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                "  ".into(),
                "Sign in with ChatGPT to use Codex as part of your paid plan".into(),
            ]),
            Line::from(vec![
                "  ".into(),
                "or connect an API key for usage-based billing".into(),
            ]),
            "".into(),
        ];

        let create_mode_item = |idx: usize,
                                selected_mode: SignInOption,
                                text: &str,
                                description: &str|
         -> Vec<Line<'static>> {
            let is_selected = self.highlighted_mode == selected_mode;
            let caret = if is_selected { ">" } else { " " };

            let line1 = if is_selected {
                Line::from(vec![
                    format!("{caret} {index}. ", index = idx + 1).cyan().dim(),
                    text.to_string().cyan(),
                ])
            } else {
                format!("  {index}. {text}", index = idx + 1).into()
            };

            let line2 = if is_selected {
                Line::from(format!("     {description}"))
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::DIM)
            } else {
                Line::from(format!("     {description}"))
                    .style(Style::default().add_modifier(Modifier::DIM))
            };

            vec![line1, line2]
        };

        let chatgpt_description = if !self.is_chatgpt_login_allowed() {
            "ChatGPT login is disabled"
        } else {
            "Usage included with Plus, Pro, Business, and Enterprise plans"
        };
        let device_code_description = "Sign in from another device with a one-time code";

        for (idx, option) in self.displayed_sign_in_options().into_iter().enumerate() {
            match option {
                SignInOption::ChatGpt => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with ChatGPT",
                        chatgpt_description,
                    ));
                }
                SignInOption::DeviceCode => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with Device Code",
                        device_code_description,
                    ));
                }
                SignInOption::ConfigureProviders => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Configure model providers",
                        "Set up API keys for different providers",
                    ));
                }
                SignInOption::AtaAccount => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Sign in with ATA account",
                        "Use your ATA account credentials",
                    ));
                }
            }
            lines.push("".into());
        }

        if !self.is_api_login_allowed() {
            lines.push(
                "  API key login is disabled by this workspace. Sign in with ChatGPT to continue."
                    .dim()
                    .into(),
            );
            lines.push("".into());
        }
        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.confirm_binding().into(),
            " to continue".dim(),
        ]));
        if let Some(err) = self.error_message() {
            lines.push("".into());
            lines.push(err.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_continue_in_browser(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = vec!["  ".into()];
        if self.animations_enabled && !self.animations_suppressed.get() {
            // Schedule a follow-up frame to keep the shimmer animation going.
            self.request_frame
                .schedule_frame_in(std::time::Duration::from_millis(100));
            spans.extend(shimmer_text(
                "Finish signing in via your browser",
                MotionMode::Animated,
            ));
        } else {
            spans.push("Finish signing in via your browser".into());
        }
        let mut lines = vec![spans.into(), "".into()];

        let sign_in_state = self.sign_in_state.read().unwrap();
        let auth_url = if let SignInState::ChatGptContinueInBrowser(state) = &*sign_in_state
            && !state.auth_url.is_empty()
        {
            lines.push("  If the link doesn't open automatically, open the following link to authenticate:".into());
            lines.push("".into());
            lines.push(Line::from(vec![
                "  ".into(),
                state.auth_url.as_str().cyan().underlined(),
            ]));
            lines.push("".into());
            lines.push(Line::from(vec![
                "  On a remote or headless machine? Press ".into(),
                self.cancel_binding().into(),
                " and choose ".into(),
                "Sign in with Device Code".cyan(),
                ".".into(),
            ]));
            lines.push("".into());
            Some(state.auth_url.clone())
        } else {
            None
        };

        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.cancel_binding().into(),
            " to cancel".dim(),
        ]));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);

        // Wrap cyan+underlined URL cells with OSC 8 so the terminal treats
        // the entire region as a single clickable hyperlink.
        if let Some(url) = &auth_url {
            mark_url_hyperlink(buf, area, url);
        }
    }

    fn render_chatgpt_success_message(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Signed in with your ChatGPT account".fg(Color::Green).into(),
            "".into(),
            "  Before you start:".into(),
            "".into(),
            "  Decide how much autonomy you want to grant Codex".into(),
            Line::from(vec![
                "  For more details see the ".into(),
                "\u{1b}]8;;https://developers.openai.com/codex/security\u{7}Codex docs\u{1b}]8;;\u{7}".underlined(),
            ])
            .dim(),
            "".into(),
            "  Codex can make mistakes".into(),
            "  Review the code it writes and commands it runs".dim().into(),
            "".into(),
            "  Powered by your ChatGPT account".into(),
            Line::from(vec![
                "  Uses your plan's rate limits and ".into(),
                "\u{1b}]8;;https://chatgpt.com/#settings\u{7}training data preferences\u{1b}]8;;\u{7}".underlined(),
            ])
            .dim(),
            "".into(),
            Line::from(vec![
                "  Press ".fg(Color::Cyan),
                self.confirm_binding().into(),
                " to continue".fg(Color::Cyan),
            ]),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_chatgpt_success(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Signed in with your ChatGPT account"
                .fg(Color::Green)
                .into(),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_copilot_success_message(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Signed in with GitHub Copilot".fg(Color::Green).into(),
            "".into(),
            "  Before you start:".into(),
            "".into(),
            "  Decide how much autonomy you want to grant Codex".into(),
            "".into(),
            "  Codex can make mistakes".into(),
            "  Review the code it writes and commands it runs"
                .dim()
                .into(),
            "".into(),
            "  Powered by your GitHub Copilot subscription".into(),
            "  Default model is gpt-4o; switch with `-c model=...`."
                .dim()
                .into(),
            "".into(),
            Line::from(vec![
                "  Press ".fg(Color::Cyan),
                self.confirm_binding().into(),
                " to continue".fg(Color::Cyan),
            ]),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_copilot_success(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec!["✓ Signed in with GitHub Copilot".fg(Color::Green).into()];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_api_key_configured(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ API key configured".fg(Color::Green).into(),
            "".into(),
            "  Codex will use usage-based billing with your API key.".into(),
        ];

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_api_key_entry(&self, area: Rect, buf: &mut Buffer, state: &ApiKeyInputState) {
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Min(4),
            Constraint::Length(3),
            Constraint::Min(2),
        ])
        .areas(area);

        let (display_name, env_var) = match state.provider() {
            Some(provider) => (provider.display_name(), provider.env_var()),
            None => ("OpenAI", Some("OPENAI_API_KEY")),
        };
        let mut intro_lines: Vec<Line> = vec![
            Line::from(vec![
                "> ".into(),
                format!("Use your own {display_name} API key for usage-based billing").bold(),
            ]),
            "".into(),
            "  Paste or type your API key below. It will be stored locally in auth.json.".into(),
            "".into(),
        ];
        if state.prepopulated_from_env {
            if let Some(env_var) = env_var {
                intro_lines.push(format!("  Detected {env_var} environment variable.").into());
            }
            intro_lines.push(
                "  Paste a different key if you prefer to use another account."
                    .dim()
                    .into(),
            );
            intro_lines.push("".into());
        }
        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        let content_line: Line = if state.value.is_empty() {
            vec!["Paste or type your API key".dim()].into()
        } else {
            Line::from(state.value.clone())
        };
        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title("API key")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .render(input_area, buf);

        let mut footer_lines: Vec<Line> = vec![
            Line::from(vec![
                "  Press ".dim(),
                self.confirm_binding().into(),
                " to save".dim(),
            ]),
            Line::from(vec![
                "  Press ".dim(),
                self.cancel_binding().into(),
                " to go back".dim(),
            ]),
        ];
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn handle_api_key_entry_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut should_save: Option<String> = None;
        let mut should_request_frame = false;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            if let SignInState::ApiKeyEntry(state) = &mut *guard {
                if keys::CANCEL.is_pressed(*key_event) {
                    *guard = SignInState::PickMode;
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if keys::CONFIRM.is_pressed(*key_event) {
                    let trimmed = state.value.trim().to_string();
                    if trimmed.is_empty() {
                        self.set_error(Some("API key cannot be empty".to_string()));
                        should_request_frame = true;
                    } else {
                        should_save = Some(trimmed);
                    }
                } else {
                    match key_event.code {
                        KeyCode::Backspace => {
                            if state.prepopulated_from_env {
                                state.value.clear();
                                state.prepopulated_from_env = false;
                            } else {
                                state.value.pop();
                            }
                            self.set_error(/*message*/ None);
                            should_request_frame = true;
                        }
                        KeyCode::Char(c)
                            if key_event.kind == KeyEventKind::Press
                                && !key_event.modifiers.contains(KeyModifiers::SUPER)
                                && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                                && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            if state.prepopulated_from_env {
                                state.value.clear();
                                state.prepopulated_from_env = false;
                            }
                            state.value.push(c);
                            self.set_error(/*message*/ None);
                            should_request_frame = true;
                        }
                        _ => {}
                    }
                }
                // handled; let guard drop before potential save
            } else {
                return false;
            }
        }

        if let Some(api_key) = should_save {
            self.save_api_key(api_key);
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    fn handle_api_key_entry_paste(&mut self, pasted: String) -> bool {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        if let SignInState::ApiKeyEntry(state) = &mut *guard {
            if state.prepopulated_from_env {
                state.value = trimmed.to_string();
                state.prepopulated_from_env = false;
            } else {
                state.value.push_str(trimmed);
            }
            self.set_error(/*message*/ None);
        } else {
            return false;
        }

        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn save_api_key(&mut self, api_key: String) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }
        self.set_error(/*message*/ None);
        // Pull the provider (if any) out of the active state so the spawned
        // task can route to the right `LoginAccountParams` variant: a
        // top-level "Provide your own API key" submit lands as the
        // legacy `ApiKey` variant (OpenAI-only); a submit from inside
        // "Configure model providers" carries the provider id and uses
        // `ProviderApiKey { provider_id, api_key }`.
        let provider = match &*self.sign_in_state.read().unwrap() {
            SignInState::ApiKeyEntry(state) => state.provider(),
            _ => None,
        };
        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let params = match provider {
                Some(p) => LoginAccountParams::ProviderApiKey {
                    provider_id: p.provider_id().to_string(),
                    api_key: api_key.clone(),
                },
                None => LoginAccountParams::ApiKey {
                    api_key: api_key.clone(),
                },
            };
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params,
                })
                .await
            {
                Ok(LoginAccountResponse::ApiKey {})
                | Ok(LoginAccountResponse::ProviderApiKey {}) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() = if provider.is_some() {
                        SignInState::ProviderConfigured
                    } else {
                        SignInState::ApiKeyConfigured
                    };
                }
                Ok(other) => {
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                    *sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState {
                        value: api_key,
                        prepopulated_from_env: false,
                        provider,
                    });
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!("Failed to save API key: {err}"));
                    *sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState {
                        value: api_key,
                        prepopulated_from_env: false,
                        provider,
                    });
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }

    fn handle_existing_chatgpt_login(&mut self) -> bool {
        if matches!(
            self.login_status,
            LoginStatus::AuthMode(AppServerAuthMode::Chatgpt)
                | LoginStatus::AuthMode(AppServerAuthMode::ChatgptAuthTokens)
        ) {
            *self.sign_in_state.write().unwrap() = SignInState::ChatGptSuccess;
            self.request_frame.schedule_frame();
            true
        } else {
            false
        }
    }

    /// Kicks off the ChatGPT auth flow and keeps the UI state consistent with the attempt.
    fn start_chatgpt_login(&mut self) {
        // If we're already authenticated with ChatGPT, don't start a new login –
        // just proceed to the success message flow.
        if self.handle_existing_chatgpt_login() {
            return;
        }

        self.set_error(/*message*/ None);
        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::Chatgpt {
                        codex_streamlined_login: false,
                    },
                })
                .await
            {
                Ok(LoginAccountResponse::Chatgpt { login_id, auth_url }) => {
                    maybe_open_auth_url_in_browser(&request_handle, &auth_url);
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() =
                        SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
                            login_id,
                            auth_url,
                        });
                }
                Ok(other) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                }
                Err(err) => {
                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                    *error.write().unwrap() = Some(err.to_string());
                }
            }
            request_frame.schedule_frame();
        });
    }

    fn start_device_code_login(&mut self) {
        if self.handle_existing_chatgpt_login() {
            return;
        }

        self.set_error(/*message*/ None);
        headless_chatgpt_login::start_headless_chatgpt_login(self);
    }

    pub(crate) fn on_account_login_completed(
        &mut self,
        notification: AccountLoginCompletedNotification,
    ) {
        let Some(login_id) = notification.login_id else {
            return;
        };
        let guard = self.sign_in_state.read().unwrap();
        let is_matching_chatgpt_login = matches!(
            &*guard,
            SignInState::ChatGptContinueInBrowser(state) if state.login_id == login_id
        ) || matches!(
            &*guard,
            SignInState::ChatGptDeviceCode(state) if state.login_id() == Some(login_id.as_str())
        );
        let is_matching_copilot_login = matches!(
            &*guard,
            SignInState::CopilotDeviceCode(state) if state.login_id() == Some(login_id.as_str())
        );
        // Per-provider OAuth (Gemini Code Assist today): completion fires
        // with the same `login_id` issued by `GeminiOauthContinueInBrowser`.
        let provider_oauth_match: Option<ProviderOption> = match &*guard {
            SignInState::ProviderOauthContinueInBrowser(state)
                if state.login_id() == Some(login_id.as_str()) =>
            {
                Some(state.provider())
            }
            _ => None,
        };
        drop(guard);
        if !is_matching_chatgpt_login
            && !is_matching_copilot_login
            && provider_oauth_match.is_none()
        {
            return;
        }

        if notification.success {
            self.set_error(/*message*/ None);
            *self.sign_in_state.write().unwrap() = if let Some(provider) = provider_oauth_match {
                SignInState::ProviderOauthSuccessMessage(provider)
            } else if is_matching_copilot_login {
                SignInState::CopilotSuccessMessage
            } else {
                SignInState::ChatGptSuccessMessage
            };
        } else {
            self.set_error(notification.error);
            *self.sign_in_state.write().unwrap() = if provider_oauth_match.is_some() {
                SignInState::PickProvider
            } else {
                SignInState::PickMode
            };
        }
        self.request_frame.schedule_frame();
    }

    pub(crate) fn on_account_updated(&mut self, notification: AccountUpdatedNotification) {
        self.login_status = notification
            .auth_mode
            .map(LoginStatus::AuthMode)
            .unwrap_or(LoginStatus::NotAuthenticated);
    }

    // === ATA: email-OTP sign-in flow ===

    pub(super) fn start_ata_email_input(&mut self) {
        self.set_error(/*message*/ None);
        *self.sign_in_state.write().unwrap() =
            SignInState::AtaEmailInput(AtaEmailInputState::default());
        self.request_frame.schedule_frame();
    }

    pub(super) fn handle_ata_input_key_event(&mut self, key_event: &KeyEvent) -> bool {
        // Returns true if the event was consumed.
        let mut action: AtaInputAction = AtaInputAction::None;
        {
            let mut guard = self.sign_in_state.write().unwrap();
            match &mut *guard {
                SignInState::AtaEmailInput(state) => match key_event.code {
                    KeyCode::Esc => {
                        action = AtaInputAction::BackToPickMode;
                    }
                    KeyCode::Enter => {
                        let trimmed = state.email.trim().to_string();
                        if trimmed.is_empty() {
                            state.error = Some("Email cannot be empty".to_string());
                            action = AtaInputAction::RequestFrame;
                        } else {
                            action = AtaInputAction::SubmitEmail(trimmed);
                        }
                    }
                    KeyCode::Backspace => {
                        state.email.pop();
                        state.error = None;
                        action = AtaInputAction::RequestFrame;
                    }
                    KeyCode::Char(c)
                        if key_event.kind == KeyEventKind::Press
                            && !key_event.modifiers.contains(KeyModifiers::SUPER)
                            && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        state.email.push(c);
                        state.error = None;
                        action = AtaInputAction::RequestFrame;
                    }
                    _ => {}
                },
                SignInState::AtaOtpInput(state) => match key_event.code {
                    KeyCode::Esc => {
                        action = AtaInputAction::BackToPickMode;
                    }
                    KeyCode::Enter => {
                        let otp = state.otp.trim().to_string();
                        if otp.is_empty() {
                            state.error = Some("Enter the 6-digit code".to_string());
                            action = AtaInputAction::RequestFrame;
                        } else {
                            action = AtaInputAction::SubmitOtp {
                                email: state.email.clone(),
                                otp,
                            };
                        }
                    }
                    KeyCode::Backspace => {
                        state.otp.pop();
                        state.error = None;
                        action = AtaInputAction::RequestFrame;
                    }
                    KeyCode::Char(c)
                        if c.is_ascii_digit()
                            && key_event.kind == KeyEventKind::Press
                            && state.otp.chars().count() < 6 =>
                    {
                        state.otp.push(c);
                        state.error = None;
                        action = AtaInputAction::RequestFrame;
                    }
                    _ => {}
                },
                _ => return false,
            }
        }

        match action {
            AtaInputAction::None => {}
            AtaInputAction::RequestFrame => {
                self.request_frame.schedule_frame();
            }
            AtaInputAction::BackToPickMode => {
                *self.sign_in_state.write().unwrap() = SignInState::PickMode;
                self.set_error(/*message*/ None);
                self.request_frame.schedule_frame();
            }
            AtaInputAction::SubmitEmail(email) => {
                self.spawn_ata_send_otp(email);
            }
            AtaInputAction::SubmitOtp { email, otp } => {
                self.spawn_ata_verify_otp(email, otp);
            }
        }
        true
    }

    fn spawn_ata_send_otp(&mut self, email: String) {
        *self.sign_in_state.write().unwrap() = SignInState::AtaSendingOtp {
            email: email.clone(),
        };
        self.request_frame.schedule_frame();

        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::AtaSendOtp {
                        email: email.clone(),
                    },
                })
                .await;
            match result {
                Ok(LoginAccountResponse::AtaSendOtp {}) => {
                    *sign_in_state.write().unwrap() = SignInState::AtaOtpInput(AtaOtpInputState {
                        email,
                        otp: String::new(),
                        error: None,
                    });
                }
                Ok(other) => {
                    *sign_in_state.write().unwrap() =
                        SignInState::AtaEmailInput(AtaEmailInputState {
                            email,
                            error: Some(format!("Unexpected response: {other:?}")),
                        });
                }
                Err(err) => {
                    *sign_in_state.write().unwrap() =
                        SignInState::AtaEmailInput(AtaEmailInputState {
                            email,
                            error: Some(err.to_string()),
                        });
                }
            }
            request_frame.schedule_frame();
        });
    }

    fn spawn_ata_verify_otp(&mut self, email: String, otp: String) {
        *self.sign_in_state.write().unwrap() = SignInState::AtaVerifyingOtp {
            _email: email.clone(),
        };
        self.request_frame.schedule_frame();

        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let result = request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::AtaVerifyOtp {
                        email: email.clone(),
                        otp: otp.clone(),
                    },
                })
                .await;
            match result {
                Ok(LoginAccountResponse::AtaVerifyOtp { email: _email }) => {
                    *sign_in_state.write().unwrap() = SignInState::AtaSuccessMessage;
                }
                Ok(other) => {
                    *sign_in_state.write().unwrap() = SignInState::AtaOtpInput(AtaOtpInputState {
                        email,
                        otp,
                        error: Some(format!("Unexpected response: {other:?}")),
                    });
                }
                Err(err) => {
                    *sign_in_state.write().unwrap() = SignInState::AtaOtpInput(AtaOtpInputState {
                        email,
                        otp,
                        error: Some(err.to_string()),
                    });
                }
            }
            request_frame.schedule_frame();
        });
    }

    fn render_ata_email_input(&self, area: Rect, buf: &mut Buffer, state: &AtaEmailInputState) {
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Sign in with ATA account".bold()]),
            "".into(),
            "  Enter your ATA account email; we'll send a one-time code.".into(),
            "".into(),
        ];
        let content: Line = if state.email.is_empty() {
            vec!["you@example.com".dim()].into()
        } else {
            Line::from(state.email.clone())
        };
        let block = Block::default()
            .title("Email")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));
        Paragraph::new(content)
            .block(block)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
        lines.push("".into());
        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.confirm_binding().into(),
            " to send a code".dim(),
        ]));
        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.cancel_binding().into(),
            " to go back".dim(),
        ]));
        if let Some(err) = &state.error {
            lines.push("".into());
            lines.push(err.clone().red().into());
        }
        // Footer below the input box. We rendered the input above; the
        // surrounding paragraph stacks under it because Paragraph::render
        // honors the full area — the duplicate render keeps the snapshot
        // simple without splitting into nested layouts.
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn render_ata_sending_otp(&self, area: Rect, buf: &mut Buffer, email: &str) {
        let lines = vec![
            "Sending one-time code…".into(),
            "".into(),
            format!("  to {email}").dim().into(),
        ];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn render_ata_otp_input(&self, area: Rect, buf: &mut Buffer, state: &AtaOtpInputState) {
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Sign in with ATA account".bold()]),
            "".into(),
            format!("  We sent a code to {}", state.email).into(),
            "".into(),
        ];
        let content: Line = if state.otp.is_empty() {
            vec!["6-digit code".dim()].into()
        } else {
            Line::from(state.otp.clone())
        };
        let block = Block::default()
            .title("OTP")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));
        Paragraph::new(content)
            .block(block)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
        lines.push("".into());
        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.confirm_binding().into(),
            " to verify".dim(),
        ]));
        lines.push(Line::from(vec![
            "  Press ".dim(),
            self.cancel_binding().into(),
            " to go back".dim(),
        ]));
        if let Some(err) = &state.error {
            lines.push("".into());
            lines.push(err.clone().red().into());
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn render_ata_verifying_otp(&self, area: Rect, buf: &mut Buffer, _email: &str) {
        let lines = vec!["Verifying one-time code…".into(), "".into()];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn render_ata_success_message(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Signed in to your ATA account".fg(Color::Green).into(),
            "".into(),
            Line::from(vec![
                "  Press ".fg(Color::Cyan),
                self.confirm_binding().into(),
                " to continue".fg(Color::Cyan),
            ]),
        ];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn render_ata_success(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec!["✓ Signed in to your ATA account".fg(Color::Green).into()];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }
}

// Internal action the ATA input key handler asks the outer method to perform
// after the state-lock is dropped.
enum AtaInputAction {
    None,
    RequestFrame,
    BackToPickMode,
    SubmitEmail(String),
    SubmitOtp { email: String, otp: String },
}

impl StepStateProvider for AuthModeWidget {
    fn get_step_state(&self) -> StepState {
        let sign_in_state = self.sign_in_state.read().unwrap();
        match &*sign_in_state {
            SignInState::PickMode
            | SignInState::ApiKeyEntry(_)
            | SignInState::ChatGptContinueInBrowser(_)
            | SignInState::ChatGptDeviceCode(_)
            | SignInState::ChatGptSuccessMessage
            | SignInState::CopilotDeviceCode(_)
            | SignInState::CopilotSuccessMessage
            // === ATA: in-progress states ===
            | SignInState::PickProvider
            | SignInState::PickProviderAuthMethod(_)
            | SignInState::ProviderList
            | SignInState::ProviderOauthContinueInBrowser(_)
            | SignInState::ProviderOauthSuccessMessage(_)
            | SignInState::AtaEmailInput(_)
            | SignInState::AtaSendingOtp { .. }
            | SignInState::AtaOtpInput(_)
            | SignInState::AtaVerifyingOtp { .. }
            | SignInState::AtaSuccessMessage => StepState::InProgress,
            SignInState::ChatGptSuccess
            | SignInState::CopilotSuccess
            | SignInState::ApiKeyConfigured
            // === ATA: terminal states ===
            | SignInState::ProviderConfigured
            | SignInState::AtaSuccess => StepState::Complete,
        }
    }
}

impl WidgetRef for AuthModeWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let sign_in_state = self.sign_in_state.read().unwrap();
        match &*sign_in_state {
            SignInState::PickMode => {
                self.render_pick_mode(area, buf);
            }
            SignInState::ChatGptContinueInBrowser(_) => {
                self.render_continue_in_browser(area, buf);
            }
            SignInState::ChatGptDeviceCode(state) => {
                headless_chatgpt_login::render_device_code_login(self, area, buf, state);
            }
            SignInState::ChatGptSuccessMessage => {
                self.render_chatgpt_success_message(area, buf);
            }
            SignInState::ChatGptSuccess => {
                self.render_chatgpt_success(area, buf);
            }
            SignInState::CopilotDeviceCode(state) => {
                headless_chatgpt_login::render_device_code_login(self, area, buf, state);
            }
            SignInState::CopilotSuccessMessage => {
                self.render_copilot_success_message(area, buf);
            }
            SignInState::CopilotSuccess => {
                self.render_copilot_success(area, buf);
            }
            SignInState::ApiKeyEntry(state) => {
                self.render_api_key_entry(area, buf, state);
            }
            // === ATA: provider-picker + ATA-account render arms ===
            SignInState::PickProvider => {
                self.render_pick_provider(area, buf);
            }
            SignInState::PickProviderAuthMethod(provider) => {
                self.render_pick_provider_auth_method(area, buf, *provider);
            }
            SignInState::ProviderList => {
                // Treat the L-shortcut "list configured providers" view as a
                // soft no-op for now — falling back to PickProvider rendering
                // keeps the state machine non-blocking.
                self.render_pick_provider(area, buf);
            }
            SignInState::ProviderOauthContinueInBrowser(state) => {
                self.render_provider_oauth_continue_in_browser(area, buf, state);
            }
            SignInState::ProviderOauthSuccessMessage(provider) => {
                self.render_provider_oauth_success_message(area, buf, *provider);
            }
            SignInState::ProviderConfigured => {
                self.render_provider_configured(area, buf);
            }
            SignInState::AtaEmailInput(state) => {
                self.render_ata_email_input(area, buf, state);
            }
            SignInState::AtaSendingOtp { email } => {
                self.render_ata_sending_otp(area, buf, email);
            }
            SignInState::AtaOtpInput(state) => {
                self.render_ata_otp_input(area, buf, state);
            }
            SignInState::AtaVerifyingOtp { _email } => {
                self.render_ata_verifying_otp(area, buf, _email);
            }
            SignInState::AtaSuccessMessage => {
                self.render_ata_success_message(area, buf);
            }
            SignInState::AtaSuccess => {
                self.render_ata_success(area, buf);
            }
            SignInState::ApiKeyConfigured => {
                self.render_api_key_configured(area, buf);
            }
        }
    }
}

pub(super) fn maybe_open_auth_url_in_browser(request_handle: &AppServerRequestHandle, url: &str) {
    if !matches!(request_handle, AppServerRequestHandle::InProcess(_)) {
        return;
    }

    if let Err(err) = webbrowser::open(url) {
        tracing::warn!("failed to open browser for login URL: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_core::config::ConfigBuilder;
    use codex_app_server_client::AppServerRequestHandle;
    use codex_app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
    use codex_app_server_client::InProcessAppServerClient;
    use codex_app_server_client::InProcessClientStartArgs;
    use codex_arg0::Arg0DispatchPaths;
    use codex_cloud_requirements::cloud_requirements_loader_for_storage;
    use codex_config::types::AuthCredentialsStoreMode;

    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn widget_default() -> (AuthModeWidget, TempDir) {
        let (mut widget, dir) = widget_forced_chatgpt().await;
        widget.forced_login_method = None;
        (widget, dir)
    }

    #[tokio::test]
    async fn provider_picker_lists_openai_anthropic_gemini_copilot() {
        // The "Configure model providers" sub-screen must surface these four
        // providers in this order — matches locus's `provider_picker.rs`
        // registry. Anthropic + Gemini are API-key only / API-key+OAuth
        // respectively; Copilot is OAuth-only (device code).
        let (mut widget, _tmp) = widget_default().await;
        widget.start_provider_selection();
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickProvider
        ));
        // Highlight defaults to OpenAI (first registry entry).
        pretty_assertions::assert_eq!(widget.highlighted_provider, ProviderOption::OpenAI);
        // Sanity-check the registry exposes Anthropic + Gemini + Copilot.
        for expected in [
            ProviderOption::OpenAI,
            ProviderOption::Anthropic,
            ProviderOption::Gemini,
            ProviderOption::Copilot,
        ] {
            assert!(
                !expected.display_name().is_empty(),
                "{expected:?} display_name must not be empty"
            );
            assert!(
                !expected.provider_id().is_empty(),
                "{expected:?} provider_id must not be empty"
            );
        }
        // Gemini supports both API key + OAuth; Copilot only OAuth.
        assert!(ProviderOption::Gemini.supports_api_key());
        assert!(ProviderOption::Gemini.supports_oauth());
        assert!(!ProviderOption::Copilot.supports_api_key());
        assert!(ProviderOption::Copilot.supports_oauth());
    }

    #[tokio::test]
    async fn pick_mode_renders_four_options() {
        // Regression: the onboarding picker must surface four entries —
        // ChatGPT / Device Code / Configure model providers / Sign in with
        // ATA account — once neither forced-login mode is active. This
        // matches the locus / ata-main layout and replaces the legacy
        // top-level Copilot + ApiKey entries that snuck in during the
        // upstream merge.
        let (widget, _tmp) = widget_default().await;
        let displayed = widget.displayed_sign_in_options();
        pretty_assertions::assert_eq!(
            displayed,
            vec![
                SignInOption::ChatGpt,
                SignInOption::DeviceCode,
                SignInOption::ConfigureProviders,
                SignInOption::AtaAccount,
            ],
            "top-level picker must show exactly four entries in this order"
        );
        // SignInOption::Copilot is not part of the enum any more — it lives
        // inside the provider picker (via ProviderOption::Copilot).
        // SignInOption::ApiKey is similarly gone; OpenAI's API key is
        // configured under "Configure model providers".
    }

    async fn widget_forced_chatgpt() -> (AuthModeWidget, TempDir) {
        let codex_home = TempDir::new().unwrap();
        let codex_home_path = codex_home.path().to_path_buf();
        let config = ConfigBuilder::default()
            .codex_home(codex_home_path.clone())
            .build()
            .await
            .unwrap();
        let client = InProcessAppServerClient::start(InProcessClientStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides: Default::default(),
            cloud_requirements: cloud_requirements_loader_for_storage(
                codex_home_path.clone(),
                /*enable_codex_api_key_env*/ false,
                AuthCredentialsStoreMode::File,
                "https://chatgpt.com/backend-api/".to_string(),
            )
            .await,
            feedback: codex_feedback::CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(
                codex_app_server_client::EnvironmentManager::default_for_tests(),
            ),
            config_warnings: Vec::new(),
            session_source: serde_json::from_value(serde_json::json!("cli"))
                .expect("cli session source should deserialize"),
            enable_codex_api_key_env: false,
            client_name: "test".to_string(),
            client_version: "test".to_string(),
            experimental_api: true,
            opt_out_notification_methods: Vec::new(),
            channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await
        .unwrap();
        let widget = AuthModeWidget {
            request_frame: FrameRequester::test_dummy(),
            highlighted_mode: SignInOption::ChatGpt,
            error: Arc::new(RwLock::new(None)),
            sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
            login_status: LoginStatus::NotAuthenticated,
            app_server_request_handle: AppServerRequestHandle::InProcess(client.request_handle()),
            forced_login_method: Some(ForcedLoginMethod::Chatgpt),
            animations_enabled: true,
            animations_suppressed: std::cell::Cell::new(false),
            highlighted_provider: ProviderOption::first(),
            highlighted_provider_auth_method: ProviderAuthMethod::ApiKey,
        };
        (widget, codex_home)
    }

    #[tokio::test]
    async fn api_key_flow_disabled_when_chatgpt_forced() {
        // When the workspace forces ChatGPT login, picking "Configure model
        // providers" from the top-level menu must surface the
        // "API key login is disabled" error and leave the picker in
        // `PickMode`, regardless of which provider would have been chosen
        // inside the sub-screen.
        let (mut widget, _tmp) = widget_forced_chatgpt().await;

        widget.handle_sign_in_option(SignInOption::ConfigureProviders);

        assert_eq!(
            widget.error_message().as_deref(),
            Some(API_KEY_DISABLED_MESSAGE)
        );
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[tokio::test]
    async fn saving_api_key_is_blocked_when_chatgpt_forced() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;

        widget.save_api_key("sk-test".to_string());

        assert_eq!(
            widget.error_message().as_deref(),
            Some(API_KEY_DISABLED_MESSAGE)
        );
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
        assert_eq!(widget.login_status, LoginStatus::NotAuthenticated);
    }

    #[tokio::test]
    async fn existing_chatgpt_auth_tokens_login_counts_as_signed_in() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        widget.login_status = LoginStatus::AuthMode(AppServerAuthMode::ChatgptAuthTokens);

        let handled = widget.handle_existing_chatgpt_login();

        assert_eq!(handled, true);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccess
        ));
    }

    #[tokio::test]
    async fn cancel_active_attempt_resets_browser_login_state() {
        let (widget, _tmp) = widget_forced_chatgpt().await;
        *widget.error.write().unwrap() = Some("still logging in".to_string());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
                login_id: "login-1".to_string(),
                auth_url: "https://auth.example.com".to_string(),
            });

        widget.cancel_active_attempt();

        assert_eq!(widget.error_message(), None);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[tokio::test]
    async fn cancel_active_attempt_notifies_device_code_login() {
        let (widget, _tmp) = widget_forced_chatgpt().await;
        *widget.error.write().unwrap() = Some("still logging in".to_string());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
                "request-1".to_string(),
                "login-1".to_string(),
                "https://chatgpt.com/device".to_string(),
                "ABCD-EFGH".to_string(),
            ));

        widget.cancel_active_attempt();

        assert_eq!(widget.error_message(), None);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    /// Collects all buffer cell symbols that contain the OSC 8 open sequence
    /// for the given URL.  Returns the concatenated "inner" characters.
    fn collect_osc8_chars(buf: &Buffer, area: Rect, url: &str) -> String {
        let open = format!("\x1B]8;;{url}\x07");
        let close = "\x1B]8;;\x07";
        let mut chars = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let sym = buf[(x, y)].symbol();
                if let Some(rest) = sym.strip_prefix(open.as_str())
                    && let Some(ch) = rest.strip_suffix(close)
                {
                    chars.push_str(ch);
                }
            }
        }
        chars
    }

    #[test]
    fn continue_in_browser_renders_osc8_hyperlink() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        let url = "https://auth.example.com/login?state=abc123";
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
                login_id: "login-1".to_string(),
                auth_url: url.to_string(),
            });

        // Render into a narrow buffer so the URL wraps across multiple rows.
        let area = Rect::new(0, 0, 30, 20);
        let mut buf = Buffer::empty(area);
        widget.render_continue_in_browser(area, &mut buf);

        // Every character of the URL should be present as an OSC 8 cell.
        let found = collect_osc8_chars(&buf, area, url);
        assert_eq!(found, url, "OSC 8 hyperlink should cover the full URL");
    }

    #[test]
    fn auth_widget_suppresses_animations_when_device_code_is_visible() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
                "request-1".to_string(),
                "login-1".to_string(),
                "https://chatgpt.com/device".to_string(),
                "ABCD-EFGH".to_string(),
            ));

        assert_eq!(widget.should_suppress_animations(), true);
    }

    #[test]
    fn auth_widget_suppresses_animations_while_requesting_device_code() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
        *widget.sign_in_state.write().unwrap() = SignInState::ChatGptDeviceCode(
            ContinueWithDeviceCodeState::pending("request-1".to_string()),
        );

        assert_eq!(widget.should_suppress_animations(), true);
    }

    #[tokio::test]
    async fn device_code_login_completion_advances_to_success_message() {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        *widget.sign_in_state.write().unwrap() =
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
                "request-1".to_string(),
                "login-1".to_string(),
                "https://chatgpt.com/device".to_string(),
                "ABCD-EFGH".to_string(),
            ));

        widget.on_account_login_completed(AccountLoginCompletedNotification {
            login_id: Some("login-1".to_string()),
            success: true,
            error: None,
        });

        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccessMessage
        ));
    }

    #[test]
    fn mark_url_hyperlink_wraps_cyan_underlined_cells() {
        let url = "https://example.com";
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);

        // Manually write some cyan+underlined characters to simulate a rendered URL.
        for (i, ch) in "example".chars().enumerate() {
            let cell = &mut buf[(i as u16, 0)];
            cell.set_symbol(&ch.to_string());
            cell.fg = Color::Cyan;
            cell.modifier = Modifier::UNDERLINED;
        }
        // Leave a plain cell that should NOT be marked.
        buf[(7, 0)].set_symbol("X");

        mark_url_hyperlink(&mut buf, area, url);

        // Each cyan+underlined cell should now carry the OSC 8 wrapper.
        let found = collect_osc8_chars(&buf, area, url);
        assert_eq!(found, "example");

        // The plain "X" cell should be untouched.
        assert_eq!(buf[(7, 0)].symbol(), "X");
    }

    #[test]
    fn mark_url_hyperlink_sanitizes_control_chars() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);

        // One cyan+underlined cell to mark.
        let cell = &mut buf[(0, 0)];
        cell.set_symbol("a");
        cell.fg = Color::Cyan;
        cell.modifier = Modifier::UNDERLINED;

        // URL contains ESC and BEL that could break the OSC 8 sequence.
        let malicious_url = "https://evil.com/\x1B]8;;\x07injected";
        mark_url_hyperlink(&mut buf, area, malicious_url);

        let sym = buf[(0, 0)].symbol().to_string();
        // The sanitized URL retains `]` (printable) but strips ESC and BEL.
        let sanitized = "https://evil.com/]8;;injected";
        assert!(
            sym.contains(sanitized),
            "symbol should contain sanitized URL, got: {sym:?}"
        );
        // The injected close-sequence must not survive: \x1B and \x07 are gone.
        assert!(
            !sym.contains("\x1B]8;;\x07injected"),
            "symbol must not contain raw control chars from URL"
        );
    }
}
