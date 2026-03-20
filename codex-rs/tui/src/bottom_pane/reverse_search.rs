//! Bash-style `Ctrl+R` reverse incremental search over prompt history.
//!
//! This module is self-contained: it reads the `history.jsonl` file once on
//! activation, merges local session history, and exposes a key-event handler
//! plus ratatui `Line` renderers for the candidate list and search prompt.

use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use serde::Deserialize;

use crate::line_truncation::truncate_line_to_width;

/// Maximum number of candidate lines visible at once.
const MAX_VISIBLE_CANDIDATES: usize = 6;

/// A history entry ready for case-insensitive search.
struct SearchableEntry {
    text: String,
    text_lower: String,
}

/// Result of processing a key event during reverse search.
pub(crate) enum ReverseSearchAction {
    /// Search continues -- caller should redraw.
    Continue,
    /// User accepted the current match.
    Accept(String),
    /// User cancelled the search.
    Cancel,
}

/// Reverse incremental search state.
pub(crate) struct ReverseSearch {
    query: String,
    entries: Vec<SearchableEntry>,
    /// Indices into `entries` that match the current query (newest-first).
    matching_indices: Vec<usize>,
    /// Index into `matching_indices` of the currently selected candidate.
    selected: usize,
    /// Scroll offset for the visible candidate window.
    scroll_offset: usize,
    /// Cached text of the selected match for display / acceptance.
    matched_text: Option<String>,
}

/// Minimal deserialisation target -- we only need the `text` field.
#[derive(Deserialize)]
struct HistoryRecord {
    text: String,
}

impl ReverseSearch {
    /// Create a new reverse search session.
    ///
    /// `history_path` points to `~/.ata/history.jsonl`. `local_texts` are the
    /// current session's submitted prompts in newest-first order.
    pub(crate) fn new(history_path: Option<&PathBuf>, local_texts: Vec<String>) -> Self {
        let mut seen = std::collections::HashSet::new();
        let mut entries: Vec<SearchableEntry> = Vec::new();

        // Local session history first (already newest-first).
        for text in local_texts {
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
                let text_lower = trimmed.to_lowercase();
                entries.push(SearchableEntry {
                    text: trimmed,
                    text_lower,
                });
            }
        }

        // Persistent history from disk (oldest-first in file, so collect and reverse).
        if let Some(path) = history_path
            && let Ok(file) = File::open(path)
        {
            let reader = BufReader::new(file);
            let mut disk_entries: Vec<String> = Vec::new();
            for line in reader.lines() {
                let Ok(line) = line else { continue };
                let Ok(record) = serde_json::from_str::<HistoryRecord>(&line) else {
                    continue;
                };
                let trimmed = record.text.trim().to_string();
                if !trimmed.is_empty() {
                    disk_entries.push(trimmed);
                }
            }
            // Newest-first.
            for text in disk_entries.into_iter().rev() {
                if seen.insert(text.clone()) {
                    let text_lower = text.to_lowercase();
                    entries.push(SearchableEntry { text, text_lower });
                }
            }
        }

