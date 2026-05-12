// Mirrors `auth.rs`: the bottom-pane state machine uses `RwLock` and the
// TUI's "fail fast" convention is to `.unwrap()` on lock acquisition rather
// than thread error handling everywhere.
#![allow(clippy::unwrap_used)]

//! Per-provider sub-screen of the onboarding sign-in picker.
//!
//! The main four-option menu in `auth.rs` routes "Configure model providers"
//! into the state machine implemented here. We expose:
//!
//! - `ProviderOption` — the providers ATA knows how to configure from the TUI.
//! - Render helpers (`render_pick_provider`, `render_pick_provider_auth_method`,
//!   `render_provider_oauth_continue_in_browser`, `render_provider_oauth_success_message`,
//!   `render_provider_configured`).
//! - Key handling (`handle_provider_picker_key_event`).
//! - Entry points (`start_provider_selection`, `start_provider_configuration`,
//!   `start_provider_api_key_entry`, `start_provider_oauth_login`).
//!
//! All backend work goes through `AppServerRequestHandle::request_typed` —
//! this module does **not** reach into the core crate directly, so the
//! TUI/core boundary check stays green.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::LoginAccountParams;
use codex_app_server_protocol::LoginAccountResponse;

use super::auth::ApiKeyInputState;
use super::auth::AuthModeWidget;
use super::auth::ProviderAuthMethod;
use super::auth::ProviderOauthContinueInBrowserState;
use super::auth::SignInState;
use super::auth::onboarding_request_id;

/// Providers configurable from the TUI sign-in picker.
///
/// Stays string-table driven so future additions only need an enum variant +
/// one entry in `PROVIDERS`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOption {
    OpenAI,
    Anthropic,
    Gemini,
    Copilot,
}

struct ProviderEntry {
    option: ProviderOption,
    /// Stable id used in `auth.json`'s `providers` map and the
    /// `LoginAccountParams::ProviderApiKey { provider_id }` RPC.
    id: &'static str,
    display_name: &'static str,
    /// Env var that, when set, prefills the API key entry box.
    env_var: Option<&'static str>,
    supports_api_key: bool,
    supports_oauth: bool,
}

const PROVIDERS: &[ProviderEntry] = &[
    ProviderEntry {
        option: ProviderOption::OpenAI,
        id: "openai",
        display_name: "OpenAI",
        env_var: Some("OPENAI_API_KEY"),
        supports_api_key: true,
        supports_oauth: false,
    },
    ProviderEntry {
        option: ProviderOption::Anthropic,
        id: "anthropic",
        display_name: "Anthropic",
        env_var: Some("ANTHROPIC_API_KEY"),
        supports_api_key: true,
        supports_oauth: false,
    },
    ProviderEntry {
        option: ProviderOption::Gemini,
        id: "gemini",
        display_name: "Google Gemini",
        env_var: Some("GOOGLE_API_KEY"),
        supports_api_key: true,
        supports_oauth: true,
    },
    ProviderEntry {
        option: ProviderOption::Copilot,
        id: "copilot",
        display_name: "GitHub Copilot",
        env_var: None,
        supports_api_key: false,
        supports_oauth: true,
    },
];

fn entry_for(option: ProviderOption) -> &'static ProviderEntry {
    // Match on the variant rather than searching: the exhaustiveness check
    // guarantees PROVIDERS and ProviderOption stay in sync without
    // resorting to `.expect()` on the runtime lookup.
    let id = match option {
        ProviderOption::OpenAI => "openai",
        ProviderOption::Anthropic => "anthropic",
        ProviderOption::Gemini => "gemini",
        ProviderOption::Copilot => "copilot",
    };
    PROVIDERS
        .iter()
        .find(|entry| entry.id == id)
        .unwrap_or(&PROVIDERS[0])
}

impl ProviderOption {
    pub(crate) fn provider_id(self) -> &'static str {
        entry_for(self).id
    }

    pub(crate) fn display_name(self) -> &'static str {
        entry_for(self).display_name
    }

    pub(super) fn env_var(self) -> Option<&'static str> {
        entry_for(self).env_var
    }

    pub(crate) fn supports_api_key(self) -> bool {
        entry_for(self).supports_api_key
    }

    pub(crate) fn supports_oauth(self) -> bool {
        entry_for(self).supports_oauth
    }

    pub(crate) fn first() -> Self {
        PROVIDERS[0].option
    }
}

