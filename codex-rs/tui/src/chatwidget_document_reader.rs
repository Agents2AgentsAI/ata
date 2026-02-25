// Document reader integration for ChatWidget.
//
// This file is included into `chatwidget.rs` via `include!` to keep
// fork-specific document reader code in a separate file, reducing merge
// conflicts when syncing with upstream.

impl ChatWidget {
    /// Returns `true` when the document reader is active and streaming output
    /// (agent messages, reasoning) should be suppressed from the chat history.
    fn is_suppressing_streaming_for_reader(&self) -> bool {
        self.bottom_pane.is_document_reader_active()
    }

    fn on_present_document(
        &mut self,
        ev: codex_protocol::document_reader::PresentDocumentEvent,
        from_replay: bool,
    ) {
        self.flush_active_cell();
        self.bottom_pane.show_document_reader(ev, from_replay);
    }

    fn on_update_document_section(
        &mut self,
        ev: codex_protocol::document_reader::UpdateDocumentSectionEvent,
    ) {
        self.bottom_pane.update_document_section(&ev);
    }

    fn on_append_document_section(
        &mut self,
        ev: codex_protocol::document_reader::AppendDocumentSectionEvent,
    ) {
        self.bottom_pane.append_document_section(&ev);
    }

    fn on_patch_document_section(
        &mut self,
        ev: codex_protocol::document_reader::PatchDocumentSectionEvent,
    ) {
        self.bottom_pane.patch_document_section(&ev);
    }
}
