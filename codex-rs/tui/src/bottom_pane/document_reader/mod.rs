//! Sectioned reading mode for long agent-produced documents.
//!
//! The agent calls `present_document` to display a long markdown document split
//! into navigable sections. The user can browse sections, ask follow-up
//! questions via an embedded composer, and exit to leave a transcript entry.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::bottom_pane::textarea::TextArea;
use crate::bottom_pane::textarea::TextAreaState;
use crate::render::renderable::Renderable;
use codex_core::protocol::Op;
use codex_protocol::user_input::UserInput;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;
use std::cell::RefCell;
use std::collections::HashSet;

mod render;

pub(crate) const DOCUMENT_READER_VIEW_ID: &str = "doc_reader";

/// A single section of a document (split on `## ` headings).
struct DocumentSection {
    heading: String,
    content: String,
    /// Cached rendered lines; invalidated on width change or content update.
    rendered: RefCell<Option<(u16, Vec<Line<'static>>)>>,
    /// Set to `true` when this section was just updated via `update_document_section`.
    /// Cleared when the user navigates away. Used to highlight changes.
    recently_updated: bool,
}

impl DocumentSection {
    fn rendered_lines(&self, width: u16) -> Vec<Line<'static>> {
        {
            let cached = self.rendered.borrow();
            if let Some((w, lines)) = cached.as_ref()
                && *w == width
            {
                return lines.clone();
            }
        }
        let lines =
            render::render_section(&self.heading, &self.content, width, self.recently_updated);
        *self.rendered.borrow_mut() = Some((width, lines.clone()));
        lines
    }

    fn invalidate_cache(&self) {
        *self.rendered.borrow_mut() = None;
    }
}

/// Which part of the reader has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderFocus {
    Content,
    Composer,
}

/// Interactive sectioned document reader shown as a `BottomPaneView`.
pub(crate) struct DocumentReaderView {
    document_id: String,
    title: String,
    sections: Vec<DocumentSection>,
    current_section: usize,
    scroll_offset: u16,
    focus: ReaderFocus,
    app_event_tx: AppEventSender,
    complete: bool,
    /// Tracks which sections have a pending agent update (by section index).
    /// "thinking..." shows only when the current section is in this set.
    pending_sections: HashSet<usize>,

    // Embedded textarea for follow-up questions.
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
}

impl DocumentReaderView {
    pub(crate) fn new(
        document_id: String,
        title: String,
        content: String,
        app_event_tx: AppEventSender,
    ) -> Self {
        let sections = parse_sections(&title, &content);
        Self {
            document_id,
            title,
            sections,
            current_section: 0,
            scroll_offset: 0,
            focus: ReaderFocus::Content,
            app_event_tx,
            complete: false,
            pending_sections: HashSet::new(),
            textarea: TextArea::new(),
            textarea_state: RefCell::new(TextAreaState::default()),
        }
    }

    /// Update a section's content (full replacement).
    pub(crate) fn update_section(&mut self, section_index: usize, content: String) {
        if let Some(section) = self.sections.get_mut(section_index) {
            // Re-derive heading from the new content if it starts with `## `.
            let (heading, body) = if let Some(rest) = content.strip_prefix("## ") {
                if let Some(nl) = rest.find('\n') {
                    (rest[..nl].trim().to_string(), rest[nl + 1..].to_string())
                } else {
                    (rest.trim().to_string(), String::new())
                }
            } else {
                (section.heading.clone(), content)
            };
            section.heading = heading;
            section.content = body;
            section.recently_updated = true;
            section.invalidate_cache();
            self.pending_sections.remove(&section_index);
        }
    }

    /// Append content to a section.
    pub(crate) fn append_to_section(&mut self, section_index: usize, content: String) {
        if let Some(section) = self.sections.get_mut(section_index) {
            if !section.content.is_empty() && !section.content.ends_with('\n') {
                section.content.push('\n');
            }
            section.content.push_str(&content);
            section.recently_updated = true;
            section.invalidate_cache();
            self.pending_sections.remove(&section_index);
        }
    }

    /// Patch a section with find-and-replace.
    pub(crate) fn patch_section(&mut self, section_index: usize, old_text: &str, new_text: &str) {
        if let Some(section) = self.sections.get_mut(section_index) {
            if section.content.contains(old_text) {
                section.content = section.content.replacen(old_text, new_text, 1);
                section.recently_updated = true;
                section.invalidate_cache();
            }
            self.pending_sections.remove(&section_index);
        }
    }

    /// Collect all section headings (for the post-exit transcript card).
    pub(crate) fn section_headings(&self) -> Vec<String> {
        self.sections.iter().map(|s| s.heading.clone()).collect()
    }