        Self {
            query: String::new(),
            entries,
            matching_indices: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            matched_text: None,
        }
    }

    /// Process a key event during reverse search.
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) -> ReverseSearchAction {
        if key.kind != KeyEventKind::Press {
            return ReverseSearchAction::Continue;
        }

        match key {
            // Ctrl+R / Down -- cycle to next older match
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Down,
                ..
            } => {
                self.select_next();
                ReverseSearchAction::Continue
            }
            // Up -- cycle to next newer match
            KeyEvent {
                code: KeyCode::Up, ..
            } => {
                self.select_prev();
                ReverseSearchAction::Continue
            }
            // Enter -- accept current match
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                let text = self.matched_text.clone().unwrap_or_default();
                ReverseSearchAction::Accept(text)
            }
            // Esc / Ctrl+C / Ctrl+G -- cancel
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('c') | KeyCode::Char('g'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => ReverseSearchAction::Cancel,
            // Backspace -- remove last query char
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.query.pop();
                self.rebuild_matches();
                ReverseSearchAction::Continue
            }
            // Printable char (no Ctrl/Alt)
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.query.push(ch);
                self.rebuild_matches();
                ReverseSearchAction::Continue
            }
            _ => ReverseSearchAction::Continue,
        }
    }

    /// Render the search prompt line showing match count status.
    pub(crate) fn search_prompt_line(&self, max_width: usize) -> Line<'static> {
        let prefix = format!("(reverse-i-search)`{}'", self.query);
        let separator = ": ";
        let status = if self.matching_indices.is_empty() {
            if self.query.is_empty() {
                String::new()
            } else {
                "no matches".to_string()
            }
        } else {
            let current = self.selected + 1;
            let total = self.matching_indices.len();
            format!("{current} of {total} matches")
        };

        let full_line = Line::from(vec![
            Span::from(prefix).dim(),
            Span::from(separator.to_string()).dim(),
            Span::from(status).dim(),
        ]);
        truncate_line_to_width(full_line, max_width)
    }

    /// Render the visible candidate lines for the candidate list area.
    pub(crate) fn candidate_lines(&self, max_width: usize) -> Vec<Line<'static>> {
        if self.matching_indices.is_empty() {
            return Vec::new();
        }
        let visible_end =
            (self.scroll_offset + MAX_VISIBLE_CANDIDATES).min(self.matching_indices.len());
        let visible_range = self.scroll_offset..visible_end;

        visible_range
            .map(|i| {
                let entry_idx = self.matching_indices[i];
                let text = &self.entries[entry_idx].text;
                let is_selected = i == self.selected;
                let line = if is_selected {
                    Line::from(vec![Span::from("> ").cyan(), Span::from(text.clone())])
                } else {
                    Line::from(vec![Span::from("  "), Span::from(text.clone()).dim()])
                };
                truncate_line_to_width(line, max_width)
            })
            .collect()
    }

    /// Number of visible candidate lines (0 when no matches).
    pub(crate) fn candidate_list_height(&self) -> usize {
        self.matching_indices.len().min(MAX_VISIBLE_CANDIDATES)
    }

    /// The current match text, if any.
    #[cfg(test)]
    fn current_match(&self) -> Option<&str> {
        self.matched_text.as_deref()
    }

    /// The current query string.
    #[cfg(test)]
    fn query(&self) -> &str {
        &self.query
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Rebuild the full match list from scratch for the current query.
    fn rebuild_matches(&mut self) {
        self.matching_indices.clear();
        self.selected = 0;
        self.scroll_offset = 0;

        if self.query.is_empty() {
            self.matched_text = None;
            return;
        }

        let query_lower = self.query.to_lowercase();
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.text_lower.contains(&query_lower) {
                self.matching_indices.push(i);
            }
        }

        self.sync_matched_text();
    }

    /// Move selection to the next (older) match.
    fn select_next(&mut self) {
        if self.matching_indices.is_empty() {
            return;
        }
        if self.selected + 1 < self.matching_indices.len() {
            self.selected += 1;
            self.sync_scroll();
            self.sync_matched_text();
        }
    }

    /// Move selection to the previous (newer) match.
    fn select_prev(&mut self) {
        if self.matching_indices.is_empty() {
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
            self.sync_scroll();
            self.sync_matched_text();
        }
    }

    /// Ensure the scroll window contains `self.selected`.
    fn sync_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + MAX_VISIBLE_CANDIDATES {
            self.scroll_offset = self.selected + 1 - MAX_VISIBLE_CANDIDATES;
        }
    }

    /// Update `matched_text` from the current selection.
    fn sync_matched_text(&mut self) {
        if let Some(&entry_idx) = self.matching_indices.get(self.selected) {
            self.matched_text = Some(self.entries[entry_idx].text.clone());
        } else {
            self.matched_text = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn make_search(local: Vec<&str>) -> ReverseSearch {
        let texts: Vec<String> = local.iter().map(std::string::ToString::to_string).collect();
        ReverseSearch::new(None, texts)
    }

    #[test]
    fn basic_search_finds_match() {
        let mut search = make_search(vec!["hello world", "foo bar", "hello there"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("hello world"));
    }

    #[test]
    fn ctrl_r_cycles_matches() {
        let mut search = make_search(vec!["hello world", "foo bar", "hello there"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("hello world"));
        search.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(search.current_match(), Some("hello there"));
    }

    #[test]
    fn backspace_narrows_query() {
        let mut search = make_search(vec!["hello world", "help me"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("hello world"));
        search.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        search.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello world"));
    }

    #[test]
    fn case_insensitive_search() {
        let mut search = make_search(vec!["Hello World", "HELLO"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("Hello World"));
    }

    #[test]
    fn empty_query_no_match() {
        let search = make_search(vec!["hello"]);
        assert_eq!(search.current_match(), None);
    }

    #[test]
    fn deduplication() {
        let search = make_search(vec!["hello", "hello", "world"]);
        assert_eq!(search.entries.len(), 2);
    }

    #[test]
    fn accept_returns_match() {
        let mut search = make_search(vec!["hello world"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let action = search.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match action {
            ReverseSearchAction::Accept(text) => assert_eq!(text, "hello world"),
            _ => panic!("expected Accept"),
        }
    }

    #[test]
    fn cancel_on_esc() {
        let mut search = make_search(vec!["hello"]);
        let action = search.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(action, ReverseSearchAction::Cancel));
    }

    #[test]
    fn prompt_line_renders() {
        let mut search = make_search(vec!["hello world"]);
        for ch in "hel".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let line = search.search_prompt_line(80);
        assert_eq!(line.spans.len(), 3);
        assert_eq!(search.query(), "hel");
    }

    #[test]
    fn prompt_line_no_panic_on_multibyte_utf8() {
        let mut search = make_search(vec!["foo \u{2013} bar \u{1F600} baz"]);
        for ch in "foo".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(
            search.current_match(),
            Some("foo \u{2013} bar \u{1F600} baz")
        );
        let line = search.search_prompt_line(30);
        assert!(!line.spans.is_empty());
        let line = search.search_prompt_line(0);
        assert!(line.spans.is_empty());
    }

    #[test]
    fn multiple_candidates_visible() {
        let mut search = make_search(vec!["hello world", "hello there", "hello again", "foo bar"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.matching_indices.len(), 3);
        assert_eq!(search.candidate_list_height(), 3);
        let lines = search.candidate_lines(80);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].spans[0].content.as_ref(), "> ");
        assert_eq!(lines[1].spans[0].content.as_ref(), "  ");
        assert_eq!(lines[2].spans[0].content.as_ref(), "  ");
    }

    #[test]
    fn up_down_arrow_navigation() {
        let mut search = make_search(vec!["hello world", "hello there", "hello again"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("hello world"));
        search.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello there"));
        search.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello again"));
        search.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello there"));
        search.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello world"));
        search.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(search.current_match(), Some("hello world"));
    }

    #[test]
    fn prompt_shows_match_count() {
        let mut search = make_search(vec!["hello world", "hello there", "foo bar"]);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let line = search.search_prompt_line(80);
        assert_eq!(line.spans[2].content.as_ref(), "1 of 2 matches");
        search.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let line = search.search_prompt_line(80);
        assert_eq!(line.spans[2].content.as_ref(), "2 of 2 matches");
    }

    #[test]
    fn prompt_shows_no_matches() {
        let mut search = make_search(vec!["hello world"]);
        for ch in "zzz".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let line = search.search_prompt_line(80);
        assert_eq!(line.spans[2].content.as_ref(), "no matches");
    }

    #[test]
    fn scroll_window_follows_selection() {
        let entries = vec![
            "hello 0", "hello 1", "hello 2", "hello 3", "hello 4", "hello 5", "hello 6", "hello 7",
            "hello 8", "hello 9",
        ];
        let mut search = make_search(entries);
        for ch in "hello".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.matching_indices.len(), 10);
        assert_eq!(search.candidate_list_height(), MAX_VISIBLE_CANDIDATES);
        for _ in 0..7 {
            search.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(search.selected, 7);
        assert!(search.scroll_offset > 0);
        let lines = search.candidate_lines(80);
        assert_eq!(lines.len(), MAX_VISIBLE_CANDIDATES);
    }
}
