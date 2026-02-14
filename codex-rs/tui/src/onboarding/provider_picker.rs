#![allow(clippy::unwrap_used)]

use codex_core::auth::PROVIDER_ANTHROPIC;
use codex_core::auth::PROVIDER_GEMINI;
use codex_core::auth::PROVIDER_OPENAI;
use codex_core::auth::ProviderAuthSource;
use codex_core::auth::list_configured_providers;
use codex_core::auth::provider_env_var;
use codex_core::auth::read_api_key_from_env;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

use super::auth::ApiKeyInputState;
use super::auth::AuthModeWidget;
use super::auth::ProviderAuthMethod;
use super::auth::SignInState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderOption {
    OpenAI,
    Anthropic,
    Gemini,
}

impl ProviderOption {
    pub(crate) fn provider_id(self) -> &'static str {
        match self {
            Self::OpenAI => PROVIDER_OPENAI,
            Self::Anthropic => PROVIDER_ANTHROPIC,
            Self::Gemini => PROVIDER_GEMINI,
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::OpenAI => "OpenAI",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Google Gemini",
        }
    }

    fn env_var_name(self) -> Option<&'static str> {
        provider_env_var(self.provider_id())
    }
}

pub(super) fn api_key_entry_provider_details(
    provider: Option<ProviderOption>,
) -> (&'static str, Option<&'static str>) {
    match provider {
        Some(provider) => (provider.display_name(), provider.env_var_name()),
        None => ("OpenAI", Some("OPENAI_API_KEY")),
    }
}

pub(super) fn api_key_entry_return_state(provider: Option<ProviderOption>) -> SignInState {
    if provider.is_some() {
        SignInState::PickProvider
    } else {
        SignInState::PickMode
    }
}

const PROVIDER_OPTIONS: [ProviderOption; 3] = [
    ProviderOption::OpenAI,
    ProviderOption::Anthropic,
    ProviderOption::Gemini,
];

fn provider_options() -> &'static [ProviderOption] {
    &PROVIDER_OPTIONS
}