    /// Collect the full final content (for transcript export).
    pub(crate) fn final_content(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if !section.heading.is_empty() {
                out.push_str("## ");
                out.push_str(&section.heading);
                out.push('\n');
            }
            out.push_str(&section.content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    fn next_section(&mut self) {
        if self.current_section + 1 < self.sections.len() {
            self.clear_updated_flag();
            self.current_section += 1;
            self.scroll_offset = 0;
        }
    }

    fn prev_section(&mut self) {
        if self.current_section > 0 {
            self.clear_updated_flag();
            self.current_section -= 1;
            self.scroll_offset = 0;
        }
    }

    fn clear_updated_flag(&mut self) {
        if let Some(section) = self.sections.get_mut(self.current_section) {
            section.recently_updated = false;
        }
    }

    fn exit_reading_mode(&mut self) {
        // Insert a history cell with the final document state.
        let cell = crate::history_cell::new_document_cell(
            self.title.clone(),
            self.section_headings(),
            self.final_content(),
        );
        self.app_event_tx
            .send(AppEvent::InsertHistoryCell(Box::new(cell)));
        self.complete = true;
    }

    fn submit_follow_up(&mut self) {
        let text = self.textarea.text().trim().to_string();
        if text.is_empty() {
            return;
        }

        let heading = self
            .sections
            .get(self.current_section)
            .map(|s| s.heading.as_str())
            .unwrap_or("");

        let context = format!(
            "[Document \"{}\" \u{2014} Section {}: \"{}\"]\n\n{}",
            self.title, self.current_section, heading, text
        );

        self.app_event_tx.send(AppEvent::CodexOp(Op::UserInput {
            items: vec![UserInput::Text {
                text: context,
                text_elements: vec![],
            }],
            final_output_json_schema: None,
        }));
        self.pending_sections.insert(self.current_section);
        self.textarea = TextArea::new();
        *self.textarea_state.borrow_mut() = TextAreaState::default();
        self.focus = ReaderFocus::Content;
    }

    fn input_height(&self, _width: u16) -> u16 {
        let lines = self.textarea.text().lines().count().max(1);
        (lines as u16).clamp(1, 4)
    }

    fn handle_content_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('n')
            | KeyCode::Char('l')
            | KeyCode::Right
            | KeyCode::PageDown
            | KeyCode::Enter => {
                self.next_section();
            }
            KeyCode::Char('p') | KeyCode::Char('h') | KeyCode::Left | KeyCode::PageUp => {
                self.prev_section();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Home => {
                self.current_section = 0;
                self.scroll_offset = 0;
            }
            KeyCode::End => {
                if !self.sections.is_empty() {
                    self.current_section = self.sections.len() - 1;
                    self.scroll_offset = 0;
                }
            }
            KeyCode::Tab => {
                self.focus = ReaderFocus::Composer;
            }
            KeyCode::Char('q') => {
                self.exit_reading_mode();
            }
            _ => {}
        }
    }

    fn handle_composer_key(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.focus = ReaderFocus::Content;
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.submit_follow_up();
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.textarea.input(key_event);
            }
            other => {
                self.textarea.input(other);
            }
        }
    }
}

impl BottomPaneView for DocumentReaderView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self.focus {
            ReaderFocus::Content => self.handle_content_key(key_event),
            ReaderFocus::Composer => self.handle_composer_key(key_event),
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.exit_reading_mode();
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        // In composer focus, Esc should switch back to content rather than dismiss.
        self.focus == ReaderFocus::Composer
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn view_id(&self) -> Option<&'static str> {
        Some(DOCUMENT_READER_VIEW_ID)
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if self.focus == ReaderFocus::Composer && !pasted.is_empty() {
            self.textarea.insert_str(&pasted);
            return true;
        }
        false
    }

    fn handle_document_section_update(
        &mut self,
        document_id: &str,
        section_index: usize,
        content: String,
    ) {
        if self.document_id == document_id {
            self.update_section(section_index, content);
        }
    }

    fn handle_document_section_append(
        &mut self,
        document_id: &str,
        section_index: usize,
        content: String,
    ) {
        if self.document_id == document_id {
            self.append_to_section(section_index, content);
        }
    }

    fn handle_document_section_patch(
        &mut self,
        document_id: &str,
        section_index: usize,
        old_text: &str,
        new_text: &str,
    ) {
        if self.document_id == document_id {
            self.patch_section(section_index, old_text, new_text);
        }
    }

    fn handle_turn_complete(&mut self) {
        // If any sections were pending updates but the turn ended without
        // the agent calling an update tool, clear all pending state so the
        // user isn't stuck with permanent "thinking..." indicators.
        self.pending_sections.clear();
    }
}

