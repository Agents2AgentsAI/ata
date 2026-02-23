//! Sectioned reading mode for long agent-produced documents.
//!
//! The agent calls `present_reading_view` to display a long markdown document split
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
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

mod render;

pub(crate) const DOCUMENT_READER_VIEW_ID: &str = "doc_reader";

/// Saved fold state that persists across view close/reopen cycles within a
/// single TUI session.  Stored as a process-level static so we don't need to
/// thread it through BottomPane (which is upstream code).
static SAVED_FOLD_STATE: std::sync::Mutex<Option<SavedFoldState>> = std::sync::Mutex::new(None);

struct SavedFoldState {
    document_id: String,
    /// Per-section fold regions (index = section index).
    section_folds: Vec<Vec<FoldRegion>>,
}

/// A collapsible region within a section's content.
#[derive(Debug, Clone)]
struct FoldRegion {
    /// Byte offset in `DocumentSection::content` where the fold starts.
    start: usize,
    /// Byte offset in `DocumentSection::content` where the fold ends.
    end: usize,
    /// Short description shown when collapsed.
    summary: String,
    /// Whether this fold is currently collapsed.
    collapsed: bool,
}

/// A single section of a document (split on `## ` headings).
struct DocumentSection {
    heading: String,
    content: String,
    /// Cached rendered lines; invalidated on width change or content update.
    rendered: RefCell<Option<(u16, Vec<Line<'static>>)>>,
    /// Set to `true` when this section was just updated via `update_document_section`.
    /// Cleared when the user navigates away. Used to highlight changes.
    recently_updated: bool,
    /// The raw-content line index from which changes start.  `Some(0)` means
    /// the entire body was replaced; `Some(n)` means lines `n..` are
    /// new/changed.  `None` means no per-line change tracking (all lines use
    /// the section-wide `recently_updated` flag for the heading indicator
    /// only).
    changed_from_line: Option<usize>,
    /// Exclusive upper bound of the changed region (raw-content line index).
    /// `None` means "to the end of the section" (used for appends and full
    /// replacements).  `Some(n)` means only lines in `[changed_from_line, n)`
    /// are highlighted.
    changed_to_line: Option<usize>,
    /// Collapsible regions within this section's content.
    folds: Vec<FoldRegion>,
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
        let heading_line_count: usize = if self.heading.is_empty() { 0 } else { 2 };
        let lines =
            render::render_section(&self.heading, &self.content, width, self.recently_updated);
        let lines =
            render::apply_folds(lines, &self.content, heading_line_count, width, &self.folds);
        *self.rendered.borrow_mut() = Some((width, lines.clone()));
        lines
    }

    fn invalidate_cache(&self) {
        *self.rendered.borrow_mut() = None;
    }

    /// Whether this section has any fold regions.
    fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }
}

/// A single match found by the search feature.
struct SearchMatch {
    section_idx: usize,
    /// Byte offset within the section's content string.
    byte_offset: usize,
}

/// State for an active search.
struct SearchState {
    query: String,
    matches: Vec<SearchMatch>,
    current_match_idx: usize,
}

/// Whether visual selection is character-wise (v) or line-wise (V).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualMode {
    Char,
    Line,
}

/// Visual selection state.
struct VisualSelect {
    mode: VisualMode,
    /// The rendered-line index where selection started.
    anchor_line: usize,
    /// Character column where selection started (only meaningful in Char mode).
    anchor_col: usize,
}

/// Which part of the reader has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderFocus {
    Content,
    Composer,
    Search,
}

/// Interactive sectioned document reader shown as a `BottomPaneView`.
pub(crate) struct DocumentReaderView {
    document_id: String,
    title: String,
    sections: Vec<DocumentSection>,
    current_section: usize,
    scroll_offset: Cell<u16>,
    focus: ReaderFocus,
    app_event_tx: AppEventSender,
    complete: bool,
    /// Tracks which sections have a pending agent update (by section index).
    /// Maps section index → (question text, submission time).
    pending_sections: HashMap<usize, (String, Instant)>,
    /// Tracks which section indices the user has viewed (for exit feedback).
    visited_sections: HashSet<usize>,
    animations_enabled: bool,
    frame_requester: crate::tui::FrameRequester,

    // Embedded textarea for follow-up questions.
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,

    // Vim motions: tracks whether a single `g` was pressed.
    pending_g: bool,
    // Fold key prefix: tracks whether `z` was pressed (for zM/zR chords).
    pending_z: bool,
    // Quit confirmation: first `q` sets this to true and shows a hint;
    // second `q` (or `y`) actually exits the reading view.
    pending_quit: bool,
    // Cursor position (absolute rendered-line index + column in the current
    // section).  The viewport scrolls to keep cursor_line visible.
    cursor_line: usize,
    cursor_col: usize,
    // Content height from the last render, used for half-page scroll.
    last_content_height: Cell<u16>,
    // Inner width from the last render, used by visual select text extraction.
    last_inner_width: Cell<u16>,

    // Search state.
    search_state: Option<SearchState>,
    search_input: String,

    // Visual line selection (vim V-mode).
    visual_select: Option<VisualSelect>,
    /// Text extracted from visual selection, to be prepended to the next
    /// follow-up question as context.
    selection_context: Option<String>,

    /// Set of section indices still awaiting content during streaming.
    /// `Some(set)` means streaming is active; `None` means all sections are filled.
    streaming_sections: Option<HashSet<usize>>,
}

impl DocumentReaderView {
    pub(crate) fn new(
        document_id: String,
        title: String,
        content: String,
        app_event_tx: AppEventSender,
        animations_enabled: bool,
        frame_requester: crate::tui::FrameRequester,
    ) -> Self {
        let mut sections = parse_sections(&title, &content);
        let mut visited_sections = HashSet::new();
        if !sections.is_empty() {
            visited_sections.insert(0);
        }

        // Restore saved fold regions from a previous viewing of the same
        // document.  Folds are matched by section index; byte ranges are only
        // applied when the section content length matches (content unchanged).
        if let Ok(mut guard) = SAVED_FOLD_STATE.lock() {
            if let Some(saved) = guard.as_ref() {
                if saved.document_id == document_id {
                    for (i, saved_folds) in saved.section_folds.iter().enumerate() {
                        if let Some(section) = sections.get_mut(i) {
                            // Only restore if content hasn't changed (byte offsets still valid).
                            let max_end = saved_folds.iter().map(|f| f.end).max().unwrap_or(0);
                            if max_end <= section.content.len() {
                                section.folds = saved_folds.clone();
                            }
                        }
                    }
                }
            }
            // Clear saved state once consumed (one-shot restore).
            *guard = None;
        }

        // Detect outline-only: all sections with headings have empty content,
        // and there are at least 2 sections. This triggers streaming mode.
        let streaming_sections = if sections.len() > 1
            && sections
                .iter()
                .all(|s| s.heading.is_empty() || s.content.trim().is_empty())
        {
            Some((0..sections.len()).collect::<HashSet<usize>>())
        } else {
            None
        };

        Self {
            document_id,
            title,
            sections,
            current_section: 0,
            scroll_offset: Cell::new(0),
            focus: ReaderFocus::Content,
            app_event_tx,
            complete: false,
            pending_sections: HashMap::new(),
            visited_sections,
            animations_enabled,
            frame_requester,
            textarea: TextArea::new(),
            textarea_state: RefCell::new(TextAreaState::default()),
            pending_g: false,
            pending_z: false,
            pending_quit: false,
            cursor_line: 0,
            cursor_col: 0,
            last_content_height: Cell::new(0),
            last_inner_width: Cell::new(40),
            search_state: None,
            search_input: String::new(),
            visual_select: None,
            selection_context: None,
            streaming_sections,
        }
    }

    /// If the resolved section is the one currently being viewed, dismiss
    /// the composer so the user sees the updated content.
    fn resolve_pending(&mut self, section_index: usize) {
        self.pending_sections.remove(&section_index);
        if section_index == self.current_section {
            // Clear visual selection — the response has arrived.
            self.visual_select = None;
            if self.focus == ReaderFocus::Composer {
                self.focus = ReaderFocus::Content;
            }
        }
    }

