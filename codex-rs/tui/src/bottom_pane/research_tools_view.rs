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

use codex_features::Feature;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height;
use super::selection_popup_common::render_rows;

/// Per-category research features with display metadata.
///
/// Each entry: (feature flag, display name, description, optional setup hint).
/// The setup hint is shown when a feature requires external configuration.
const RESEARCH_FEATURES: &[(Feature, &str, &str, &str)] = &[
    (
        Feature::ResearchPaperSearch,
        "Paper Search",
        "Search academic papers via Semantic Scholar, arXiv, OpenAlex",
        "",
    ),
    (
        Feature::ResearchZotero,
        "Zotero",
        "Zotero library search, notes, annotations, citations",
        "config: zotero_api_key + zotero_user_id",
    ),
    (
        Feature::ResearchHackerNews,
        "Hacker News",
        "Search and browse Hacker News stories and comments",
        "",
    ),
    (
        Feature::ResearchPatents,
        "Patents",
        "Search worldwide patent data from 90+ patent offices",
        "config: EPO_CONSUMER_KEY + EPO_CONSUMER_SECRET",
    ),
    (
        Feature::ResearchRepoAnalysis,
        "Repo Analysis",
        "Clone, summarize, and analyze code repositories",
        "",
    ),
    (
        Feature::ResearchKnowledgeBase,
        "Knowledge Base",
        "Persist research cards, journal entries, and cross-paper reports",
        "",
    ),
];

pub(crate) struct ResearchToolItem {
    pub feature: Feature,
    pub name: &'static str,
    pub description: &'static str,
    pub setup_hint: &'static str,
    pub enabled: bool,
    /// For the Reading View item, tracks the current mode instead of just on/off.
    pub reading_view_mode: Option<crate::app_event::ReadingViewMode>,
}

pub(crate) struct ResearchToolsView {
    items: Vec<ResearchToolItem>,
    state: ScrollState,
    complete: bool,
    app_event_tx: AppEventSender,
    header: Box<dyn Renderable>,
    footer_hint: Line<'static>,
}

impl ResearchToolsView {
    pub(crate) fn new(items: Vec<ResearchToolItem>, app_event_tx: AppEventSender) -> Self {
        Self::with_header(
            items,
            app_event_tx,
            "Research tools",
            "Toggle research tool integrations. Changes are saved to config.toml.",
        )
    }

    pub(crate) fn new_reading_view(
        items: Vec<ResearchToolItem>,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self::with_header(
            items,
            app_event_tx,
            "Reading view",
            "Choose how long-form documents are displayed. Changes are saved to config.toml.",
        )
    }

    fn with_header(
        items: Vec<ResearchToolItem>,
        app_event_tx: AppEventSender,
        title: &'static str,
        subtitle: &'static str,
    ) -> Self {
        let mut header = ColumnRenderable::new();
        header.push(Line::from(title.bold()));
        header.push(Line::from(subtitle.dim()));

        let mut view = Self {
            items,
            state: ScrollState::new(),
            complete: false,
            app_event_tx,
            header: Box::new(header),
            footer_hint: research_popup_hint_line(),
        };
        view.initialize_selection();
        view
    }

    fn initialize_selection(&mut self) {
        if self.items.is_empty() {
            self.state.selected_idx = None;
        } else if self.state.selected_idx.is_none() {
            self.state.selected_idx = Some(0);
        }
    }

    #[cfg(test)]
    pub(crate) fn select_feature(&mut self, feature: Feature) {
        let Some(idx) = self.items.iter().position(|item| item.feature == feature) else {
            return;
        };
        self.state.selected_idx = Some(idx);
        let len = self.visible_len();
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn visible_len(&self) -> usize {
        self.items.len()
    }

    fn build_rows(&self) -> Vec<GenericDisplayRow> {
        let mut rows = Vec::with_capacity(self.items.len());
        let selected_idx = self.state.selected_idx;
        for (idx, item) in self.items.iter().enumerate() {
            let prefix = if selected_idx == Some(idx) {
                '›'
            } else {
                ' '
            };
            let name = if let Some(mode) = item.reading_view_mode {
                format!("{prefix} [{}] {}", mode.label(), item.name)
            } else {
                let marker = if item.enabled { 'x' } else { ' ' };
                format!("{prefix} [{marker}] {}", item.name)
            };
            let description = if !item.enabled && !item.setup_hint.is_empty() {
                format!("{} ({})", item.description, item.setup_hint)
            } else {
                item.description.to_string()
            };
            rows.push(GenericDisplayRow {
                name,
                description: Some(description),
                ..Default::default()
            });
        }
        rows
    }

    fn move_up(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn move_down(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            return;
        }
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    fn toggle_selected(&mut self) {
        let Some(selected_idx) = self.state.selected_idx else {
            return;
        };
        if let Some(item) = self.items.get_mut(selected_idx) {
            if let Some(ref mut mode) = item.reading_view_mode {
                // Cycle: tui -> browser -> disabled -> tui
                *mode = match *mode {
                    crate::app_event::ReadingViewMode::Tui => {
                        crate::app_event::ReadingViewMode::Browser
                    }
                    crate::app_event::ReadingViewMode::Browser => {
                        crate::app_event::ReadingViewMode::Disabled
                    }
                    crate::app_event::ReadingViewMode::Disabled => {
                        crate::app_event::ReadingViewMode::Tui
                    }
                };
                item.enabled = *mode != crate::app_event::ReadingViewMode::Disabled;
            } else {
                item.enabled = !item.enabled;
            }
        }
    }

    fn rows_width(total_width: u16) -> u16 {
        total_width.saturating_sub(2)
    }
}

impl BottomPaneView for ResearchToolsView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
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
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.toggle_selected(),
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.on_ctrl_c();
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if !self.items.is_empty() {
            let mut updates: Vec<(Feature, bool)> = self
                .items
                .iter()
                .filter(|item| item.feature != Feature::ReadingView)
                .map(|item| (item.feature, item.enabled))
                .collect();
            if !updates.is_empty() {
                // Clear master research flag so per-category flags take sole authority.
                updates.push((Feature::Research, false));
                self.app_event_tx
                    .send(AppEvent::UpdateFeatureFlags { updates });
            }

            // Propagate reading view mode change if present.
            if let Some(rv_item) = self
                .items
                .iter()
                .find(|i| i.feature == Feature::ReadingView)
                && let Some(mode) = rv_item.reading_view_mode
            {
                self.app_event_tx
                    .send(AppEvent::ReadingViewModeChanged(mode));
            }
        }
        self.complete = true;
        CancellationEvent::Handled
    }
}