impl Renderable for DocumentReaderView {
    fn desired_height(&self, width: u16) -> u16 {
        // Content lines inside the card (accounting for 2-char padding each side).
        let inner_width = width.saturating_sub(4);
        let section_lines = self
            .sections
            .get(self.current_section)
            .map(|s| s.rendered_lines(inner_width).len() as u16)
            .unwrap_or(1);

        let composer_rows = if self.focus == ReaderFocus::Composer {
            // separator(1) + composer(input_height)
            1 + self.input_height(inner_width)
        } else {
            0
        };

        // top_border(1) + header(1) + separator(1) + content + separator(1) + hints(1)
        //   + composer_rows + bottom_border(1)
        let ideal = 1 + 1 + 1 + section_lines + 1 + 1 + composer_rows + 1;
        ideal.clamp(8, 30)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height < 4 || area.width < 6 {
            return;
        }
        Clear.render(area, buf);

        let w = area.width;
        let section_count = self.sections.len();
        let section_num = self.current_section + 1;

        // Build card from bottom up to anchor controls at the bottom.
        //
        // Layout (bottom-anchored):
        //   row -1 (bottom):  ╰───────────────────────╯
        //   row -2:           composer input (if focused)
        //   row -3:           ├───────────────────────┤  (if composer focused)
        //   row -4:           │ hints                  │
        //   row -5:           ├───────────────────────┤
        //   rows ...:         │ content (scrollable)   │
        //   row 2:            ├───────────────────────┤
        //   row 1:            │ title          3/7     │
        //   row 0:            ╭───────────────────────╮

        let mut y = area.y;
        let bottom = area.y + area.height;

        // --- Bottom border ---
        let bottom_y = bottom.saturating_sub(1);

        // --- Composer (if focused) ---
        let composer_rows = if self.focus == ReaderFocus::Composer {
            let inner_w = w.saturating_sub(4);
            1 + self.input_height(inner_w) // separator + input
        } else {
            0
        };

        // --- Fixed rows from bottom: bottom_border(1) + composer_rows + hints(1) + separator(1) ---
        let fixed_bottom = 1 + composer_rows + 1 + 1;
        // --- Fixed rows from top: top_border(1) + header(1) + separator(1) ---
        let fixed_top: u16 = 3;
        let content_height = area
            .height
            .saturating_sub(fixed_top)
            .saturating_sub(fixed_bottom);

        // === Render top-down ===

        // Top border
        Paragraph::new(render::top_border(w)).render(
            Rect {
                y,
                height: 1,
                ..area
            },
            buf,
        );
        y += 1;

        // Header — show "thinking..." only when the current section is pending.
        let current_pending = self.pending_sections.contains(&self.current_section);
        let header =
            render::header_line(&self.title, section_num, section_count, current_pending, w);
        Paragraph::new(header).render(
            Rect {
                y,
                height: 1,
                ..area
            },
            buf,
        );
        y += 1;

        // Separator after header
        Paragraph::new(render::separator(w)).render(
            Rect {
                y,
                height: 1,
                ..area
            },
            buf,
        );
        y += 1;

        // === Content area (fills remaining space between top and bottom fixed rows) ===
        if content_height > 0 {
            let inner_width = w.saturating_sub(4);
            if let Some(section) = self.sections.get(self.current_section) {
                let updated = section.recently_updated;
                let raw_lines = section.rendered_lines(inner_width);
                let visible: Vec<Line<'static>> = raw_lines
                    .into_iter()
                    .skip(self.scroll_offset as usize)
                    .take(content_height as usize)
                    .collect();

                // Render each visible line wrapped in side borders.
                for (i, line) in visible.into_iter().enumerate() {
                    let row_y = y + i as u16;
                    if row_y >= bottom {
                        break;
                    }
                    let bordered = render::bordered_line(line, w, updated);
                    Paragraph::new(bordered).render(
                        Rect {
                            y: row_y,
                            height: 1,
                            ..area
                        },
                        buf,
                    );
                }

                // Fill remaining content rows with empty bordered lines.
                let rendered_count = self
                    .sections
                    .get(self.current_section)
                    .map(|s| {
                        s.rendered_lines(inner_width)
                            .len()
                            .saturating_sub(self.scroll_offset as usize)
                    })
                    .unwrap_or(0);
                let filled = rendered_count.min(content_height as usize);
                for i in filled..content_height as usize {
                    let row_y = y + i as u16;
                    if row_y >= bottom {
                        break;
                    }
                    let empty = render::bordered_line(Line::from(""), w, updated);
                    Paragraph::new(empty).render(
                        Rect {
                            y: row_y,
                            height: 1,
                            ..area
                        },
                        buf,
                    );
                }
            } else {
                // No section — fill with empty bordered lines.
                for i in 0..content_height as usize {
                    let row_y = y + i as u16;
                    if row_y >= bottom {
                        break;
                    }
                    let empty = render::bordered_line(Line::from(""), w, false);
                    Paragraph::new(empty).render(
                        Rect {
                            y: row_y,
                            height: 1,
                            ..area
                        },
                        buf,
                    );
                }
            }
        }

        // === Render bottom-up from bottom_y ===
        let mut by = bottom_y;

        // Bottom border
        Paragraph::new(render::bottom_border(w)).render(
            Rect {
                y: by,
                height: 1,
                ..area
            },
            buf,
        );

        // Composer (if focused)
        if self.focus == ReaderFocus::Composer {
            let input_h = self.input_height(w.saturating_sub(4));
            by = by.saturating_sub(input_h);
            // Render composer with side borders approximated by the TextArea's own chrome.
            // We inset the textarea by 2 chars on each side to sit inside the card.
            let ta_area = Rect {
                x: area.x + 2,
                y: by,
                width: w.saturating_sub(4),
                height: input_h,
            };
            // Draw side borders for the composer rows.
            for row in 0..input_h {
                let ry = by + row;
                if ry < bottom {
                    // Left border
                    if let Some(cell) = buf.cell_mut((area.x, ry)) {
                        cell.set_symbol("│");
                        cell.set_style(ratatui::style::Style::default().dim());
                    }
                    if let Some(cell) = buf.cell_mut((area.x + 1, ry)) {
                        cell.set_symbol(" ");
                    }
                    // Right border
                    let rx = area.x + w.saturating_sub(1);
                    if let Some(cell) = buf.cell_mut((rx, ry)) {
                        cell.set_symbol("│");
                        cell.set_style(ratatui::style::Style::default().dim());
                    }
                    if rx > 0
                        && let Some(cell) = buf.cell_mut((rx - 1, ry)) {
                            cell.set_symbol(" ");
                        }
                }
            }
            let mut state = self.textarea_state.borrow_mut();
            StatefulWidgetRef::render_ref(&(&self.textarea), ta_area, buf, &mut state);
            // Placeholder text.
            if self.textarea.text().is_empty() {
                let placeholder = if current_pending {
                    "Waiting for update..."
                } else {
                    "Ask about this section..."
                };
                Paragraph::new(placeholder.dim().italic()).render(ta_area, buf);
            }

            // Separator above composer
            by = by.saturating_sub(1);
            Paragraph::new(render::separator(w)).render(
                Rect {
                    y: by,
                    height: 1,
                    ..area
                },
                buf,
            );
        }

        // Hints bar
        by = by.saturating_sub(1);
        let hints = render::hints_line(self.focus == ReaderFocus::Composer, w);
        Paragraph::new(hints).render(
            Rect {
                y: by,
                height: 1,
                ..area
            },
            buf,
        );

        // Separator above hints
        by = by.saturating_sub(1);
        Paragraph::new(render::separator(w)).render(
            Rect {
                y: by,
                height: 1,
                ..area
            },
            buf,
        );
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if self.focus != ReaderFocus::Composer {
            return None;
        }
        // Cursor is inside the textarea area, which is inset 2 chars from card edge.
        let input_h = self.input_height(area.width.saturating_sub(4));
        let bottom_y = area.y + area.height - 1; // bottom border
        let ta_y = bottom_y.saturating_sub(input_h);
        let text_len = self.textarea.text().lines().last().map_or(0, str::len);
        Some((area.x + 2 + text_len as u16, ta_y))
    }
}

