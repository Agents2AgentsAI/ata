#![allow(clippy::unwrap_used)]

use codex_core::AuthManager;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::CLIENT_ID;
use codex_core::auth::PROVIDER_ANTHROPIC;
use codex_core::auth::PROVIDER_GEMINI;
use codex_core::auth::PROVIDER_OPENAI;
use codex_core::auth::ProviderAuthSource;
use codex_core::auth::list_configured_providers;
use codex_core::auth::login_with_provider_api_key;
use codex_core::auth::provider_env_var;
use codex_core::auth::read_api_key_from_env;
use codex_core::auth::read_openai_api_key_from_env;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::edit::default_model_for_provider;
use codex_login::DeviceCode;
use codex_login::ServerOptions;
use codex_login::ShutdownHandle;
use codex_login::run_login_server;
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

use codex_app_server_protocol::AuthMode;
use codex_protocol::config_types::ForcedLoginMethod;
use std::sync::RwLock;

use crate::LoginStatus;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::shimmer::shimmer_spans;
use crate::tui::FrameRequester;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Notify;

use super::onboarding_screen::StepState;

mod headless_chatgpt_login;

#[derive(Clone)]
pub(crate) enum SignInState {
    PickMode,
    ChatGptContinueInBrowser(ContinueInBrowserState),
    ChatGptDeviceCode(ContinueWithDeviceCodeState),
    ChatGptSuccessMessage,
    ChatGptSuccess,
    ApiKeyEntry(ApiKeyInputState),
    ApiKeyConfigured,
    PickProvider, // Select which provider to configure
    ProviderList, // Show all configured providers
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SignInOption {
    ChatGpt,
    DeviceCode,
    ApiKey,
    ConfigureProviders,
}

/// Provider options for the multi-provider configuration screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOption {
    OpenAI,
    Anthropic,
    Gemini,
}

impl ProviderOption {
    fn provider_id(&self) -> &'static str {
        match self {
            Self::OpenAI => PROVIDER_OPENAI,
            Self::Anthropic => PROVIDER_ANTHROPIC,
            Self::Gemini => PROVIDER_GEMINI,
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Google Gemini",
        }
    }

    fn env_var_name(&self) -> Option<&'static str> {
        provider_env_var(self.provider_id())
    }
}

const API_KEY_DISABLED_MESSAGE: &str = "API key login is disabled.";

#[derive(Clone, Default)]
pub(crate) struct ApiKeyInputState {
    value: String,
    prepopulated_from_env: bool,
    /// The provider being configured. None means legacy OpenAI-only flow.
    provider: Option<ProviderOption>,
}

#[derive(Clone)]
/// Used to manage the lifecycle of SpawnedLogin and ensure it gets cleaned up.
pub(crate) struct ContinueInBrowserState {
    auth_url: String,
    shutdown_flag: Option<ShutdownHandle>,
}

#[derive(Clone)]
pub(crate) struct ContinueWithDeviceCodeState {
    device_code: Option<DeviceCode>,
    cancel: Option<Arc<Notify>>,
}

impl Drop for ContinueInBrowserState {
    fn drop(&mut self) {
        if let Some(handle) = &self.shutdown_flag {
            handle.shutdown();
        }
    }
}

