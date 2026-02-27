// Reverse-search integration for ChatComposer.
//
// This file is included into `chat_composer.rs` via `include!` to keep
// fork-specific reverse-search code in a separate file, reducing merge
// conflicts when syncing with upstream.

impl ChatComposer {
    /// Set the path to `history.jsonl` for reverse search.
    pub(crate) fn set_history_path(&mut self, path: PathBuf) {
        self.history_path = Some(path);
    }

    /// Check whether the reverse search is active.
    pub(crate) fn is_reverse_search_active(&self) -> bool {
        self.reverse_search.is_some()
    }

    /// Try to handle a key event as a reverse-search action.
    ///
    /// Returns `Some((result, redraw))` if the key was consumed (either by the
    /// active search session or by `Ctrl+R` to activate one), `None` otherwise.
    pub(crate) fn try_reverse_search_key(
        &mut self,
        key_event: KeyEvent,
    ) -> Option<(InputResult, bool)> {
        // Route to active session.
        if self.reverse_search.is_some() {
            return Some(self.handle_reverse_search_key(key_event));
        }
        // Ctrl+R activates reverse search.
        if matches!(
            key_event,
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            }
        ) {
            self.activate_reverse_search();
            return Some((InputResult::None, true));
        }
        None
    }

    /// Try to render the reverse-search prompt in the footer area.
    ///
    /// Returns `true` if reverse search is active and rendered, `false`
    /// otherwise (caller should proceed with normal footer rendering).
    pub(crate) fn try_render_reverse_search_footer(
        &self,
        hint_rect: Rect,
        buf: &mut Buffer,
    ) -> bool {
        if let Some(search) = &self.reverse_search {
            let max_w = hint_rect.width.saturating_sub(FOOTER_INDENT_COLS as u16) as usize;
            search
                .search_prompt_line(max_w)
                .render(inset_footer_hint_area(hint_rect), buf);
            true
        } else {
            false
        }
    }

    /// Activate reverse incremental search (`Ctrl+R`).
    fn activate_reverse_search(&mut self) {
        use super::reverse_search::ReverseSearch;
        self.pre_search_text = Some(self.textarea.text().to_string());
        let local_texts = self.history.local_history_texts_newest_first();
        self.reverse_search = Some(ReverseSearch::new(self.history_path.as_ref(), local_texts));
    }

    /// Route a key event to the active reverse search session.
    fn handle_reverse_search_key(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        use super::reverse_search::ReverseSearchAction;
        let Some(search) = self.reverse_search.as_mut() else {
            return (InputResult::None, false);
        };
        match search.handle_key_event(key_event) {
            ReverseSearchAction::Continue => (InputResult::None, true),
            ReverseSearchAction::Accept(text) => {
                self.reverse_search = None;
                self.pre_search_text = None;
                self.textarea.set_text_clearing_elements(&text);
                self.move_cursor_to_end();
                (InputResult::None, true)
            }
            ReverseSearchAction::Cancel => {
                let restore = self.pre_search_text.take().unwrap_or_default();
                self.reverse_search = None;
                self.textarea.set_text_clearing_elements(&restore);
                self.move_cursor_to_end();
                (InputResult::None, true)
            }
        }
    }
}
