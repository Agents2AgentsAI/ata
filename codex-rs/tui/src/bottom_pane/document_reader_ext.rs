// Document reader forwarding methods for BottomPane.
//
// This file is included into `bottom_pane/mod.rs` via `include!` to keep
// fork-specific document reader code in a separate file, reducing merge
// conflicts when syncing with upstream.

impl BottomPane {
    /// Show the sectioned document reader for a `PresentDocument` event.
    ///
    /// When `from_replay` is true, skip documents the user already closed
    /// (replayed events after an agent switch). Live events always open the
    /// reader — the agent is deliberately re-presenting a document — and
    /// clear the document from the closed set so future replays also work.
    pub(crate) fn show_document_reader(
        &mut self,
        ev: codex_protocol::document_reader::PresentDocumentEvent,
        from_replay: bool,
    ) {
        if from_replay && self.closed_document_ids.contains(&ev.document_id) {
            return;
        }
        // If the active view is already a document reader for the same document,
        // don't push a new view (which would reset navigation to section 0).
        if let Some(view) = self.view_stack.last()
            && view.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID)
        {
            return;
        }
        // Live re-presentation of a previously-closed document: clear the
        // closed marker so the reader can be opened again.
        self.closed_document_ids.remove(&ev.document_id);
        let view = document_reader::DocumentReaderView::new(
            ev.document_id,
            ev.title,
            ev.content,
            self.app_event_tx.clone(),
            self.animations_enabled,
            self.frame_requester.clone(),
        );
        self.push_view(Box::new(view));
    }

    /// Forward a section update to the active document reader (if matching).
    pub(crate) fn update_document_section(
        &mut self,
        ev: &codex_protocol::document_reader::UpdateDocumentSectionEvent,
    ) {
        if let Some(view) = self.view_stack.last_mut()
            && view.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID)
        {
            // We know this is a DocumentReaderView because of the view_id.
            // Use the trait method to forward the update.
            view.handle_document_section_update(
                &ev.document_id,
                ev.section_index,
                ev.content.clone(),
            );
            self.request_redraw();
        }
    }

    /// Forward a section append to the active document reader (if matching).
    pub(crate) fn append_document_section(
        &mut self,
        ev: &codex_protocol::document_reader::AppendDocumentSectionEvent,
    ) {
        if let Some(view) = self.view_stack.last_mut()
            && view.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID)
        {
            view.handle_document_section_append(
                &ev.document_id,
                ev.section_index,
                ev.content.clone(),
                ev.foldable,
                ev.summary.clone(),
            );
            self.request_redraw();
        }
    }

    /// Forward a section patch (find-and-replace) to the active document reader.
    pub(crate) fn patch_document_section(
        &mut self,
        ev: &codex_protocol::document_reader::PatchDocumentSectionEvent,
    ) {
        if let Some(view) = self.view_stack.last_mut()
            && view.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID)
        {
            view.handle_document_section_patch(
                &ev.document_id,
                ev.section_index,
                &ev.old_text,
                &ev.new_text,
                ev.foldable,
                ev.summary.clone(),
            );
            self.request_redraw();
        }
    }

    /// Returns `true` when the document reader is the active bottom pane view.
    ///
    /// Used by `ChatWidget` to suppress agent message streaming into the chat
    /// history while the reader is open — responses should go through
    /// `update_document_section` into the card instead.
    pub(crate) fn is_document_reader_active(&self) -> bool {
        self.view_stack
            .last()
            .is_some_and(|v| v.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID))
    }

    /// Notify the active view that the agent turn has completed.
    ///
    /// Views that wait for tool calls (e.g. document reader waiting for
    /// `update_document_section`) use this to clear stale "waiting" state.
    pub(crate) fn notify_turn_complete(&mut self) {
        if let Some(view) = self.view_stack.last_mut() {
            view.handle_turn_complete();
            self.request_redraw();
        }
    }

    /// Return reading view context for voice mode integration.
    ///
    /// When the active view is a document reader, this extracts the current
    /// section context so voice transcriptions can be routed with
    /// reading-view-aware instructions.
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn reading_view_voice_context(
        &self,
    ) -> Option<bottom_pane_view::ReadingViewVoiceContext> {
        self.view_stack.last().and_then(|v| v.voice_context())
    }

    /// Returns `true` when the active view's embedded composer has keyboard focus.
    ///
    /// Used by voice mode to skip PTT interception so Space types into the
    /// composer immediately instead of starting voice recording.
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn is_view_composer_focused(&self) -> bool {
        self.view_stack
            .last()
            .is_some_and(|v| v.is_composer_focused())
    }

    /// Update the voice mode status text in the active document reader.
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn set_document_reader_voice_status(&mut self, status: Option<String>) {
        if let Some(view) = self.view_stack.last_mut() {
            view.set_voice_status(status);
            self.request_redraw();
        }
    }

    /// Mark a section as pending a voice question answer (same inline
    /// indicator as text questions).
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn set_document_reader_pending_voice_question(
        &mut self,
        section: usize,
        question: String,
    ) {
        if let Some(view) = self.view_stack.last_mut() {
            view.set_pending_voice_question(section, question);
            self.request_redraw();
        }
    }

    /// Push karaoke-highlighted lines into the active document reader's content area.
    /// When `append` is true, lines are shown after the existing content (Q&A mode).
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn set_document_reader_karaoke_lines(
        &mut self,
        lines: Option<Vec<ratatui::text::Line<'static>>>,
        append: bool,
    ) {
        if let Some(view) = self.view_stack.last_mut() {
            view.set_voice_karaoke_lines(lines, append);
            self.request_redraw();
        }
    }

    /// Set the reading cursor in the active document reader by word index.
    ///
    /// During narration this highlights the rendered line containing the
    /// given word, preserving full markdown formatting.
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    pub(crate) fn set_document_reader_reading_progress(
        &mut self,
        word_idx: Option<usize>,
        heading_words_to_skip: usize,
    ) {
        if let Some(view) = self.view_stack.last_mut() {
            view.set_voice_reading_progress(word_idx, heading_words_to_skip);
            self.request_redraw();
        }
    }
}
