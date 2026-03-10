use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
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

enum MobileSetupItemKind {
    /// Numeric port value adjusted with left/right arrows.
    Port { value: u16 },
    /// Editable text field for the auth token.
    Token {
        value: String,
        editing: bool,
        edit_buffer: String,
    },
    /// Boolean toggle (Space/Enter to flip).
    Toggle { value: bool },
}

/// Which logical setting this item represents.
enum MobileSetupItemId {
    Enabled,
    Port,
    Token,
    Background,
}

struct MobileSetupItem {
    item_id: MobileSetupItemId,
    name: String,
    description: String,
    kind: MobileSetupItemKind,
}

pub(crate) struct MobileSetupView {
    items: Vec<MobileSetupItem>,
    state: ScrollState,
    complete: bool,
    app_event_tx: AppEventSender,
    footer_hint: Line<'static>,
    /// Whether the embedded server is already running (shown in header).
    already_running: bool,
    /// Whether the daemon was already running when the view opened.
    daemon_was_running: bool,
    /// Pre-rendered QR code lines (present when the server is running with a
    /// known token so the mobile app can scan to connect).
    qr_lines: Option<Vec<String>>,
}

impl MobileSetupView {
    pub(crate) fn new(
        port: u16,
        token: Option<String>,
        already_running: bool,
        app_event_tx: AppEventSender,
    ) -> Self {
        let token_value = token.unwrap_or_default();
        let daemon_running = crate::mobile_daemon::is_daemon_running().is_some();

        let qr_lines = if already_running {
            build_qr_lines(port, &token_value)
        } else {
            None
        };

        let items = vec![
            MobileSetupItem {
                item_id: MobileSetupItemId::Enabled,
                name: "Enabled".to_string(),
                description: "Start or stop the WebSocket server".to_string(),
                kind: MobileSetupItemKind::Toggle {
                    value: already_running,
                },
            },
            MobileSetupItem {
                item_id: MobileSetupItemId::Port,
                name: "Port".to_string(),
                description: "WebSocket listen port".to_string(),
                kind: MobileSetupItemKind::Port { value: port },
            },
            MobileSetupItem {
                item_id: MobileSetupItemId::Token,
                name: "Token".to_string(),
                description: "Auth token (empty = random)".to_string(),
                kind: MobileSetupItemKind::Token {
                    value: token_value,
                    editing: false,
                    edit_buffer: String::new(),
                },
            },
            MobileSetupItem {
                item_id: MobileSetupItemId::Background,
                name: "Background".to_string(),
                description: "Keep server running after TUI exits".to_string(),
                kind: MobileSetupItemKind::Toggle {
                    value: daemon_running,
                },
            },
        ];

        let mut view = Self {
            items,
            state: ScrollState::new(),
            complete: false,
            app_event_tx,
            footer_hint: mobile_setup_hint_line(already_running),
            already_running,
            daemon_was_running: daemon_running,
            qr_lines,
        };
        view.initialize_selection();
        view
    }

    fn build_header(&self) -> Box<dyn Renderable> {
        let mut header = ColumnRenderable::new();
        header.push(Line::from("Mobile remote control".bold()));
        header.push(Line::from(
            "Toggle Enabled to start/stop. Adjust settings, then Esc to close.".dim(),
        ));
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

    fn is_editing_token(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(&item.kind, MobileSetupItemKind::Token { editing: true, .. }))
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
                MobileSetupItemKind::Port { value } => {
                    format!(
                        "{prefix}  {:<10} [{:<13}\u{25C0} \u{25B6}]",
                        item.name, value
                    )
                }
                MobileSetupItemKind::Token {
                    value,
                    editing,
                    edit_buffer,
                } => {
                    if *editing {
                        let display = if edit_buffer.is_empty() {
                            "type token...".to_string()
                        } else {
                            edit_buffer.clone()
                        };
                        format!("{prefix}  {:<10} [{:<15}\u{2588}]", item.name, display)
                    } else if value.is_empty() {
                        format!("{prefix}  {:<10} [{:<16}]", item.name, "(random)")
                    } else {
                        format!("{prefix}  {:<10} [{:<16}]", item.name, mask_token(value))
                    }
                }
                MobileSetupItemKind::Toggle { value } => {
                    let indicator = if *value {
                        "\u{25C9} on"
                    } else {
                        "\u{25CB} off"
                    };
                    format!("{prefix}  {:<10} [{:<16}]", item.name, indicator)
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
        if self.is_editing_token() {
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
        if self.is_editing_token() {
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
            let is_enabled_toggle = matches!(item.item_id, MobileSetupItemId::Enabled);
            match &mut item.kind {
                MobileSetupItemKind::Port { value } => {
                    *value = value.saturating_add(1);
                }
                MobileSetupItemKind::Token {
                    editing,
                    edit_buffer,
                    value,
                } => {
                    if !*editing {
                        *editing = true;
                        *edit_buffer = value.clone();
                        self.footer_hint = token_edit_hint_line();
                    }
                }
                MobileSetupItemKind::Toggle { value } => {
                    *value = !*value;
                }
            }
            if is_enabled_toggle {
                self.refresh_qr();
            }
        }
    }

    /// Recompute the QR code based on current Enabled/port/token state.
    fn refresh_qr(&mut self) {
        if self.server_enabled() || self.already_running {
            self.qr_lines = build_qr_lines(
                self.current_port(),
                &self.current_token().unwrap_or_default(),
            );
        } else {
            self.qr_lines = None;
        }
    }

    fn adjust_left(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            if let MobileSetupItemKind::Port { value } = &mut item.kind {
                *value = value.saturating_sub(1).max(1024);
            }
        }
    }

    fn adjust_right(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            if let MobileSetupItemKind::Port { value } = &mut item.kind {
                *value = value.saturating_add(1).min(65535);
            }
        }
    }