impl KeyboardHandler for AuthModeWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.handle_api_key_entry_key_event(&key_event) {
            return;
        }

        let sign_in_state = { (*self.sign_in_state.read().unwrap()).clone() };

        // Handle PickProvider state
        if matches!(sign_in_state, SignInState::PickProvider) {
            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_provider_highlight(-1);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_provider_highlight(1);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Char('1') => {
                    self.start_provider_api_key_entry(ProviderOption::OpenAI);
                }
                KeyCode::Char('2') => {
                    self.start_provider_api_key_entry(ProviderOption::Anthropic);
                }
                KeyCode::Char('3') => {
                    self.start_provider_api_key_entry(ProviderOption::Gemini);
                }
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    self.show_provider_list();
                }
                KeyCode::Enter => {
                    self.start_provider_api_key_entry(self.highlighted_provider);
                }
                KeyCode::Esc => {
                    *self.sign_in_state.write().unwrap() = SignInState::PickMode;
                    self.request_frame.schedule_frame();
                }
                _ => {}
            }
            return;
        }

        // Handle ProviderList state
        if matches!(sign_in_state, SignInState::ProviderList) {
            // Any key goes back to provider selection
            *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
            self.request_frame.schedule_frame();
            return;
        }

        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_highlight(-1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_highlight(1);
            }
            KeyCode::Char('1') => {
                self.select_option_by_index(0);
            }
            KeyCode::Char('2') => {
                self.select_option_by_index(1);
            }
            KeyCode::Char('3') => {
                self.select_option_by_index(2);
            }
            KeyCode::Char('4') => {
                self.select_option_by_index(3);
            }
            KeyCode::Enter => match sign_in_state {
                SignInState::PickMode => {
                    self.handle_sign_in_option(self.highlighted_mode);
                }
                SignInState::ChatGptSuccessMessage => {
                    *self.sign_in_state.write().unwrap() = SignInState::ChatGptSuccess;
                }
                _ => {}
            },
            KeyCode::Esc => {
                tracing::info!("Esc pressed");
                let mut sign_in_state = self.sign_in_state.write().unwrap();
                match &*sign_in_state {
                    SignInState::ChatGptContinueInBrowser(_) => {
                        *sign_in_state = SignInState::PickMode;
                        drop(sign_in_state);
                        self.request_frame.schedule_frame();
                    }
                    SignInState::ChatGptDeviceCode(state) => {
                        if let Some(cancel) = &state.cancel {
                            cancel.notify_one();
                        }
                        *sign_in_state = SignInState::PickMode;
                        drop(sign_in_state);
                        self.request_frame.schedule_frame();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        let _ = self.handle_api_key_entry_paste(pasted);
    }
}

#[derive(Clone)]
pub(crate) struct AuthModeWidget {
    pub request_frame: FrameRequester,
    pub highlighted_mode: SignInOption,
    pub highlighted_provider: ProviderOption,
    pub error: Option<String>,
    pub sign_in_state: Arc<RwLock<SignInState>>,
    pub codex_home: PathBuf,
    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
    pub login_status: LoginStatus,
    pub auth_manager: Arc<AuthManager>,
    pub forced_chatgpt_workspace_id: Option<String>,
    pub forced_login_method: Option<ForcedLoginMethod>,
    pub animations_enabled: bool,
}

impl AuthModeWidget {
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
            options.push(SignInOption::ApiKey);
            options.push(SignInOption::ConfigureProviders);
        }
        options
    }

    fn selectable_sign_in_options(&self) -> Vec<SignInOption> {
        let mut options = Vec::new();
        if self.is_chatgpt_login_allowed() {
            options.push(SignInOption::ChatGpt);
            options.push(SignInOption::DeviceCode);
        }
        if self.is_api_login_allowed() {
            options.push(SignInOption::ApiKey);
            options.push(SignInOption::ConfigureProviders);
        }
        options
    }

    fn provider_options(&self) -> Vec<ProviderOption> {
        vec![
            ProviderOption::OpenAI,
            ProviderOption::Anthropic,
            ProviderOption::Gemini,
        ]
    }

    fn move_provider_highlight(&mut self, delta: isize) {
        let options = self.provider_options();
        if options.is_empty() {
            return;
        }

        let current_index = options
            .iter()
            .position(|option| *option == self.highlighted_provider)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_provider = options[next_index];
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
            SignInOption::ApiKey => {
                if self.is_api_login_allowed() {
                    self.start_api_key_entry();
                } else {
                    self.disallow_api_login();
                }
            }
            SignInOption::ConfigureProviders => {
                if self.is_api_login_allowed() {
                    self.start_provider_selection();
                } else {
                    self.disallow_api_login();
                }
            }
        }
    }

    fn start_provider_selection(&mut self) {
        self.error = None;
        self.highlighted_provider = ProviderOption::OpenAI;
        *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
        self.request_frame.schedule_frame();
    }

    fn start_provider_api_key_entry(&mut self, provider: ProviderOption) {
        self.error = None;
        let prefill_from_env = read_api_key_from_env(provider.provider_id());
        *self.sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState {
            value: prefill_from_env.clone().unwrap_or_default(),
            prepopulated_from_env: prefill_from_env.is_some(),
            provider: Some(provider),
        });
        self.request_frame.schedule_frame();
    }

    fn show_provider_list(&mut self) {
        self.error = None;
        *self.sign_in_state.write().unwrap() = SignInState::ProviderList;
        self.request_frame.schedule_frame();
    }

    fn disallow_api_login(&mut self) {
        self.highlighted_mode = SignInOption::ChatGpt;
        self.error = Some(API_KEY_DISABLED_MESSAGE.to_string());
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
            "Usage included with Plus, Pro, Team, and Enterprise plans"
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
                SignInOption::ApiKey => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Provide your own API key",
                        "Pay for what you use (OpenAI)",
                    ));
                }
                SignInOption::ConfigureProviders => {
                    lines.extend(create_mode_item(
                        idx,
                        option,
                        "Configure providers",
                        "Set up API keys for multiple providers",
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
        lines.push(
            // AE: Following styles.md, this should probably be Cyan because it's a user input tip.
            //     But leaving this for a future cleanup.
            "  Press Enter to continue".dim().into(),
        );
        if let Some(err) = &self.error {
            lines.push("".into());
            lines.push(err.as_str().red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_continue_in_browser(&self, area: Rect, buf: &mut Buffer) {
        let mut spans = vec!["  ".into()];
        if self.animations_enabled {
            // Schedule a follow-up frame to keep the shimmer animation going.
            self.request_frame
                .schedule_frame_in(std::time::Duration::from_millis(100));
            spans.extend(shimmer_spans("Finish signing in via your browser"));
        } else {
            spans.push("Finish signing in via your browser".into());
        }
        let mut lines = vec![spans.into(), "".into()];

        let sign_in_state = self.sign_in_state.read().unwrap();
        if let SignInState::ChatGptContinueInBrowser(state) = &*sign_in_state
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
                "  On a remote or headless machine? Press Esc and choose ".into(),
                "Sign in with Device Code".cyan(),
                ".".into(),
            ]));
            lines.push("".into());
        }

        lines.push("  Press Esc to cancel".dim().into());
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
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
                "\u{1b}]8;;https://github.com/openai/codex\u{7}Codex docs\u{1b}]8;;\u{7}".underlined(),
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
            "  Press Enter to continue".fg(Color::Cyan).into(),
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

        // Get provider-specific text
        let (provider_name, env_var_name) = match &state.provider {
            Some(provider) => (provider.display_name(), provider.env_var_name()),
            None => ("OpenAI", Some("OPENAI_API_KEY")),
        };

        let title = format!("Use your own {provider_name} API key for usage-based billing");
        let mut intro_lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), title.bold()]),
            "".into(),
            "  Paste or type your API key below. It will be stored locally in auth.json.".into(),
            "".into(),
        ];
        if state.prepopulated_from_env {
            if let Some(env_var) = env_var_name {
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

        let block_title = format!("{provider_name} API key");
        let content_line: Line = if state.value.is_empty() {
            vec!["Paste or type your API key".dim()].into()
        } else {
            Line::from(state.value.clone())
        };
        Paragraph::new(content_line)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(block_title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .render(input_area, buf);

        let mut footer_lines: Vec<Line> = vec![
            "  Press Enter to save".dim().into(),
            "  Press Esc to go back".dim().into(),
        ];
        if let Some(error) = &self.error {
            footer_lines.push("".into());
            footer_lines.push(error.as_str().red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    fn render_pick_provider(&self, area: Rect, buf: &mut Buffer) {
        let providers =
            list_configured_providers(&self.codex_home, self.cli_auth_credentials_store_mode);

        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Configure API keys for providers".bold()]),
            "".into(),
            "  Select a provider to configure:".into(),
            "".into(),
        ];

        for (idx, option) in self.provider_options().into_iter().enumerate() {
            let is_selected = self.highlighted_provider == option;
            let caret = if is_selected { ">" } else { " " };

            // Check if this provider is configured
            let status = providers
                .iter()
                .find(|p| p.provider_id == option.provider_id())
                .map(|p| match p.source {
                    ProviderAuthSource::Stored => " [stored]",
                    ProviderAuthSource::Environment => " [env]",
                })
                .unwrap_or("");

            let name = option.display_name();
            let line = if is_selected {
                Line::from(vec![
                    format!("{caret} {index}. ", index = idx + 1).cyan().dim(),
                    name.to_string().cyan(),
                    status.to_string().dim(),
                ])
            } else {
                Line::from(vec![
                    format!("  {index}. {name}", index = idx + 1).into(),
                    status.to_string().dim(),
                ])
            };
            lines.push(line);
        }

        lines.push("".into());
        lines.push("  Press Enter to configure selected provider".dim().into());
        lines.push("  Press L to list all configured providers".dim().into());
        lines.push("  Press Esc to go back".dim().into());

        if let Some(error) = &self.error {
            lines.push("".into());
            lines.push(error.as_str().red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn render_provider_list(&self, area: Rect, buf: &mut Buffer) {
        let providers =
            list_configured_providers(&self.codex_home, self.cli_auth_credentials_store_mode);

        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Configured providers".bold()]),
            "".into(),
        ];

        if providers.is_empty() {
            lines.push("  No providers configured yet.".dim().into());
        } else {
            for provider in &providers {
                let source_text = match provider.source {
                    ProviderAuthSource::Stored => "(stored)",
                    ProviderAuthSource::Environment => "(env)",
                };
                lines.push(
                    format!("  {} {}", provider.provider_id, source_text)
                        .fg(Color::Green)
                        .into(),
                );
            }
        }

        lines.push("".into());
        lines.push("  Press any key to go back".dim().into());

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    fn handle_api_key_entry_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut should_save: Option<(String, Option<ProviderOption>)> = None;
        let mut should_request_frame = false;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            if let SignInState::ApiKeyEntry(state) = &mut *guard {
                match key_event.code {
                    KeyCode::Esc => {
                        // Go back to provider selection if we came from there, otherwise to main menu
                        if state.provider.is_some() {
                            *guard = SignInState::PickProvider;
                        } else {
                            *guard = SignInState::PickMode;
                        }
                        self.error = None;
                        should_request_frame = true;
                    }
                    KeyCode::Enter => {
                        let trimmed = state.value.trim().to_string();
                        if trimmed.is_empty() {
                            self.error = Some("API key cannot be empty".to_string());
                            should_request_frame = true;
                        } else {
                            should_save = Some((trimmed, state.provider));
                        }
                    }
                    KeyCode::Backspace => {
                        if state.prepopulated_from_env {
                            state.value.clear();
                            state.prepopulated_from_env = false;
                        } else {
                            state.value.pop();
                        }
                        self.error = None;
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
                        self.error = None;
                        should_request_frame = true;
                    }
                    _ => {}
                }
                // handled; let guard drop before potential save
            } else {
                return false;
            }
        }

        if let Some((api_key, provider)) = should_save {
            self.save_api_key(api_key, provider);
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
            self.error = None;
        } else {
            return false;
        }

        drop(guard);
        self.request_frame.schedule_frame();
        true
    }

    fn start_api_key_entry(&mut self) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }
        self.error = None;
        let prefill_from_env = read_openai_api_key_from_env();
        let mut guard = self.sign_in_state.write().unwrap();
        match &mut *guard {
            SignInState::ApiKeyEntry(state) => {
                if state.value.is_empty() {
                    if let Some(prefill) = prefill_from_env {
                        state.value = prefill;
                        state.prepopulated_from_env = true;
                    } else {
                        state.prepopulated_from_env = false;
                    }
                }
            }
            _ => {
                *guard = SignInState::ApiKeyEntry(ApiKeyInputState {
                    value: prefill_from_env.clone().unwrap_or_default(),
                    prepopulated_from_env: prefill_from_env.is_some(),
                    provider: None, // Legacy OpenAI-only flow
                });
            }
        }
        drop(guard);
        self.request_frame.schedule_frame();
    }

    fn save_api_key(&mut self, api_key: String, provider: Option<ProviderOption>) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }

        let provider_id = provider
            .as_ref()
            .map(|p| p.provider_id())
            .unwrap_or(PROVIDER_OPENAI);

        match login_with_provider_api_key(
            &self.codex_home,
            provider_id,
            &api_key,
            self.cli_auth_credentials_store_mode,
        ) {
            Ok(()) => {
                self.error = None;
                self.auth_manager.reload();

                // Set provider-specific default model
                let default_model = default_model_for_provider(provider_id);
                if let Err(e) = ConfigEditsBuilder::new(&self.codex_home)
                    .set_model(default_model, None)
                    .apply_blocking()
                {
                    tracing::error!("failed to set default model for provider {provider_id}: {e}");
                }

                // Update login status and go to configured screen for all providers
                self.login_status = LoginStatus::AuthMode(AuthMode::ApiKey);
                *self.sign_in_state.write().unwrap() = SignInState::ApiKeyConfigured;
            }
            Err(err) => {
                self.error = Some(format!("Failed to save API key: {err}"));
                let mut guard = self.sign_in_state.write().unwrap();
                if let SignInState::ApiKeyEntry(existing) = &mut *guard {
                    if existing.value.is_empty() {
                        existing.value.push_str(&api_key);
                    }
                    existing.prepopulated_from_env = false;
                } else {
                    *guard = SignInState::ApiKeyEntry(ApiKeyInputState {
                        value: api_key,
                        prepopulated_from_env: false,
                        provider,
                    });
                }
            }
        }

        self.request_frame.schedule_frame();
    }

    fn handle_existing_chatgpt_login(&mut self) -> bool {
        if matches!(
            self.login_status,
            LoginStatus::AuthMode(AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens)
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

        self.error = None;
        let opts = ServerOptions::new(
            self.codex_home.clone(),
            CLIENT_ID.to_string(),
            self.forced_chatgpt_workspace_id.clone(),
            self.cli_auth_credentials_store_mode,
        );

        match run_login_server(opts) {
            Ok(child) => {
                let sign_in_state = self.sign_in_state.clone();
                let request_frame = self.request_frame.clone();
                let auth_manager = self.auth_manager.clone();
                let codex_home = self.codex_home.clone();
                tokio::spawn(async move {
                    let auth_url = child.auth_url.clone();
                    {
                        *sign_in_state.write().unwrap() =
                            SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
                                auth_url,
                                shutdown_flag: Some(child.cancel_handle()),
                            });
                    }
                    request_frame.schedule_frame();
                    let r = child.block_until_done().await;
                    match r {
                        Ok(()) => {
                            // Force the auth manager to reload the new auth information.
                            auth_manager.reload();

                            // Clear model to use remote default for ChatGPT
                            if let Err(e) = ConfigEditsBuilder::new(&codex_home)
                                .set_model(None, None)
                                .apply_blocking()
                            {
                                tracing::error!("failed to clear model on ChatGPT login: {e}");
                            }

                            *sign_in_state.write().unwrap() = SignInState::ChatGptSuccessMessage;
                            request_frame.schedule_frame();
                        }
                        _ => {
                            *sign_in_state.write().unwrap() = SignInState::PickMode;
                            // self.error = Some(e.to_string());
                            request_frame.schedule_frame();
                        }
                    }
                });
            }
            Err(e) => {
                *self.sign_in_state.write().unwrap() = SignInState::PickMode;
                self.error = Some(e.to_string());
                self.request_frame.schedule_frame();
            }
        }
    }

    fn start_device_code_login(&mut self) {
        if self.handle_existing_chatgpt_login() {
            return;
        }

        self.error = None;
        let opts = ServerOptions::new(
            self.codex_home.clone(),
            CLIENT_ID.to_string(),
            self.forced_chatgpt_workspace_id.clone(),
            self.cli_auth_credentials_store_mode,
        );
        headless_chatgpt_login::start_headless_chatgpt_login(self, opts);
    }
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
            | SignInState::PickProvider
            | SignInState::ProviderList => StepState::InProgress,
            SignInState::ChatGptSuccess | SignInState::ApiKeyConfigured => StepState::Complete,
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
            SignInState::ApiKeyEntry(state) => {
                self.render_api_key_entry(area, buf, state);
            }
            SignInState::ApiKeyConfigured => {
                self.render_api_key_configured(area, buf);
            }
            SignInState::PickProvider => {
                self.render_pick_provider(area, buf);
            }
            SignInState::ProviderList => {
                self.render_provider_list(area, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use codex_core::auth::AuthCredentialsStoreMode;

    fn widget_forced_chatgpt() -> (AuthModeWidget, TempDir) {
        let codex_home = TempDir::new().unwrap();
        let codex_home_path = codex_home.path().to_path_buf();
        let widget = AuthModeWidget {
            request_frame: FrameRequester::test_dummy(),
            highlighted_mode: SignInOption::ChatGpt,
            highlighted_provider: ProviderOption::OpenAI,
            error: None,
            sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
            codex_home: codex_home_path.clone(),
            cli_auth_credentials_store_mode: AuthCredentialsStoreMode::File,
            login_status: LoginStatus::NotAuthenticated,
            auth_manager: AuthManager::shared(
                codex_home_path,
                false,
                AuthCredentialsStoreMode::File,
            ),
            forced_chatgpt_workspace_id: None,
            forced_login_method: Some(ForcedLoginMethod::Chatgpt),
            animations_enabled: true,
        };
        (widget, codex_home)
    }

    #[test]
    fn api_key_flow_disabled_when_chatgpt_forced() {
        let (mut widget, _tmp) = widget_forced_chatgpt();

        widget.start_api_key_entry();

        assert_eq!(widget.error.as_deref(), Some(API_KEY_DISABLED_MESSAGE));
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
    }

    #[test]
    fn saving_api_key_is_blocked_when_chatgpt_forced() {
        let (mut widget, _tmp) = widget_forced_chatgpt();

        widget.save_api_key("sk-test".to_string(), None);

        assert_eq!(widget.error.as_deref(), Some(API_KEY_DISABLED_MESSAGE));
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
        assert_eq!(widget.login_status, LoginStatus::NotAuthenticated);
    }
}