impl Renderable for ResearchToolsView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);

        Block::default()
            .style(user_message_style())
            .render(content_area, buf);

        let header_height = self
            .header
            .desired_height(content_area.width.saturating_sub(4));
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

        self.header.render(header_area, buf);

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
                "  No research tools available for now",
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
        let rows = self.build_rows();
        let rows_width = Self::rows_width(width);
        let rows_height = measure_rows_height(
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            rows_width.saturating_add(1),
        );

        let mut height = self.header.desired_height(width.saturating_sub(4));
        height = height.saturating_add(rows_height + 3);
        height.saturating_add(1)
    }
}

/// Build `ResearchToolItem`s from the hardcoded feature list, using current config state.
pub(crate) fn build_research_tool_items(
    features: &codex_features::Features,
) -> Vec<ResearchToolItem> {
    RESEARCH_FEATURES
        .iter()
        .map(|&(feature, name, description, setup_hint)| {
            let enabled = if feature == Feature::ResearchKnowledgeBase {
                features.enabled(feature)
            } else {
                features.enabled(Feature::Research) || features.enabled(feature)
            };
            ResearchToolItem {
                feature,
                name,
                description,
                setup_hint,
                enabled,
                reading_view_mode: None,
            }
        })
        .collect()
}

pub(crate) fn build_reading_view_tool_items(
    reading_view_mode: crate::app_event::ReadingViewMode,
) -> Vec<ResearchToolItem> {
    vec![ResearchToolItem {
        feature: Feature::ReadingView,
        name: "Reading View",
        description: "Present output as a navigable document with foldable sections",
        setup_hint: "",
        enabled: reading_view_mode != crate::app_event::ReadingViewMode::Disabled,
        reading_view_mode: Some(reading_view_mode),
    }]
}

fn research_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        "Press ".into(),
        key_hint::plain(KeyCode::Char(' ')).into(),
        " to select or ".into(),
        key_hint::plain(KeyCode::Enter).into(),
        " to save for next conversation".into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn reading_view_mode_is_not_persisted_as_feature_flag() {
        let (tx, mut rx) = unbounded_channel();
        let app_event_tx = AppEventSender::new(tx);
        let mut view = ResearchToolsView::new(
            vec![
                ResearchToolItem {
                    feature: Feature::ReadingView,
                    name: "Reading View",
                    description: "Display long research syntheses",
                    setup_hint: "",
                    enabled: false,
                    reading_view_mode: Some(crate::app_event::ReadingViewMode::Disabled),
                },
                ResearchToolItem {
                    feature: Feature::ResearchPaperSearch,
                    name: "Paper Search",
                    description: "Search papers",
                    setup_hint: "",
                    enabled: true,
                    reading_view_mode: None,
                },
            ],
            app_event_tx,
        );

        view.on_ctrl_c();

        let first = rx.try_recv().expect("feature flag update");
        let AppEvent::UpdateFeatureFlags { updates } = first else {
            panic!("expected UpdateFeatureFlags");
        };
        assert!(
            !updates
                .iter()
                .any(|(feature, _)| *feature == Feature::ReadingView)
        );
        assert!(updates.contains(&(Feature::ResearchPaperSearch, true)));
        assert!(updates.contains(&(Feature::Research, false)));

        let second = rx.try_recv().expect("reading view mode update");
        assert!(matches!(
            second,
            AppEvent::ReadingViewModeChanged(crate::app_event::ReadingViewMode::Disabled)
        ));
    }

    #[test]
    fn select_feature_focuses_reading_view_row() {
        let (tx, _rx) = unbounded_channel();
        let app_event_tx = AppEventSender::new(tx);
        let mut view = ResearchToolsView::new(
            build_reading_view_tool_items(crate::app_event::ReadingViewMode::Tui),
            app_event_tx,
        );

        view.select_feature(Feature::ReadingView);

        let selected_idx = view.state.selected_idx.expect("selected row");
        assert_eq!(view.items[selected_idx].feature, Feature::ReadingView);
        assert!(
            view.build_rows()[selected_idx]
                .name
                .starts_with("› [tui] Reading View")
        );
    }
}