    /// Update a section's content (full replacement).
    pub(crate) fn update_section(&mut self, section_index: usize, content: String) {
        // Check if this is an initial streaming fill (not a user-triggered update).
        let is_streaming_fill = self
            .streaming_sections
            .as_ref()
            .is_some_and(|set| set.contains(&section_index));

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
            section.folds.clear();

            // Only show green "recently updated" highlight for actual edits,
            // not for the initial streaming fill.
            if !is_streaming_fill {
                section.recently_updated = true;
                section.changed_from_line = Some(0);
                section.changed_to_line = None;
            }
            section.invalidate_cache();
            self.resolve_pending(section_index);
            self.refresh_search();

            // Remove from streaming set; clear streaming when all filled.
            if let Some(ref mut set) = self.streaming_sections {
                set.remove(&section_index);
                if set.is_empty() {
                    self.streaming_sections = None;
                }
            }
        }
    }

    /// Append content to a section.
    pub(crate) fn append_to_section(
        &mut self,
        section_index: usize,
        content: String,
        foldable: bool,
        summary: Option<String>,
    ) {
        // Check if this resolves a pending follow-up question BEFORE
        // resolve_pending removes it — used for auto-fold below.
        let pending_question = self
            .pending_sections
            .get(&section_index)
            .map(|(q, _)| q.clone());

        if let Some(section) = self.sections.get_mut(section_index) {
            // Record the line index where new content starts (in the raw
            // content, before markdown rendering).  The heading adds ~2
            // rendered lines (heading text + blank) so we account for that in
            // the render loop, not here.
            let existing_line_count = section.content.lines().count();
            if !section.content.is_empty() && !section.content.ends_with('\n') {
                section.content.push('\n');
            }
            let fold_start = section.content.len();
            section.content.push_str(&content);

            // Record fold region: explicit foldable flag from the model, or
            // auto-fold when this resolves a follow-up question and the model
            // didn't set foldable explicitly (only if the content is long enough).
            let auto_fold = !foldable && pending_question.is_some() && content.len() >= 200;
            if foldable || auto_fold {
                // Auto-collapse previous folds so only the latest Q&A
                // answer stays expanded.
                for fold in &mut section.folds {
                    fold.collapsed = true;
                }
                let fold_summary = summary.or(pending_question).unwrap_or_else(|| {
                    content
                        .lines()
                        .next()
                        .unwrap_or("...")
                        .chars()
                        .take(60)
                        .collect()
                });
                section.folds.push(FoldRegion {
                    start: fold_start,
                    end: section.content.len(),
                    summary: fold_summary,
                    collapsed: false,
                });
            }

            section.recently_updated = true;
            section.changed_from_line = Some(existing_line_count);
            section.changed_to_line = None; // appended content runs to the end
            section.invalidate_cache();
            self.resolve_pending(section_index);

            // Auto-scroll to show the new content when appending to the
            // currently viewed section.  We set a large value here and let
            // `render()` clamp it to the actual maximum.
            if section_index == self.current_section {
                self.scroll_offset.set(u16::MAX);
            }
            self.refresh_search();
        }
    }

    /// Patch a section with find-and-replace.
    pub(crate) fn patch_section(
        &mut self,
        section_index: usize,
        old_text: &str,
        new_text: &str,
        foldable: bool,
        summary: Option<String>,
    ) {
        // Check if this resolves a pending follow-up question BEFORE
        // resolve_pending removes it — used for auto-fold below.
        let pending_question = self
            .pending_sections
            .get(&section_index)
            .map(|(q, _)| q.clone());

        if let Some(section) = self.sections.get_mut(section_index) {
            if let Some(byte_offset) = section.content.find(old_text) {
                let old_len = old_text.len();
                let lines_before_match = section.content[..byte_offset].matches('\n').count();

                // Compare old and new text line-by-line to narrow the
                // highlighted region to only the lines that actually differ.
                let old_lines: Vec<&str> = old_text.lines().collect();
                let new_lines: Vec<&str> = new_text.lines().collect();
                let common_prefix = old_lines
                    .iter()
                    .zip(new_lines.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                let max_suffix = old_lines
                    .len()
                    .saturating_sub(common_prefix)
                    .min(new_lines.len().saturating_sub(common_prefix));
                let common_suffix = old_lines
                    .iter()
                    .rev()
                    .zip(new_lines.iter().rev())
                    .take(max_suffix)
                    .take_while(|(a, b)| a == b)
                    .count();

                let changed_from = lines_before_match + common_prefix;
                let changed_to = lines_before_match + new_lines.len() - common_suffix;

                section.content = section.content.replacen(old_text, new_text, 1);

                // Shift existing fold regions to account for the length change.
                let delta = new_text.len() as isize - old_len as isize;
                for fold in &mut section.folds {
                    if fold.start >= byte_offset + old_len {
                        // Fold is entirely after the replaced region — shift both bounds.
                        fold.start = (fold.start as isize + delta).max(0) as usize;
                        fold.end = (fold.end as isize + delta).max(0) as usize;
                    } else if fold.end > byte_offset {
                        // Fold overlaps the replaced region — adjust end.
                        fold.end = (fold.end as isize + delta).max(fold.start as isize) as usize;
                    }
                }

                // Record fold region for the changed portion.
                // Auto-fold when this resolves a pending follow-up question,
                // even if the model didn't set foldable explicitly — but only
                // if the new text is substantially longer (≥3 more lines).
                let auto_fold = !foldable
                    && pending_question.is_some()
                    && new_text.len().saturating_sub(old_text.len()) >= 200;
                if foldable || auto_fold {
                    // Auto-collapse previous folds so only the latest Q&A
                    // answer stays expanded.
                    for fold in &mut section.folds {
                        fold.collapsed = true;
                    }
                    // Only fold the *changed* portion — compute common
                    // prefix/suffix at byte level so the fold covers inserted
                    // content, not the unchanged surrounding text.
                    let old_bytes = old_text.as_bytes();
                    let new_bytes = new_text.as_bytes();
                    let prefix_len = old_bytes
                        .iter()
                        .zip(new_bytes.iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    let max_suffix_bytes = old_bytes
                        .len()
                        .saturating_sub(prefix_len)
                        .min(new_bytes.len().saturating_sub(prefix_len));
                    let suffix_len = old_bytes
                        .iter()
                        .rev()
                        .zip(new_bytes.iter().rev())
                        .take(max_suffix_bytes)
                        .take_while(|(a, b)| a == b)
                        .count();

                    let fold_start = byte_offset + prefix_len;
                    let fold_end = byte_offset + new_text.len() - suffix_len;

                    if fold_start < fold_end {
                        let diff_text = &new_text[prefix_len..new_text.len() - suffix_len];
                        let fold_summary = summary.or(pending_question).unwrap_or_else(|| {
                            diff_text
                                .trim()
                                .lines()
                                .next()
                                .unwrap_or("...")
                                .chars()
                                .take(60)
                                .collect()
                        });
                        section.folds.push(FoldRegion {
                            start: fold_start,
                            end: fold_end,
                            summary: fold_summary,
                            collapsed: false,
                        });
                    }
                }

                section.recently_updated = true;
                section.changed_from_line = Some(changed_from);
                section.changed_to_line = if changed_to < section.content.lines().count() {
                    Some(changed_to)
                } else {
                    None // runs to end
                };
                section.invalidate_cache();
            }
            self.resolve_pending(section_index);
            self.refresh_search();
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
            self.scroll_offset.set(0);
            self.cursor_line = 0;
            self.cursor_col = 0;
            self.visited_sections.insert(self.current_section);
        }
    }

    fn prev_section(&mut self) {
        if self.current_section > 0 {
            self.clear_updated_flag();
            self.current_section -= 1;
            self.scroll_offset.set(0);
            self.cursor_line = 0;
            self.cursor_col = 0;
            self.visited_sections.insert(self.current_section);
        }
    }

    fn clear_updated_flag(&mut self) {
        if let Some(section) = self.sections.get_mut(self.current_section)
            && section.recently_updated
        {
            section.recently_updated = false;
            section.changed_from_line = None;
            section.changed_to_line = None;
            section.invalidate_cache();
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

        // Send brief feedback to the agent about user interaction.
        let total = self.sections.len();
        let viewed = self.visited_sections.len();
        let streaming_note = if let Some(ref set) = self.streaming_sections {
            if !set.is_empty() {
                " Some sections were still being generated. \
                 Stop calling update_document_section."
            } else {
                ""
            }
        } else {
            ""
        };
        let feedback = format!(
            "[The user closed the document reader for \"{}\". \
             They viewed {viewed} of {total} sections.{streaming_note}]",
            self.title,
        );
        self.app_event_tx.send(AppEvent::CodexOp(Op::UserInput {
            items: vec![UserInput::Text {
                text: feedback,
                text_elements: vec![],
            }],
            final_output_json_schema: None,
        }));

        // Save fold state so it can be restored if the same document is
        // re-opened in this session.
        self.save_fold_state();

        self.complete = true;
    }

    /// Persist current fold regions to the process-level static so they
    /// survive view close/reopen cycles.
    fn save_fold_state(&self) {
        let folds: Vec<Vec<FoldRegion>> = self.sections.iter().map(|s| s.folds.clone()).collect();
        // Only save if there are any folds worth preserving.
        if folds.iter().any(|f| !f.is_empty()) {
            if let Ok(mut guard) = SAVED_FOLD_STATE.lock() {
                *guard = Some(SavedFoldState {
                    document_id: self.document_id.clone(),
                    section_folds: folds,
                });
            }
        }
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

        let selection = self.selection_context.take();

        // Include the current section content so the agent can reliably
        // locate the right passage for inline patching.
        let section_content = self
            .sections
            .get(self.current_section)
            .map(|s| s.content.as_str())
            .unwrap_or("");

        // Formatting guidance shared by both selection and no-selection paths.
        // The goal: answers must be self-contained when re-read later without
        // remembering the original question.  A short italic lead-in line
        let formatting_guidance = "\
            Write your answer as straight prose that continues the section's voice. \
            Do NOT use a Q:/A: format. If the answer would be unclear without context, \
            a short italic lead-in is fine (e.g. *On dropout:* …), but skip it when \
            the meaning is obvious from placement. Don't overuse it.\n\n\
            SUMMARY (required): Always set the `summary` parameter to a short descriptive \
            label of your answer (5-10 words), e.g. summary=\"Role of attention heads in GPT\". \
            This is used as a section label regardless of foldable.\n\n\
            FOLDABLE CONTENT: For supplementary content (explanations, examples, deep dives), \
            set foldable=true. Direct answers, corrections, \
            and rewrites should NOT be foldable (foldable=false, the default).";

        // Extract a few rendered lines around the cursor and the word under
        // the cursor so the agent knows what the user was looking at when they
        // asked the question.  Helps resolve deictic references ("this", "here").
        let (cursor_context, cursor_word): (Option<String>, Option<String>) = if selection.is_none()
        {
            let inner_w = self.last_inner_width.get().max(80);
            self.sections
                .get(self.current_section)
                .and_then(|section| {
                    let lines = section.rendered_lines(inner_w);
                    if lines.is_empty() {
                        return None;
                    }
                    let cursor = self.cursor_line.min(lines.len().saturating_sub(1));
                    let start = cursor.saturating_sub(1);
                    let end = (cursor + 2).min(lines.len());
                    let snippet: Vec<String> = (start..end)
                        .map(|i| {
                            let prefix = if i == cursor { ">>  " } else { "    " };
                            let text: String =
                                lines[i].spans.iter().map(|s| s.content.as_ref()).collect();
                            format!("{prefix}{text}")
                        })
                        .collect();

                    // Extract the word under the cursor.
                    let cursor_line_text: String = lines[cursor]
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    let col = self.cursor_col.min(cursor_line_text.len());
                    let word = {
                        let bytes = cursor_line_text.as_bytes();
                        let word_start = bytes[..col]
                            .iter()
                            .rposition(|&b| b == b' ' || b == b'\t')
                            .map_or(0, |p| p + 1);
                        let word_end = bytes[col..]
                            .iter()
                            .position(|&b| b == b' ' || b == b'\t')
                            .map_or(bytes.len(), |p| col + p);
                        cursor_line_text
                            .get(word_start..word_end)
                            .unwrap_or("")
                            .trim_matches(|c: char| c.is_ascii_punctuation())
                            .to_string()
                    };
                    let word_opt = if word.is_empty() { None } else { Some(word) };

                    Some((Some(snippet.join("\n")), word_opt))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        let tool_instructions = if let Some(ref sel) = selection {
            // The user highlighted specific text — tell the agent to patch
            // the answer in right after the selection.
            format!(
                "The user selected specific text from the section (shown below) and is asking about it.\n\
                 [Selected text:]\n{sel}\n\n\
                 DEFAULT — insert your answer after the selection:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={idx}, \
                 old_text=\"<the selected text exactly>\", \
                 new_text=\"<the selected text>\\n\\n<your answer>\")\n\
                 This inserts your answer right after the selected passage. \
                 Reproduce the selected text verbatim as old_text so the patch matches.\n\n\
                 REWRITE — if the user asks to rewrite, simplify, or rephrase the selection:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={idx}, \
                 old_text=\"<the selected text exactly>\", \
                 new_text=\"<the rewritten version that replaces it>\")\n\
                 The new_text must NOT contain the old_text — it fully replaces it.\n\n\
                 {formatting_guidance}",
                doc_id = self.document_id,
                idx = self.current_section,
            )
        } else {
            // No selection — strongly prefer inline patch so answers appear
            // next to the passage they explain.
            let word_hint = cursor_word
                .as_deref()
                .map(|w| format!(" (cursor on the word \"{w}\")"))
                .unwrap_or_default();
            let cursor_hint = cursor_context
                .as_deref()
                .map(|ctx| {
                    format!(
                        "\nThe user's cursor was near this text{word_hint} \
                         (>> marks the cursor line):\n{ctx}\n\
                         If the question uses words like \"this\", \"here\", \"above\", etc., \
                         they likely refer to the passage near the cursor.\n"
                    )
                })
                .unwrap_or_default();
            format!(
                "Current section content:\n---\n{section_content}\n---\n\
                 {cursor_hint}\n\
                 PREFERRED — use patch_section to insert your answer inline:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={idx}, \
                 old_text=\"<the passage the question is about>\", \
                 new_text=\"<that same passage>\\n\\n<your answer>\")\n\
                 Find the most relevant passage (paragraph, bullet list, or sentence) \
                 and insert your answer right after it so it reads naturally in context. \
                 Copy old_text verbatim from the section content above.\n\n\
                 REWRITE — use patch_section to REPLACE content (not insert after) when \
                 the user explicitly asks to rewrite, simplify, restructure, or rephrase:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={idx}, \
                 old_text=\"<the passage to rewrite>\", \
                 new_text=\"<the rewritten version that replaces it>\")\n\
                 The new_text must NOT contain the old_text — it fully replaces it. \
                 Target the specific passage the user wants rewritten; \
                 do not rewrite the whole section unless the user asks for it.\n\n\
                 FULL SECTION REWRITE — use update_document_section ONLY when the user \
                 explicitly asks to rewrite, restructure, or simplify the entire section:\n\
                 update_document_section(document_id=\"{doc_id}\", section_index={idx}, \
                 content=\"<the complete rewritten section>\")\n\
                 This replaces all section content. Use sparingly — it removes any \
                 previous inline annotations. Only use when the user clearly wants \
                 the whole section replaced.\n\n\
                 FALLBACK — use append ONLY when the question is about the section as a whole \
                 and no specific passage is relevant:\n\
                 append_to_section(document_id=\"{doc_id}\", section_index={idx}, content=\"...\")\n\n\
                 {formatting_guidance}",
                doc_id = self.document_id,
                idx = self.current_section,
            )
        };

        let context = format!(
            "[The user is reading \"{title}\" and asked about the section titled \"{heading}\"]\n\n\
             {text}\n\n\
             {tool_instructions}\n\
             Do NOT rewrite the entire section unless the user explicitly asks for a rewrite. \
             Do NOT output plain text; only tool calls are visible to the user.",
            title = self.title,
            heading = heading,
        );

        self.app_event_tx.send(AppEvent::CodexOp(Op::UserInput {
            items: vec![UserInput::Text {
                text: context,
                text_elements: vec![],
            }],
            final_output_json_schema: None,
        }));
        self.pending_sections
            .insert(self.current_section, (text, Instant::now()));
        self.textarea = TextArea::new();
        *self.textarea_state.borrow_mut() = TextAreaState::default();
        // Keep composer focused — it will render the question + spinner while pending.
    }

    fn input_height(&self, width: u16) -> u16 {
        let max_h = (self.last_content_height.get() / 3).max(4);
        self.textarea.desired_height(width).clamp(1, max_h)
    }

    fn handle_content_key(&mut self, key_event: KeyEvent) {
        // Ctrl+d / Ctrl+u: half-page, Ctrl+f / Ctrl+b: full-page cursor jump.
        if key_event.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_g = false;
            self.pending_z = false;
            match key_event.code {
                KeyCode::Char('d') => {
                    let half = (self.last_content_height.get() / 2).max(1) as usize;
                    self.cursor_line = self.cursor_line.saturating_add(half);
                    self.clamp_and_scroll();
                    return;
                }
                KeyCode::Char('u') => {
                    let half = (self.last_content_height.get() / 2).max(1) as usize;
                    self.cursor_line = self.cursor_line.saturating_sub(half);
                    self.clamp_and_scroll();
                    return;
                }
                KeyCode::Char('f') => {
                    let page = self.last_content_height.get().max(1) as usize;
                    self.cursor_line = self.cursor_line.saturating_add(page);
                    self.clamp_and_scroll();
                    return;
                }
                KeyCode::Char('b') => {
                    let page = self.last_content_height.get().max(1) as usize;
                    self.cursor_line = self.cursor_line.saturating_sub(page);
                    self.clamp_and_scroll();
                    return;
                }
                _ => {}
            }
        }

        // Handle `z` prefix key (zM = collapse all, zR = expand all).
        if self.pending_z {
            self.pending_z = false;
            match key_event.code {
                KeyCode::Char('M') => {
                    self.collapse_all_folds();
                    return;
                }
                KeyCode::Char('R') => {
                    self.expand_all_folds();
                    return;
                }
                KeyCode::Char('a') => {
                    self.toggle_fold_at_cursor();
                    return;
                }
                _ => {} // invalid z-chord, fall through
            }
        }

        if key_event.code == KeyCode::Char('z')
            && !key_event.modifiers.contains(KeyModifiers::SHIFT)
            && self.visual_select.is_none()
        {
            self.pending_z = true;
            self.pending_g = false;
            return;
        }

        // Handle `g`-prefix chords: `gg` = go to top, `gx` = open link.
        if self.pending_g {
            self.pending_g = false;
            self.pending_z = false;
            match key_event.code {
                KeyCode::Char('g') if !key_event.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.cursor_line = 0;
                    self.cursor_col = 0;
                    self.clamp_and_scroll();
                    return;
                }
                KeyCode::Char('x') => {
                    self.open_url_at_cursor();
                    return;
                }
                _ => {
                    // Unknown g-chord: fall through to normal key handling.
                }
            }
        } else if key_event.code == KeyCode::Char('g')
            && !key_event.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.pending_g = true;
            self.pending_z = false;
            return;
        }
        self.pending_g = false;
        self.pending_z = false;

        // --- Keys shared between normal and visual modes ---

        // Full-page scroll (Ctrl-f / Ctrl-b).
        if key_event.modifiers.contains(KeyModifiers::CONTROL) {
            match key_event.code {
                KeyCode::Char('f') => {
                    let page = self.last_content_height.get() as usize;
                    self.cursor_line = self.cursor_line.saturating_add(page);
                    self.cursor_col = 0;
                    self.clamp_and_scroll();
                    return;
                }
                KeyCode::Char('b') => {
                    let page = self.last_content_height.get() as usize;
                    self.cursor_line = self.cursor_line.saturating_sub(page);
                    self.cursor_col = 0;
                    self.clamp_and_scroll();
                    return;
                }
                _ => {}
            }
        }

        match key_event.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor_line = self.cursor_line.saturating_add(1);
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor_line = self.cursor_line.saturating_sub(1);
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.cursor_col == 0 {
                    // Wrap to end of previous line.
                    if self.cursor_line > 0 {
                        self.cursor_line -= 1;
                        self.cursor_col = usize::MAX; // clamp_and_scroll caps it
                    }
                } else {
                    self.cursor_col -= 1;
                }
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let line_len = self.current_line_char_len();
                if self.cursor_col >= line_len {
                    // Wrap to start of next line.
                    self.cursor_line += 1;
                    self.cursor_col = 0;
                } else {
                    self.cursor_col += 1;
                }
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('G') => {
                self.cursor_line = usize::MAX;
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('0') => {
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('$') => {
                self.cursor_col = usize::MAX; // clamp_and_scroll caps it
                self.clamp_and_scroll();
                return;
            }
            // Paragraph navigation: jump to next/prev blank line.
            KeyCode::Char('}') => {
                self.jump_paragraph_down();
                return;
            }
            KeyCode::Char('{') => {
                self.jump_paragraph_up();
                return;
            }
            // Word navigation.
            KeyCode::Char('w') | KeyCode::Char('e') => {
                self.jump_word_forward();
                return;
            }
            KeyCode::Char('b') => {
                self.jump_word_backward();
                return;
            }
            // Viewport positioning.
            KeyCode::Char('H') => {
                self.cursor_line = self.scroll_offset.get() as usize;
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('M') => {
                let offset = self.scroll_offset.get() as usize;
                let height = self.last_content_height.get() as usize;
                self.cursor_line = offset + height / 2;
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            KeyCode::Char('L') => {
                let offset = self.scroll_offset.get() as usize;
                let height = self.last_content_height.get() as usize;
                self.cursor_line = offset + height.saturating_sub(1);
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            _ => {}
        }

        // --- Visual mode only ---
        if self.visual_select.is_some() {
            match key_event.code {
                KeyCode::Tab => {
                    let inner_w = self.last_inner_width.get();
                    let text = self.selected_text(inner_w);
                    if let Some(text) = text {
                        self.selection_context = Some(text);
                    }
                    self.focus = ReaderFocus::Composer;
                }
                KeyCode::Esc => {
                    self.visual_select = None;
                }
                // Toggle mode: v → char, V → line.  Same key cancels.
                KeyCode::Char('v') => {
                    let is_char = self
                        .visual_select
                        .as_ref()
                        .is_some_and(|vs| vs.mode == VisualMode::Char);
                    if is_char {
                        self.visual_select = None;
                    } else {
                        // Switch from line to char.
                        if let Some(vs) = &mut self.visual_select {
                            vs.mode = VisualMode::Char;
                        }
                    }
                }
                KeyCode::Char('V') => {
                    let is_line = self
                        .visual_select
                        .as_ref()
                        .is_some_and(|vs| vs.mode == VisualMode::Line);
                    if is_line {
                        self.visual_select = None;
                    } else if let Some(vs) = &mut self.visual_select {
                        vs.mode = VisualMode::Line;
                    }
                }
                KeyCode::Char('q') => {
                    self.visual_select = None;
                    if self.pending_quit {
                        self.pending_quit = false;
                        self.exit_reading_mode();
                    } else {
                        self.pending_quit = true;
                    }
                }
                KeyCode::Char('y') if self.pending_quit => {
                    self.visual_select = None;
                    self.pending_quit = false;
                    self.exit_reading_mode();
                }
                // Section navigation cancels visual mode.
                KeyCode::Char('n') | KeyCode::Char('p') | KeyCode::PageDown | KeyCode::PageUp => {
                    self.visual_select = None;
                    self.handle_content_key(key_event);
                }
                _ => {}
            }
            return;
        }

        // --- Normal mode only ---
        let has_search = self.search_state.is_some();

        match key_event.code {
            KeyCode::Char('n') => {
                if has_search {
                    self.next_match();
                } else {
                    self.next_section();
                }
            }
            KeyCode::Char('N') => {
                if has_search {
                    self.prev_match();
                }
            }
            KeyCode::PageDown | KeyCode::Enter => {
                self.next_section();
            }
            KeyCode::Char('p') | KeyCode::PageUp => {
                self.prev_section();
            }
            KeyCode::Char('v') => {
                self.visual_select = Some(VisualSelect {
                    mode: VisualMode::Char,
                    anchor_line: self.cursor_line,
                    anchor_col: self.cursor_col,
                });
            }
            KeyCode::Char('V') => {
                self.visual_select = Some(VisualSelect {
                    mode: VisualMode::Line,
                    anchor_line: self.cursor_line,
                    anchor_col: 0,
                });
            }
            KeyCode::Home => {
                self.current_section = 0;
                self.scroll_offset.set(0);
                self.cursor_line = 0;
                self.cursor_col = 0;
                self.visited_sections.insert(0);
            }
            KeyCode::End => {
                if !self.sections.is_empty() {
                    self.current_section = self.sections.len() - 1;
                    self.scroll_offset.set(0);
                    self.cursor_line = 0;
                    self.cursor_col = 0;
                    self.visited_sections.insert(self.current_section);
                }
            }
            KeyCode::Char('/') => {
                self.search_input.clear();
                self.focus = ReaderFocus::Search;
            }
            KeyCode::Esc => {
                if has_search {
                    self.clear_search();
                }
            }
            KeyCode::Char(' ') => {
                self.toggle_fold_at_cursor();
            }
            KeyCode::Char(']') => {
                self.jump_to_next_fold();
            }
            KeyCode::Char('[') => {
                self.jump_to_prev_fold();
            }
            KeyCode::Tab => {
                self.focus = ReaderFocus::Composer;
            }
            KeyCode::Char('q') => {
                if self.pending_quit {
                    self.pending_quit = false;
                    self.exit_reading_mode();
                } else {
                    self.pending_quit = true;
                }
            }
            KeyCode::Char('y') if self.pending_quit => {
                self.pending_quit = false;
                self.exit_reading_mode();
            }
            _ => {
                self.pending_quit = false;
            }
        }
    }

    /// Character length of the rendered line at `cursor_line`.
    fn current_line_char_len(&self) -> usize {
        let inner_w = self.last_inner_width.get();
        self.sections
            .get(self.current_section)
            .and_then(|s| {
                let lines = s.rendered_lines(inner_w);
                lines
                    .get(self.cursor_line)
                    .map(|l| l.spans.iter().map(|sp| sp.content.len()).sum::<usize>())
            })
            .unwrap_or(0)
    }

    /// Get the plain text of a rendered line by index.
    fn rendered_line_text(&self, line_idx: usize) -> Option<String> {
        let inner_w = self.last_inner_width.get();
        self.sections.get(self.current_section).and_then(|s| {
            s.rendered_lines(inner_w).get(line_idx).map(|l| {
                l.spans
                    .iter()
                    .map(|sp| sp.content.as_ref())
                    .collect::<String>()
            })
        })
    }

    /// Total rendered line count for the current section.
    fn current_section_line_count(&self) -> usize {
        let inner_w = self.last_inner_width.get();
        self.sections
            .get(self.current_section)
            .map(|s| s.rendered_lines(inner_w).len())
            .unwrap_or(0)
    }

    /// Jump cursor to the next blank line (vim `}`).
    fn jump_paragraph_down(&mut self) {
        let total = self.current_section_line_count();
        let mut line = self.cursor_line + 1;
        // Skip current non-blank lines.
        while line < total {
            if self
                .rendered_line_text(line)
                .is_some_and(|t| t.trim().is_empty())
            {
                break;
            }
            line += 1;
        }
        // Skip consecutive blank lines.
        while line < total {
            if self
                .rendered_line_text(line)
                .is_some_and(|t| !t.trim().is_empty())
            {
                break;
            }
            line += 1;
        }
        self.cursor_line = line;
        self.cursor_col = 0;
        self.clamp_and_scroll();
    }

    /// Jump cursor to the previous blank line (vim `{`).
    fn jump_paragraph_up(&mut self) {
        let mut line = self.cursor_line.saturating_sub(1);
        // Skip current non-blank lines.
        loop {
            if self
                .rendered_line_text(line)
                .is_some_and(|t| t.trim().is_empty())
            {
                break;
            }
            if line == 0 {
                self.cursor_line = 0;
                self.cursor_col = 0;
                self.clamp_and_scroll();
                return;
            }
            line -= 1;
        }
        // Skip consecutive blank lines.
        loop {
            if self
                .rendered_line_text(line)
                .is_some_and(|t| !t.trim().is_empty())
            {
                line += 1; // land on the blank line
                break;
            }
            if line == 0 {
                break;
            }
            line -= 1;
        }
        self.cursor_line = line;
        self.cursor_col = 0;
        self.clamp_and_scroll();
    }

    /// Jump cursor to the start of the next word (vim `w`).
    fn jump_word_forward(&mut self) {
        let text = self
            .rendered_line_text(self.cursor_line)
            .unwrap_or_default();
        let bytes = text.as_bytes();
        let col = self.cursor_col;

        // Skip current word chars.
        let mut pos = col;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip whitespace.
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        if pos < bytes.len() {
            self.cursor_col = pos;
        } else {
            // Wrap to next line.
            self.cursor_line += 1;
            self.cursor_col = 0;
        }
        self.clamp_and_scroll();
    }

    /// Jump cursor to the start of the previous word (vim `b`).
    fn jump_word_backward(&mut self) {
        let text = self
            .rendered_line_text(self.cursor_line)
            .unwrap_or_default();
        let bytes = text.as_bytes();

        if self.cursor_col == 0 {
            // Wrap to end of previous line.
            if self.cursor_line > 0 {
                self.cursor_line -= 1;
                self.cursor_col = usize::MAX;
                self.clamp_and_scroll();
            }
            return;
        }

        let mut pos = self.cursor_col.min(bytes.len()).saturating_sub(1);
        // Skip whitespace backwards.
        while pos > 0 && bytes[pos].is_ascii_whitespace() {
            pos -= 1;
        }
        // Skip word chars backwards.
        while pos > 0 && !bytes[pos - 1].is_ascii_whitespace() {
            pos -= 1;
        }
        self.cursor_col = pos;
        self.clamp_and_scroll();
    }

    /// Clamp cursor to valid bounds and adjust scroll_offset so that
    /// `self.cursor_line` is visible.  Must be called after every cursor
    /// mutation.
    fn clamp_and_scroll(&mut self) {
        // Clamp cursor_line to [0, total_lines - 1].
        let inner_w = self.last_inner_width.get();
        let total = self
            .sections
            .get(self.current_section)
            .map(|s| s.rendered_lines(inner_w).len())
            .unwrap_or(0);
        if total == 0 {
            self.cursor_line = 0;
            self.cursor_col = 0;
        } else {
            self.cursor_line = self.cursor_line.min(total - 1);
            // Clamp cursor_col to [0, line_char_len].
            let line_len = self.current_line_char_len();
            self.cursor_col = self.cursor_col.min(line_len);
        }

        // Scroll viewport to keep cursor visible.
        let content_h = self.last_content_height.get() as usize;
        let offset = self.scroll_offset.get() as usize;
        if self.cursor_line < offset {
            self.scroll_offset.set(self.cursor_line as u16);
        } else if content_h > 0 && self.cursor_line >= offset + content_h {
            self.scroll_offset
                .set((self.cursor_line - content_h + 1) as u16);
        }
    }

    /// Toggle the fold region under the cursor (if any).
    ///
    /// When collapsing, the cursor is moved to the collapsed summary line
    /// so the user stays on the fold they just toggled.
    fn toggle_fold_at_cursor(&mut self) {
        let Some(section) = self.sections.get_mut(self.current_section) else {
            return;
        };
        if section.folds.is_empty() {
            return;
        }

        let width = self.last_inner_width.get();
        let heading_lines: usize = if section.heading.is_empty() { 0 } else { 2 };

        // For each fold compute its post-fold start (and end for expanded
        // folds) using adjust_line_for_folds which correctly handles
        // collapsed folds in any order.
        let mut best_fold_idx: Option<usize> = None;
        let mut best_fold_start: usize = 0;

        for (i, fold) in section.folds.iter().enumerate() {
            let pre_start = heading_lines
                + render::rendered_body_line_count(
                    &section.content[..fold.start.min(section.content.len())],
                    width,
                );
            let adjusted_start = render::adjust_line_for_folds(
                pre_start,
                &section.content,
                heading_lines,
                width,
                &section.folds,
            );

            if fold.collapsed {
                // Collapsed fold occupies a single summary line.
                if self.cursor_line == adjusted_start {
                    best_fold_idx = Some(i);
                    best_fold_start = adjusted_start;
                    break;
                }
            } else {
                let pre_end = heading_lines
                    + render::rendered_body_line_count(
                        &section.content[..fold.end.min(section.content.len())],
                        width,
                    );
                let adjusted_end = render::adjust_line_for_folds(
                    pre_end,
                    &section.content,
                    heading_lines,
                    width,
                    &section.folds,
                );
                if self.cursor_line >= adjusted_start && self.cursor_line < adjusted_end {
                    best_fold_idx = Some(i);
                    best_fold_start = adjusted_start;
                    // Don't break — a nested fold might be more specific.
                }
            }
        }

        if let Some(idx) = best_fold_idx {
            let was_collapsed = section.folds[idx].collapsed;
            section.folds[idx].collapsed = !was_collapsed;
            section.invalidate_cache();

            // When collapsing, move cursor to the fold's summary line so
            // the user stays on the fold they just toggled.
            if !was_collapsed {
                self.cursor_line = best_fold_start;
                self.cursor_col = 0;
            }

            self.clamp_and_scroll();
        }
    }

    /// Collapse all folds in the current section.
    fn collapse_all_folds(&mut self) {
        if let Some(section) = self.sections.get_mut(self.current_section) {
            for fold in &mut section.folds {
                fold.collapsed = true;
            }
            section.invalidate_cache();
            self.clamp_and_scroll();
        }
    }

    /// Expand all folds in the current section.
    fn expand_all_folds(&mut self) {
        if let Some(section) = self.sections.get_mut(self.current_section) {
            for fold in &mut section.folds {
                fold.collapsed = false;
            }
            section.invalidate_cache();
            self.clamp_and_scroll();
        }
    }

    /// Compute the rendered line position of each fold's start in the current
    /// section, accounting for collapsed folds shifting lines up.
    fn fold_rendered_starts(&self) -> Vec<usize> {
        let Some(section) = self.sections.get(self.current_section) else {
            return vec![];
        };
        let width = self.last_inner_width.get();
        let heading_lines: usize = if section.heading.is_empty() { 0 } else { 2 };
        let mut positions = Vec::with_capacity(section.folds.len());

        for (i, fold) in section.folds.iter().enumerate() {
            let start_rendered = render::rendered_body_line_count(
                &section.content[..fold.start.min(section.content.len())],
                width,
            );
            let fold_rendered_start = heading_lines + start_rendered;

            let shift: usize = section.folds[..i]
                .iter()
                .filter(|f| f.collapsed)
                .map(|f| {
                    let fsl = render::rendered_body_line_count(
                        &section.content[..f.start.min(section.content.len())],
                        width,
                    );
                    let fel = render::rendered_body_line_count(
                        &section.content[..f.end.min(section.content.len())],
                        width,
                    );
                    fel.saturating_sub(fsl).max(1).saturating_sub(1)
                })
                .sum();

            positions.push(fold_rendered_start.saturating_sub(shift));
        }
        positions
    }

    /// Jump cursor to the next fold region's start line.
    fn jump_to_next_fold(&mut self) {
        let positions = self.fold_rendered_starts();
        if let Some(&pos) = positions.iter().find(|&&p| p > self.cursor_line) {
            self.cursor_line = pos;
            self.cursor_col = 0;
            self.clamp_and_scroll();
        }
    }

    /// Jump cursor to the previous fold region's start line.
    fn jump_to_prev_fold(&mut self) {
        let positions = self.fold_rendered_starts();
        if let Some(&pos) = positions.iter().rev().find(|&&p| p < self.cursor_line) {
            self.cursor_line = pos;
            self.cursor_col = 0;
            self.clamp_and_scroll();
        }
    }

    fn handle_composer_key(&mut self, key_event: KeyEvent) {
        // While the current section has a pending answer, block editing.
        let pending = self.pending_sections.contains_key(&self.current_section);

        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.focus = ReaderFocus::Content;
            }
            _ if pending => {
                // Ignore all other keys while waiting for the agent.
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

    fn handle_search_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Enter => {
                // Finalise search and return to content.
                self.execute_search();
                self.focus = ReaderFocus::Content;
            }
            KeyCode::Esc => {
                self.search_state = None;
                self.focus = ReaderFocus::Content;
            }
            KeyCode::Backspace => {
                self.search_input.pop();
                self.execute_search(); // incremental
            }
            KeyCode::Char(c) => {
                self.search_input.push(c);
                self.execute_search(); // incremental
            }
            _ => {}
        }
    }

    fn execute_search(&mut self) {
        let query = self.search_input.clone();
        if query.is_empty() {
            self.search_state = None;
            return;
        }

        let query_lower = query.to_lowercase();
        let mut matches = Vec::new();
        let section_idx = self.current_section;

        if let Some(section) = self.sections.get(section_idx) {
            // Search in heading.
            let heading_lower = section.heading.to_lowercase();
            let mut start = 0;
            while let Some(pos) = heading_lower[start..].find(&query_lower) {
                matches.push(SearchMatch {
                    section_idx,
                    byte_offset: start + pos,
                });
                start += pos + 1;
            }

            // Search in content, with byte_offset shifted past heading.
            let content_lower = section.content.to_lowercase();
            let heading_byte_len = section.heading.len();
            let mut start = 0;
            while let Some(pos) = content_lower[start..].find(&query_lower) {
                matches.push(SearchMatch {
                    section_idx,
                    byte_offset: heading_byte_len + start + pos,
                });
                start += pos + 1;
            }
        }

        let has_matches = !matches.is_empty();
        self.search_state = Some(SearchState {
            query,
            matches,
            current_match_idx: 0,
        });

        if has_matches {
            self.jump_to_current_match();
        }
    }

    fn next_match(&mut self) {
        if let Some(state) = &mut self.search_state {
            if state.matches.is_empty() {
                return;
            }
            state.current_match_idx = (state.current_match_idx + 1) % state.matches.len();
        }
        self.jump_to_current_match();
    }

    fn prev_match(&mut self) {
        if let Some(state) = &mut self.search_state {
            if state.matches.is_empty() {
                return;
            }
            state.current_match_idx = if state.current_match_idx == 0 {
                state.matches.len() - 1
            } else {
                state.current_match_idx - 1
            };
        }
        self.jump_to_current_match();
    }

    fn jump_to_current_match(&mut self) {
        // Extract match info before mutating self.
        let (section_idx, byte_offset) = {
            let Some(state) = &self.search_state else {
                return;
            };
            if state.matches.is_empty() {
                return;
            }
            let m = &state.matches[state.current_match_idx];
            (m.section_idx, m.byte_offset)
        };

        if section_idx != self.current_section {
            self.clear_updated_flag();
            self.current_section = section_idx;
            self.visited_sections.insert(self.current_section);
        }

        // Estimate the rendered line for this match by counting newlines in the
        // heading + content up to byte_offset. The heading adds ~2 lines
        // (heading text + blank line).
        let section = &self.sections[section_idx];
        let heading_lines: u16 = if section.heading.is_empty() { 0 } else { 2 };
        let text = &section.content;
        // byte_offset is relative to heading start; content starts after heading.
        let content_offset = byte_offset.saturating_sub(section.heading.len());
        let safe_offset = content_offset.min(text.len());
        let newlines_before = text[..safe_offset].matches('\n').count() as u16;
        let estimated_line = heading_lines + newlines_before;

        // Move cursor to the match and center in viewport.
        self.cursor_line = estimated_line as usize;
        let half_viewport = self.last_content_height.get() / 2;
        self.scroll_offset
            .set(estimated_line.saturating_sub(half_viewport));
    }

    fn clear_search(&mut self) {
        self.search_state = None;
    }

    /// Re-run the current search after content changes.
    fn refresh_search(&mut self) {
        if self.search_state.is_some() {
            self.execute_search();
        }
    }

    /// Extract the plain text of the current selection (visual mode).
    fn selected_text(&self, inner_width: u16) -> Option<String> {
        let vs = self.visual_select.as_ref()?;
        let section = self.sections.get(self.current_section)?;
        let lines = section.rendered_lines(inner_width);
        if lines.is_empty() {
            return None;
        }

        let (start_line, start_col, end_line, end_col) = self.selection_bounds(vs, lines.len())?;

        let line_text = |idx: usize| -> String {
            lines
                .get(idx)
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .unwrap_or_default()
        };

        if vs.mode == VisualMode::Line {
            let selected: String = (start_line..=end_line)
                .map(line_text)
                .collect::<Vec<_>>()
                .join("\n");
            return Some(selected);
        }

        // Character-wise selection.
        if start_line == end_line {
            let text = line_text(start_line);
            let s = start_col.min(text.len());
            let e = end_col.min(text.len());
            return Some(text[s..e].to_string());
        }

        let mut parts: Vec<String> = Vec::new();
        // First line: from start_col to end.
        let first = line_text(start_line);
        parts.push(
            first
                .get(start_col.min(first.len())..)
                .unwrap_or("")
                .to_string(),
        );
        // Middle lines: full.
        for idx in (start_line + 1)..end_line {
            parts.push(line_text(idx));
        }
        // Last line: from start to end_col.
        let last = line_text(end_line);
        parts.push(
            last.get(..end_col.min(last.len()))
                .unwrap_or("")
                .to_string(),
        );
        Some(parts.join("\n"))
    }

    /// Open the URL nearest to the cursor on the current line in the browser.
    ///
    /// Scans the rendered text of `cursor_line` for `http://` or `https://`
    /// URLs and picks the one closest to `cursor_col`. Emits
    /// `AppEvent::OpenUrlInBrowser` if found; does nothing otherwise.
    fn open_url_at_cursor(&self) {
        let Some(section) = self.sections.get(self.current_section) else {
            return;
        };
        let inner_width = self.last_inner_width.get();
        let lines = section.rendered_lines(inner_width);
        let Some(line) = lines.get(self.cursor_line) else {
            return;
        };

        // Build the plain text of the line from spans.
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();

        // Extract all URLs from the line text.
        let urls = extract_urls(&text);
        if urls.is_empty() {
            return;
        }

        // Pick the URL nearest to cursor_col.
        let col = self.cursor_col;
        let best = urls.into_iter().min_by_key(|(start, end, _)| {
            if col >= *start && col < *end {
                0usize
            } else if col < *start {
                *start - col
            } else {
                col - *end + 1
            }
        });

        if let Some((_, _, url)) = best {
            self.app_event_tx.send(AppEvent::OpenUrlInBrowser { url });
        }
    }

    /// Compute normalised selection bounds: (start_line, start_col, end_line, end_col).
    /// For line mode, start_col=0 and end_col=usize::MAX (callers must clamp to
    /// line width before use).
    fn selection_bounds(
        &self,
        vs: &VisualSelect,
        total_lines: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        if total_lines == 0 {
            return None;
        }
        let max_line = total_lines - 1;
        let cursor_line = self.cursor_line.min(max_line);
        let anchor_line = vs.anchor_line.min(max_line);

        if vs.mode == VisualMode::Line {
            let start = anchor_line.min(cursor_line);
            let end = anchor_line.max(cursor_line);
            return Some((start, 0, end, usize::MAX));
        }

        // Character-wise: order by (line, col).
        let (start_line, start_col, end_line, end_col) =
            if (anchor_line, vs.anchor_col) <= (cursor_line, self.cursor_col) {
                (anchor_line, vs.anchor_col, cursor_line, self.cursor_col)
            } else {
                (cursor_line, self.cursor_col, anchor_line, vs.anchor_col)
            };
        Some((start_line, start_col, end_line, end_col))
    }
}

impl BottomPaneView for DocumentReaderView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self.focus {
            ReaderFocus::Content => self.handle_content_key(key_event),
            ReaderFocus::Composer => self.handle_composer_key(key_event),
            ReaderFocus::Search => self.handle_search_key(key_event),
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.exit_reading_mode();
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        // In composer/search focus, Esc should switch back to content rather than dismiss.
        // In content focus with active search, Esc should clear search rather than exit.
        // In visual select mode, Esc should cancel the selection.
        matches!(self.focus, ReaderFocus::Composer | ReaderFocus::Search)
            || self.search_state.is_some()
            || self.visual_select.is_some()
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn view_id(&self) -> Option<&'static str> {
        Some(DOCUMENT_READER_VIEW_ID)
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if pasted.is_empty() {
            return false;
        }
        match self.focus {
            ReaderFocus::Composer => {
                self.textarea.insert_str(&pasted);
                true
            }
            ReaderFocus::Search => {
                self.search_input.push_str(&pasted);
                true
            }
            ReaderFocus::Content => false,
        }
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
        foldable: bool,
        summary: Option<String>,
    ) {
        if self.document_id == document_id {
            self.append_to_section(section_index, content, foldable, summary);
        }
    }

    fn handle_document_section_patch(
        &mut self,
        document_id: &str,
        section_index: usize,
        old_text: &str,
        new_text: &str,
        foldable: bool,
        summary: Option<String>,
    ) {
        if self.document_id == document_id {
            self.patch_section(section_index, old_text, new_text, foldable, summary);
        }
    }

    fn handle_turn_complete(&mut self) {
        // If any sections were pending updates but the turn ended without
        // the agent calling an update tool, clear all pending state so the
        // user isn't stuck with permanent spinner indicators.
        if !self.pending_sections.is_empty() {
            self.pending_sections.clear();
            self.visual_select = None;
            if self.focus == ReaderFocus::Composer {
                self.focus = ReaderFocus::Content;
            }
        }
    }
}

impl Renderable for DocumentReaderView {
    fn desired_height(&self, width: u16) -> u16 {
        // Content lines inside the card (accounting for 2-char padding each side).
        let inner_width = width.saturating_sub(4);

        // When the current section is streaming-empty, the render path uses
        // `render_section_loading` (3 lines: heading + blank + indicator)
        // instead of `rendered_lines` (2 lines: heading + blank).  Use the
        // same line count here so the card requests enough vertical space
        // for the loading indicator to be visible.
        let is_streaming_empty = self
            .streaming_sections
            .as_ref()
            .is_some_and(|set| set.contains(&self.current_section))
            && self
                .sections
                .get(self.current_section)
                .is_some_and(|s| s.content.trim().is_empty());

        let section_lines = if is_streaming_empty {
            let heading = self
                .sections
                .get(self.current_section)
                .map_or("", |s| s.heading.as_str());
            render::render_section_loading(heading, false).len() as u16
        } else {
            self.sections
                .get(self.current_section)
                .map(|s| s.rendered_lines(inner_width).len() as u16)
                .unwrap_or(1)
        };

        let extra_rows = match self.focus {
            ReaderFocus::Composer => 1 + self.input_height(inner_width), // separator + input
            ReaderFocus::Search => 1 + 1,                                // separator + search bar
            ReaderFocus::Content => 0,
        };

        // top_border(1) + header(1) + separator(1) + content + separator(1) + hints(1)
        //   + extra_rows + bottom_border(1)
        let ideal = 1 + 1 + 1 + section_lines + 1 + 1 + extra_rows + 1;
        ideal.max(8)
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

        // --- Extra rows below content for composer or search bar ---
        let extra_bottom_rows: u16 = match self.focus {
            ReaderFocus::Composer => {
                let inner_w = w.saturating_sub(4);
                1 + self.input_height(inner_w) // separator + input
            }
            ReaderFocus::Search => 1 + 1, // separator + search bar
            ReaderFocus::Content => 0,
        };

        // --- Fixed rows from bottom: bottom_border(1) + extra_bottom_rows + hints(1) + separator(1) ---
        let fixed_bottom = 1 + extra_bottom_rows + 1 + 1;
        // --- Fixed rows from top: top_border(1) + header(1) + separator(1) ---
        let fixed_top: u16 = 3;
        let content_height = area
            .height
            .saturating_sub(fixed_top)
            .saturating_sub(fixed_bottom);

        self.last_content_height.set(content_height);

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

        // Header — no longer shows "thinking..." here; the spinner is in the composer.
        let current_pending = self.pending_sections.contains_key(&self.current_section);
        let streaming_status: Option<String> = self.streaming_sections.as_ref().and_then(|set| {
            if set.is_empty() {
                return None;
            }
            let filled = self.sections.len() - set.len();
            Some(format!(
                "generating {}/{}\u{2026}",
                filled + 1,
                self.sections.len()
            ))
        });
        let header = render::header_line(
            &self.title,
            section_num,
            section_count,
            false,
            streaming_status.as_deref(),
            w,
        );
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
        let mut has_more_below = false;
        if content_height > 0 {
            let inner_width = w.saturating_sub(4);
            self.last_inner_width.set(inner_width);
            if let Some(section) = self.sections.get(self.current_section) {
                // If this section is awaiting content during streaming, show
                // the loading indicator instead of the normal rendered lines.
                let is_streaming_empty = self
                    .streaming_sections
                    .as_ref()
                    .is_some_and(|set| set.contains(&self.current_section))
                    && section.content.trim().is_empty();

                let mut raw_lines = if is_streaming_empty {
                    render::render_section_loading(&section.heading, self.animations_enabled)
                } else {
                    section.rendered_lines(inner_width)
                };

                // Compute the rendered-line range that should get green
                // borders.  We render the unchanged prefix/suffix of the raw
                // content separately to get an accurate rendered-line count
                // (markdown word-wrapping can change line counts).
                let heading_rendered_lines: usize = if section.heading.is_empty() { 0 } else { 2 };
                let changed_from_rendered: Option<usize> = if section.recently_updated {
                    section.changed_from_line.map(|content_line| {
                        let prefix: String = section
                            .content
                            .lines()
                            .take(content_line)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let pre_fold = heading_rendered_lines
                            + render::rendered_body_line_count(&prefix, inner_width);
                        render::adjust_line_for_folds(
                            pre_fold,
                            &section.content,
                            heading_rendered_lines,
                            inner_width,
                            &section.folds,
                        )
                    })
                } else {
                    None
                };
                let changed_to_rendered: Option<usize> = if section.recently_updated {
                    section.changed_to_line.map(|content_line| {
                        let prefix: String = section
                            .content
                            .lines()
                            .take(content_line)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let pre_fold = heading_rendered_lines
                            + render::rendered_body_line_count(&prefix, inner_width);
                        render::adjust_line_for_folds(
                            pre_fold,
                            &section.content,
                            heading_rendered_lines,
                            inner_width,
                            &section.folds,
                        )
                    })
                } else {
                    None
                };

                // Compute scroll overflow from section content alone — the
                // pending indicator is transient chrome and should not trigger
                // the "▼ scroll for more" indicator.
                let content_total = raw_lines.len();

                // Append pending indicator if the current section has a pending question.
                if let Some((question, _)) = self.pending_sections.get(&self.current_section) {
                    raw_lines.extend(render::pending_indicator_lines(question, inner_width));
                }

                let total = raw_lines.len();

                // Clamp cursor for rendering (the actual field is clamped
                // in handle_content_key; here we just guard against stale
                // values after content changes).
                // Clamp scroll_offset so it never overshoots the content.
                let max_offset = total.saturating_sub(content_height as usize) as u16;
                let offset = self.scroll_offset.get().min(max_offset);
                self.scroll_offset.set(offset);

                has_more_below = (offset as usize) + (content_height as usize) < content_total;

                // Apply search highlights if a search is active.
                let search_query = self
                    .search_state
                    .as_ref()
                    .filter(|s| !s.query.is_empty())
                    .map(|s| s.query.as_str());

                let visible: Vec<Line<'static>> = raw_lines
                    .into_iter()
                    .skip(offset as usize)
                    .take(content_height as usize)
                    .map(|line| {
                        if let Some(query) = search_query {
                            render::apply_search_highlights(line, query)
                        } else {
                            line
                        }
                    })
                    .collect();

                // Compute visual selection bounds (if active).
                let sel_bounds = self
                    .visual_select
                    .as_ref()
                    .and_then(|vs| self.selection_bounds(vs, total));

                // Render each visible line wrapped in side borders.
                for (i, line) in visible.into_iter().enumerate() {
                    let row_y = y + i as u16;
                    if row_y >= bottom {
                        break;
                    }
                    let abs_line = offset as usize + i;
                    let is_changed = changed_from_rendered.is_some_and(|from| abs_line >= from)
                        && changed_to_rendered.is_none_or(|to| abs_line < to);

                    // Apply char-level selection highlight when inside the
                    // visual selection range, then wrap in side borders.
                    let bordered = if let Some((sl, sc, el, ec)) = sel_bounds {
                        if abs_line >= sl && abs_line <= el {
                            let line_text_len: usize =
                                line.spans.iter().map(|s| s.content.len()).sum();
                            let (col_start, col_end) = if sl == el {
                                (sc.min(line_text_len), ec.min(line_text_len))
                            } else if abs_line == sl {
                                (sc.min(line_text_len), line_text_len)
                            } else if abs_line == el {
                                (0, ec.min(line_text_len))
                            } else {
                                (0, line_text_len)
                            };
                            let highlighted =
                                render::apply_char_selection(line, col_start, col_end);
                            render::bordered_line(highlighted, w, is_changed)
                        } else {
                            render::bordered_line(line, w, is_changed)
                        }
                    } else {
                        render::bordered_line(line, w, is_changed)
                    };
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
                let rendered_count = total.saturating_sub(offset as usize);
                let filled = rendered_count.min(content_height as usize);
                for i in filled..content_height as usize {
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

        // Schedule periodic refresh while streaming is active so loading
        // indicators stay visually responsive even between section updates.
        if self
            .streaming_sections
            .as_ref()
            .is_some_and(|set| !set.is_empty())
        {
            self.frame_requester
                .schedule_frame_in(Duration::from_millis(100));
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

        // Composer (if focused) or Search bar (if searching)
        match self.focus {
            ReaderFocus::Composer => {
                // When a question is pending, show the question text + spinner instead
                // of the editable textarea.
                let pending_data = if current_pending {
                    self.pending_sections.get(&self.current_section).cloned()
                } else {
                    None
                };

                if let Some((question, _start_time)) = pending_data {
                    // Render the question with shimmer animation.
                    let inner_w = w.saturating_sub(4);
                    let display = format!("\u{25B8} {question}");
                    let composer_lines: Vec<Line<'static>> = if self.animations_enabled {
                        let shimmer = crate::shimmer::shimmer_spans(&display);
                        vec![Line::from(shimmer)]
                    } else {
                        textwrap::wrap(&display, inner_w.max(1) as usize)
                            .iter()
                            .map(|cow| Line::from(cow.to_string().dim().italic()))
                            .collect()
                    };
                    self.frame_requester
                        .schedule_frame_in(Duration::from_millis(32));

                    let input_h = (composer_lines.len() as u16).clamp(1, 4);
                    by = by.saturating_sub(input_h);
                    let ta_area = Rect {
                        x: area.x + 2,
                        y: by,
                        width: inner_w,
                        height: input_h,
                    };
                    render::draw_side_borders(buf, area.x, w, by, input_h, bottom);
                    Paragraph::new(composer_lines).render(ta_area, buf);
                } else {
                    let input_h = self.input_height(w.saturating_sub(4));
                    by = by.saturating_sub(input_h);
                    let ta_area = Rect {
                        x: area.x + 2,
                        y: by,
                        width: w.saturating_sub(4),
                        height: input_h,
                    };
                    render::draw_side_borders(buf, area.x, w, by, input_h, bottom);
                    let mut state = self.textarea_state.borrow_mut();
                    StatefulWidgetRef::render_ref(&(&self.textarea), ta_area, buf, &mut state);
                    if self.textarea.text().is_empty() {
                        Paragraph::new("Ask about this section...".dim().italic())
                            .render(ta_area, buf);
                    }
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
            ReaderFocus::Search => {
                // 1-line search bar: "/ " prefix + query text
                by = by.saturating_sub(1);
                let inner_w = w.saturating_sub(4);
                let search_bar_area = Rect {
                    x: area.x + 2,
                    y: by,
                    width: inner_w,
                    height: 1,
                };
                render::draw_side_borders(buf, area.x, w, by, 1, bottom);
                let match_info = self
                    .search_state
                    .as_ref()
                    .filter(|s| !s.matches.is_empty())
                    .map(|s| format!(" [{}/{}]", s.current_match_idx + 1, s.matches.len()));
                let search_line = Line::from(vec![
                    "/".cyan(),
                    Span::from(self.search_input.clone()),
                    match_info.map_or_else(Span::default, |info| Span::from(info).dim()),
                ]);
                Paragraph::new(search_line).render(search_bar_area, buf);

                // Separator above search bar
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
            ReaderFocus::Content => {}
        }

        // Hints bar
        by = by.saturating_sub(1);
        let current_has_folds = self
            .sections
            .get(self.current_section)
            .is_some_and(DocumentSection::has_folds);
        let hints = render::hints_line(
            self.focus == ReaderFocus::Composer,
            self.focus == ReaderFocus::Search,
            self.search_state.is_some(),
            self.visual_select.is_some(),
            current_has_folds,
            self.pending_quit,
            w,
        );
        Paragraph::new(hints).render(
            Rect {
                y: by,
                height: 1,
                ..area
            },
            buf,
        );

        // Separator above hints (with scroll indicator when content overflows).
        by = by.saturating_sub(1);
        let sep = if has_more_below {
            render::separator_with_indicator(w, " \u{25BC} scroll for more ")
        } else {
            render::separator(w)
        };
        Paragraph::new(sep).render(
            Rect {
                y: by,
                height: 1,
                ..area
            },
            buf,
        );
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        match self.focus {
            ReaderFocus::Composer => {
                // Cursor is inside the textarea area, which is inset 2 chars from card edge.
                let input_h = self.input_height(area.width.saturating_sub(4));
                let bottom_y = area.y + area.height - 1; // bottom border
                let ta_y = bottom_y.saturating_sub(input_h);
                let text_len = self.textarea.text().lines().last().map_or(0, str::len);
                Some((area.x + 2 + text_len as u16, ta_y))
            }
            ReaderFocus::Search => {
                // Cursor after "/" prefix + search text.
                let bottom_y = area.y + area.height - 1; // bottom border
                // Search bar is 1 row above bottom border.
                let search_y = bottom_y.saturating_sub(1);
                // +1 for "/" prefix
                let x = area.x + 2 + 1 + self.search_input.len() as u16;
                Some((x, search_y))
            }
            ReaderFocus::Content => {
                let fixed_top: u16 = 3; // top_border + header + separator
                let offset = self.scroll_offset.get() as usize;
                let cursor = self.cursor_line;
                let content_h = self.last_content_height.get() as usize;
                if cursor < offset || (content_h > 0 && cursor >= offset + content_h) {
                    return None; // cursor not in visible viewport
                }
                let screen_y = area.y + fixed_top + (cursor - offset) as u16;
                // Clamp cursor_col to the line's text length.
                let inner_width = area.width.saturating_sub(4);
                let line_len = self
                    .sections
                    .get(self.current_section)
                    .map(|s| {
                        let lines = s.rendered_lines(inner_width);
                        lines.get(cursor).map_or(0, |l| {
                            l.spans.iter().map(|sp| sp.content.len()).sum::<usize>()
                        })
                    })
                    .unwrap_or(0);
                let col = self.cursor_col.min(line_len);
                let screen_x = area.x + 2 + col as u16; // +2 for "│ " border
                Some((screen_x, screen_y))
            }
        }
    }
}

/// Parse markdown content into sections split on `## ` headings.
///
/// Extract all URLs from a line of text.
///
/// Returns `(start_byte, end_byte, url_string)` tuples. URLs are sequences
/// starting with `http://` or `https://` and extending to the next whitespace
/// or `)` character (since markdown-rendered links use `text (https://...)`
/// format).
fn extract_urls(text: &str) -> Vec<(usize, usize, String)> {
    let mut urls = Vec::new();
    let mut search_from = 0;
    while search_from < text.len() {
        let haystack = &text[search_from..];
        let offset = if let Some(pos) = haystack.find("https://") {
            pos
        } else if let Some(pos) = haystack.find("http://") {
            pos
        } else {
            break;
        };
        let start = search_from + offset;
        // Extend to the next whitespace, ')' or end of string.
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || c == ')')
            .map_or(text.len(), |pos| start + pos);
        if end > start {
            urls.push((start, end, text[start..end].to_string()));
        }
        search_from = end;
    }
    urls
}

/// Content before the first `## ` becomes section 0 with the document title
/// as heading.
fn parse_sections(_title: &str, content: &str) -> Vec<DocumentSection> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_content = String::new();

    for line in content.lines() {
        if let Some(heading_text) = line.strip_prefix("## ") {
            // Flush the previous section.
            sections.push(DocumentSection {
                heading: current_heading,
                content: current_content,
                rendered: RefCell::new(None),
                recently_updated: false,
                changed_from_line: None,
                changed_to_line: None,
                folds: Vec::new(),
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
        changed_from_line: None,
        changed_to_line: None,
        folds: Vec::new(),
    });

    // Drop the empty preamble section when the document starts with `## `.
    if sections.len() > 1 && sections[0].heading.is_empty() && sections[0].content.trim().is_empty()
    {
        sections.remove(0);
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::history_cell::DocumentCell;
    use crate::render::renderable::Renderable;
    use codex_protocol::protocol::Op;
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
        "Introduction paragraph line one.\n\
         Introduction paragraph line two.\n\
         Introduction paragraph line three.\n\
         Introduction paragraph line four.\n\
         Introduction paragraph line five.\n\
         ## Methodology\n\
         Method details here.\n\
         More method details.\n\
         ## Results\n\
         Result findings.\n\
         More result findings.\n\
         ## Discussion\n\
         Discussion text.\n\
         More discussion text."
            .to_string()
    }

    fn make_view(tx: AppEventSender) -> DocumentReaderView {
        DocumentReaderView::new(
            "test-doc".to_string(),
            "Test Report".to_string(),
            test_content(),
            tx,
            true,
            crate::tui::FrameRequester::test_dummy(),
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
        assert_eq!(sections[0].heading, "");
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
        assert_eq!(sections[0].heading, "");
        assert!(sections[0].content.contains("preamble"));
        assert_eq!(sections[1].heading, "First");
        assert_eq!(sections[1].content, "Body");
    }

    #[test]
    fn parse_sections_no_headings() {
        let content = "Just a single block of text\nwith multiple lines";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "");
    }

    #[test]
    fn parse_sections_single_heading() {
        let content = "## Only Section\nContent here";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "Only Section");
        assert_eq!(sections[0].content, "Content here");
    }

    #[test]
    fn parse_sections_empty_sections() {
        let content = "## A\n## B\n## C";
        let sections = parse_sections("Title", content);
        // Empty preamble is dropped, leaving 3 sections.
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].heading, "A");
        assert!(sections[0].content.is_empty());
        assert!(sections[1].content.is_empty());
    }

    // -----------------------------------------------------------------------
    // Rendering tests
    // -----------------------------------------------------------------------

    #[test]
    fn streaming_outline_shows_generating_indicator() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        // Outline-only content: all sections have headings but no body.
        let outline = "## Overview\n\n## Methodology\n\n## Results";
        let view = DocumentReaderView::new(
            "test-streaming".to_string(),
            "Streaming Test".to_string(),
            outline.to_string(),
            tx,
            false, // animations_enabled = false so we get static text
            crate::tui::FrameRequester::test_dummy(),
        );
        // streaming_sections should be active.
        assert!(
            view.streaming_sections.is_some(),
            "streaming should be detected for outline-only content"
        );
        let area = Rect::new(0, 0, 60, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        eprintln!("--- streaming outline snapshot ---\n{snap}\n---");
        assert!(
            snap.contains("Generating"),
            "loading indicator should show for unfilled streaming section"
        );
        assert!(
            !snap.contains("scroll for more"),
            "scroll indicator should NOT show for streaming-empty section"
        );
    }

    #[test]
    fn citation_annotations_are_stripped() {
        let input = "Some text \u{e200}cite\u{e202}turn2view0\u{e201} more text";
        let result = render::strip_citation_annotations(input);
        assert_eq!(result, "Some text more text");

        // Multiple citations.
        let input2 = "A \u{e200}cite\u{e202}turn0view0\u{e202}turn2view0\u{e201} B \u{e200}cite\u{e202}turn2view1\u{e201} C";
        let result2 = render::strip_citation_annotations(input2);
        assert_eq!(result2, "A B C");

        // No citations — passthrough.
        let plain = "No citations here";
        assert_eq!(render::strip_citation_annotations(plain), plain);
    }

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
            snap.contains("move"),
            "hints bar should show cursor movement keys"
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

        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 2);

        view.handle_content_key(key(KeyCode::PageDown));
        assert_eq!(view.current_section, 3);

        // Should not go past the last section.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 3);

        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 2);

        view.handle_content_key(key(KeyCode::Char('p')));
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
    fn cursor_moves_within_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.cursor_line, 0);

        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(view.cursor_line, 1);

        view.handle_content_key(key(KeyCode::Down));
        assert_eq!(view.cursor_line, 2);

        view.handle_content_key(key(KeyCode::Char('k')));
        assert_eq!(view.cursor_line, 1);

        view.handle_content_key(key(KeyCode::Up));
        assert_eq!(view.cursor_line, 0);

        // Should not go below zero.
        view.handle_content_key(key(KeyCode::Char('k')));
        assert_eq!(view.cursor_line, 0);
    }

    #[test]
    fn navigation_resets_cursor() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_content_key(key(KeyCode::Char('j')));
        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(view.cursor_line, 2);

        // Navigate to next section — cursor and scroll should reset.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.cursor_line, 0);
        assert_eq!(view.scroll_offset.get(), 0);
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
        assert!(
            !view.complete,
            "first q should not exit — pending confirmation"
        );
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
        assert!(view.pending_sections.contains_key(&2));
        // Composer should be cleared and focus stays on composer (shows question + spinner).
        assert!(view.textarea.text().is_empty());
        assert_eq!(view.focus, ReaderFocus::Composer);

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
                    text.contains("section_index=2"),
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

        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.update_section(1, "Updated methodology content.".to_string());

        assert!(
            !view.pending_sections.contains_key(&1),
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

        view.pending_sections
            .insert(99, ("test question".into(), Instant::now()));
        view.update_section(99, "Does not exist.".to_string());

        // pending should stay since the update was ignored (section doesn't exist).
        assert!(view.pending_sections.contains_key(&99));
    }

    #[test]
    fn handle_document_section_update_via_trait() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections
            .insert(2, ("test question".into(), Instant::now()));

        // Matching document_id should update.
        view.handle_document_section_update("test-doc", 2, "New results.".to_string());
        assert!(!view.pending_sections.contains_key(&2));
        assert_eq!(view.sections[2].content, "New results.");

        // Non-matching document_id should be ignored.
        view.pending_sections
            .insert(0, ("test question".into(), Instant::now()));
        view.handle_document_section_update("other-doc", 0, "Ignored.".to_string());
        assert!(view.pending_sections.contains_key(&0));
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
        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.pending_sections
            .insert(2, ("test question".into(), Instant::now()));

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

        assert!(view.pending_sections.contains_key(&2));
        assert_eq!(view.focus, ReaderFocus::Composer);

        // Drain the UserInput event.
        let mut got_input = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::CodexOp(Op::UserInput { items, .. }) = ev
                && let UserInput::Text { text, .. } = &items[0]
            {
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
        assert!(!view.pending_sections.contains_key(&2));

        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
        assert!(
            snap.contains("Expanded result"),
            "updated content should be visible"
        );

        // 6. Exit via 'q' (requires confirmation: q, then q again).
        assert!(!view.is_complete());
        view.handle_key_event(key(KeyCode::Char('q')));
        assert!(
            !view.is_complete(),
            "first q should not exit — pending confirmation"
        );
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

        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.append_to_section(1, "Additional details here.".to_string(), false, None);

        assert!(view.sections[1].content.contains("Method details here."));
        assert!(
            view.sections[1]
                .content
                .contains("Additional details here.")
        );
        assert!(view.sections[1].recently_updated);
        assert!(!view.pending_sections.contains_key(&1));
    }

    #[test]
    fn append_via_trait_method() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.handle_document_section_append(
            "test-doc",
            1,
            "Extra content.".to_string(),
            false,
            None,
        );
        assert!(view.sections[1].content.contains("Extra content."));
        assert!(view.sections[1].recently_updated);

        // Non-matching document_id should be ignored.
        let original = view.sections[0].content.clone();
        view.handle_document_section_append("wrong-doc", 0, "Ignored.".to_string(), false, None);
        assert_eq!(view.sections[0].content, original);
    }

    #[test]
    fn patch_section_replaces_text() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.patch_section(
            1,
            "Method details here.",
            "Improved method details.",
            false,
            None,
        );

        assert_eq!(
            view.sections[1].content,
            "Improved method details.\nMore method details."
        );
        assert!(view.sections[1].recently_updated);
        assert!(!view.pending_sections.contains_key(&1));
    }

    #[test]
    fn patch_section_no_match_still_clears_pending() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        let original = view.sections[1].content.clone();
        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.patch_section(1, "nonexistent text", "replacement", false, None);

        // Content unchanged since old_text wasn't found.
        assert_eq!(view.sections[1].content, original);
        // But pending should still be cleared.
        assert!(!view.pending_sections.contains_key(&1));
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
            false,
            None,
        );
        assert_eq!(
            view.sections[2].content,
            "Improved result findings with more data.\nMore result findings."
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
        view.pending_sections
            .insert(1, ("test question".into(), Instant::now()));
        view.pending_sections
            .insert(3, ("test question".into(), Instant::now()));

        // Update section 1 — only section 1 should clear.
        view.update_section(1, "Updated.".to_string());
        assert!(!view.pending_sections.contains_key(&1));
        assert!(view.pending_sections.contains_key(&3));

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
        view.pending_sections
            .insert(0, ("test question".into(), Instant::now()));
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        // Section 0 should still be pending.
        assert!(view.pending_sections.contains_key(&0));

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