    fn handle_token_edit(&mut self, key_event: KeyEvent) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        let Some(item) = self.items.get_mut(selected_idx) else {
            return;
        };
        let MobileSetupItemKind::Token {
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
                *value = edit_buffer.trim().to_string();
                *editing = false;
                self.footer_hint = mobile_setup_hint_line(self.already_running);
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                *editing = false;
                edit_buffer.clear();
                self.footer_hint = mobile_setup_hint_line(self.already_running);
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                edit_buffer.pop();
            }
            KeyEvent {
                code: KeyCode::Char('v'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => {
                // Bracket paste fallback — actual paste via Char events.
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

    fn current_port(&self) -> u16 {
        for item in &self.items {
            if let MobileSetupItemKind::Port { value } = &item.kind {
                return *value;
            }
        }
        19285
    }

    fn current_token(&self) -> Option<String> {
        for item in &self.items {
            if let MobileSetupItemKind::Token { value, .. } = &item.kind {
                if value.is_empty() {
                    return None;
                }
                return Some(value.clone());
            }
        }
        None
    }

    fn server_enabled(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(item.item_id, MobileSetupItemId::Enabled)
                && matches!(&item.kind, MobileSetupItemKind::Toggle { value: true })
        })
    }

    fn daemon_enabled(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(item.item_id, MobileSetupItemId::Background)
                && matches!(&item.kind, MobileSetupItemKind::Toggle { value: true })
        })
    }
}

impl BottomPaneView for MobileSetupView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.is_editing_token() {
            self.handle_token_edit(key_event);
            return;
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.move_up(),
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
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
                code: KeyCode::Char(' ') | KeyCode::Enter,
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
        if !self.is_editing_token() {
            return false;
        }
        let Some(idx) = self.state.selected_idx else {
            return false;
        };
        let Some(item) = self.items.get_mut(idx) else {
            return false;
        };
        if let MobileSetupItemKind::Token { edit_buffer, .. } = &mut item.kind {
            edit_buffer.push_str(pasted.trim());
            return true;
        }
        false
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        let enabled = self.server_enabled();
        let daemon_wanted = self.daemon_enabled();
        let port = self.current_port();
        let token = self.current_token();

        // Handle daemon toggle changes.
        if daemon_wanted && !self.daemon_was_running {
            self.app_event_tx.send(AppEvent::StartMobileDaemon { port });
        } else if !daemon_wanted && self.daemon_was_running {
            self.app_event_tx.send(AppEvent::StopMobileDaemon);
        }

