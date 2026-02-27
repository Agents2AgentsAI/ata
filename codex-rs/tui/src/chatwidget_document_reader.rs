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

    /// Returns `true` when the document reader view is the active bottom pane view.
    pub(crate) fn is_document_reader_active(&self) -> bool {
        self.bottom_pane.is_document_reader_active()
    }

    /// Whether the reading view feature is enabled.
    fn is_reading_view_enabled(&self) -> bool {
        self.config.features.enabled(Feature::ReadingView)
    }

    fn on_present_document(
        &mut self,
        ev: codex_protocol::document_reader::PresentDocumentEvent,
        from_replay: bool,
    ) {
        if !self.is_reading_view_enabled() {
            // Reading view disabled — render the document as inline chat markdown.
            self.flush_active_cell();
            let markdown = format!("# {}\n\n{}", ev.title, ev.content);
            self.handle_streaming_delta(markdown);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }
        self.flush_active_cell();
        self.bottom_pane.show_document_reader(ev, from_replay);
    }

    fn on_update_document_section(
        &mut self,
        ev: codex_protocol::document_reader::UpdateDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            // Render section update as inline chat content.
            self.flush_active_cell();
            self.handle_streaming_delta(ev.content);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }
        self.bottom_pane.update_document_section(&ev);
    }

    fn on_append_document_section(
        &mut self,
        ev: codex_protocol::document_reader::AppendDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            self.flush_active_cell();
            self.handle_streaming_delta(ev.content);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }
        self.bottom_pane.append_document_section(&ev);
    }

    fn on_patch_document_section(
        &mut self,
        ev: codex_protocol::document_reader::PatchDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            // For patches, show the new text inline.
            self.flush_active_cell();
            self.handle_streaming_delta(ev.new_text);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }
        self.bottom_pane.patch_document_section(&ev);
    }
}
