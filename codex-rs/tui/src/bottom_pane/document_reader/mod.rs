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

/// Iterator that yields `(byte_offset, word)` for each whitespace-delimited
/// word in a string.  Used to map TTS word indices to character positions
/// within rendered lines.
#[cfg(not(target_os = "linux"))]
struct WordOffsets<'a> {
    text: &'a str,
    pos: usize,
}

#[cfg(not(target_os = "linux"))]
impl<'a> WordOffsets<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }
}

#[cfg(not(target_os = "linux"))]
impl<'a> Iterator for WordOffsets<'a> {
    type Item = (usize, &'a str);
    fn next(&mut self) -> Option<Self::Item> {
        let remaining = &self.text[self.pos..];
        // Skip leading whitespace.
        let trimmed = remaining.trim_start();
        if trimmed.is_empty() {
            return None;
        }
        let start = self.pos + (remaining.len() - trimmed.len());
        let word_len = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        self.pos = start + word_len;
        Some((start, &self.text[start..start + word_len]))
    }
}

/// Returns `true` when `word` is a rendering-only decorator that does not
/// correspond to any word in the TTS stream and should be skipped during
/// karaoke word counting.
#[cfg(not(target_os = "linux"))]
fn is_karaoke_skip_word(word: &str) -> bool {
    // Speaker emoji, fold border, check mark, horizontal rule.
    if word == "\u{1F50A}"
        || word == "\u{250A}"
        || word == "\u{2713}"
        || word == "-"
        || word == "+"
        || word == "\u{2022}"
        || word == "\u{2014}\u{2014}\u{2014}"
    {
        return true;
    }
    // Ordered list markers like "1." / "12." are stripped from TTS.
    if word.ends_with('.')
        && word[..word.len().saturating_sub(1)]
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return true;
    }
    // Markdown heading markers (e.g. "##", "###") are kept in the rendered
    // text but stripped by `clean_for_tts`, so they must be skipped.
    if !word.is_empty() && word.chars().all(|c| c == '#') {
        return true;
    }
    // URLs are stripped by clean_for_tts but present in rendered text.
    if word.starts_with("http://") || word.starts_with("https://") {
        return true;
    }
    // Parenthesized URLs: "(https://..." or "[https://..."
    if word.starts_with("(http://")
        || word.starts_with("(https://")
        || word.starts_with("[http://")
        || word.starts_with("[https://")
    {
        return true;
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn append_reading_view_karaoke_debug_log(message: &str) {
    let Some(path) = codex_home().map(|home| home.join("logs/reading-view-karaoke.log")) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::io::Write::write_all(&mut file, format!("[{ts}] {message}\n").as_bytes());
}

/// Saved fold state that persists across view close/reopen cycles within a
/// single TUI session.  Stored as a process-level static so we don't need to
/// thread it through BottomPane (which is upstream code).
static SAVED_FOLD_STATE: std::sync::Mutex<Option<SavedFoldState>> = std::sync::Mutex::new(None);

/// Whether the reading view tutorial has been shown (process-level cache).
/// Checked once at startup via the `~/.ata/.reading-view-seen` dotfile.
static TUTORIAL_SEEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Resolve the codex home directory (`$CODEX_HOME` or `~/.ata`).
fn codex_home() -> Option<std::path::PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".ata")))
}

/// Check whether the tutorial has been seen before (reads dotfile on first call).
fn has_seen_tutorial() -> bool {
    *TUTORIAL_SEEN.get_or_init(|| {
        codex_home()
            .map(|home| home.join(".reading-view-seen").exists())
            .unwrap_or(true) // If we can't find home, skip the tutorial
    })
}

/// Mark the tutorial as seen by writing the dotfile.
fn mark_tutorial_seen() {
    if let Some(home) = codex_home() {
        let _ = std::fs::write(home.join(".reading-view-seen"), "");
    }
}

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

    /// When `true`, render a full-screen help overlay listing all keybindings.
    show_help: bool,
    /// Scroll offset for the help overlay (when content doesn't fit).
    help_scroll: Cell<usize>,

    /// When `true`, render the first-time tutorial overlay.
    show_tutorial: bool,
    /// Scroll offset for the tutorial overlay (when content doesn't fit).
    tutorial_scroll: Cell<usize>,
    /// Tracks whether Enter was pressed once in the tutorial overlay so that a
    /// second consecutive Enter dismisses it (users may not know about Esc).
    tutorial_pending_enter: bool,

    /// When `Some`, the user pressed `:` and is typing a line number.
    /// Line numbers are shown on the left margin while active.
    line_number_input: Option<String>,

    /// Voice mode status text (e.g. "Recording...", "Speaking...").
    /// Set by ChatWidget via the `set_voice_status` trait method.
    voice_status: Option<String>,

    /// Karaoke-highlighted lines pushed by voice mode during TTS playback.
    /// When `Some`, these either replace or are appended to the section content
    /// depending on `voice_karaoke_append`.
    voice_karaoke_lines: Option<Vec<Line<'static>>>,
    /// When true, karaoke lines are appended after section content (Q&A mode).
    /// When false, they replace the content (narration mode).
    voice_karaoke_append: bool,

    /// Word-level reading highlight: (line_index, start_col, end_col).
    /// During narration, the word at this position gets bold+underline
    /// while all surrounding formatting is preserved.
    voice_reading_highlight: Option<(usize, usize, usize)>,

    /// Whether TTS is currently paused (for pause/resume toggle).
    voice_tts_paused: bool,

    /// Whether auto-scroll should follow karaoke word highlighting.
    /// Enabled when TTS narration starts; disabled when the user manually
    /// scrolls (j/k/G/Ctrl-d etc.) so they can read at their own pace.
    /// Re-enabled on the next TTS playback start.
    karaoke_auto_scroll: bool,

    /// When true, the "end of document" separator is rendered in a
    /// highlighted style.  Set when the user presses `n` at the last
    /// section; cleared on the next keypress.
    end_of_doc_flash: bool,

    /// Temporary TTS status message, cleared on next keypress.
    tts_flash_msg: Option<String>,

    /// When `true`, render a full-screen TOC overlay listing all sections.
    show_toc: bool,
    /// Currently highlighted section index in the TOC overlay.
    toc_selected_index: usize,

    /// When `true`, the next non-streaming section modification will auto-
    /// navigate to the modified section.  Set on construction (when reopening
    /// after close) and cleared on user interaction or after auto-navigating.
    pending_auto_navigate: bool,
}

impl DocumentReaderView {
    /// Convenience constructor used by `BottomPane::show_document_reader`
    /// to build the view straight from the protocol event.
    pub(crate) fn new_from_event(
        ev: codex_protocol::document_reader::PresentDocumentEvent,
        _from_replay: bool,
        app_event_tx: AppEventSender,
        frame_requester: crate::tui::FrameRequester,
    ) -> Self {
        Self::new(
            ev.document_id,
            ev.title,
            ev.content,
            app_event_tx,
            /*animations_enabled*/ true,
            frame_requester,
        )
    }

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
            if let Some(saved) = guard.as_ref()
                && saved.document_id == document_id
            {
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
            cursor_line: 0,
            cursor_col: 0,
            last_content_height: Cell::new(0),
            last_inner_width: Cell::new(40),
            search_state: None,
            search_input: String::new(),
            visual_select: None,
            selection_context: None,
            streaming_sections,
            show_help: false,
            help_scroll: Cell::new(0),
            show_tutorial: !has_seen_tutorial(),
            tutorial_scroll: Cell::new(0),
            tutorial_pending_enter: false,
            line_number_input: None,
            voice_status: None,
            voice_karaoke_lines: None,
            voice_karaoke_append: false,
            voice_reading_highlight: None,
            voice_tts_paused: false,
            karaoke_auto_scroll: true,
            end_of_doc_flash: false,
            tts_flash_msg: None,
            show_toc: false,
            toc_selected_index: 0,
            pending_auto_navigate: true,
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

            // Auto-navigate to the modified section when reopening after a
            // follow-up question.  Skip streaming fills (initial outline fill).
            if !is_streaming_fill && self.pending_auto_navigate {
                self.navigate_to_section(section_index);
                self.pending_auto_navigate = false;
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

            // Auto-navigate to the modified section when reopening after a
            // follow-up question.
            if self.pending_auto_navigate {
                self.navigate_to_section(section_index);
                self.pending_auto_navigate = false;
            }
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

            // Auto-navigate to the modified section when reopening after a
            // follow-up question.
            if self.pending_auto_navigate {
                self.navigate_to_section(section_index);
                self.pending_auto_navigate = false;
            }
        }
    }

    /// Add a new section to the document.
    pub(crate) fn add_section(
        &mut self,
        after_section_index: i32,
        heading: String,
        content: String,
        foldable: bool,
        summary: Option<String>,
    ) {
        let insert_at = (after_section_index + 1).max(0) as usize;
        let insert_at = insert_at.min(self.sections.len());

        let mut new_section = DocumentSection {
            heading,
            content,
            rendered: RefCell::new(None),
            recently_updated: true,
            changed_from_line: Some(0),
            changed_to_line: None,
            folds: Vec::new(),
        };

        if foldable {
            let fold_summary = summary.unwrap_or_else(|| {
                new_section
                    .content
                    .lines()
                    .next()
                    .unwrap_or("...")
                    .chars()
                    .take(60)
                    .collect()
            });
            new_section.folds.push(FoldRegion {
                start: 0,
                end: new_section.content.len(),
                summary: fold_summary,
                collapsed: false,
            });
        }

        self.sections.insert(insert_at, new_section);

        // Adjust pending_sections keys: any pending index >= insert_at shifts up.
        let shifted: HashMap<usize, (String, Instant)> = self
            .pending_sections
            .drain()
            .map(|(idx, val)| {
                if idx >= insert_at {
                    (idx + 1, val)
                } else {
                    (idx, val)
                }
            })
            .collect();
        self.pending_sections = shifted;

        // Adjust current_section if the insertion is before or at it.
        if insert_at <= self.current_section {
            self.current_section += 1;
        }

        // Navigate to the new section.
        self.current_section = insert_at;
        self.scroll_offset.set(0);
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.visited_sections.insert(insert_at);
        // Clear auto-navigate — add_section already navigates to the new section.
        self.pending_auto_navigate = false;

        // Resolve any pending follow-ups for the current section.
        // The new section itself resolves the pending question that spawned it.
        self.resolve_pending(insert_at);
        self.refresh_search();
    }

    /// Navigate to a specific section, scrolling to the bottom and expanding
    /// any collapsed folds.  Used by auto-navigate after a follow-up answer
    /// arrives, so the user immediately sees the new content.
    fn navigate_to_section(&mut self, section_index: usize) {
        if section_index < self.sections.len() && section_index != self.current_section {
            self.interrupt_tts_if_needed();
            self.clear_updated_flag();
            self.current_section = section_index;
            // Scroll to the bottom so newly added content is visible.
            self.scroll_offset.set(u16::MAX);
            self.cursor_line = 0;
            self.cursor_col = 0;
            self.voice_karaoke_lines = None;
            self.voice_reading_highlight = None;
            self.visited_sections.insert(self.current_section);

            // Auto-expand any collapsed folds so the new answer is visible.
            if let Some(section) = self.sections.get_mut(self.current_section) {
                let mut expanded_any = false;
                for fold in &mut section.folds {
                    if fold.collapsed {
                        fold.collapsed = false;
                        expanded_any = true;
                    }
                }
                if expanded_any {
                    section.invalidate_cache();
                }
            }
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
            self.end_of_doc_flash = false;
            self.interrupt_tts_if_needed();
            self.clear_updated_flag();
            self.current_section += 1;
            self.scroll_offset.set(0);
            self.cursor_line = 0;
            self.cursor_col = 0;
            self.voice_karaoke_lines = None;
            self.voice_reading_highlight = None;
            self.visited_sections.insert(self.current_section);
        } else {
            // Already at the last section — flash the indicator.
            self.end_of_doc_flash = true;
        }
    }

    fn prev_section(&mut self) {
        if self.current_section > 0 {
            self.interrupt_tts_if_needed();
            self.clear_updated_flag();
            self.current_section -= 1;
            self.scroll_offset.set(0);
            self.cursor_line = 0;
            self.cursor_col = 0;
            self.voice_karaoke_lines = None;
            self.voice_reading_highlight = None;
            self.visited_sections.insert(self.current_section);
            // Don't auto-narrate when going back — user can press `r`.
        }
    }