/// Parse markdown content into sections split on `## ` headings.
///
/// Content before the first `## ` becomes section 0 with the document title
/// as heading.
fn parse_sections(title: &str, content: &str) -> Vec<DocumentSection> {
    let mut sections = Vec::new();
    let mut current_heading = title.to_string();
    let mut current_content = String::new();

    for line in content.lines() {
        if let Some(heading_text) = line.strip_prefix("## ") {
            // Flush the previous section.
            sections.push(DocumentSection {
                heading: current_heading,
                content: current_content,
                rendered: RefCell::new(None),
                recently_updated: false,
            });
            current_heading = heading_text.trim().to_string();
            current_content = String::new();
        } else {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            current_content.push_str(line);
        }
    }

    // Flush the last section.
    sections.push(DocumentSection {
        heading: current_heading,
        content: current_content,
        rendered: RefCell::new(None),
        recently_updated: false,
    });

    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::history_cell::DocumentCell;
    use crate::render::renderable::Renderable;
    use codex_core::protocol::Op;
    use crossterm::event::KeyModifiers;
    use pretty_assertions::assert_eq;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    fn snapshot_buffer(buf: &Buffer) -> String {
        let mut lines = Vec::new();
        for y in 0..buf.area().height {
            let mut row = String::new();
            for x in 0..buf.area().width {
                row.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            lines.push(row);
        }
        lines.join("\n")
    }

    fn test_content() -> String {
        "Introduction paragraph.\n\
         ## Methodology\n\
         Method details here.\n\
         ## Results\n\
         Result findings.\n\
         ## Discussion\n\
         Discussion text."
            .to_string()
    }

    fn make_view(tx: AppEventSender) -> DocumentReaderView {
        DocumentReaderView::new(
            "test-doc".to_string(),
            "Test Report".to_string(),
            test_content(),
            tx,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    // -----------------------------------------------------------------------
    // Parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_sections_splits_on_headings() {
        let content = "Intro text\n## Methodology\nMethod content\n## Results\nResult content";
        let sections = parse_sections("My Report", content);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].heading, "My Report");
        assert_eq!(sections[0].content, "Intro text");
        assert_eq!(sections[1].heading, "Methodology");
        assert_eq!(sections[1].content, "Method content");
        assert_eq!(sections[2].heading, "Results");
        assert_eq!(sections[2].content, "Result content");
    }

    #[test]
    fn parse_sections_content_before_first_heading() {
        let content = "Some preamble\nMore preamble\n## First\nBody";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "Title");
        assert!(sections[0].content.contains("preamble"));
        assert_eq!(sections[1].heading, "First");
        assert_eq!(sections[1].content, "Body");
    }

    #[test]
    fn parse_sections_no_headings() {
        let content = "Just a single block of text\nwith multiple lines";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "Title");
    }

    #[test]
    fn parse_sections_single_heading() {
        let content = "## Only Section\nContent here";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "Title");
        assert!(sections[0].content.is_empty());
        assert_eq!(sections[1].heading, "Only Section");
    }

    #[test]
    fn parse_sections_empty_sections() {
        let content = "## A\n## B\n## C";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 4);
        assert!(sections[1].content.is_empty());
        assert!(sections[2].content.is_empty());
    }

    // -----------------------------------------------------------------------
    // Rendering tests
    // -----------------------------------------------------------------------

    #[test]
    fn initial_render_shows_title_and_first_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = make_view(tx);

        let area = Rect::new(0, 0, 50, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);

        assert!(snap.contains("Test Report"), "title should be visible");
        assert!(snap.contains("1/4"), "section indicator should show 1/4");
        assert!(
            snap.contains("navigate"),
            "hints bar should show navigation keys"
        );
    }

    #[test]
    fn render_after_navigation_shows_next_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to section 2.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        let area = Rect::new(0, 0, 50, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);

        assert!(snap.contains("2/4"), "section indicator should show 2/4");
        assert!(
            snap.contains("Methodology"),
            "second section heading should be visible"
        );
    }

    #[test]
    fn render_with_composer_focus_shows_hints_and_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Switch to composer focus.
        view.focus = ReaderFocus::Composer;

        let area = Rect::new(0, 0, 50, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);

        assert!(
            snap.contains("send"),
            "composer hints should show 'send' for Enter key"
        );
    }

    // -----------------------------------------------------------------------
    // Navigation tests
    // -----------------------------------------------------------------------

    #[test]
    fn navigate_forward_and_backward() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.current_section, 0);

        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        view.handle_content_key(key(KeyCode::Right));
        assert_eq!(view.current_section, 2);

        view.handle_content_key(key(KeyCode::PageDown));
        assert_eq!(view.current_section, 3);

        // Should not go past the last section.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 3);

        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 2);

        view.handle_content_key(key(KeyCode::Left));
        assert_eq!(view.current_section, 1);

        view.handle_content_key(key(KeyCode::PageUp));
        assert_eq!(view.current_section, 0);

        // Should not go before the first section.
        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 0);
    }

    #[test]
    fn home_and_end_navigation() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_content_key(key(KeyCode::End));
        assert_eq!(view.current_section, 3);

        view.handle_content_key(key(KeyCode::Home));
        assert_eq!(view.current_section, 0);
    }

    #[test]
    fn scroll_within_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.scroll_offset, 0);

        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(view.scroll_offset, 1);

        view.handle_content_key(key(KeyCode::Down));
        assert_eq!(view.scroll_offset, 2);

        view.handle_content_key(key(KeyCode::Char('k')));
        assert_eq!(view.scroll_offset, 1);

        view.handle_content_key(key(KeyCode::Up));
        assert_eq!(view.scroll_offset, 0);

        // Should not go below zero.
        view.handle_content_key(key(KeyCode::Char('k')));
        assert_eq!(view.scroll_offset, 0);
    }

    #[test]
    fn navigation_resets_scroll_offset() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_content_key(key(KeyCode::Char('j')));
        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(view.scroll_offset, 2);

        // Navigate to next section — scroll should reset.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.scroll_offset, 0);
        assert_eq!(view.current_section, 1);
    }

    // -----------------------------------------------------------------------
    // Focus switching tests
    // -----------------------------------------------------------------------

    #[test]
    fn tab_switches_between_content_and_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.focus, ReaderFocus::Content);

        // Tab → composer.
        view.handle_content_key(key(KeyCode::Tab));
        assert_eq!(view.focus, ReaderFocus::Composer);

        // Tab → back to content.
        view.handle_composer_key(key(KeyCode::Tab));
        assert_eq!(view.focus, ReaderFocus::Content);
    }

    #[test]
    fn esc_in_composer_returns_to_content() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.focus = ReaderFocus::Composer;
        view.handle_composer_key(key(KeyCode::Esc));
        assert_eq!(view.focus, ReaderFocus::Content);
        assert!(
            !view.complete,
            "Esc in composer should not exit reading mode"
        );
    }

    #[test]
    fn prefer_esc_depends_on_focus() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // In content mode, Esc should NOT be preferred (it exits via on_ctrl_c).
        assert!(!view.prefer_esc_to_handle_key_event());

        // In composer mode, Esc SHOULD be preferred (it returns to content).
        view.focus = ReaderFocus::Composer;
        assert!(view.prefer_esc_to_handle_key_event());
    }

    // -----------------------------------------------------------------------
    // Exit tests
    // -----------------------------------------------------------------------

    #[test]
    fn q_exits_reading_mode_and_emits_history_cell() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.complete);
        view.handle_content_key(key(KeyCode::Char('q')));
        assert!(view.complete);

        // Verify InsertHistoryCell was emitted.
        let mut found_cell = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = ev {
                let doc = cell
                    .as_any()
                    .downcast_ref::<DocumentCell>()
                    .expect("expected DocumentCell");
                assert_eq!(doc.title, "Test Report");
                assert_eq!(doc.section_headings.len(), 4);
                assert_eq!(doc.section_headings[1], "Methodology");
                assert!(doc.final_content.contains("Method details"));
                found_cell = true;
            }
        }
        assert!(found_cell, "expected InsertHistoryCell event");
    }

    #[test]
    fn ctrl_c_exits_reading_mode() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.complete);
        let result = view.on_ctrl_c();
        assert_eq!(result, CancellationEvent::Handled);
        assert!(view.complete);
    }

    // -----------------------------------------------------------------------
    // Follow-up submission tests
    // -----------------------------------------------------------------------

    #[test]
    fn submit_follow_up_sends_user_input_op() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to the Results section (index 2).
        view.handle_content_key(key(KeyCode::Char('n')));
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 2);

        // Switch to composer and type a question.
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('W')));
        view.textarea.input(key(KeyCode::Char('h')));
        view.textarea.input(key(KeyCode::Char('y')));
        view.textarea.input(key(KeyCode::Char('?')));

        // Submit.
        view.handle_composer_key(key(KeyCode::Enter));

        // Should be waiting for update on section 2.
        assert!(view.pending_sections.contains(&2));
        // Composer should be cleared and focus returned to content.
        assert!(view.textarea.text().is_empty());
        assert_eq!(view.focus, ReaderFocus::Content);

        // Verify the Op was emitted with section context.
        let mut found_op = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::CodexOp(Op::UserInput { items, .. }) = ev {
                let text = match &items[0] {
                    UserInput::Text { text, .. } => text.clone(),
                    _ => String::new(),
                };
                assert!(
                    text.contains("Results"),
                    "context should include section heading: {text}"
                );
                assert!(
                    text.contains("Why?"),
                    "context should include the question: {text}"
                );
                assert!(
                    text.contains("Section 2"),
                    "context should include section index: {text}"
                );
                found_op = true;
            }
        }
        assert!(found_op, "expected CodexOp(UserInput) event");
    }

    #[test]
    fn empty_follow_up_is_not_submitted() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.focus = ReaderFocus::Composer;
        // Submit without typing anything.
        view.handle_composer_key(key(KeyCode::Enter));

        assert!(view.pending_sections.is_empty());
        assert!(rx.try_recv().is_err(), "no event should be emitted");
    }

    // -----------------------------------------------------------------------
    // Section update tests
    // -----------------------------------------------------------------------

    #[test]
    fn update_section_replaces_content() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections.insert(1);
        view.update_section(1, "Updated methodology content.".to_string());

        assert!(
            !view.pending_sections.contains(&1),
            "update should clear pending flag"
        );
        assert_eq!(view.sections[1].content, "Updated methodology content.");
        assert_eq!(
            view.sections[1].heading, "Methodology",
            "heading should be preserved when content has no heading prefix"
        );
    }

    #[test]
    fn update_section_with_heading_prefix() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.update_section(1, "## New Heading\nNew body.".to_string());

        assert_eq!(view.sections[1].heading, "New Heading");
        assert_eq!(view.sections[1].content, "New body.");
    }

    #[test]
    fn update_section_out_of_bounds_is_no_op() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections.insert(99);
        view.update_section(99, "Does not exist.".to_string());

        // pending should stay since the update was ignored (section doesn't exist).
        assert!(view.pending_sections.contains(&99));
    }

    #[test]
    fn handle_document_section_update_via_trait() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections.insert(2);

        // Matching document_id should update.
        view.handle_document_section_update("test-doc", 2, "New results.".to_string());
        assert!(!view.pending_sections.contains(&2));
        assert_eq!(view.sections[2].content, "New results.");

        // Non-matching document_id should be ignored.
        view.pending_sections.insert(0);
        view.handle_document_section_update("other-doc", 0, "Ignored.".to_string());
        assert!(view.pending_sections.contains(&0));
    }

    // -----------------------------------------------------------------------
    // Turn-complete clears waiting state
    // -----------------------------------------------------------------------

    #[test]
    fn turn_complete_clears_waiting_for_update() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate submitting follow-ups for sections 1 and 2.
        view.pending_sections.insert(1);
        view.pending_sections.insert(2);

        // Turn completes without the agent calling update tools.
        view.handle_turn_complete();

        assert!(
            view.pending_sections.is_empty(),
            "turn completion should clear all pending sections so user is not stuck"
        );
    }

    // -----------------------------------------------------------------------
    // Render cache invalidation test
    // -----------------------------------------------------------------------

    #[test]
    fn section_update_invalidates_render_cache() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to section 1 and render to populate its cache.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        let area = Rect::new(0, 0, 50, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        // Cache should be populated for the current section.
        assert!(view.sections[1].rendered.borrow().is_some());

        // Update section — cache should be invalidated.
        view.update_section(1, "Changed content.".to_string());
        assert!(view.sections[1].rendered.borrow().is_none());

        // Re-render to verify no crash and new content appears.
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            snap.contains("Changed content"),
            "updated content should be visible after re-render"
        );
    }

    // -----------------------------------------------------------------------
    // End-to-end: full lifecycle through BottomPaneView trait
    // -----------------------------------------------------------------------

    #[test]
    fn full_lifecycle_navigate_ask_update_exit() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // 1. Render initial state.
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(snap.contains("Test Report"));
        assert!(snap.contains("1/4"));

        // 2. Navigate to section 3 (Results).
        view.handle_key_event(key(KeyCode::Char('n')));
        view.handle_key_event(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 2);

        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(snap.contains("3/4"));

        // 3. Tab to composer, type a question, submit.
        view.handle_key_event(key(KeyCode::Tab));
        assert_eq!(view.focus, ReaderFocus::Composer);

        view.handle_key_event(key(KeyCode::Char('m')));
        view.handle_key_event(key(KeyCode::Char('o')));
        view.handle_key_event(key(KeyCode::Char('r')));
        view.handle_key_event(key(KeyCode::Char('e')));
        view.handle_key_event(key(KeyCode::Enter));

        assert!(view.pending_sections.contains(&2));
        assert_eq!(view.focus, ReaderFocus::Content);

        // Drain the UserInput event.
        let mut got_input = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::CodexOp(Op::UserInput { items, .. }) = ev
                && let UserInput::Text { text, .. } = &items[0] {
                    assert!(text.contains("more"));
                    got_input = true;
                }
        }
        assert!(got_input, "expected follow-up submission");

        // 4. Render while waiting — should show "thinking..." indicator.
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            snap.contains("thinking"),
            "should show thinking indicator while waiting"
        );

        // 5. Receive section update.
        view.handle_document_section_update(
            "test-doc",
            2,
            "## Results\nExpanded result findings with more detail.".to_string(),
        );
        assert!(!view.pending_sections.contains(&2));

        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            snap.contains("Expanded result"),
            "updated content should be visible"
        );

        // 6. Exit via 'q'.
        assert!(!view.is_complete());
        view.handle_key_event(key(KeyCode::Char('q')));
        assert!(view.is_complete());

        // Verify history cell was emitted with the updated content.
        let mut found_cell = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = ev {
                let doc = cell
                    .as_any()
                    .downcast_ref::<DocumentCell>()
                    .expect("DocumentCell");
                assert!(doc.final_content.contains("Expanded result"));
                found_cell = true;
            }
        }
        assert!(found_cell, "expected InsertHistoryCell on exit");
    }

    // -----------------------------------------------------------------------
    // DocumentCell rendering test
    // -----------------------------------------------------------------------

    #[test]
    fn document_cell_display_lines_show_sections() {
        use crate::history_cell::HistoryCell;
        use crate::history_cell::{self};
        let cell = history_cell::new_document_cell(
            "My Report".to_string(),
            vec![
                "Introduction".to_string(),
                "Methodology".to_string(),
                "Results".to_string(),
            ],
            "full content here".to_string(),
        );
        let lines = cell.display_lines(80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join("");

        assert!(text.contains("My Report"), "should contain title");
        assert!(
            text.contains("Methodology"),
            "should contain section heading"
        );
        assert!(text.contains("3 sections"), "should contain section count");
    }

    // -----------------------------------------------------------------------
    // Append and patch tests
    // -----------------------------------------------------------------------

    #[test]
    fn append_to_section_adds_content() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections.insert(1);
        view.append_to_section(1, "Additional details here.".to_string());

        assert!(view.sections[1].content.contains("Method details here."));
        assert!(
            view.sections[1]
                .content
                .contains("Additional details here.")
        );
        assert!(view.sections[1].recently_updated);
        assert!(!view.pending_sections.contains(&1));
    }

    #[test]
    fn append_via_trait_method() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_document_section_append("test-doc", 1, "Extra content.".to_string());
        assert!(view.sections[1].content.contains("Extra content."));
        assert!(view.sections[1].recently_updated);

        // Non-matching document_id should be ignored.
        let original = view.sections[0].content.clone();
        view.handle_document_section_append("wrong-doc", 0, "Ignored.".to_string());
        assert_eq!(view.sections[0].content, original);
    }

    #[test]
    fn patch_section_replaces_text() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections.insert(1);
        view.patch_section(1, "Method details here.", "Improved method details.");

        assert_eq!(view.sections[1].content, "Improved method details.");
        assert!(view.sections[1].recently_updated);
        assert!(!view.pending_sections.contains(&1));
    }

    #[test]
    fn patch_section_no_match_still_clears_pending() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        let original = view.sections[1].content.clone();
        view.pending_sections.insert(1);
        view.patch_section(1, "nonexistent text", "replacement");

        // Content unchanged since old_text wasn't found.
        assert_eq!(view.sections[1].content, original);
        // But pending should still be cleared.
        assert!(!view.pending_sections.contains(&1));
    }

    #[test]
    fn patch_via_trait_method() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_document_section_patch(
            "test-doc",
            2,
            "Result findings.",
            "Improved result findings with more data.",
        );
        assert_eq!(
            view.sections[2].content,
            "Improved result findings with more data."
        );
        assert!(view.sections[2].recently_updated);
    }

    // -----------------------------------------------------------------------
    // Per-section pending tracking tests
    // -----------------------------------------------------------------------

    #[test]
    fn pending_sections_tracks_multiple_sections() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate asking about sections 1 and 3.
        view.pending_sections.insert(1);
        view.pending_sections.insert(3);

        // Update section 1 — only section 1 should clear.
        view.update_section(1, "Updated.".to_string());
        assert!(!view.pending_sections.contains(&1));
        assert!(view.pending_sections.contains(&3));

        // Update section 3.
        view.update_section(3, "Also updated.".to_string());
        assert!(view.pending_sections.is_empty());
    }

    #[test]
    fn navigate_away_from_pending_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Ask about section 0, then navigate away.
        view.pending_sections.insert(0);
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        // Section 0 should still be pending.
        assert!(view.pending_sections.contains(&0));

        // Render — should NOT show "thinking..." since current section (1) is not pending.
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            !snap.contains("thinking"),
            "should not show thinking when viewing a non-pending section"
        );

        // Navigate back to section 0 — should show "thinking...".
        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 0);

        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            snap.contains("thinking"),
            "should show thinking when viewing a pending section"
        );
    }
}
