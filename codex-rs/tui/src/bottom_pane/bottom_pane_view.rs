use crate::bottom_pane::ApprovalRequest;
use crate::render::renderable::Renderable;
use codex_protocol::request_user_input::RequestUserInputEvent;
use crossterm::event::KeyEvent;

use super::CancellationEvent;

/// Context extracted from a reading view for voice mode integration.
///
/// When voice mode is active and the user is in a document reader, this struct
/// carries the current reading context so the agent can provide reading-view-aware
/// explanations rather than generic voice responses.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(all(not(target_os = "linux"), feature = "voice-input")),
    allow(dead_code)
)]
pub(crate) struct ReadingViewVoiceContext {
    pub(crate) title: String,
    pub(crate) document_id: String,
    pub(crate) section_index: usize,
    pub(crate) heading: String,
    pub(crate) selection: Option<String>,
}

/// Trait implemented by every view that can be shown in the bottom pane.
pub(crate) trait BottomPaneView: Renderable {
    /// Handle a key event while the view is active. A redraw is always
    /// scheduled after this call.
    fn handle_key_event(&mut self, _key_event: KeyEvent) {}

    /// Return `true` if the view has finished and should be removed.
    fn is_complete(&self) -> bool {
        false
    }

    /// Stable identifier for views that need external refreshes while open.
    fn view_id(&self) -> Option<&'static str> {
        None
    }

    /// Handle Ctrl-C while this view is active.
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        CancellationEvent::NotHandled
    }

    /// Return true if Esc should be routed through `handle_key_event` instead
    /// of the `on_ctrl_c` cancellation path.
    fn prefer_esc_to_handle_key_event(&self) -> bool {
        false
    }

    /// Optional paste handler. Return true if the view modified its state and
    /// needs a redraw.
    fn handle_paste(&mut self, _pasted: String) -> bool {
        false
    }

    /// Flush any pending paste-burst state. Return true if state changed.
    ///
    /// This lets a modal that reuses `ChatComposer` participate in the same
    /// time-based paste burst flushing as the primary composer.
    fn flush_paste_burst_if_due(&mut self) -> bool {
        false
    }

    /// Whether the view is currently holding paste-burst transient state.
    ///
    /// When `true`, the bottom pane will schedule a short delayed redraw to
    /// give the burst time window a chance to flush.
    fn is_in_paste_burst(&self) -> bool {
        false
    }

    /// Try to handle approval request; return the original value if not
    /// consumed.
    fn try_consume_approval_request(
        &mut self,
        request: ApprovalRequest,
    ) -> Option<ApprovalRequest> {
        Some(request)
    }

    /// Try to handle request_user_input; return the original value if not
    /// consumed.
    fn try_consume_user_input_request(
        &mut self,
        request: RequestUserInputEvent,
    ) -> Option<RequestUserInputEvent> {
        Some(request)
    }

    /// Forward a document section update (full replace) to this view (no-op by default).
    #[allow(dead_code)]
    fn handle_document_section_update(
        &mut self,
        _document_id: &str,
        _section_index: usize,
        _content: String,
    ) {
    }

    /// Forward a document section append to this view (no-op by default).
    #[allow(dead_code)]
    fn handle_document_section_append(
        &mut self,
        _document_id: &str,
        _section_index: usize,
        _content: String,
        _foldable: bool,
        _summary: Option<String>,
    ) {
    }

    /// Forward a new document section insertion to this view (no-op by default).
    #[allow(dead_code)]
    fn handle_document_section_add(
        &mut self,
        _document_id: &str,
        _after_section_index: i32,
        _heading: String,
        _content: String,
        _foldable: bool,
        _summary: Option<String>,
    ) {
    }

    /// Forward a document section patch (find-and-replace) to this view (no-op by default).
    #[allow(dead_code)]
    fn handle_document_section_patch(
        &mut self,
        _document_id: &str,
        _section_index: usize,
        _old_text: &str,
        _new_text: &str,
        _foldable: bool,
        _summary: Option<String>,
    ) {
    }

    /// Notify this view that the agent turn has completed. Views that wait for
    /// tool calls (e.g. document reader waiting for `update_document_section`)
    /// should use this to clear any "waiting" state.
    #[allow(dead_code)]
    fn handle_turn_complete(&mut self) {}

    /// Return the document ID if this view is a document reader that was closed.
    ///
    /// Used by `BottomPane` to track which documents have been dismissed so
    /// that replayed `PresentDocument` events (e.g. after an agent switch) do
    /// not re-open a reader the user already closed.
    fn closed_document_id(&self) -> Option<&str> {
        None
    }

    // ─── Voice-mode-only trait methods ─────────────────────────────────
    // These exist only when voice-input is available (non-Linux + feature).

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn voice_context(&self) -> Option<ReadingViewVoiceContext> {
        None
    }

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn set_voice_status(&mut self, _status: Option<String>) {}

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn set_voice_tts_paused(&mut self, _paused: bool) {}

    #[cfg(all(test, not(target_os = "linux"), feature = "voice-input"))]
    fn voice_tts_paused(&self) -> bool {
        false
    }

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn set_pending_voice_question(&mut self, _section: usize, _question: String) {}

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn set_voice_karaoke_lines(
        &mut self,
        _lines: Option<Vec<ratatui::text::Line<'static>>>,
        _append: bool,
    ) {
    }

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn set_voice_reading_progress(
        &mut self,
        _word_idx: Option<usize>,
        _heading_words_to_skip: usize,
    ) {
    }

    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    fn is_composer_focused(&self) -> bool {
        false
    }
}