impl AuthModeWidget {
    pub(super) fn handle_provider_picker_key_event(
        &mut self,
        key_event: &KeyEvent,
        sign_in_state: &SignInState,
    ) -> bool {
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
                    self.start_provider_configuration(ProviderOption::OpenAI);
                }
                KeyCode::Char('2') => {
                    self.start_provider_configuration(ProviderOption::Anthropic);
                }
                KeyCode::Char('3') => {
                    self.start_provider_configuration(ProviderOption::Gemini);
                }
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    self.show_provider_list();
                }
                KeyCode::Enter => {
                    self.start_provider_configuration(self.highlighted_provider);
                }
                KeyCode::Esc => {
                    *self.sign_in_state.write().unwrap() = SignInState::PickMode;
                    self.request_frame.schedule_frame();
                }
                _ => {}
            }
            return true;
        }

        if let SignInState::PickProviderAuthMethod(provider) = sign_in_state {
            match key_event.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_provider_auth_method_highlight(-1);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_provider_auth_method_highlight(1);
                    self.request_frame.schedule_frame();
                }
                KeyCode::Char('1') => {
                    self.start_provider_api_key_entry(*provider);
                }
                KeyCode::Char('2') => {
                    self.start_provider_oauth_login(*provider);
                }
                KeyCode::Enter => match self.highlighted_provider_auth_method {
                    ProviderAuthMethod::ApiKey => self.start_provider_api_key_entry(*provider),
                    ProviderAuthMethod::Oauth => self.start_provider_oauth_login(*provider),
                },
                KeyCode::Esc => {
                    *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
                    self.request_frame.schedule_frame();
                }
                _ => {}
            }
            return true;
        }

        if matches!(sign_in_state, SignInState::ProviderList) {
            *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
            self.request_frame.schedule_frame();
            return true;
        }

        false
    }

    pub(super) fn start_provider_selection(&mut self) {
        self.error = None;
        self.highlighted_provider = ProviderOption::OpenAI;
        self.highlighted_provider_auth_method = ProviderAuthMethod::ApiKey;
        *self.sign_in_state.write().unwrap() = SignInState::PickProvider;
        self.request_frame.schedule_frame();
    }

    pub(super) fn render_pick_provider(&self, area: Rect, buf: &mut Buffer) {
        let providers =
            list_configured_providers(&self.codex_home, self.cli_auth_credentials_store_mode);

        let mut lines: Vec<Line> = vec![
            Line::from(vec!["> ".into(), "Configure API keys for providers".bold()]),
            "".into(),
            "  Select a provider to configure:".into(),
            "".into(),
        ];

        for (idx, option) in provider_options().iter().copied().enumerate() {
            let is_selected = self.highlighted_provider == option;
            let caret = if is_selected { ">" } else { " " };

            let status = providers
                .iter()
                .find(|provider| provider.provider_id == option.provider_id())
                .map(|provider| match provider.source {
                    ProviderAuthSource::Stored => " [stored]",
                    ProviderAuthSource::Environment => " [env]",
                })
                .unwrap_or("");

            let index = idx + 1;
            let line = if is_selected {
                Line::from(vec![
                    format!("{caret} {index}. ").cyan().dim(),
                    option.display_name().cyan(),
                    status.dim(),
                ])
            } else {
                let name = option.display_name();
                Line::from(vec![format!("  {index}. {name}").into(), status.dim()])
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

        for (idx, (method, title, description)) in [
            (
                ProviderAuthMethod::ApiKey,
                "API key",
                "Use usage-based billing with your own API key",
            ),
            (
                ProviderAuthMethod::Oauth,
                "OAuth (Gemini Code Assist)",
                "Sign in with your Google account and use Code Assist",
            ),
        ]
        .iter()
        .copied()
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

        if let Some(error) = &self.error {
            lines.push("".into());
            lines.push(error.as_str().red().into());
        }

        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    pub(super) fn render_provider_list(&self, area: Rect, buf: &mut Buffer) {
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
                let provider_id = &provider.provider_id;
                lines.push(
                    format!("  {provider_id} {source_text}")
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

    fn move_provider_highlight(&mut self, delta: isize) {
        let options = provider_options();
        let current_index = options
            .iter()
            .position(|option| *option == self.highlighted_provider)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_provider = options[next_index];
    }

    fn move_provider_auth_method_highlight(&mut self, delta: isize) {
        let options = [ProviderAuthMethod::ApiKey, ProviderAuthMethod::Oauth];
        let current_index = options
            .iter()
            .position(|option| *option == self.highlighted_provider_auth_method)
            .unwrap_or(0);
        let next_index =
            (current_index as isize + delta).rem_euclid(options.len() as isize) as usize;
        self.highlighted_provider_auth_method = options[next_index];
    }

    fn start_provider_configuration(&mut self, provider: ProviderOption) {
        if provider == ProviderOption::Gemini {
            self.start_provider_auth_method_picker(provider);
        } else {
            self.start_provider_api_key_entry(provider);
        }
    }

    fn start_provider_auth_method_picker(&mut self, provider: ProviderOption) {
        self.error = None;
        self.highlighted_provider_auth_method = ProviderAuthMethod::ApiKey;
        *self.sign_in_state.write().unwrap() = SignInState::PickProviderAuthMethod(provider);
        self.request_frame.schedule_frame();
    }

    fn start_provider_api_key_entry(&mut self, provider: ProviderOption) {
        self.error = None;
        let prefill_from_env = read_api_key_from_env(provider.provider_id());
        *self.sign_in_state.write().unwrap() = SignInState::ApiKeyEntry(ApiKeyInputState::new(
            prefill_from_env.clone().unwrap_or_default(),
            prefill_from_env.is_some(),
            Some(provider),
        ));
        self.request_frame.schedule_frame();
    }

    fn show_provider_list(&mut self) {
        self.error = None;
        *self.sign_in_state.write().unwrap() = SignInState::ProviderList;
        self.request_frame.schedule_frame();
    }
}
