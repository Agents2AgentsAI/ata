use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::key_hint;
use crate::render::Insets;
use crate::render::RectExt as _;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height;
use super::selection_popup_common::render_rows;

enum VoiceSetupItemKind {
    Toggle {
        enabled: bool,
    },
    ApiKey {
        value: String,
        editing: bool,
        edit_buffer: String,
    },
    /// Cycle through a list of options with Space/Enter or left/right arrows.
    Selection {
        current_idx: usize,
        /// (value, display_label) pairs.
        options: Vec<(String, String)>,
    },
    /// Numeric stepper with left/right arrows.
    Stepper {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    },
}

struct VoiceSetupItem {
    name: String,
    description: String,
    kind: VoiceSetupItemKind,
}

pub(crate) struct VoiceSetupView {
    items: Vec<VoiceSetupItem>,
    state: ScrollState,
    complete: bool,
    app_event_tx: AppEventSender,
    /// Whether an API key was available at construction (or has been set).
    api_key_available: bool,
    /// The original API key value at construction — used to detect changes.
    original_api_key: String,
    footer_hint: Line<'static>,
}

/// Language options for the Selection item.
const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("auto", "Auto-detect"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ko", "Korean"),
    ("pt", "Portuguese"),
    ("it", "Italian"),
    ("ar", "Arabic"),
];

impl VoiceSetupView {
    pub(crate) fn new(
        tts_enabled: bool,
        stt_enabled: bool,
        api_key: Option<String>,
        language_code: Option<String>,
        speed: Option<f64>,
        app_event_tx: AppEventSender,
    ) -> Self {
        // Use key from config/caller, falling back to env var.
        let existing_key = api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
            .unwrap_or_default();
        let api_key_available = !existing_key.is_empty();

        let lang_options: Vec<(String, String)> = LANGUAGE_OPTIONS
            .iter()
            .map(|(v, l)| (v.to_string(), l.to_string()))
            .collect();
        let lang_idx = language_code
            .as_deref()
            .and_then(|code| lang_options.iter().position(|(v, _)| v == code))
            .unwrap_or(0); // default to "auto"

        let items = vec![
            VoiceSetupItem {
                name: "TTS".to_string(),
                description: "Agent responses read aloud".to_string(),
                kind: VoiceSetupItemKind::Toggle {
                    enabled: tts_enabled,
                },
            },
            VoiceSetupItem {
                name: "STT".to_string(),
                description: "Hold Space to speak".to_string(),
                kind: VoiceSetupItemKind::Toggle {
                    enabled: stt_enabled,
                },
            },
            VoiceSetupItem {
                name: "API Key".to_string(),
                description: "ElevenLabs key for TTS".to_string(),
                kind: VoiceSetupItemKind::ApiKey {
                    value: existing_key.clone(),
                    editing: false,
                    edit_buffer: String::new(),
                },
            },
            VoiceSetupItem {
                name: "Language".to_string(),
                description: "TTS language (auto-detect if unset)".to_string(),
                kind: VoiceSetupItemKind::Selection {
                    current_idx: lang_idx,
                    options: lang_options,
                },
            },
            VoiceSetupItem {
                name: "Speed".to_string(),
                description: "Speech rate (0.7\u{00D7} \u{2013} 1.2\u{00D7})".to_string(),
                kind: VoiceSetupItemKind::Stepper {
                    value: speed.unwrap_or(1.0),
                    min: 0.7,
                    max: 1.2,
                    step: 0.1,
                },
            },
        ];

        let mut view = Self {
            items,
            state: ScrollState::new(),
            complete: false,
            app_event_tx,
            api_key_available,
            original_api_key: existing_key,
            footer_hint: voice_setup_hint_line(),
        };
        view.initialize_selection();
        view
    }