        // Handle embedded server — only act on explicit Enabled toggle changes.
        if enabled && !self.already_running {
            // User explicitly turned the server ON.
            self.app_event_tx
                .send(AppEvent::StartMobileServer { port, token });
        } else if !enabled && self.already_running {
            // User explicitly turned the server OFF.
            self.app_event_tx.send(AppEvent::StopMobileServer);
        }
        // If enabled && already_running → no change needed (settings take effect on
        // next restart). If !enabled && !already_running → nothing to do.

        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for MobileSetupView {
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
            // Determine how much space the QR code needs (if any).
            let qr_w = self
                .qr_lines
                .as_ref()
                .and_then(|lines| lines.first())
                .map_or(0, |l| l.chars().count() as u16);
            let qr_h = self.qr_lines.as_ref().map_or(0, |l| l.len() as u16);
            // 2 = gap between list and QR
            let side_by_side = qr_w > 0 && list_area.width >= rows_width + 2 + qr_w + 2;

            let list_render_width = if side_by_side {
                rows_width.saturating_sub(qr_w + 2)
            } else {
                rows_width
            };

            let render_area = Rect {
                x: list_area.x.saturating_sub(2),
                y: list_area.y,
                width: list_render_width.max(1),
                height: list_area.height,
            };
            render_rows(
                render_area,
                buf,
                &rows,
                &self.state,
                MAX_POPUP_ROWS,
                "  No settings available",
            );

            // Render QR code.
            if let Some(qr) = &self.qr_lines {
                let (qr_x, qr_y) = if side_by_side {
                    (
                        list_area.x + list_area.width.saturating_sub(qr_w + 1),
                        list_area.y,
                    )
                } else {
                    // Below the list — only if there is room.
                    (list_area.x, list_area.y + rows_height + 1)
                };
                for (i, line) in qr.iter().enumerate() {
                    let y = qr_y + i as u16;
                    if y >= content_area.y + content_area.height {
                        break;
                    }
                    let qr_line = Line::from(line.as_str().dim());
                    let qr_area = Rect {
                        x: qr_x,
                        y,
                        width: qr_w.min(content_area.width.saturating_sub(qr_x - content_area.x)),
                        height: 1,
                    };
                    qr_line.render(qr_area, buf);
                }
                // "Scan to connect" label below QR.
                let label_y = qr_y + qr_h;
                if label_y < content_area.y + content_area.height {
                    let label = Line::from("Scan to connect".dim());
                    let label_area = Rect {
                        x: qr_x,
                        y: label_y,
                        width: qr_w.min(content_area.width.saturating_sub(qr_x - content_area.x)),
                        height: 1,
                    };
                    label.render(label_area, buf);
                }
            }
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

        // If QR code won't fit side-by-side, add space below.
        if let Some(qr) = &self.qr_lines {
            let qr_w = qr.first().map_or(0, |l| l.chars().count() as u16);
            let qr_h = qr.len() as u16;
            let side_by_side = qr_w > 0 && width >= rows_width + 2 + qr_w + 2;
            if side_by_side {
                // QR might be taller than the list; account for the difference.
                let extra = (qr_h + 1).saturating_sub(rows_height);
                height = height.saturating_add(extra);
            } else {
                // +1 gap + qr_h + 1 label
                height = height.saturating_add(1 + qr_h + 1);
            }
        }

        height.saturating_add(1)
    }
}

/// Build QR code lines encoding connection info as an `ata://connect` deep link.
///
/// The QR encodes `ata://connect?host=<lan_ip>&port=<port>&token=<token>` so
/// that scanning with the iOS Camera app opens ATA and auto-fills the fields.
/// Also includes `openai_key` and `elevenlabs_key` from environment variables
/// when available, so the mobile app can auto-configure API keys.
/// Returns `None` if the token is empty (no useful QR to show).
fn build_qr_lines(port: u16, token: &str) -> Option<Vec<String>> {
    if token.is_empty() {
        return None;
    }
    let ip = crate::remote_control::local_lan_ip();
    let mut url = format!("ata://connect?host={ip}&port={port}&token={token}");
    if let Ok(openai_key) = std::env::var("OPENAI_API_KEY") {
        if !openai_key.is_empty() {
            url.push_str(&format!("&openai_key={openai_key}"));
        }
    }
    if let Ok(elevenlabs_key) = std::env::var("ELEVENLABS_API_KEY") {
        if !elevenlabs_key.is_empty() {
            url.push_str(&format!("&elevenlabs_key={elevenlabs_key}"));
        }
    }
    let lines = crate::qr_render::render_qr_to_lines(&url);
    if lines.is_empty() { None } else { Some(lines) }
}

fn mask_token(token: &str) -> String {
    let len = token.len();
    if len <= 8 {
        "\u{2022}".repeat(len)
    } else {
        let start: String = token.chars().take(4).collect();
        let end: String = token.chars().skip(len - 4).collect();
        format!("{start}{}{end}", "\u{2022}".repeat(4))
    }
}

fn mobile_setup_hint_line(_already_running: bool) -> Line<'static> {
    Line::from(vec![
        key_hint::plain(KeyCode::Char(' ')).into(),
        " toggle  ".into(),
        key_hint::plain(KeyCode::Left).into(),
        key_hint::plain(KeyCode::Right).into(),
        " adjust  ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " close".into(),
    ])
}

fn token_edit_hint_line() -> Line<'static> {
    Line::from(vec![
        "Type token, ".into(),
        key_hint::plain(KeyCode::Enter).into(),
        " to confirm, ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " to cancel".into(),
    ])
}
