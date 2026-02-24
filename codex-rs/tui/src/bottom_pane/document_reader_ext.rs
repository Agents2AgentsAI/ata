// Document reader forwarding methods for BottomPane.
//
// This file is included into `bottom_pane/mod.rs` via `include!` to keep
// fork-specific document reader code in a separate file, reducing merge
// conflicts when syncing with upstream.

impl BottomPane {
    /// Show the sectioned document reader for a `PresentDocument` event.
    pub(crate) fn show_document_reader(
        &mut self,
        ev: codex_protocol::document_reader::PresentDocumentEvent,
    ) {
        // If the active view is already a document reader for the same document,
        // don't push a new view (which would reset navigation to section 0).
        if let Some(view) = self.view_stack.last()
            && view.view_id() == Some(document_reader::DOCUMENT_READER_VIEW_ID)
        {
            return;
        }
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
}