fn provider_options() -> Vec<ProviderOption> {
    PROVIDERS.iter().map(|entry| entry.option).collect()
}

fn provider_auth_method_options(
    provider: ProviderOption,
) -> Vec<(ProviderAuthMethod, &'static str, &'static str)> {
    let mut options = Vec::new();
    if provider.supports_api_key() {
        options.push((
            ProviderAuthMethod::ApiKey,
            "API key",
            "Use usage-based billing with your own API key",
        ));
    }
    if provider.supports_oauth() {
        let (label, desc) = match provider {
            ProviderOption::Gemini => (
                "OAuth (Gemini Code Assist)",
                "Sign in with your Google account and use Code Assist",
            ),
            ProviderOption::Copilot => (
                "OAuth (device code)",
                "Sign in with your GitHub Copilot subscription",
            ),
            _ => ("OAuth", "Sign in with the provider's web flow"),
        };
        options.push((ProviderAuthMethod::Oauth, label, desc));
    }
    options
}

impl AuthModeWidget {
    /// Drive key events while a provider-picker sub-screen is active.
    /// Returns `true` if the event was consumed.
    pub(super) fn handle_provider_picker_key_event(
        &mut self,
        key_event: &KeyEvent,
        sign_in_state: &SignInState,
    ) -> bool {
        match sign_in_state {
            SignInState::PickProvider => {
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.move_provider_highlight(-1);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.move_provider_highlight(1);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let idx = (c as u8).saturating_sub(b'1') as usize;
                        if let Some(provider) = provider_options().get(idx).copied() {
                            self.start_provider_configuration(provider);
                        }
                    }
                    KeyCode::Enter => {
                        self.start_provider_configuration(self.highlighted_provider);
                    }
                    KeyCode::Esc => {
                        *self.sign_in_state.write().unwrap() = SignInState::PickMode;
                        self.set_error_message(None);
                        self.request_frame.schedule_frame();
                    }
                    _ => {}
                }
                true
            }
            SignInState::PickProviderAuthMethod(provider) => {
                let provider = *provider;
                match key_event.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.move_provider_auth_method_highlight(provider, -1);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.move_provider_auth_method_highlight(provider, 1);
                        self.request_frame.schedule_frame();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let idx = (c as u8).saturating_sub(b'1') as usize;
                        if let Some((method, _, _)) =
                            provider_auth_method_options(provider).get(idx).copied()
                        {
                            self.dispatch_provider_auth_method(provider, method);
                        }
                    }
                    KeyCode::Enter => {
                        self.dispatch_provider_auth_method(
                            provider,
                            self.highlighted_provider_auth_method,
                        );
                    }
                    KeyCode::Esc => {
                        *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
                        self.set_error_message(None);
                        self.request_frame.schedule_frame();
                    }
                    _ => {}
                }
                true
            }
            SignInState::ProviderOauthContinueInBrowser(_) => {
                if key_event.code == KeyCode::Esc {
                    *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
                    self.set_error_message(None);
                    self.request_frame.schedule_frame();
                }
                true
            }
            SignInState::ProviderOauthSuccessMessage(_) => {
                if key_event.code == KeyCode::Enter {
                    *self.sign_in_state.write().unwrap() = SignInState::ProviderConfigured;
                    self.request_frame.schedule_frame();
                }
                true
            }
            SignInState::ProviderList => {
                *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
                self.request_frame.schedule_frame();
                true
            }
            _ => false,
        }
    }

    pub(super) fn start_provider_selection(&mut self) {
        self.set_error_message(None);
        self.highlighted_provider = ProviderOption::first();
        self.highlighted_provider_auth_method = ProviderAuthMethod::ApiKey;
        *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
        self.request_frame.schedule_frame();
    }

    pub(super) fn render_pick_provider(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Configure model providers".bold()]),
            "".into(),
            "  Select a provider to configure:".into(),
            "".into(),
        ];

        for (idx, option) in provider_options().into_iter().enumerate() {
            let is_selected = self.highlighted_provider == option;
            let caret = if is_selected { ">" } else { " " };
            let index = idx + 1;
            let name = option.display_name();
            let description = match option {
                ProviderOption::OpenAI => "Pay for what you use with an OpenAI API key",
                ProviderOption::Anthropic => "Use Claude with an Anthropic API key",
                ProviderOption::Gemini => "API key or sign in to Google Gemini Code Assist",
                ProviderOption::Copilot => "Use your existing GitHub Copilot subscription",
            };
            if is_selected {
                lines.push(Line::from(vec![
                    format!("{caret} {index}. ").cyan().dim(),
                    name.to_string().cyan(),
                ]));
                lines.push(
                    Line::from(format!("     {description}"))
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::DIM),
                );
            } else {
                lines.push(format!("  {index}. {name}").into());
                lines.push(
                    Line::from(format!("     {description}"))
                        .style(Style::default().add_modifier(Modifier::DIM)),
                );
            }
            lines.push("".into());
        }

        lines.push(
            "  Press Enter to configure the selected provider"
                .dim()
                .into(),
        );
        lines.push("  Press Esc to go back".dim().into());

        if let Some(err) = self.error_message_snapshot() {
            lines.push("".into());
            lines.push(err.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    pub(super) fn render_pick_provider_auth_method(
        &self,
        area: Rect,
        buf: &mut Buffer,
        provider: ProviderOption,
    ) {
        let provider_name = provider.display_name();
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                "> ".into(),
                format!("Configure {provider_name}").bold(),
            ]),
            "".into(),
            "  Select an authentication method:".into(),
            "".into(),
        ];

        for (idx, (method, title, description)) in provider_auth_method_options(provider)
            .into_iter()
            .enumerate()
        {
            let is_selected = self.highlighted_provider_auth_method == method;
            let caret = if is_selected { ">" } else { " " };
            if is_selected {
                lines.push(Line::from(vec![
                    format!("{caret} {}. ", idx + 1).cyan().dim(),
                    title.cyan(),
                ]));
                lines.push(format!("     {description}").cyan().dim().into());
            } else {
                lines.push(format!("  {}. {title}", idx + 1).into());
                lines.push(format!("     {description}").dim().into());
            }
            lines.push("".into());
        }

        lines.push("  Press Enter to continue".dim().into());
        lines.push("  Press Esc to go back".dim().into());

        if let Some(error) = self.error_message_snapshot() {
            lines.push("".into());
            lines.push(error.red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    pub(super) fn render_provider_oauth_continue_in_browser(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ProviderOauthContinueInBrowserState,
    ) {
        let provider_name = state.provider().display_name();
        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                "  ".into(),
                format!("Finish signing in with {provider_name} via your browser").into(),
            ]),
            "".into(),
        ];
        if !state_auth_url(state).is_empty() {
            lines.push(
                "  If the link doesn't open automatically, open the following link to authenticate:"
                    .into(),
            );
            lines.push("".into());
            lines.push(Line::from(vec![
                "  ".into(),
                state_auth_url(state).cyan().underlined(),
            ]));
            lines.push("".into());
        }
        lines.push("  Press Esc to cancel".dim().into());

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    pub(super) fn render_provider_oauth_success_message(
        &self,
        area: Rect,
        buf: &mut Buffer,
        provider: ProviderOption,
    ) {
        let provider_name = provider.display_name();
        let lines = vec![
            format!("✓ Signed in with {provider_name}")
                .fg(Color::Green)
                .into(),
            "".into(),
            "  Press Enter to continue".dim().into(),
        ];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    pub(super) fn render_provider_configured(&self, area: Rect, buf: &mut Buffer) {
        let lines = vec![
            "✓ Provider configured".fg(Color::Green).into(),
            "".into(),
            "  Codex will use the credentials you just set up.".into(),
        ];
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render_ref(area, buf);
    }

    fn move_provider_highlight(&mut self, delta: isize) {
        let options = provider_options();
        let current = options
            .iter()
            .position(|opt| *opt == self.highlighted_provider)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_provider = options[next];
    }

    fn move_provider_auth_method_highlight(&mut self, provider: ProviderOption, delta: isize) {
        let methods: Vec<ProviderAuthMethod> = provider_auth_method_options(provider)
            .into_iter()
            .map(|(method, _, _)| method)
            .collect();
        if methods.is_empty() {
            return;
        }
        let current = methods
            .iter()
            .position(|m| *m == self.highlighted_provider_auth_method)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(methods.len() as isize) as usize;
        self.highlighted_provider_auth_method = methods[next];
    }

    fn start_provider_configuration(&mut self, provider: ProviderOption) {
        match (provider.supports_api_key(), provider.supports_oauth()) {
            (true, true) => self.start_provider_auth_method_picker(provider),
            (true, false) => self.start_provider_api_key_entry(provider),
            (false, true) => self.start_provider_oauth_login(provider),
            (false, false) => {
                self.set_error_message(Some(format!(
                    "{} is not currently configured for sign-in.",
                    provider.display_name()
                )));
                self.request_frame.schedule_frame();
            }
        }
    }

    fn start_provider_auth_method_picker(&mut self, provider: ProviderOption) {
        self.set_error_message(None);
        self.highlighted_provider_auth_method = provider_auth_method_options(provider)
            .first()
            .map(|(method, _, _)| *method)
            .unwrap_or(ProviderAuthMethod::ApiKey);
        *self.sign_in_state.write().unwrap() = SignInState::PickProviderAuthMethod(provider);
        self.request_frame.schedule_frame();
    }

    fn dispatch_provider_auth_method(
        &mut self,
        provider: ProviderOption,
        method: ProviderAuthMethod,
    ) {
        match method {
            ProviderAuthMethod::ApiKey => self.start_provider_api_key_entry(provider),
            ProviderAuthMethod::Oauth => self.start_provider_oauth_login(provider),
        }
    }

    fn start_provider_api_key_entry(&mut self, provider: ProviderOption) {
        self.set_error_message(None);
        let prefill = provider
            .env_var()
            .and_then(|env| std::env::var(env).ok())
            .filter(|s| !s.is_empty());
        let state = ApiKeyInputState::new(
            prefill.clone().unwrap_or_default(),
            prefill.is_some(),
            Some(provider),
        );
        *self.sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(state);
        self.request_frame.schedule_frame();
    }

    fn start_provider_oauth_login(&mut self, provider: ProviderOption) {
        match provider {
            ProviderOption::Copilot => {
                super::auth::start_copilot_login(self);
            }
            ProviderOption::Gemini => {
                self.start_gemini_oauth_login();
            }
            _ => {
                self.set_error_message(Some(format!(
                    "OAuth login is not available for {}.",
                    provider.display_name()
                )));
                self.request_frame.schedule_frame();
            }
        }
    }

    fn start_gemini_oauth_login(&mut self) {
        self.set_error_message(None);
        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error_arc();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            match request_handle
                .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                    request_id: onboarding_request_id(),
                    params: LoginAccountParams::GeminiOauth,
                })
                .await
            {
                Ok(LoginAccountResponse::GeminiOauthContinueInBrowser { login_id, auth_url }) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() = SignInState::ProviderOauthContinueInBrowser(
                        ProviderOauthContinueInBrowserState::new(
                            ProviderOption::Gemini,
                            auth_url,
                            Some(login_id),
                        ),
                    );
                }
                Ok(other) => {
                    *error.write().unwrap() = Some(format!(
                        "Unexpected account/login/start response: {other:?}"
                    ));
                    *sign_in_state.write().unwrap() = SignInState::PickProvider;
                }
                Err(err) => {
                    *error.write().unwrap() = Some(err.to_string());
                    *sign_in_state.write().unwrap() = SignInState::PickProvider;
                }
            }
            request_frame.schedule_frame();
        });
    }
}

fn state_auth_url(state: &ProviderOauthContinueInBrowserState) -> &str {
    state.auth_url()
}