    fn build_header(&self) -> Box<dyn Renderable> {
        let mut header = ColumnRenderable::new();
        header.push(Line::from("Voice mode setup".bold()));
        header.push(Line::from(
            "Toggle TTS and STT independently. Changes are saved to config.toml.".dim(),
        ));

        if !self.api_key_available {
            header.push(Line::from(""));
            header.push(Line::from(vec![Span::styled(
                "\u{26A0} ElevenLabs API key not found.",
                ratatui::style::Style::default().fg(Color::Yellow),
            )]));
            header.push(Line::from(vec![Span::styled(
                "Paste your key below or set ELEVENLABS_API_KEY env var.",
                ratatui::style::Style::default().fg(Color::Yellow),
            )]));
            header.push(Line::from(vec![Span::styled(
                "Get a key at: https://elevenlabs.io/app/settings/api-keys",
                ratatui::style::Style::default().fg(Color::Yellow),
            )]));
        }

        Box::new(header)
    }

    fn initialize_selection(&mut self) {
        if self.items.is_empty() {
            self.state.selected_idx = None;
        } else if self.state.selected_idx.is_none() {
            self.state.selected_idx = Some(0);
        }
    }

    fn visible_len(&self) -> usize {
        self.items.len()
    }

    /// Check if any API key item is in editing mode.
    fn is_editing_api_key(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(&item.kind, VoiceSetupItemKind::ApiKey { editing: true, .. }))
    }

    fn build_rows(&self) -> Vec<GenericDisplayRow> {
        let mut rows = Vec::with_capacity(self.items.len());
        let selected_idx = self.state.selected_idx;
        for (idx, item) in self.items.iter().enumerate() {
            let prefix = if selected_idx == Some(idx) {
                '\u{203A}'
            } else {
                ' '
            };
            let name = match &item.kind {
                VoiceSetupItemKind::Toggle { enabled } => {
                    let marker = if *enabled { 'x' } else { ' ' };
                    format!("{prefix}  [{marker}] {}", item.name)
                }
                VoiceSetupItemKind::ApiKey {
                    value,
                    editing,
                    edit_buffer,
                } => {
                    if *editing {
                        let display = if edit_buffer.is_empty() {
                            "type or paste key...".to_string()
                        } else {
                            mask_key(edit_buffer)
                        };
                        format!("{prefix}  {:<10} [{:<15}\u{2588}]", item.name, display)
                    } else if value.is_empty() {
                        format!("{prefix}  {:<10} [{:<16}]", item.name, "paste key here")
                    } else {
                        format!("{prefix}  {:<10} [{:<16}]", item.name, mask_key(value))
                    }
                }
                VoiceSetupItemKind::Selection {
                    current_idx,
                    options,
                } => {
                    let label = options
                        .get(*current_idx)
                        .map(|(_, l)| l.as_str())
                        .unwrap_or("?");
                    // Pad label to fixed width so description column stays aligned.
                    format!(
                        "{prefix}  {:<10} [{:<13}\u{25C0} \u{25B6}]",
                        item.name, label
                    )
                }
                VoiceSetupItemKind::Stepper { value, .. } => {
                    let val_str = format!("{value:.1}\u{00D7}");
                    format!(
                        "{prefix}  {:<10} [{:<13}\u{25C0} \u{25B6}]",
                        item.name, val_str
                    )
                }
            };
            rows.push(GenericDisplayRow {
                name,
                description: Some(item.description.clone()),
                ..Default::default()
            });
        }
        rows
    }

    fn move_up(&mut self) {
        if self.is_editing_api_key() {
            return;
        }
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn move_down(&mut self) {
        if self.is_editing_api_key() {
            return;
        }
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn activate_selected(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            match &mut item.kind {
                VoiceSetupItemKind::Toggle { enabled } => {
                    *enabled = !*enabled;
                }
                VoiceSetupItemKind::ApiKey {
                    editing,
                    edit_buffer,
                    value,
                } => {
                    if !*editing {
                        *editing = true;
                        *edit_buffer = value.clone();
                        self.footer_hint = api_key_edit_hint_line();
                    }
                }
                VoiceSetupItemKind::Selection {
                    current_idx,
                    options,
                } => {
                    if !options.is_empty() {
                        *current_idx = (*current_idx + 1) % options.len();
                    }
                }
                VoiceSetupItemKind::Stepper {
                    value, max, step, ..
                } => {
                    *value = (*value + *step).min(*max);
                    // Round to 1 decimal to avoid floating point drift.
                    *value = (*value * 10.0).round() / 10.0;
                }
            }
        }
    }

    /// Cycle selection backward or decrease stepper.
    fn adjust_left(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            match &mut item.kind {
                VoiceSetupItemKind::Selection {
                    current_idx,
                    options,
                } => {
                    if !options.is_empty() {
                        *current_idx = current_idx.checked_sub(1).unwrap_or(options.len() - 1);
                    }
                }
                VoiceSetupItemKind::Stepper {
                    value, min, step, ..
                } => {
                    *value = (*value - *step).max(*min);
                    *value = (*value * 10.0).round() / 10.0;
                }
                _ => {}
            }
        }
    }

    /// Cycle selection forward or increase stepper.
    fn adjust_right(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            match &mut item.kind {
                VoiceSetupItemKind::Selection {
                    current_idx,
                    options,
                } => {
                    if !options.is_empty() {
                        *current_idx = (*current_idx + 1) % options.len();
                    }
                }
                VoiceSetupItemKind::Stepper {
                    value, max, step, ..
                } => {
                    *value = (*value + *step).min(*max);
                    *value = (*value * 10.0).round() / 10.0;
                }
                _ => {}
            }
        }
    }

    fn handle_api_key_edit(&mut self, key_event: KeyEvent) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        let Some(item) = self.items.get_mut(selected_idx) else {
            return;
        };
        let VoiceSetupItemKind::ApiKey {
            value,
            editing,
            edit_buffer,
        } = &mut item.kind
        else {
            return;
        };
        if !*editing {
            return;
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                // Confirm: save the edit buffer as the new value.
                *value = edit_buffer.trim().to_string();
                *editing = false;
                if !value.is_empty() {
                    self.api_key_available = true;
                }
                self.footer_hint = voice_setup_hint_line();
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                // Cancel: discard changes.
                *editing = false;
                edit_buffer.clear();
                self.footer_hint = voice_setup_hint_line();
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                edit_buffer.pop();
            }
            // Ctrl+V paste (or Ctrl+Shift+V).
            KeyEvent {
                code: KeyCode::Char('v'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                // Terminal pastes are delivered as a bracket paste (sequence of
                // Char events), so Ctrl+V typically doesn't carry content.
                // We handle it here as a no-op fallback — the actual paste
                // arrives via Char events from the bracketed paste.
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => {
                edit_buffer.push(c);
            }
            _ => {}
        }
    }

    fn rows_width(total_width: u16) -> u16 {
        total_width.saturating_sub(2)
    }

    /// Extract the current API key value (if changed from original).
    fn changed_api_key(&self) -> Option<String> {
        for item in &self.items {
            if let VoiceSetupItemKind::ApiKey { value, .. } = &item.kind
                && !value.is_empty()
                && *value != self.original_api_key
            {
                return Some(value.clone());
            }
        }
        None
    }

    /// Extract the current language code selection.
    /// Returns `Some(code)` for a specific language, `None` for auto-detect.
    fn current_language_code(&self) -> Option<String> {
        for item in &self.items {
            if let VoiceSetupItemKind::Selection {
                current_idx,
                options,
            } = &item.kind
                && item.name == "Language"
            {
                let current_value = options.get(*current_idx).map(|(v, _)| v.as_str());
                return match current_value {
                    Some("auto") | None => None,
                    Some(code) => Some(code.to_string()),
                };
            }
        }
        None
    }

    /// Extract the current speed value.
    fn current_speed(&self) -> f64 {
        for item in &self.items {
            if let VoiceSetupItemKind::Stepper { value, .. } = &item.kind
                && item.name == "Speed"
            {
                return (*value * 10.0).round() / 10.0;
            }
        }
        1.0
    }
}