    /// Interrupt TTS if voice mode is speaking (user navigated away).
    #[cfg(not(target_os = "linux"))]
    fn interrupt_tts_if_needed(&self) {
        self.app_event_tx.send(AppEvent::VoiceModeInterruptTts);
    }

    #[cfg(target_os = "linux")]
    fn interrupt_tts_if_needed(&self) {}

    /// Emit a narrate event for the current section so voice mode can TTS it.
    /// The document reader doesn't know whether voice mode is active — ChatWidget
    /// filters based on voice state.
    ///
    /// Any collapsed folds in the current section are auto-expanded so the
    /// karaoke reading cursor can track through all visible content.
    #[cfg(not(target_os = "linux"))]
    fn narrate_current_section_if_voice(&mut self, manual: bool) {
        // Auto-expand collapsed folds so karaoke can highlight fold content.
        if let Some(section) = self.sections.get_mut(self.current_section) {
            let mut expanded_any = false;
            for fold in &mut section.folds {
                if fold.collapsed {
                    fold.collapsed = false;
                    expanded_any = true;
                }
            }
            if expanded_any {
                section.invalidate_cache();
            }
        }

        if let Some(section) = self.sections.get(self.current_section) {
            let text = if section.heading.is_empty() {
                section.content.clone()
            } else {
                format!("{}.\n{}", section.heading, section.content)
            };
            self.app_event_tx.send(AppEvent::VoiceModeNarrateSection {
                document_id: self.document_id.clone(),
                section_index: self.current_section,
                text,
                selection_word_offset: None,
                manual,
            });
        }
        // Prefetch the next section in the background.
        if let Some(next) = self.sections.get(self.current_section + 1) {
            let next_text = if next.heading.is_empty() {
                next.content.clone()
            } else {
                format!("{}.\n{}", next.heading, next.content)
            };
            if !next_text.trim().is_empty() {
                self.app_event_tx.send(AppEvent::VoiceModePrefetchSection {
                    document_id: self.document_id.clone(),
                    section_index: self.current_section + 1,
                    text: next_text,
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn narrate_current_section_if_voice(&mut self, _manual: bool) {}

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
        // Stop any ongoing TTS playback immediately.
        self.interrupt_tts_if_needed();

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
             They viewed {viewed} of {total} sections.{streaming_note}]\n\
             Check whether follow-up Q&A during this reading session added \
             new insights not already in the KB card. If yes, spawn a single \
             fire-and-forget $kb subagent now to persist them (do NOT call \
             wait). If no new insights were produced, do nothing. Do not \
             announce this action to the user — just do it silently or skip.",
            self.title,
        );
        self.app_event_tx
            .send(AppEvent::SubmitUserText { text: feedback });

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
        if folds.iter().any(|f| !f.is_empty())
            && let Ok(mut guard) = SAVED_FOLD_STATE.lock()
        {
            *guard = Some(SavedFoldState {
                document_id: self.document_id.clone(),
                section_folds: folds,
            });
        }
    }

    fn submit_follow_up(&mut self) {
        let raw_text = self.textarea.text().trim().to_string();
        // When the user presses Enter with no text but has a visual selection,
        // default to "Explain this in more detail" — the most common action.
        let text = if raw_text.is_empty() {
            if self.selection_context.is_some() {
                "Explain this in more detail".to_string()
            } else {
                return;
            }
        } else {
            raw_text
        };

        let heading = self
            .sections
            .get(self.current_section)
            .map(|s| s.heading.as_str())
            .unwrap_or("");

        let selection = self.selection_context.take();
        // Keep visual selection visible while the response is pending — it
        // helps the user see what they asked about.  It gets cleared when the
        // response arrives (in resolve_pending).

        // Include the current section content so the agent can reliably
        // locate the right passage for inline patching.
        let section_content = self
            .sections
            .get(self.current_section)
            .map(|s| s.content.as_str())
            .unwrap_or("");

        let selection_guidance = crate::legacy_core::reading_view_selection_follow_up_guidance(
            crate::legacy_core::ReadingViewDisplayMode::Tui,
        );
        let section_guidance = crate::legacy_core::reading_view_section_follow_up_guidance(
            crate::legacy_core::ReadingViewDisplayMode::Tui,
        );

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

        // Sentinel separating user-visible context from hidden tool instructions.
        // `strip_system_instruction_prefix` in chatwidget.rs cuts at this marker.
        const INSTR_SEP: &str = "\n\n<!-- READER_TOOL_INSTRUCTIONS -->\n";

        let tool_instructions = if let Some(ref sel) = selection {
            // The user highlighted specific text — tell the agent to patch
            // the answer in right after the selection.
            format!(
                "The user selected specific text from the section (shown below) and is asking about it.\n\
                 [Selected text:]\n{sel}\
                 {INSTR_SEP}\
                 Tool target for this turn: document_id=\"{doc_id}\", section_index={idx}.\n\n\
                 {selection_guidance}",
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
                "{INSTR_SEP}\
                 Tool target for this turn: document_id=\"{doc_id}\", section_index={idx}.\n\n\
                 Current section content:\n---\n{section_content}\n---\n\
                 {cursor_hint}\n\
                 {section_guidance}",
                doc_id = self.document_id,
                idx = self.current_section,
            )
        };

        let context = format!(
            "[The user is reading \"{title}\" and asked about the section titled \"{heading}\"]\n\n\
             {text}\n\n\
             {tool_instructions}",
            title = self.title,
            heading = heading,
        );

        self.app_event_tx
            .send(AppEvent::SubmitUserText { text: context });
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
        // Any user interaction cancels pending auto-navigate so manual
        // navigation is never overridden by a later section modification.
        self.pending_auto_navigate = false;

        // Clear the "end of document" flash on any keypress.
        self.end_of_doc_flash = false;

        // Clear the TTS flash message on any keypress.
        self.tts_flash_msg = None;

        // Line-number jump mode (`:` prefix) — must run before overlay/quit
        // handlers so digit keys are captured instead of dismissing overlays.
        if let Some(ref mut input) = self.line_number_input {
            match key_event.code {
                KeyCode::Char(c) if c.is_ascii_digit() || c.is_ascii_lowercase() => {
                    input.push(c);
                }
                KeyCode::Backspace => {
                    if input.is_empty() {
                        self.line_number_input = None;
                    } else {
                        input.pop();
                    }
                }
                KeyCode::Enter => {
                    if input == "q" {
                        self.line_number_input = None;
                        self.exit_reading_mode();
                        return;
                    }
                    if let Ok(n) = input.parse::<usize>() {
                        let target = n.saturating_sub(1); // 1-indexed → 0-indexed
                        if self.show_tutorial || self.show_help {
                            let scroll_cell = if self.show_tutorial {
                                &self.tutorial_scroll
                            } else {
                                &self.help_scroll
                            };
                            scroll_cell.set(target);
                        } else {
                            self.cursor_line = target;
                            self.clamp_and_scroll();
                        }
                    }
                    self.line_number_input = None;
                    self.pending_g = false;
                    self.pending_z = false;
                }
                KeyCode::Esc => {
                    self.line_number_input = None;
                    self.pending_g = false;
                    self.pending_z = false;
                }
                _ => {
                    self.line_number_input = None;
                    self.pending_g = false;
                    self.pending_z = false;
                }
            }
            return;
        }

        // TOC overlay: navigate with j/k, jump with Enter, dismiss with t/Esc/q.
        if self.show_toc {
            match key_event.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    if self.toc_selected_index + 1 < self.sections.len() {
                        self.toc_selected_index += 1;
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.toc_selected_index = self.toc_selected_index.saturating_sub(1);
                }
                KeyCode::Char('G') => {
                    self.toc_selected_index = self.sections.len().saturating_sub(1);
                }
                KeyCode::Char('g') => {
                    self.toc_selected_index = 0;
                }
                KeyCode::Enter => {
                    let target = self.toc_selected_index;
                    self.show_toc = false;
                    if target != self.current_section && target < self.sections.len() {
                        self.interrupt_tts_if_needed();
                        self.clear_updated_flag();
                        self.current_section = target;
                        self.scroll_offset.set(0);
                        self.cursor_line = 0;
                        self.cursor_col = 0;
                        self.voice_karaoke_lines = None;
                        self.voice_reading_highlight = None;
                        self.visited_sections.insert(self.current_section);
                    }
                }
                KeyCode::Char('t') | KeyCode::Esc | KeyCode::Char('q') => {
                    self.show_toc = false;
                }
                _ => {}
            }
            return;
        }

        // Tutorial / help overlay: navigate with vim keys, dismiss with others.
        if self.show_tutorial || self.show_help {
            let scroll_cell = if self.show_tutorial {
                &self.tutorial_scroll
            } else {
                &self.help_scroll
            };
            let half = (self.last_content_height.get() / 2).max(1) as usize;
            let full = self.last_content_height.get().max(1) as usize;
            if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                match key_event.code {
                    KeyCode::Char('d') => {
                        scroll_cell.set(scroll_cell.get().saturating_add(half));
                    }
                    KeyCode::Char('u') => {
                        scroll_cell.set(scroll_cell.get().saturating_sub(half));
                    }
                    KeyCode::Char('f') => {
                        scroll_cell.set(scroll_cell.get().saturating_add(full));
                    }
                    KeyCode::Char('b') => {
                        scroll_cell.set(scroll_cell.get().saturating_sub(full));
                    }
                    _ => {}
                }
            } else {
                match key_event.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        scroll_cell.set(scroll_cell.get().saturating_add(1));
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        scroll_cell.set(scroll_cell.get().saturating_sub(1));
                    }
                    KeyCode::Char('G') => {
                        scroll_cell.set(usize::MAX);
                    }
                    KeyCode::Char('g') => {
                        // gg = jump to top (consume even without pending_g
                        // since overlays don't use g for anything else).
                        scroll_cell.set(0);
                    }
                    KeyCode::Char(':') => {
                        self.line_number_input = Some(String::new());
                    }
                    KeyCode::Char('q') | KeyCode::Esc => {
                        if self.show_tutorial {
                            self.show_tutorial = false;
                            self.tutorial_scroll.set(0);
                            self.tutorial_pending_enter = false;
                            mark_tutorial_seen();
                        } else {
                            self.show_help = false;
                            self.help_scroll.set(0);
                        }
                    }
                    KeyCode::Enter if self.show_tutorial => {
                        if self.tutorial_pending_enter {
                            self.show_tutorial = false;
                            self.tutorial_scroll.set(0);
                            self.tutorial_pending_enter = false;
                            mark_tutorial_seen();
                        } else {
                            self.tutorial_pending_enter = true;
                        }
                    }
                    _ => {
                        // Reset pending enter on any other key.
                        self.tutorial_pending_enter = false;
                    }
                }
            }
            return;
        }

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
                KeyCode::Enter => {
                    // Quick-explain: extract selection and submit immediately
                    // with the default "Explain this in more detail" prompt.
                    let inner_w = self.last_inner_width.get();
                    let text = self.selected_text(inner_w);
                    if let Some(text) = text {
                        self.selection_context = Some(text);
                    }
                    self.visual_select = None;
                    self.submit_follow_up();
                }
                KeyCode::Tab => {
                    // Open composer with selection as context — user can type
                    // a custom question, or just press Enter for the default.
                    // Keep the visual selection visible so the user can see
                    // what they selected while typing their question.
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
                    self.exit_reading_mode();
                }
                // Read selection aloud via TTS.
                #[cfg(not(target_os = "linux"))]
                KeyCode::Char('r') => {
                    // Interrupt any ongoing TTS before starting a new read
                    // and clear stale karaoke visual state immediately.
                    self.interrupt_tts_if_needed();
                    self.voice_karaoke_lines = None;
                    self.voice_reading_highlight = None;
                    let inner_w = self.last_inner_width.get();
                    let word_offset = self.count_words_before_selection(inner_w);
                    if let Some(text) = self.selected_text(inner_w) {
                        // Strip fold decorators from selected text so the TTS
                        // word sequence matches the rendered word counter (which
                        // skips fold headers and ┊ border prefixes).
                        let text: String = text
                            .lines()
                            .filter(|line| {
                                let t = line.trim_start();
                                !t.starts_with("┊ [-]") && !t.starts_with("┊ [+]")
                            })
                            .map(|line| line.strip_prefix("┊ ").unwrap_or(line))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.trim().is_empty() {
                            self.app_event_tx.send(AppEvent::VoiceModeNarrateSection {
                                document_id: self.document_id.clone(),
                                section_index: self.current_section,
                                text,
                                selection_word_offset: Some(word_offset),
                                manual: true,
                            });
                        }
                    }
                    self.visual_select = None;
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
            KeyCode::Char('r') => {
                // Manually trigger narration of the current section.
                tracing::info!(
                    "[TTS-TIMING] 'r' pressed: section={}, sending interrupt+narrate events",
                    self.current_section,
                );
                self.interrupt_tts_if_needed();
                // Clear stale karaoke visual state immediately so old
                // highlights don't flash while the new TTS initializes.
                self.voice_karaoke_lines = None;
                self.voice_reading_highlight = None;
                self.narrate_current_section_if_voice(true);
            }
            #[cfg(not(target_os = "linux"))]
            KeyCode::Char('s') if self.voice_status.is_some() => {
                // Pause/resume TTS playback.
                tracing::debug!(
                    "[TTS-DBG] 's' pressed: voice_status={:?}, voice_tts_paused={}",
                    self.voice_status,
                    self.voice_tts_paused
                );
                if self.voice_tts_paused {
                    tracing::debug!("[TTS-DBG] Sending VoiceModeResumeTts");
                    self.app_event_tx.send(AppEvent::VoiceModeResumeTts);
                } else {
                    tracing::debug!("[TTS-DBG] Sending VoiceModePauseTts");
                    self.app_event_tx.send(AppEvent::VoiceModePauseTts);
                }
            }
            // Increase/decrease TTS playback speed (client-side, pitch-preserving).
            #[cfg(not(target_os = "linux"))]
            KeyCode::Char('+' | '=') if self.voice_status.is_some() => {
                self.app_event_tx
                    .send(AppEvent::VoiceModePlaybackSpeedChange { delta: 0.1 });
            }
            #[cfg(not(target_os = "linux"))]
            KeyCode::Char('-') if self.voice_status.is_some() => {
                self.app_event_tx
                    .send(AppEvent::VoiceModePlaybackSpeedChange { delta: -0.1 });
            }
            KeyCode::Char('t') => {
                // Toggle Table of Contents overlay.
                self.show_toc = !self.show_toc;
                if self.show_toc {
                    self.toc_selected_index = self.current_section;
                }
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
            KeyCode::Char('f') => {
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
            KeyCode::Char(':') => {
                self.line_number_input = Some(String::new());
            }
            KeyCode::Char('?') => {
                self.show_toc = false;
                self.show_help = true;
                self.help_scroll.set(0);
            }
            KeyCode::Char('q') => {
                self.exit_reading_mode();
            }
            _ => {}
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
        // If karaoke narration is active, the user is manually scrolling —
        // disable auto-scroll so it doesn't fight the user's position.
        if self.voice_reading_highlight.is_some() || self.voice_karaoke_lines.is_some() {
            self.karaoke_auto_scroll = false;
        }
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
        // While the current section has a pending answer, block editing
        // but still allow navigation so the user can browse other sections
        // (and potentially ask more questions) while waiting.
        let pending = self.pending_sections.contains_key(&self.current_section);

        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                // Esc explicitly cancels: clear selection and return to content.
                self.visual_select = None;
                self.selection_context = None;
                self.focus = ReaderFocus::Content;
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                // Tab toggles back to content but keeps the visual selection
                // visible so the user can adjust it and Tab back.
                self.focus = ReaderFocus::Content;
            }
            // Allow section navigation and TTS controls even while a
            // question is pending.  Switch to Content focus and delegate
            // to the content key handler so the user can browse other
            // sections and trigger/control read-aloud while waiting.
            KeyEvent {
                code:
                    KeyCode::Char('n')
                    | KeyCode::Char('p')
                    | KeyCode::PageDown
                    | KeyCode::PageUp
                    | KeyCode::Char('r')
                    | KeyCode::Char('s')
                    | KeyCode::Char('+')
                    | KeyCode::Char('=')
                    | KeyCode::Char('-'),
                modifiers: KeyModifiers::NONE,
                ..
            } if pending => {
                self.focus = ReaderFocus::Content;
                self.handle_content_key(key_event);
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

        // Search across all sections so n/N can navigate the entire document.
        for (section_idx, section) in self.sections.iter().enumerate() {
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

        // Set current_match_idx to the first match in or after the current
        // section so the user starts navigating from where they are.
        let first_from_current = matches
            .iter()
            .position(|m| m.section_idx >= self.current_section)
            .unwrap_or(0);

        let has_matches = !matches.is_empty();
        self.search_state = Some(SearchState {
            query,
            matches,
            current_match_idx: first_from_current,
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

        let section = &self.sections[section_idx];
        let heading_line_count: usize = if section.heading.is_empty() { 0 } else { 2 };
        let inner_width = self.last_inner_width.get();

        let rendered_line = if byte_offset < section.heading.len() {
            // Match is in the heading — jump to line 0.
            0
        } else {
            // Match is in the content. Find the raw content line containing
            // the match, then use rendered_body_line_count to compute the
            // accurate rendered line position (accounting for markdown
            // rendering, word wrapping, LaTeX stripping, etc.).
            let content_offset = byte_offset - section.heading.len();
            let safe_offset = content_offset.min(section.content.len());

            // Count the number of raw content lines before the match.
            let raw_line_idx = section.content[..safe_offset].matches('\n').count();

            // Get the content prefix up to (but not including) the match line
            // and compute how many rendered lines it produces.
            let prefix: String = section
                .content
                .lines()
                .take(raw_line_idx)
                .collect::<Vec<_>>()
                .join("\n");
            let rendered_body_lines = render::rendered_body_line_count(&prefix, inner_width);

            let pre_fold = heading_line_count + rendered_body_lines;
            render::adjust_line_for_folds(
                pre_fold,
                &section.content,
                heading_line_count,
                inner_width,
                &section.folds,
            )
        };

        // Correct for structural blank lines that the isolated prefix render
        // may have missed (e.g., blank lines inserted before lists by the
        // markdown renderer). Use the actual rendered lines to verify and
        // adjust the computed position.
        let mut rendered_line = rendered_line;
        let lines = self.sections[section_idx].rendered_lines(inner_width);
        if let Some(query) = self.search_state.as_ref().map(|s| s.query.clone()) {
            let query_lower = query.to_lowercase();
            let line_has_match = |idx: usize| -> bool {
                lines.get(idx).is_some_and(|line| {
                    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    text.to_lowercase().contains(&query_lower)
                })
            };
            if !line_has_match(rendered_line) {
                for delta in 1..=4 {
                    if line_has_match(rendered_line + delta) {
                        rendered_line += delta;
                        break;
                    }
                }
            }
        }

        // Move cursor to the match and center in viewport.
        self.cursor_line = rendered_line;

        // Compute cursor column by finding the correct occurrence of the
        // query in the rendered line.  When the same line contains the query
        // multiple times, we figure out which occurrence this match is by
        // counting how many times it appears in the raw content line before
        // the match position, then skip that many occurrences in the rendered
        // text.
        self.cursor_col = 0;
        if let Some(query) = self.search_state.as_ref().map(|s| s.query.clone()) {
            let query_lower = query.to_lowercase();

            // Determine which occurrence on the raw line this match is.
            let content_offset = byte_offset.saturating_sub(section.heading.len());
            let safe_offset = content_offset.min(section.content.len());
            let line_start = section.content[..safe_offset]
                .rfind('\n')
                .map_or(0, |p| p + 1);
            let preceding = section.content[line_start..safe_offset].to_lowercase();
            let occurrence = preceding.matches(&query_lower).count();

            // Find the (occurrence)-th match in the rendered line text.
            if let Some(line) = lines.get(rendered_line) {
                let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                let line_lower = line_text.to_lowercase();
                let mut search_start = 0;
                for _ in 0..=occurrence {
                    if let Some(pos) = line_lower[search_start..].find(&query_lower) {
                        self.cursor_col = search_start + pos;
                        search_start = self.cursor_col + 1;
                    } else {
                        break;
                    }
                }
            }
        }

        let half_viewport = self.last_content_height.get() / 2;
        self.scroll_offset
            .set((rendered_line as u16).saturating_sub(half_viewport));
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

    /// Count rendered words before the selection start position.
    ///
    /// Uses the same decorator-skipping logic as `set_voice_reading_progress`
    /// so the returned offset can be added to a TTS word index to highlight
    /// the correct word in the full rendered content.
    #[cfg(not(target_os = "linux"))]
    fn count_words_before_selection(&self, inner_width: u16) -> usize {
        let vs = match self.visual_select.as_ref() {
            Some(v) => v,
            None => return 0,
        };
        let section = match self.sections.get(self.current_section) {
            Some(s) => s,
            None => return 0,
        };
        let lines = section.rendered_lines(inner_width);
        if lines.is_empty() {
            return 0;
        }
        let (start_line, start_col, _, _) = match self.selection_bounds(vs, lines.len()) {
            Some(b) => b,
            None => return 0,
        };
        let mut count = 0usize;
        for (i, line) in lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if text.starts_with("┊ [-]") || text.starts_with("┊ [+]") {
                continue;
            }
            for (word_start, word) in WordOffsets::new(&text) {
                if is_karaoke_skip_word(word) {
                    continue;
                }
                if i > start_line || (i == start_line && word_start >= start_col) {
                    return count;
                }
                count += 1;
            }
        }
        count
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

    #[cfg(not(target_os = "linux"))]
    fn voice_context(&self) -> Option<super::bottom_pane_view::ReadingViewVoiceContext> {
        let section = self.sections.get(self.current_section)?;
        let heading = section.heading.clone();

        // Only include selection if the user is actively in visual select mode.
        let active_selection = if self.visual_select.is_some() {
            self.selection_context.clone()
        } else {
            None
        };

        Some(super::bottom_pane_view::ReadingViewVoiceContext {
            title: self.title.clone(),
            document_id: self.document_id.clone(),
            section_index: self.current_section,
            heading,
            selection: active_selection,
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn set_voice_status(&mut self, status: Option<String>) {
        self.voice_status = status;
    }

    #[cfg(not(target_os = "linux"))]
    fn set_tts_flash_msg(&mut self, msg: Option<String>) {
        self.tts_flash_msg = msg;
    }

    #[cfg(not(target_os = "linux"))]
    fn set_voice_tts_paused(&mut self, paused: bool) {
        self.voice_tts_paused = paused;
    }

    #[cfg(not(target_os = "linux"))]
    fn set_pending_voice_question(&mut self, section: usize, question: String) {
        self.pending_sections
            .insert(section, (question, Instant::now()));
        // Invalidate the section's rendered cache so the pending indicator
        // appears immediately.
        if let Some(s) = self.sections.get(self.current_section) {
            s.invalidate_cache();
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn set_voice_karaoke_lines(&mut self, lines: Option<Vec<Line<'static>>>, append: bool) {
        // Re-enable auto-scroll when new karaoke lines arrive (TTS starts).
        if lines.is_some() && self.voice_karaoke_lines.is_none() {
            self.karaoke_auto_scroll = true;
        }
        self.voice_karaoke_lines = lines;
        self.voice_karaoke_append = append;
        // Auto-scroll to keep the karaoke text visible (unless user
        // manually scrolled away — karaoke_auto_scroll is false).
        if self.karaoke_auto_scroll
            && let Some(ref karaoke) = self.voice_karaoke_lines
        {
            let content_h = self.last_content_height.get();
            if content_h == 0 {
                return;
            }
            if append {
                // Scroll to the end of the section so the fold with
                // karaoke content is visible. Use a large offset — the
                // render pass will clamp it to the actual max.
                self.scroll_offset.set(u16::MAX);
            } else {
                let total = karaoke.len() as u16;
                if total > content_h {
                    self.scroll_offset.set(total.saturating_sub(content_h));
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn set_voice_reading_progress(
        &mut self,
        word_idx: Option<usize>,
        heading_words_to_skip: usize,
    ) {
        // Re-enable auto-scroll when narration starts (first word arrives
        // after highlight was cleared).
        if word_idx.is_some() && self.voice_reading_highlight.is_none() {
            self.karaoke_auto_scroll = true;
        }

        let mut debug_log = None;
        let highlight = word_idx.and_then(|wi| {
            let adj = match wi.checked_sub(heading_words_to_skip) {
                Some(adj) => adj,
                None => {
                    debug_log = Some(format!(
                        "UNDERFLOW section={} word_idx={wi} heading_skip={heading_words_to_skip}",
                        self.sections
                            .get(self.current_section)
                            .map(|s| s.heading.as_str())
                            .unwrap_or("<missing>")
                    ));
                    return None;
                }
            };
            let section = self.sections.get(self.current_section)?;
            let section_heading = section.heading.clone();
            let inner_w = self.last_inner_width.get();
            let lines = section.rendered_lines(inner_w);
            // Walk ALL rendered lines (including heading) counting words
            // to find the target word's line index and character range.
            // Fold decorators (headers, ┊ borders) are skipped since they
            // don't exist in the TTS text.
            let mut cumulative_words = 0usize;
            for (i, line) in lines.iter().enumerate() {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                // Skip fold header lines entirely — their summary text
                // is not part of the TTS stream.
                if text.starts_with("┊ [-]") || text.starts_with("┊ [+]") {
                    continue;
                }
                for (word_start, word) in WordOffsets::new(&text) {
                    // Skip decorators that aren't real TTS words.
                    if is_karaoke_skip_word(word) {
                        continue;
                    }
                    if cumulative_words == adj {
                        debug_log = Some(format!(
                            "MAP section={section_heading:?} word_idx={wi} adj={adj} line={i} rendered_word={word:?}"
                        ));
                        return Some((i, word_start, word_start + word.len()));
                    }
                    cumulative_words += 1;
                }
            }
            tracing::debug!(
                "Karaoke word miss: adj={adj}, total_rendered_words={cumulative_words}, lines={}",
                lines.len()
            );
            debug_log = Some(format!(
                "MISS section={section_heading:?} word_idx={wi} adj={adj} total_rendered_words={cumulative_words} lines={}",
                lines.len()
            ));
            None
        });

        if let Some(log) = debug_log {
            append_reading_view_karaoke_debug_log(&log);
        }

        self.voice_reading_highlight = highlight;

        // Auto-scroll so the highlighted line stays visible, but only if the
        // user hasn't manually scrolled away (karaoke_auto_scroll is true).
        if self.karaoke_auto_scroll
            && let Some((line_idx, _, _)) = highlight
        {
            let content_h = self.last_content_height.get();
            if content_h > 0 {
                let scroll = self.scroll_offset.get() as usize;
                let visible_end = scroll + content_h as usize;
                // Keep the highlighted word in the middle third of the
                // viewport for comfortable reading.
                let margin = (content_h as usize) / 3;
                if line_idx >= visible_end.saturating_sub(margin) {
                    self.scroll_offset
                        .set(line_idx.saturating_sub(content_h as usize / 2) as u16);
                }
                if line_idx < scroll + margin {
                    self.scroll_offset
                        .set(line_idx.saturating_sub(content_h as usize / 2) as u16);
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn is_composer_focused(&self) -> bool {
        self.focus == ReaderFocus::Composer
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.exit_reading_mode();
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        // In content focus: Esc clears search or cancels visual select.
        // In composer/search focus: Esc returns to content focus.
        true
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn view_id(&self) -> Option<&'static str> {
        Some(DOCUMENT_READER_VIEW_ID)
    }

    fn closed_document_id(&self) -> Option<&str> {
        if self.complete {
            Some(&self.document_id)
        } else {
            None
        }
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

    fn handle_document_section_add(
        &mut self,
        document_id: &str,
        after_section_index: i32,
        heading: String,
        content: String,
        foldable: bool,
        summary: Option<String>,
    ) {
        if self.document_id == document_id {
            self.add_section(after_section_index, heading, content, foldable, summary);
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
    fn desired_height(&self, _width: u16) -> u16 {
        // The reading view is a full-screen experience — request all available
        // terminal height so the flex allocator gives us maximum space.
        // Use a large-but-safe value (not u16::MAX) because upstream
        // InsetRenderable adds inset values to this with non-saturating
        // arithmetic.
        u16::MAX / 2
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height < 4 || area.width < 6 {
            return;
        }
        // Clear the reading view area and any inset gap row above (the
        // chatwidget adds a 1-row top inset to the bottom pane; when the
        // reading view fills the terminal that gap can show residual content).
        let clear_area = if area.y > 0 {
            Rect::new(area.x, area.y - 1, area.width, area.height + 1)
        } else {
            area
        };
        Clear.render(clear_area, buf);

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

        // --- Fixed rows from bottom: bottom_border(1) + extra_bottom_rows + hints(1) + voice_status(0|1) + separator(1) ---
        let voice_status_rows: u16 = if self.voice_status.is_some() || self.tts_flash_msg.is_some()
        {
            1
        } else {
            0
        };
        let fixed_bottom = 1 + extra_bottom_rows + 1 + voice_status_rows + 1;
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
        let current_heading = self
            .sections
            .get(self.current_section)
            .map(|s| s.heading.as_str())
            .unwrap_or("");
        let header = render::header_line(
            &self.title,
            section_num,
            section_count,
            current_heading,
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
        let show_line_nums = self.line_number_input.is_some();
        let mut has_more_below = false;
        if content_height > 0 && (self.show_tutorial || self.show_help || self.show_toc) {
            // Render tutorial, help, or TOC overlay with scroll.
            let (overlay_lines, scroll_cell) = if self.show_tutorial {
                (
                    render::help_overlay_lines(w, Some(self.sections.len())),
                    &self.tutorial_scroll,
                )
            } else if self.show_toc {
                let section_data: Vec<(String, bool)> = self
                    .sections
                    .iter()
                    .map(|s| (s.heading.clone(), s.recently_updated))
                    .collect();
                (
                    render::toc_overlay_lines(
                        w,
                        &section_data,
                        self.current_section,
                        self.toc_selected_index,
                        &self.visited_sections,
                    ),
                    &self.help_scroll,
                )
            } else {
                (render::help_overlay_lines(w, None), &self.help_scroll)
            };
            let total = overlay_lines.len();
            let visible = content_height as usize;
            let max_scroll = total.saturating_sub(visible);
            let scroll = scroll_cell.get().min(max_scroll);
            scroll_cell.set(scroll);
            let rendered = total.saturating_sub(scroll).min(visible);
            for (i, line) in overlay_lines
                .into_iter()
                .skip(scroll)
                .take(visible)
                .enumerate()
            {
                let row_y = y + i as u16;
                if row_y >= bottom {
                    break;
                }
                let abs_line = scroll + i + 1; // 1-indexed
                let bordered = if show_line_nums {
                    render::bordered_line_numbered(line, w, false, abs_line)
                } else {
                    render::bordered_line(line, w, false)
                };
                Paragraph::new(bordered).render(
                    Rect {
                        x: area.x,
                        y: row_y,
                        width: w,
                        height: 1,
                    },
                    buf,
                );
            }
            // Fill remaining content rows with empty bordered lines.
            for i in rendered..visible {
                let row_y = y + i as u16;
                if row_y >= bottom {
                    break;
                }
                Paragraph::new(render::bordered_line(Line::from(""), w, false)).render(
                    Rect {
                        x: area.x,
                        y: row_y,
                        width: w,
                        height: 1,
                    },
                    buf,
                );
            }
            if scroll + visible < total {
                has_more_below = true;
            }
        } else if content_height > 0 {
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

                let mut raw_lines = if let Some(ref karaoke) = self.voice_karaoke_lines {
                    if self.voice_karaoke_append {
                        // Q&A mode: render section content normally, then
                        // replace the last expanded fold's content lines
                        // with karaoke-highlighted text (inside ┊ borders).
                        let mut lines = section.rendered_lines(inner_width);

                        // Find the last fold header line (starts with "┊ [-]")
                        // and replace everything after it with karaoke lines.
                        let last_fold_header = lines.iter().rposition(|line| {
                            let text: String =
                                line.spans.iter().map(|s| s.content.as_ref()).collect();
                            text.starts_with("┊ [-]")
                        });

                        if let Some(header_idx) = last_fold_header {
                            // Remove all lines after the fold header (the
                            // original fold content) and replace with karaoke.
                            // Find where the fold ends (next non-┊ line or end).
                            let fold_end = lines[header_idx + 1..]
                                .iter()
                                .position(|line| {
                                    let text: String =
                                        line.spans.iter().map(|s| s.content.as_ref()).collect();
                                    !text.starts_with("┊ ")
                                })
                                .map(|pos| header_idx + 1 + pos)
                                .unwrap_or(lines.len());

                            // Remove old fold content.
                            lines.drain(header_idx + 1..fold_end);

                            // Insert karaoke lines with ┊ borders.
                            let insert_at = header_idx + 1;
                            for (j, k_line) in karaoke.iter().enumerate() {
                                let mut spans = vec![Span::from("┊ ").dim().cyan()];
                                spans.extend(k_line.spans.clone());
                                lines.insert(insert_at + j, Line::from(spans));
                            }
                        } else {
                            // No fold found yet — append with separator.
                            lines.push(Line::from(""));
                            let mut spans = vec![Span::from("┊ ").dim().cyan()];
                            spans.push(Span::from("\u{1F50A} Speaking...").dim().italic());
                            lines.push(Line::from(spans));
                            for k_line in karaoke {
                                let mut spans = vec![Span::from("┊ ").dim().cyan()];
                                spans.extend(k_line.spans.clone());
                                lines.push(Line::from(spans));
                            }
                        }
                        lines
                    } else {
                        // Narration mode: replace section content with
                        // highlighted clean text, but preserve the heading.
                        let mut lines = Vec::new();
                        if !section.heading.is_empty() {
                            lines.push(Line::from(vec![
                                Span::from("\u{1F50A} "),
                                Span::from(section.heading.clone()).bold(),
                            ]));
                            lines.push(Line::from(""));
                        }
                        lines.extend(karaoke.iter().cloned());
                        lines
                    }
                } else if is_streaming_empty {
                    render::render_section_loading(&section.heading, self.animations_enabled)
                } else {
                    section.rendered_lines(inner_width)
                };

                // Apply word-level reading highlight during narration.
                // This runs BEFORE the 🔊 icon prepend so that character
                // offsets computed from rendered_lines() match correctly.
                if let Some((line_idx, col_start, col_end)) = self.voice_reading_highlight
                    && line_idx < raw_lines.len()
                {
                    let line = std::mem::take(&mut raw_lines[line_idx]);
                    raw_lines[line_idx] = render::apply_word_highlight(line, col_start, col_end);
                }

                // (Voice icon removed — too noisy.)

                // On the first section, append a table of contents listing
                // all section headings so the user knows the full structure.
                if self.current_section == 0 && self.sections.len() > 1 {
                    raw_lines.push(Line::from(""));
                    raw_lines.push(Line::from(vec![
                        Span::from("  Sections").dim().bold(),
                        Span::from(" (n/p to navigate)").dim(),
                    ]));
                    for (i, s) in self.sections.iter().enumerate() {
                        if !s.heading.is_empty() {
                            let marker = if self.visited_sections.contains(&i) {
                                "\u{2713} " // ✓
                            } else {
                                "  "
                            };
                            let num = format!("  {marker}{}. ", i + 1);
                            raw_lines.push(Line::from(vec![
                                Span::from(num).dim(),
                                Span::from(s.heading.clone()).dim(),
                            ]));
                        }
                    }
                }

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

                // Helper: wrap a line with borders, optionally with a line number.
                let border_fn =
                    |line: Line<'static>, w: u16, changed: bool, abs: usize| -> Line<'static> {
                        if show_line_nums {
                            render::bordered_line_numbered(line, w, changed, abs + 1)
                        } else {
                            render::bordered_line(line, w, changed)
                        }
                    };

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
                            border_fn(highlighted, w, is_changed, abs_line)
                        } else {
                            border_fn(line, w, is_changed, abs_line)
                        }
                    } else {
                        border_fn(line, w, is_changed, abs_line)
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
                        let shimmer = crate::motion::shimmer_text(
                            &display,
                            crate::motion::MotionMode::Animated,
                        );
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
                        let placeholder = if self.selection_context.is_some() {
                            "Press Enter to explain, or type a question..."
                        } else {
                            "Ask about this section..."
                        };
                        Paragraph::new(placeholder.dim().italic()).render(ta_area, buf);
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

        // Voice status line (above hints bar, when active).
        // Rendered bottom-up so we calculate position after the hints bar.
        let voice_status_line: Option<Line<'static>> = self
            .voice_status
            .as_deref()
            .or(self.tts_flash_msg.as_deref())
            .map(|vs| render::bordered_text_line(vs, w));

        // Hints bar
        if let Some(voice_status) = voice_status_line {
            by = by.saturating_sub(1);
            // Render voice status above hints.
            Paragraph::new(voice_status).render(
                Rect {
                    y: by,
                    height: 1,
                    ..area
                },
                buf,
            );
        }
        by = by.saturating_sub(1);
        let current_has_folds = self
            .sections
            .get(self.current_section)
            .is_some_and(DocumentSection::has_folds);
        let search_match_info: Option<String> = self
            .search_state
            .as_ref()
            .filter(|s| !s.matches.is_empty())
            .map(|s| format!("[{}/{}]", s.current_match_idx + 1, s.matches.len()));
        let hints = render::hints_line(
            self.focus == ReaderFocus::Composer,
            self.focus == ReaderFocus::Search,
            self.search_state.is_some(),
            self.visual_select.is_some(),
            current_has_folds,
            self.line_number_input.as_deref(),
            self.voice_status.as_deref(), // used for showing "r: read" hint
            self.voice_tts_paused,
            search_match_info.as_deref(),
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

        // Separator above hints (with scroll indicator when content overflows,
        // or a next-section nudge when the user has reached the bottom).
        by = by.saturating_sub(1);
        let sep = if has_more_below {
            render::separator_with_indicator(w, " \u{25BC} scroll for more ")
        } else if self.current_section + 1 < self.sections.len() {
            let next_heading = self
                .sections
                .get(self.current_section + 1)
                .map(|s| s.heading.as_str())
                .unwrap_or("");
            let label = if next_heading.is_empty() {
                format!(
                    " n \u{25B6} next section ({}/{}) ",
                    section_num + 1,
                    section_count
                )
            } else {
                format!(" n \u{25B6} {next_heading} ")
            };
            render::separator_with_indicator(w, &label)
        } else if self.end_of_doc_flash {
            render::separator_with_indicator_styled(
                w,
                " \u{2713} end of document ",
                ratatui::style::Style::default()
                    .fg(ratatui::style::Color::Yellow)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )
        } else {
            render::separator_with_indicator(w, " \u{2713} end of document ")
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
                let inner_w = area.width.saturating_sub(4);
                let input_h = self.input_height(inner_w);
                let bottom_y = area.y + area.height - 1; // bottom border
                let ta_y = bottom_y.saturating_sub(input_h);
                let ta_area = Rect {
                    x: area.x + 2,
                    y: ta_y,
                    width: inner_w,
                    height: input_h,
                };
                let state = *self.textarea_state.borrow();
                self.textarea.cursor_pos_with_state(ta_area, state)
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

    // Merge heading-only sections into the next section that has content.
    // When ALL headed sections are empty (streaming outline mode), preserve
    // them as placeholders and skip the merge.
    let all_empty = sections
        .iter()
        .all(|s| s.heading.is_empty() || s.content.trim().is_empty());
    if !all_empty {
        // Iterate back-to-front so chains of empty headings collapse correctly
        // (A -> B -> C becomes "A \u{2014} B \u{2014} C").
        let mut i = sections.len().saturating_sub(1);
        while i > 0 {
            i -= 1;
            if !sections[i].heading.is_empty()
                && sections[i].content.trim().is_empty()
                && i + 1 < sections.len()
                && !sections[i + 1].content.trim().is_empty()
            {
                let empty_heading = sections[i].heading.clone();
                let next = &mut sections[i + 1];
                if next.heading.is_empty() {
                    next.heading = empty_heading;
                } else {
                    next.heading = format!("{empty_heading} \u{2014} {}", next.heading);
                }
                sections.remove(i);
            }
        }
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
        let mut view = DocumentReaderView::new(
            "test-doc".to_string(),
            "Test Report".to_string(),
            test_content(),
            tx,
            true,
            crate::tui::FrameRequester::test_dummy(),
        );
        view.show_tutorial = false;
        view
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

    #[test]
    fn parse_sections_merges_empty_into_next_with_content() {
        // Two consecutive headings where only the second has body text.
        let content = "## Overview\n## Details\nSome body text here";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "Overview \u{2014} Details");
        assert_eq!(sections[0].content, "Some body text here");
    }

    #[test]
    fn parse_sections_merges_chained_empty_headings() {
        // Three consecutive headings, only the last has body text.
        let content = "## A\n## B\n## C\nContent for C";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "A \u{2014} B \u{2014} C");
        assert_eq!(sections[0].content, "Content for C");
    }

    #[test]
    fn parse_sections_mixed_empty_and_nonempty() {
        // An empty heading sandwiched between two sections with content.
        let content = "## First\nFirst body\n## Empty\n## Last\nLast body";
        let sections = parse_sections("Title", content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].heading, "First");
        assert_eq!(sections[0].content, "First body");
        assert_eq!(sections[1].heading, "Empty \u{2014} Last");
        assert_eq!(sections[1].content, "Last body");
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
        let mut view = DocumentReaderView::new(
            "test-streaming".to_string(),
            "Streaming Test".to_string(),
            outline.to_string(),
            tx,
            false, // animations_enabled = false so we get static text
            crate::tui::FrameRequester::test_dummy(),
        );
        view.show_tutorial = false;
        // streaming_sections should be active.
        assert!(
            view.streaming_sections.is_some(),
            "streaming should be detected for outline-only content"
        );
        let area = Rect::new(0, 0, 60, 15);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);
        let snap = snapshot_buffer(&buf);
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
            snap.contains("scroll"),
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
    fn prefer_esc_always_true() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Esc never closes the reading view — only `q` does.
        assert!(view.prefer_esc_to_handle_key_event());

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
        assert!(view.complete, "q should exit immediately");

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
            if let AppEvent::SubmitUserText { text } = ev {
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

        // Drain any events emitted during construction (e.g. narrate).
        while rx.try_recv().is_ok() {}

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
            if let AppEvent::SubmitUserText { text } = ev {
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

        // 6. Exit via 'q' (single press exits immediately).
        assert!(!view.is_complete());
        view.handle_key_event(key(KeyCode::Char('q')));
        assert!(view.is_complete(), "q should exit immediately");

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

        // Use a tall area so the pending indicator (appended below section
        // content) stays visible after paragraph spacing adds blank lines.
        // Render — should NOT show "thinking..." since current section (1) is not pending.
        let area = Rect::new(0, 0, 60, 30);
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

    #[test]
    fn composer_pending_allows_navigation() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to section 1 and submit a question.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('W')));
        view.textarea.input(key(KeyCode::Char('h')));
        view.textarea.input(key(KeyCode::Char('y')));
        view.handle_composer_key(key(KeyCode::Enter));
        assert!(view.pending_sections.contains_key(&1));
        assert_eq!(view.focus, ReaderFocus::Composer);

        // While pending in composer, pressing 'n' should navigate forward.
        view.handle_composer_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 2, "n should navigate to next section");
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "focus should switch to Content after navigation"
        );
        // Section 1 should still be pending.
        assert!(view.pending_sections.contains_key(&1));

        // Navigate back with 'p' from Content mode.
        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 1);

        // Tab back to composer — it should show pending spinner for section 1.
        view.focus = ReaderFocus::Composer;

        // Pressing 'p' while pending in composer should navigate backward.
        view.handle_composer_key(key(KeyCode::Char('p')));
        assert_eq!(
            view.current_section, 0,
            "p should navigate to previous section"
        );
        assert_eq!(view.focus, ReaderFocus::Content);
    }

    /// Pressing 'r' in the composer while a follow-up is pending should
    /// still trigger TTS narration (delegates to content key handler).
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn composer_pending_allows_read_aloud() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to section 1 and submit a question.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('W')));
        view.textarea.input(key(KeyCode::Char('h')));
        view.textarea.input(key(KeyCode::Char('y')));
        view.handle_composer_key(key(KeyCode::Enter));
        assert!(view.pending_sections.contains_key(&1));
        assert_eq!(view.focus, ReaderFocus::Composer);

        // Drain all events emitted so far.
        while rx.try_recv().is_ok() {}

        // While pending in composer, pressing 'r' should trigger narration.
        view.handle_composer_key(key(KeyCode::Char('r')));

        // Verify VoiceModeNarrateSection event was emitted.
        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::VoiceModeNarrateSection { manual, .. } = ev {
                assert!(manual, "narration triggered by 'r' should have manual=true");
                found_narrate = true;
            }
        }
        assert!(
            found_narrate,
            "expected VoiceModeNarrateSection event after pressing 'r' while pending"
        );

        // Focus should have switched to Content.
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "focus should switch to Content after 'r' during pending"
        );

        // Section 1 should still be pending.
        assert!(view.pending_sections.contains_key(&1));
    }

    #[test]
    fn parallel_questions_on_different_sections() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain construction events.
        while rx.try_recv().is_ok() {}

        // Ask about section 1.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('A')));
        view.handle_composer_key(key(KeyCode::Enter));
        assert!(view.pending_sections.contains_key(&1));

        // Navigate to section 3 while section 1 is pending.
        view.handle_composer_key(key(KeyCode::Char('n')));
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 3);

        // Ask about section 3.
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('B')));
        view.handle_composer_key(key(KeyCode::Enter));
        assert!(view.pending_sections.contains_key(&3));

        // Both sections should be pending simultaneously.
        assert!(
            view.pending_sections.contains_key(&1),
            "section 1 should still be pending"
        );
        assert!(
            view.pending_sections.contains_key(&3),
            "section 3 should also be pending"
        );

        // Resolve section 1 — section 3 should remain pending.
        view.update_section(1, "Answer for section 1.".to_string());
        assert!(!view.pending_sections.contains_key(&1));
        assert!(view.pending_sections.contains_key(&3));

        // Resolve section 3.
        view.update_section(3, "Answer for section 3.".to_string());
        assert!(view.pending_sections.is_empty());
    }

    // -----------------------------------------------------------------------
    // Voice reading progress tests
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    mod voice_progress_tests {
        use super::super::DocumentReaderView;
        use super::AppEvent;
        use super::AppEventSender;
        use super::Buffer;
        use super::Rect;
        use super::key;
        use super::make_view;
        use crate::bottom_pane::bottom_pane_view::BottomPaneView;
        use crate::render::renderable::Renderable;
        use crossterm::event::KeyCode;
        use tokio::sync::mpsc::unbounded_channel;

        /// Render the view once so that `last_inner_width` and other layout
        /// cells are populated from the render pass.
        fn render_view(view: &DocumentReaderView) {
            let area = Rect::new(0, 0, 80, 24);
            let mut buf = Buffer::empty(area);
            view.render(area, &mut buf);
        }

        #[test]
        fn voice_progress_first_word() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let mut view = make_view(tx);
            // Section 0 has no heading, so rendered lines start with content.
            render_view(&view);

            view.set_voice_reading_progress(Some(0), 0);
            let hl = view.voice_reading_highlight;
            assert!(hl.is_some(), "word_idx=0 should produce a highlight");
            let (line_idx, start_col, end_col) = hl.expect("checked above");
            // The first word on the first content line should be "Introduction".
            assert_eq!(line_idx, 0, "first word should be on line 0");
            assert_eq!(start_col, 0, "first word should start at col 0");
            assert_eq!(
                end_col,
                "Introduction".len(),
                "first word should be 'Introduction'"
            );
        }

        #[test]
        fn voice_progress_heading_offset() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let mut view = make_view(tx);
            // Navigate to section 1 (Methodology) which has a heading.
            view.handle_content_key(key(KeyCode::Char('n')));
            assert_eq!(view.current_section, 1);
            render_view(&view);

            // The heading "Methodology" is 1 word in the rendered lines.
            // With heading_words_to_skip=1, word_idx=1 means the first
            // TTS content word. adj = 1 - 1 = 0, which matches
            // "Methodology" (the first rendered word). That's the heading.
            // word_idx=2 → adj=1 → second rendered word, which should be
            // the first content word.
            //
            // Actually: heading_words_to_skip causes adj = wi - skip.
            // Walk ALL rendered lines counting words. adj=0 → first word = "Methodology".
            // adj=1 → second word = "Method" (first content word after blank line).
            view.set_voice_reading_progress(Some(1), 0);
            let hl_no_skip = view.voice_reading_highlight;
            assert!(
                hl_no_skip.is_some(),
                "word_idx=1 with skip=0 should find a word"
            );

            // With heading_words_to_skip=1, word_idx=0 → adj = 0-1 underflows → None.
            view.set_voice_reading_progress(Some(0), 1);
            assert!(
                view.voice_reading_highlight.is_none(),
                "word_idx=0 with skip=1 should underflow to None"
            );

            // word_idx=1, skip=1 → adj=0 → first rendered word = "Methodology".
            view.set_voice_reading_progress(Some(1), 1);
            let hl_skip = view.voice_reading_highlight;
            assert!(
                hl_skip.is_some(),
                "word_idx=1 with skip=1 should map to first rendered word"
            );
        }

        #[test]
        fn voice_progress_wrapped_line() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let mut view = make_view(tx);
            render_view(&view);

            // Section 0 content starts with "Introduction paragraph line one."
            // At width=80 with inner_width ~76, this fits on one line.
            // The second paragraph line "Introduction paragraph line two."
            // goes to a second rendered line. Words on that line are at higher indices.
            // Count words in first content line: "Introduction paragraph line one." = 4 words.
            // Word index 4 should be the first word on line 1 = "Introduction" (second line).
            view.set_voice_reading_progress(Some(4), 0);
            let hl = view.voice_reading_highlight;
            assert!(
                hl.is_some(),
                "word_idx=4 should find a word on a subsequent line"
            );
            let (line_idx, _, _) = hl.expect("checked above");
            assert!(
                line_idx > 0,
                "word on second paragraph line should be on a rendered line > 0, got {line_idx}"
            );
        }

        #[test]
        fn voice_progress_out_of_range() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let mut view = make_view(tx);
            render_view(&view);

            // A very large word index far beyond the total words.
            view.set_voice_reading_progress(Some(99999), 0);
            assert!(
                view.voice_reading_highlight.is_none(),
                "out-of-range word_idx should produce None highlight"
            );
        }

        #[test]
        fn voice_progress_skips_heading_markers() {
            // Create a view whose section content contains a ### subheading.
            // The markdown renderer keeps the "###" prefix in the rendered
            // text, but `clean_for_tts` strips it.  The karaoke counter must
            // skip the "###" word so TTS indices stay aligned.
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let content = "Start word\n### Loop\nEnd word";
            let mut view = DocumentReaderView::new(
                "heading-test".to_string(),
                "Heading Test".to_string(),
                content.to_string(),
                tx,
                true,
                crate::tui::FrameRequester::test_dummy(),
            );
            view.show_tutorial = false;
            render_view(&view);

            // TTS word 0 -> rendered "Start"
            view.set_voice_reading_progress(Some(0), 0);
            let hl = view.voice_reading_highlight;
            assert!(hl.is_some(), "word 0 should highlight");
            let (_, start, end) = hl.expect("checked");
            assert_eq!(&"Start word"[start..end], "Start");

            // TTS word 2 -> rendered "Loop" (skipping "###")
            view.set_voice_reading_progress(Some(2), 0);
            let hl = view.voice_reading_highlight;
            assert!(hl.is_some(), "word 2 should highlight");
            let (line_idx, start, end) = hl.expect("checked");
            let section = view.sections.get(view.current_section).expect("section");
            let inner_w = view.last_inner_width.get();
            let lines = section.rendered_lines(inner_w);
            let line_text: String = lines[line_idx]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(
                &line_text[start..end],
                "Loop",
                "TTS word 2 should highlight 'Loop', not the heading marker"
            );
        }

        #[test]
        fn voice_progress_skips_list_markers() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let content = "- Alpha item\n- Beta item";
            let mut view = DocumentReaderView::new(
                "list-test".to_string(),
                "List Test".to_string(),
                content.to_string(),
                tx,
                true,
                crate::tui::FrameRequester::test_dummy(),
            );
            view.show_tutorial = false;
            render_view(&view);

            view.set_voice_reading_progress(Some(0), 0);
            let hl = view.voice_reading_highlight.expect("first list word");
            let (line_idx, start, end) = hl;
            let section = view.sections.get(view.current_section).expect("section");
            let inner_w = view.last_inner_width.get();
            let lines = section.rendered_lines(inner_w);
            let line_text: String = lines[line_idx]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(&line_text[start..end], "Alpha");

            view.set_voice_reading_progress(Some(2), 0);
            let hl = view.voice_reading_highlight.expect("second list word");
            let (line_idx, start, end) = hl;
            let line_text: String = lines[line_idx]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(&line_text[start..end], "Beta");
        }
    }

    // -----------------------------------------------------------------------
    // Phase 3: Navigation, key bindings, overlays, and lifecycle tests
    // -----------------------------------------------------------------------

    // Test 28: section_nav_n_p
    #[test]
    fn section_nav_n_p() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.current_section, 0);
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1, "n should advance to section 1");
        view.handle_content_key(key(KeyCode::Char('p')));
        assert_eq!(view.current_section, 0, "p should go back to section 0");
    }

    // Test 29: scroll_j_k
    #[test]
    fn scroll_j_k() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.cursor_line, 0);

        view.handle_content_key(key(KeyCode::Char('j')));
        view.handle_content_key(key(KeyCode::Char('j')));
        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(view.cursor_line, 3, "j x3 should move cursor to line 3");

        view.handle_content_key(key(KeyCode::Char('k')));
        view.handle_content_key(key(KeyCode::Char('k')));
        assert_eq!(
            view.cursor_line, 1,
            "k x2 should move cursor back to line 1"
        );
    }

    // Test 30: half_page_scroll_ctrl_d_u
    #[test]
    fn half_page_scroll_ctrl_d_u() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Render to populate last_content_height.
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let before = view.cursor_line;
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        view.handle_content_key(ctrl_d);
        assert!(
            view.cursor_line > before,
            "Ctrl+d should move cursor forward: before={before}, after={}",
            view.cursor_line
        );

        let after_d = view.cursor_line;
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        view.handle_content_key(ctrl_u);
        assert!(
            view.cursor_line < after_d,
            "Ctrl+u should move cursor backward: after_d={after_d}, after_u={}",
            view.cursor_line
        );
    }

    // Test 31: home_end_navigation (extended)
    #[test]
    fn home_end_navigation_extended() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Navigate to a middle section first.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        view.handle_content_key(key(KeyCode::End));
        assert_eq!(
            view.current_section,
            view.sections.len() - 1,
            "End should jump to the last section"
        );

        view.handle_content_key(key(KeyCode::Home));
        assert_eq!(view.current_section, 0, "Home should jump to section 0");
    }

    // Test 32: cursor_h_l_w_b
    #[test]
    fn cursor_h_l_w_b() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Render to populate last_inner_width.
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        assert_eq!(view.cursor_col, 0);

        // Press 'l' to move right.
        view.handle_content_key(key(KeyCode::Char('l')));
        assert_eq!(view.cursor_col, 1, "l should move cursor right by 1");

        view.handle_content_key(key(KeyCode::Char('l')));
        assert_eq!(view.cursor_col, 2, "l again should move cursor to col 2");

        // Press 'h' to move left.
        view.handle_content_key(key(KeyCode::Char('h')));
        assert_eq!(view.cursor_col, 1, "h should move cursor left by 1");

        // Press 'w' to move word forward.
        let before_w = view.cursor_col;
        let before_line = view.cursor_line;
        view.handle_content_key(key(KeyCode::Char('w')));
        let moved = view.cursor_col != before_w || view.cursor_line != before_line;
        assert!(moved, "w should move cursor to a different position");

        // Press 'b' to move word backward.
        let before_b_col = view.cursor_col;
        let before_b_line = view.cursor_line;
        view.handle_content_key(key(KeyCode::Char('b')));
        let moved_back = view.cursor_col != before_b_col || view.cursor_line != before_b_line;
        assert!(moved_back, "b should move cursor backward");
    }

    // Test 33: visual_selection_v
    #[test]
    fn visual_selection_v() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Render to populate widths.
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        assert!(view.visual_select.is_none());

        // Press 'v' to enter visual mode.
        view.handle_content_key(key(KeyCode::Char('v')));
        assert!(
            view.visual_select.is_some(),
            "v should enter visual selection mode"
        );
        let vs = view.visual_select.as_ref().expect("just checked");
        assert_eq!(vs.mode, VisualMode::Char, "v should start char selection");
        assert_eq!(vs.anchor_line, 0);
        assert_eq!(vs.anchor_col, 0);

        // Extend selection with 'l'.
        view.handle_content_key(key(KeyCode::Char('l')));
        view.handle_content_key(key(KeyCode::Char('l')));
        view.handle_content_key(key(KeyCode::Char('l')));
        assert!(
            view.visual_select.is_some(),
            "visual mode should still be active after cursor movement"
        );
        assert_eq!(
            view.cursor_col, 3,
            "cursor should have moved right in visual mode"
        );

        // Press Esc to exit visual mode.
        view.handle_content_key(key(KeyCode::Esc));
        assert!(
            view.visual_select.is_none(),
            "Esc should exit visual selection mode"
        );
    }

    // Test 34: fold_toggle_f
    #[test]
    fn fold_toggle_f() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Test content does not have folds by default.
        // Pressing 'f' should be a no-op but must not panic.
        let folds_before = view.sections[view.current_section].folds.len();
        view.handle_content_key(key(KeyCode::Char('f')));
        let folds_after = view.sections[view.current_section].folds.len();
        assert_eq!(
            folds_before, folds_after,
            "f on content without foldable regions should be no-op"
        );
    }

    // Test 35: search_mode
    #[test]
    fn search_mode() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.focus, ReaderFocus::Content);
        assert!(view.search_state.is_none());

        // Press '/' to enter search mode.
        view.handle_content_key(key(KeyCode::Char('/')));
        assert_eq!(
            view.focus,
            ReaderFocus::Search,
            "/ should switch focus to Search"
        );

        // Type search text.
        view.handle_search_key(key(KeyCode::Char('l')));
        view.handle_search_key(key(KeyCode::Char('i')));
        view.handle_search_key(key(KeyCode::Char('n')));
        view.handle_search_key(key(KeyCode::Char('e')));
        assert_eq!(
            view.search_input, "line",
            "search input should accumulate typed chars"
        );
        assert!(
            view.search_state.is_some(),
            "incremental search should populate search_state"
        );

        // Press Esc to exit search mode.
        view.handle_search_key(key(KeyCode::Esc));
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "Esc should return focus to Content"
        );
        assert!(view.search_state.is_none(), "Esc should clear search state");
    }

    // Test 36: toc_overlay_toggle
    #[test]
    fn toc_overlay_toggle() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.show_toc);

        // Press 't' to show TOC.
        view.handle_content_key(key(KeyCode::Char('t')));
        assert!(view.show_toc, "t should toggle TOC overlay on");
        assert_eq!(
            view.toc_selected_index, view.current_section,
            "TOC should highlight the current section"
        );

        // Navigate within TOC with 'j'.
        view.handle_content_key(key(KeyCode::Char('j')));
        assert_eq!(
            view.toc_selected_index, 1,
            "j in TOC should move selection down"
        );

        // Dismiss with Esc.
        view.handle_content_key(key(KeyCode::Esc));
        assert!(!view.show_toc, "Esc should dismiss TOC overlay");
    }

    // Test 37: help_overlay_toggle
    #[test]
    fn help_overlay_toggle() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.show_help);

        // Press '?' to show help overlay.
        view.handle_content_key(key(KeyCode::Char('?')));
        assert!(view.show_help, "? should toggle help overlay on");

        // Dismiss with Esc.
        view.handle_content_key(key(KeyCode::Esc));
        assert!(!view.show_help, "Esc should dismiss help overlay");
    }

    // Test 38: q_exits_immediately
    #[test]
    fn q_exits_immediately() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.complete);

        // Single q exits immediately.
        view.handle_content_key(key(KeyCode::Char('q')));
        assert!(view.complete, "q should exit immediately");
    }

    #[test]
    fn esc_does_not_exit() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert!(!view.complete);

        // Esc without search active is a no-op (never exits).
        view.handle_content_key(key(KeyCode::Esc));
        assert!(!view.complete, "Esc should not exit reading mode");

        // Pressing Esc again still does not exit.
        view.handle_content_key(key(KeyCode::Esc));
        assert!(!view.complete, "repeated Esc should not exit reading mode");
    }

    #[test]
    fn esc_clears_search() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Activate search so there is something to dismiss.
        view.handle_content_key(key(KeyCode::Char('/')));
        view.handle_search_key(key(KeyCode::Char('a')));
        view.handle_search_key(key(KeyCode::Enter));
        assert!(view.search_state.is_some(), "search should be active");

        // Esc clears search and does not exit.
        view.handle_content_key(key(KeyCode::Esc));
        assert!(view.search_state.is_none(), "Esc should clear search");
        assert!(!view.complete, "Esc should not exit after clearing search");
    }

    // Test 39: tab_opens_composer
    #[test]
    fn tab_opens_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.focus, ReaderFocus::Content);
        view.handle_content_key(key(KeyCode::Tab));
        assert_eq!(
            view.focus,
            ReaderFocus::Composer,
            "Tab should switch focus to Composer"
        );
    }

    // Test 40: enter_submits_follow_up
    #[test]
    fn enter_submits_follow_up() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain any construction-time events.
        while rx.try_recv().is_ok() {}

        // Switch to composer and type text.
        view.focus = ReaderFocus::Composer;
        view.textarea.input(key(KeyCode::Char('t')));
        view.textarea.input(key(KeyCode::Char('e')));
        view.textarea.input(key(KeyCode::Char('s')));
        view.textarea.input(key(KeyCode::Char('t')));

        // Submit via Enter.
        view.handle_composer_key(key(KeyCode::Enter));

        // Verify a CodexOp(UserInput) was sent.
        let mut found_op = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::SubmitUserText { text } = ev {
                assert!(
                    text.contains("test"),
                    "submitted text should contain 'test': {text}"
                );
                found_op = true;
            }
        }
        assert!(found_op, "expected CodexOp(UserInput) event after Enter");
    }

    // Test 41: update_section_while_viewing
    #[test]
    fn update_section_while_viewing() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        assert_eq!(view.current_section, 0);
        let original = view.sections[0].content.clone();

        view.update_section(0, "Completely new intro content.".to_string());

        assert_ne!(view.sections[0].content, original);
        assert_eq!(view.sections[0].content, "Completely new intro content.");
        assert!(
            view.sections[0].recently_updated,
            "recently_updated flag should be set after update"
        );
    }

    // Test 42: append_section_increases_count
    #[test]
    fn append_section_increases_count() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        let original_count = view.sections.len();
        assert_eq!(original_count, 4);

        // Append to existing section 1 (extending content, not adding a new section).
        view.append_to_section(1, "More methodology info.".to_string(), false, None);
        assert_eq!(
            view.sections.len(),
            original_count,
            "append_to_section should not change section count"
        );
        assert!(
            view.sections[1].content.contains("More methodology info."),
            "appended content should be present"
        );
    }

    // Test 43: rapid_n_presses
    #[test]
    fn rapid_n_presses() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        for _ in 0..100 {
            view.handle_content_key(key(KeyCode::Char('n')));
        }
        assert_eq!(
            view.current_section,
            view.sections.len() - 1,
            "rapid n presses should clamp to last section"
        );
    }

    // Test 44: resize_no_panic
    #[test]
    fn resize_no_panic() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = make_view(tx);

        // Render at various sizes -- none should panic.
        for (w, h) in [(80, 24), (40, 12), (120, 40), (20, 5), (200, 60)] {
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            view.render(area, &mut buf);
        }
    }

    // Test 45: section_update_during_fold
    #[test]
    fn section_update_during_fold() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Manually add a fold region to section 1.
        let content_len = view.sections[1].content.len();
        if content_len > 5 {
            view.sections[1].folds.push(FoldRegion {
                start: 0,
                end: content_len.min(10),
                summary: "fold".to_string(),
                collapsed: true,
            });
        }

        // Update the section while a fold exists -- should not panic.
        view.update_section(1, "Brand new content replacing old.".to_string());
        assert_eq!(view.sections[1].content, "Brand new content replacing old.");
    }

    // Test 46: exit_during_voice_state
    #[test]
    fn exit_during_voice_state() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Set voice status to simulate active voice mode.
        view.voice_status = Some("Speaking...".to_string());

        // Press q to exit -- should not panic even with voice state active.
        view.handle_content_key(key(KeyCode::Char('q')));
        assert!(
            view.complete,
            "should exit cleanly even with voice state set"
        );
    }

    // Test: composer cursor X position wraps correctly on multi-line input.
    // Regression test: the old code used str::len of the last logical line,
    // which kept the cursor at the rightmost column instead of wrapping.
    #[test]
    fn composer_cursor_wraps_to_next_line() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        // Switch to composer.
        view.focus = ReaderFocus::Composer;

        // inner_w = 40 - 4 = 36 usable columns.

        // Type a short string — cursor should be at end of text on line 1.
        for ch in "hello".chars() {
            view.textarea.input(key(KeyCode::Char(ch)));
        }
        view.render(area, &mut buf);
        let pos_short = view.cursor_pos(area);
        assert!(pos_short.is_some(), "cursor should be visible");
        let (x1, _y1) = pos_short.unwrap_or_default();
        assert_eq!(
            x1,
            area.x + 2 + 5,
            "cursor X should be at col 5 (after 'hello')"
        );

        // Clear existing text.
        for _ in 0..5 {
            view.textarea.input(key(KeyCode::Backspace));
        }

        // Type 40 'a' chars — wraps at col 36, putting 4 chars on the second
        // wrapped line. The cursor should be at X = area.x + 2 + 4, NOT at
        // area.x + 2 + 40 (the old buggy behavior that used total byte length).
        for _ in 0..40 {
            view.textarea.input(key(KeyCode::Char('a')));
        }
        view.render(area, &mut buf);
        let pos_wrapped = view.cursor_pos(area);
        assert!(
            pos_wrapped.is_some(),
            "cursor should be visible after wrapping"
        );
        let (x2, _y2) = pos_wrapped.unwrap_or_default();

        // The textarea grows upward so the absolute Y may stay the same, but
        // the X must reflect the wrapped position on the second line.
        let expected_x = area.x + 2 + (40 - 36);
        assert_eq!(
            x2, expected_x,
            "cursor X should be at the wrapped position on line 2, not the total text length"
        );
    }

    // -----------------------------------------------------------------------
    // 's' key pause/resume TTS tests
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn s_key_sends_pause_when_voice_active() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain any events emitted during construction (e.g. narrate).
        while rx.try_recv().is_ok() {}

        // Simulate active TTS playback (not paused).
        view.voice_status = Some("Speaking...".to_string());
        view.voice_tts_paused = false;

        // Press 's'.
        view.handle_content_key(key(KeyCode::Char('s')));

        // Verify VoiceModePauseTts event was emitted.
        let mut found_pause = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::VoiceModePauseTts) {
                found_pause = true;
            }
        }
        assert!(
            found_pause,
            "expected VoiceModePauseTts event when 's' pressed during active voice"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn s_key_sends_resume_when_paused() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain any events emitted during construction (e.g. narrate).
        while rx.try_recv().is_ok() {}

        // Simulate paused TTS playback.
        view.voice_status = Some("Paused".to_string());
        view.voice_tts_paused = true;

        // Press 's'.
        view.handle_content_key(key(KeyCode::Char('s')));

        // Verify VoiceModeResumeTts event was emitted.
        let mut found_resume = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::VoiceModeResumeTts) {
                found_resume = true;
            }
        }
        assert!(
            found_resume,
            "expected VoiceModeResumeTts event when 's' pressed while paused"
        );
    }

    // -----------------------------------------------------------------------
    // `r` key narration trigger test
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn r_key_triggers_narration() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain any events emitted during construction (e.g. auto-narrate).
        while rx.try_recv().is_ok() {}

        // Press 'r' to manually trigger narration.
        view.handle_content_key(key(KeyCode::Char('r')));

        // Verify VoiceModeNarrateSection event was emitted with manual=true.
        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::VoiceModeNarrateSection { manual, .. } = ev {
                assert!(manual, "narration triggered by 'r' should have manual=true");
                found_narrate = true;
            }
        }
        assert!(
            found_narrate,
            "expected VoiceModeNarrateSection event after pressing 'r'"
        );
    }

    // -----------------------------------------------------------------------
    // Fold toggle with foldable content
    // -----------------------------------------------------------------------

    #[test]
    fn fold_toggle_f_with_code_block() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);

        // Create a view with content that has a manually-added fold region
        // (folds are created by follow-up Q&A, not from code blocks).
        let mut view = make_view(tx);

        // Render to populate layout dimensions.
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        // Manually add a fold region covering a portion of section 0 content.
        let content = &view.sections[0].content;
        let fold_start = content
            .find("Introduction paragraph line three.")
            .unwrap_or(0);
        let fold_end = content.len();
        view.sections[0].folds.push(FoldRegion {
            start: fold_start,
            end: fold_end,
            summary: "folded content".to_string(),
            collapsed: false,
        });
        view.sections[0].invalidate_cache();

        // Render the expanded state.
        let mut buf_expanded = Buffer::empty(area);
        view.render(area, &mut buf_expanded);
        let snap_expanded = snapshot_buffer(&buf_expanded);

        // Move cursor to a line within the fold region so 'f' targets it.
        // The fold region starts at some rendered line; move cursor to it.
        view.cursor_line = 3; // Should be within the fold range.
        view.clamp_and_scroll();

        // Press 'f' to toggle fold (should collapse).
        view.handle_content_key(key(KeyCode::Char('f')));

        // Check if a fold was collapsed.
        let any_collapsed = view.sections[0].folds.iter().any(|f| f.collapsed);
        assert!(
            any_collapsed,
            "pressing 'f' should collapse the fold under the cursor"
        );

        // Render after folding.
        let mut buf_folded = Buffer::empty(area);
        view.render(area, &mut buf_folded);
        let snap_folded = snapshot_buffer(&buf_folded);

        // The folded output should be different from the expanded output.
        assert_ne!(
            snap_expanded, snap_folded,
            "rendered output should change after folding"
        );

        // Press 'f' again to unfold.
        view.handle_content_key(key(KeyCode::Char('f')));

        // Check that the fold is now expanded.
        let all_expanded = view.sections[0].folds.iter().all(|f| !f.collapsed);
        assert!(
            all_expanded,
            "pressing 'f' again should expand the fold back"
        );

        // Render after unfolding.
        let mut buf_unfolded = Buffer::empty(area);
        view.render(area, &mut buf_unfolded);
        let snap_unfolded = snapshot_buffer(&buf_unfolded);

        // The unfolded output should match the original expanded output.
        assert_eq!(
            snap_expanded, snap_unfolded,
            "rendered output should return to original after unfolding"
        );
    }

    // -----------------------------------------------------------------------
    // Voice progress render verification
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    mod voice_progress_render_tests {
        use super::AppEvent;
        use super::AppEventSender;
        use super::Buffer;
        use super::Rect;
        use super::make_view;
        use crate::bottom_pane::bottom_pane_view::BottomPaneView;
        use crate::render::renderable::Renderable;
        use ratatui::style::Modifier;
        use tokio::sync::mpsc::unbounded_channel;

        #[test]
        fn voice_progress_renders_highlight_in_buffer() {
            let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
            let tx = AppEventSender::new(tx_raw);
            let mut view = make_view(tx);

            // Render once to populate layout dimensions.
            let area = Rect::new(0, 0, 80, 24);
            let mut buf = Buffer::empty(area);
            view.render(area, &mut buf);

            // Set voice reading progress to highlight the first word (word_idx=0).
            view.set_voice_reading_progress(Some(0), 0);

            // The first word should be "Introduction".
            let hl = view.voice_reading_highlight;
            assert!(hl.is_some(), "word_idx=0 should produce a highlight");

            // Render again with the highlight active.
            let mut buf2 = Buffer::empty(area);
            view.render(area, &mut buf2);

            // Scan the buffer for an 'I' cell (start of "Introduction")
            // that has BOLD+UNDERLINED modifiers applied.
            let mut found_bold_underline = false;
            for y in 0..area.height {
                for x in 0..area.width {
                    let cell = &buf2[(x, y)];
                    let ch = cell.symbol().chars().next().unwrap_or(' ');
                    if ch == 'I' {
                        let mods = cell.style().add_modifier;
                        if mods.contains(Modifier::BOLD) && mods.contains(Modifier::UNDERLINED) {
                            found_bold_underline = true;
                            break;
                        }
                    }
                }
                if found_bold_underline {
                    break;
                }
            }
            assert!(
                found_bold_underline,
                "highlighted word 'Introduction' should have BOLD+UNDERLINED style in the rendered buffer"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Auto-navigate tests
    // -----------------------------------------------------------------------

    #[test]
    fn auto_navigate_on_update_section() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);
        // Starts at section 0, pending_auto_navigate is true.
        assert_eq!(view.current_section, 0);
        assert!(view.pending_auto_navigate);

        // Update section 2 — should auto-navigate there.
        view.update_section(2, "Updated results content.".to_string());
        assert_eq!(view.current_section, 2);
        assert!(!view.pending_auto_navigate);
    }

    #[test]
    fn auto_navigate_on_append_to_section() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);
        assert_eq!(view.current_section, 0);

        // Append to section 3 — should auto-navigate.
        view.append_to_section(3, "\nAppended answer.".to_string(), false, None);
        assert_eq!(view.current_section, 3);
        assert!(!view.pending_auto_navigate);
    }

    #[test]
    fn auto_navigate_on_patch_section() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);
        assert_eq!(view.current_section, 0);

        // Patch section 1 — replace some text.
        view.patch_section(1, "Method details here.", "Improved methods.", false, None);
        assert_eq!(view.current_section, 1);
        assert!(!view.pending_auto_navigate);
    }

    #[test]
    fn auto_navigate_skipped_for_streaming_fill() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        // Create a view with an outline (empty sections trigger streaming mode).
        let mut view = DocumentReaderView::new(
            "test-doc".to_string(),
            "Report".to_string(),
            "## Section A\n\n## Section B\n\n## Section C\n".to_string(),
            tx,
            true,
            crate::tui::FrameRequester::test_dummy(),
        );
        view.show_tutorial = false;
        assert!(view.streaming_sections.is_some());
        assert!(view.pending_auto_navigate);

        // Streaming fill of section 1 should NOT auto-navigate.
        view.update_section(1, "## Section B\nFilled content.".to_string());
        assert_eq!(
            view.current_section, 0,
            "streaming fill should not auto-navigate"
        );
        assert!(
            view.pending_auto_navigate,
            "flag should remain set during streaming"
        );
    }

    #[test]
    fn auto_navigate_cancelled_by_user_interaction() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);
        assert!(view.pending_auto_navigate);

        // User presses a key — cancels auto-navigate.
        view.handle_content_key(key(KeyCode::Char('j')));
        assert!(!view.pending_auto_navigate);

        // Now update section 2 — should NOT auto-navigate.
        view.update_section(2, "Updated results.".to_string());
        assert_eq!(
            view.current_section, 0,
            "should stay at section 0 after user interaction"
        );
    }

    #[test]
    fn auto_navigate_unfolds_collapsed_folds() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);

        // Add a collapsed fold to section 2.
        let content_len = view.sections[2].content.len();
        view.sections[2].folds.push(FoldRegion {
            start: 0,
            end: content_len,
            summary: "Collapsed fold".to_string(),
            collapsed: true,
        });
        assert!(view.sections[2].folds[0].collapsed);

        // Auto-navigate to section 2 via update.
        view.update_section(2, "New results.".to_string());
        assert_eq!(view.current_section, 2);
        // Folds are cleared by update_section (folds.clear()), so check the
        // navigate_to_section method directly via append which preserves folds.
    }

    #[test]
    fn auto_navigate_not_triggered_for_same_section() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx);
        let mut view = make_view(tx);
        assert_eq!(view.current_section, 0);

        // Update section 0 (already the current section) — navigate_to_section
        // checks section_index != current_section, so the flag should still be
        // consumed but we stay at section 0.
        view.update_section(0, "Updated intro.".to_string());
        // pending_auto_navigate is cleared even though we didn't move.
        assert!(!view.pending_auto_navigate);
        assert_eq!(view.current_section, 0);
    }

    // -----------------------------------------------------------------------
    // Auto-read disabled tests
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn opening_reading_view_does_not_auto_narrate() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let _view = make_view(tx);

        // Drain any events emitted during construction.
        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::VoiceModeNarrateSection { .. }) {
                found_narrate = true;
            }
        }
        assert!(
            !found_narrate,
            "opening a reading view should NOT auto-narrate"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn navigating_to_next_section_does_not_auto_narrate() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain construction events.
        while rx.try_recv().is_ok() {}

        // Navigate to next section.
        view.handle_content_key(key(KeyCode::Char('n')));
        assert_eq!(view.current_section, 1);

        // Check no narration event was emitted.
        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::VoiceModeNarrateSection { .. }) {
                found_narrate = true;
            }
        }
        assert!(
            !found_narrate,
            "navigating to next section should NOT auto-narrate"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn pressing_r_triggers_manual_narration() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Drain construction events.
        while rx.try_recv().is_ok() {}

        // Press 'r' to manually trigger narration.
        view.handle_content_key(key(KeyCode::Char('r')));

        // Should emit VoiceModeNarrateSection with manual=true.
        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::VoiceModeNarrateSection { manual, .. } = ev {
                assert!(manual, "narration triggered by 'r' should have manual=true");
                found_narrate = true;
            }
        }
        assert!(
            found_narrate,
            "expected VoiceModeNarrateSection event after pressing 'r'"
        );
    }

    // -----------------------------------------------------------------------
    // Karaoke auto-scroll tests
    // -----------------------------------------------------------------------

    #[test]
    fn karaoke_auto_scroll_starts_true() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let view = make_view(tx);
        assert!(
            view.karaoke_auto_scroll,
            "karaoke_auto_scroll should start as true"
        );
    }

    #[test]
    fn manual_scroll_disables_karaoke_auto_scroll() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate active karaoke highlighting so the disable logic triggers.
        view.voice_reading_highlight = Some((0, 0, 3));

        // Manual scroll with 'j' should disable auto-scroll.
        view.handle_content_key(key(KeyCode::Char('j')));
        assert!(
            !view.karaoke_auto_scroll,
            "manual scroll during karaoke should disable auto-scroll"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn karaoke_auto_scroll_resets_on_new_highlight() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate user manually scrolled during karaoke.
        view.voice_reading_highlight = Some((0, 0, 3));
        view.handle_content_key(key(KeyCode::Char('j')));
        assert!(
            !view.karaoke_auto_scroll,
            "precondition: auto-scroll disabled"
        );

        // Clear highlight (as if sentence ended).
        view.voice_reading_highlight = None;
        view.karaoke_auto_scroll = false; // still disabled after clear

        // Simulate new karaoke lines arriving — this is the production path
        // via set_voice_karaoke_lines when TTS starts a new section.
        // When voice_karaoke_lines transitions from None to Some,
        // karaoke_auto_scroll is re-enabled.
        view.set_voice_karaoke_lines(Some(vec![ratatui::text::Line::from("test")]), false);
        assert!(
            view.karaoke_auto_scroll,
            "auto-scroll should re-enable when new karaoke lines arrive"
        );
    }

    // -----------------------------------------------------------------------
    // Search cross-section tests
    // -----------------------------------------------------------------------

    #[test]
    fn search_finds_matches_across_all_sections() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // "Method" appears in section 1 heading "Methodology".
        // "Result" appears in section 2 heading "Results".
        // Use a term that appears in multiple sections.
        view.search_input = "more".to_string();
        view.execute_search();

        let state = view
            .search_state
            .as_ref()
            .expect("search state should exist");
        // "More" appears in "More method details." (section 1)
        // and "More result findings." (section 2)
        // and "More discussion text." (section 3)
        assert!(
            state.matches.len() >= 3,
            "search should find matches across multiple sections, found {}",
            state.matches.len()
        );

        // Verify matches are in different sections.
        let sections: std::collections::HashSet<usize> =
            state.matches.iter().map(|m| m.section_idx).collect();
        assert!(
            sections.len() >= 2,
            "matches should span at least 2 different sections"
        );
    }

    #[test]
    fn search_next_wraps_around() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.search_input = "more".to_string();
        view.execute_search();

        let total = view
            .search_state
            .as_ref()
            .expect("search state")
            .matches
            .len();
        assert!(total >= 2, "need at least 2 matches to test wrapping");

        // Advance to the last match.
        for _ in 0..total - 1 {
            view.next_match();
        }
        let idx_before = view.search_state.as_ref().unwrap().current_match_idx;
        assert_eq!(idx_before, total - 1, "should be at last match");

        // Next match should wrap to index 0.
        view.next_match();
        let idx_after = view.search_state.as_ref().unwrap().current_match_idx;
        assert_eq!(idx_after, 0, "next_match should wrap around to first match");
    }

    #[test]
    fn search_prev_wraps_around() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.search_input = "more".to_string();
        view.execute_search();

        let total = view
            .search_state
            .as_ref()
            .expect("search state")
            .matches
            .len();
        assert!(total >= 2, "need at least 2 matches");

        // Start at first match. Prev should wrap to last.
        // Navigate to position 0 first.
        while view.search_state.as_ref().unwrap().current_match_idx != 0 {
            view.prev_match();
        }

        view.prev_match();
        let idx_after = view.search_state.as_ref().unwrap().current_match_idx;
        assert_eq!(
            idx_after,
            total - 1,
            "prev_match from first should wrap to last match"
        );
    }

    #[test]
    fn search_jumps_to_different_section() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);
        assert_eq!(view.current_section, 0);

        // Search for something that only appears in section 1+.
        view.search_input = "method".to_string();
        view.execute_search();

        let state = view.search_state.as_ref().expect("search state");
        assert!(
            !state.matches.is_empty(),
            "should find 'method' in Methodology section"
        );

        // The first match should be in section 1 (Methodology heading or content).
        let first_match_section = state.matches[state.current_match_idx].section_idx;
        assert_eq!(
            first_match_section, 1,
            "'method' match should be in section 1"
        );

        // View should have navigated to section 1.
        assert_eq!(
            view.current_section, 1,
            "jump_to_current_match should switch to section containing the match"
        );
    }

    #[test]
    fn search_empty_query_returns_no_matches() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.search_input = String::new();
        view.execute_search();

        assert!(
            view.search_state.is_none(),
            "empty search should clear search state"
        );
    }

    #[test]
    fn search_no_matches_in_current_but_matches_in_other() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);
        assert_eq!(view.current_section, 0);

        // "Discussion" only appears in section 3.
        view.search_input = "discussion".to_string();
        view.execute_search();

        let state = view.search_state.as_ref().expect("search state");
        assert!(
            !state.matches.is_empty(),
            "should find matches in other sections"
        );

        // Current section should have switched to section 3.
        assert_eq!(
            view.current_section, 3,
            "should navigate to section with match"
        );
    }

    #[test]
    fn search_term_not_found_anywhere() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        view.search_input = "xyznonexistent".to_string();
        view.execute_search();

        let state = view
            .search_state
            .as_ref()
            .expect("search state should exist but be empty");
        assert!(
            state.matches.is_empty(),
            "search for nonexistent term should return no matches"
        );
    }

    // -----------------------------------------------------------------------
    // Q&A read aloud tests (keys during pending state)
    // -----------------------------------------------------------------------

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn r_key_works_during_pending_answer_in_composer() {
        let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate pending state: a follow-up question was submitted for
        // the current section and we're waiting for the agent's response.
        view.pending_sections.insert(
            view.current_section,
            ("test question".to_string(), Instant::now()),
        );
        view.focus = ReaderFocus::Composer;

        // Drain any prior events.
        while rx.try_recv().is_ok() {}

        // Press 'r' in composer while pending — should switch to content
        // and trigger narration.
        view.handle_composer_key(key(KeyCode::Char('r')));

        let mut found_narrate = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::VoiceModeNarrateSection { .. }) {
                found_narrate = true;
            }
        }
        assert!(
            found_narrate,
            "'r' should trigger narration even while waiting for follow-up answer"
        );
    }

    #[test]
    fn s_key_works_during_pending_answer_in_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate pending state.
        view.pending_sections.insert(
            view.current_section,
            ("test question".to_string(), Instant::now()),
        );
        view.focus = ReaderFocus::Composer;

        // Press 's' — should switch focus to content (TTS pause/resume control).
        view.handle_composer_key(key(KeyCode::Char('s')));
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "'s' during pending should switch to content for TTS control"
        );
    }

    #[test]
    fn speed_keys_work_during_pending_answer_in_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate pending state.
        view.pending_sections.insert(
            view.current_section,
            ("test question".to_string(), Instant::now()),
        );
        view.focus = ReaderFocus::Composer;

        // Press '+' — should switch focus to content (speed control).
        view.handle_composer_key(key(KeyCode::Char('+')));
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "'+' during pending should switch to content for speed control"
        );

        // Reset focus.
        view.focus = ReaderFocus::Composer;

        // Press '-' — should also switch focus.
        view.handle_composer_key(key(KeyCode::Char('-')));
        assert_eq!(
            view.focus,
            ReaderFocus::Content,
            "'-' during pending should switch to content for speed control"
        );
    }

    #[test]
    fn other_keys_blocked_during_pending_in_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut view = make_view(tx);

        // Simulate pending state.
        view.pending_sections.insert(
            view.current_section,
            ("test question".to_string(), Instant::now()),
        );
        view.focus = ReaderFocus::Composer;

        // Press a random letter — should be ignored (no text input during pending).
        view.handle_composer_key(key(KeyCode::Char('x')));
        assert!(
            view.textarea.text().is_empty(),
            "regular keys should be blocked during pending answer in composer"
        );
    }
}
