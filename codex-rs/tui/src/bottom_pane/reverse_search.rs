//! Bash-style `Ctrl+R` reverse incremental search over prompt history.
//!
//! This module is self-contained: it reads the `history.jsonl` file once on
//! activation, merges local session history, and exposes a key-event handler
//! plus a ratatui `Line` renderer for the search prompt.

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

/// A history entry ready for case-insensitive search.
struct SearchableEntry {
    text: String,
    text_lower: String,
}

/// Result of processing a key event during reverse search.
pub(crate) enum ReverseSearchAction {
    /// Search continues — caller should redraw.
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
    /// Index into `entries` of the current match (newest-first).
    match_index: Option<usize>,
    /// Cached text of the current match for display.
    matched_text: Option<String>,
}

/// Minimal deserialisation target — we only need the `text` field.
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
            match_index: None,
            matched_text: None,
        }
    }

    /// Process a key event during reverse search.
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) -> ReverseSearchAction {
        if key.kind != KeyEventKind::Press {
            return ReverseSearchAction::Continue;
        }

        match key {
            // Ctrl+R — cycle to next older match
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.search_next();
                ReverseSearchAction::Continue
            }
            // Enter — accept current match
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                let text = self.matched_text.clone().unwrap_or_default();
                ReverseSearchAction::Accept(text)
            }
            // Esc / Ctrl+C / Ctrl+G — cancel
            KeyEvent {
                code: KeyCode::Esc, ..
            }
            | KeyEvent {
                code: KeyCode::Char('c') | KeyCode::Char('g'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => ReverseSearchAction::Cancel,
            // Backspace — remove last query char
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.query.pop();
                self.search_from_start();
                ReverseSearchAction::Continue
            }
            // Printable char (no Ctrl/Alt)
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.query.push(ch);
                self.search_from_start();
                ReverseSearchAction::Continue
            }
            _ => ReverseSearchAction::Continue,
        }
    }

    /// Render the search prompt line: `(reverse-i-search)'query': preview`
    pub(crate) fn search_prompt_line(&self, max_width: usize) -> Line<'static> {
        let prefix = format!("(reverse-i-search)`{}'", self.query);
        let separator = ": ";
        let preview = self.matched_text.as_deref().unwrap_or("");

        let full_line = Line::from(vec![
            Span::from(prefix).dim(),
            Span::from(separator.to_string()).dim(),
            Span::from(preview.to_string()),
        ]);
        truncate_line_to_width(full_line, max_width)
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

    /// Search from the beginning (newest entry).
    fn search_from_start(&mut self) {
        if self.query.is_empty() {
            self.match_index = None;
            self.matched_text = None;
            return;
        }
        let query_lower = self.query.to_lowercase();
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.text_lower.contains(&query_lower) {
                self.match_index = Some(i);
                self.matched_text = Some(entry.text.clone());
                return;
            }
        }
        self.match_index = None;
        self.matched_text = None;
    }

    /// Search for the next older match after the current one.
    fn search_next(&mut self) {
        if self.query.is_empty() {
            return;
        }
        let start = self.match_index.map(|i| i + 1).unwrap_or(0);
        let query_lower = self.query.to_lowercase();
        for (i, entry) in self.entries.iter().enumerate().skip(start) {
            if entry.text_lower.contains(&query_lower) {
                self.match_index = Some(i);
                self.matched_text = Some(entry.text.clone());
                return;
            }
        }
        // No more matches — keep current.
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
        // Type 'hello' — should match newest first ("hello world")
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
        // Ctrl+R cycles to next older match
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
        // Backspace twice: query becomes "hel" — matches both, newest first
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
        // Just verify it has 3 spans
        assert_eq!(line.spans.len(), 3);
        assert_eq!(search.query(), "hel");
    }

    #[test]
    fn prompt_line_no_panic_on_multibyte_utf8() {
        // En-dash (U+2013) is 3 bytes in UTF-8 but 1 column wide.
        // Emoji (U+1F600) is 4 bytes in UTF-8 and 2 columns wide.
        // The old code sliced at byte offsets, panicking on multi-byte chars.
        let mut search = make_search(vec!["foo – bar 😀 baz"]);
        for ch in "foo".chars() {
            search.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(search.current_match(), Some("foo – bar 😀 baz"));

        // Use a narrow width that forces truncation mid-preview (inside multi-byte territory).
        // Prefix "(reverse-i-search)`foo'" = 24 chars + ": " = 26 display cols.
        // Width 30 leaves only 4 cols for the preview — must truncate safely.
        let line = search.search_prompt_line(30);
        assert!(!line.spans.is_empty());

        // Width 0 should not panic.
        let line = search.search_prompt_line(0);
        assert!(line.spans.is_empty());
    }
}