impl BottomPaneView for VoiceSetupView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        // If editing an API key, route all keys to the edit handler.
        if self.is_editing_api_key() {
            self.handle_api_key_edit(key_event);
            return;
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('\u{0010}'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_up(),
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('\u{000e}'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_down(),
            KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_down(),
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            } => self.adjust_left(),
            KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } => self.adjust_right(),
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => self.activate_selected(),
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.on_ctrl_c();
            }
            _ => {}
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if !self.is_editing_api_key() {
            return false;
        }
        // Append pasted text into the active API key edit buffer.
        let Some(idx) = self.state.selected_idx else {
            return false;
        };
        let Some(item) = self.items.get_mut(idx) else {
            return false;
        };
        if let VoiceSetupItemKind::ApiKey { edit_buffer, .. } = &mut item.kind {
            edit_buffer.push_str(pasted.trim());
            return true;
        }
        false
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        // Extract toggle states.
        let tts_enabled = self
            .items
            .iter()
            .find_map(|i| {
                if i.name == "TTS" {
                    if let VoiceSetupItemKind::Toggle { enabled } = &i.kind {
                        Some(*enabled)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or(true);
        let stt_enabled = self
            .items
            .iter()
            .find_map(|i| {
                if i.name == "STT" {
                    if let VoiceSetupItemKind::Toggle { enabled } = &i.kind {
                        Some(*enabled)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or(true);
        let api_key = self.changed_api_key();
        let language_code = self.current_language_code();
        let speed = self.current_speed();

        self.app_event_tx.send(AppEvent::UpdateVoiceSettings {
            tts_enabled,
            stt_enabled,
            elevenlabs_api_key: api_key,
            language_code: Some(language_code),
            speed: Some(speed),
        });
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for VoiceSetupView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        Block::default()
            .style(user_message_style())
            .render(content_area, buf);

        let header = self.build_header();
        let header_height = header.desired_height(content_area.width.saturating_sub(4));
        let rows = self.build_rows();
        let rows_width = Self::rows_width(content_area.width);
        let rows_height = measure_rows_height(
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            rows_width.saturating_add(1),
        );
        let [header_area, _, list_area] = Layout::vertical([
            Constraint::Max(header_height),
            Constraint::Max(1),
            Constraint::Length(rows_height),
        ])
        .areas(content_area.inset(Insets::vh(1, 2)));

        header.render(header_area, buf);

        if list_area.height > 0 {
            let render_area = Rect {
                x: list_area.x.saturating_sub(2),
                y: list_area.y,
                width: rows_width.max(1),
                height: list_area.height,
            };
            render_rows(
                render_area,
                buf,
                &rows,
                &self.state,
                MAX_POPUP_ROWS,
                "  No voice settings available",
            );
        }

        let hint_area = Rect {
            x: footer_area.x + 2,
            y: footer_area.y,
            width: footer_area.width.saturating_sub(2),
            height: footer_area.height,
        };
        self.footer_hint.clone().dim().render(hint_area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let header = self.build_header();
        let rows = self.build_rows();
        let rows_width = Self::rows_width(width);
        let rows_height = measure_rows_height(
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            rows_width.saturating_add(1),
        );

        let mut height = header.desired_height(width.saturating_sub(4));
        height = height.saturating_add(rows_height + 3);
        height.saturating_add(1)
    }
}

/// Mask an API key for display: show first 4 and last 4 chars with dots.
fn mask_key(key: &str) -> String {
    let len = key.len();
    if len <= 8 {
        "\u{2022}".repeat(len)
    } else {
        let start: String = key.chars().take(4).collect();
        let end: String = key.chars().skip(len - 4).collect();
        format!("{start}{}{end}", "\u{2022}".repeat(4))
    }
}

fn voice_setup_hint_line() -> Line<'static> {
    Line::from(vec![
        key_hint::plain(KeyCode::Char(' ')).into(),
        " select  ".into(),
        key_hint::plain(KeyCode::Left).into(),
        key_hint::plain(KeyCode::Right).into(),
        " adjust  ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " save & close".into(),
    ])
}

fn api_key_edit_hint_line() -> Line<'static> {
    Line::from(vec![
        "Type or paste key, ".into(),
        key_hint::plain(KeyCode::Enter).into(),
        " to confirm, ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " to cancel".into(),
    ])
}
