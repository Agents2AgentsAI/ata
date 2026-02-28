//! Voice mode state machine and TTS sentence buffer.
//!
//! Data flow (push-to-talk):
//! ```text
//! INPUT:  Space hold → VoiceCapture → Space release → WAV → STT → text → agent
//! OUTPUT: AgentMessageDelta → VoiceTagParser → <voice> content → TTS → PCM → speaker
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_core::config::types::VoiceModeToml;
use codex_core::config::types::VoiceOutput;

// ─── Voice mode instruction prefix ──────────────────────────────────────────

/// Instruction prepended to voice transcriptions so the agent wraps spoken
/// content in `<voice>` tags — only tagged text goes to TTS while everything
/// else is displayed as regular chat output.
pub(crate) const VOICE_MODE_INSTRUCTION: &str = "\
[VOICE MODE] The user is speaking to you via voice. \
Wrap any text you want spoken aloud in <voice></voice> tags. \
Never put code, file paths, or markdown in <voice> tags — only natural, \
conversational text.\n\
\n\
Follow this pattern:\n\
1. Start with a brief <voice> acknowledgment (1-2 sentences).\n\
2. Do any technical work (tool calls, code, file listings) without voice tags.\n\
3. End with a <voice> summary of what you found or did (2-3 sentences).\n\
\n\
For multi-step tasks, add brief <voice> progress updates between steps \
so the user knows what is happening (e.g. \"Found the issue, fixing it now.\" \
or \"Checking a few more files.\"). Keep these to one sentence.\n\
\n\
For purely conversational responses with no code or tools, wrap the entire \
response in <voice> tags.\n\n";


// ─── Voice mode phase ────────────────────────────────────────────────────────

/// Phases of the voice mode state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VoiceModePhase {
    /// Voice mode is off.
    Off,
    /// Waiting for user to hold Space to speak.
    Idle,
    /// User is holding Space — recording audio.
    Recording,
    /// Audio sent to STT — waiting for transcription.
    Transcribing,
    /// Agent turn complete, TTS is playing the response.
    Speaking,
}

impl VoiceModePhase {
    pub(crate) fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub(crate) fn status_label(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Idle => "\u{1F3A4}  Hold Space to speak",
            Self::Recording => "\u{1F534}  Recording...",
            Self::Transcribing => "\u{23F3}  Transcribing...",
            Self::Speaking => "\u{1F50A}  Speaking...",
        }
    }
}

// ─── Sentence buffer ─────────────────────────────────────────────────────────

/// Accumulates streaming text deltas and splits on sentence boundaries
/// so TTS can start speaking before the full response is available.
pub(crate) struct SentenceBuffer {
    buffer: String,
}

#[allow(dead_code)]
impl SentenceBuffer {
    pub(crate) fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Push a text delta. Returns completed sentences ready for TTS.
    pub(crate) fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut sentences = Vec::new();

        loop {
            if let Some(split_pos) = self.find_sentence_boundary() {
                let sentence: String = self.buffer[..split_pos].trim().to_string();
                self.buffer = self.buffer[split_pos..].trim_start().to_string();
                if !sentence.is_empty() {
                    sentences.push(sentence);
                }
            } else {
                break;
            }
        }
        sentences
    }

    /// Flush remaining text as a final sentence.
    pub(crate) fn flush(&mut self) -> Option<String> {
        let text = self.buffer.trim().to_string();
        self.buffer.clear();
        if text.is_empty() { None } else { Some(text) }
    }

    /// Clear without returning.
    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
    }

    fn find_sentence_boundary(&self) -> Option<usize> {
        for (i, ch) in self.buffer.char_indices() {
            if matches!(ch, '.' | '!' | '?') {
                let after = i + ch.len_utf8();
                if after < self.buffer.len() {
                    let next = self.buffer[after..].chars().next();
                    if matches!(next, Some(' ' | '\n')) {
                        return Some(after);
                    }
                }
            }
            // Double newline is also a sentence boundary.
            if ch == '\n' && self.buffer[i + 1..].starts_with('\n') {
                return Some(i);
            }
        }
        None
    }
}

// ─── Voice tag parser (streaming) ────────────────────────────────────────────

/// Result of pushing a delta through the voice tag parser.
pub(crate) struct VoiceParseResult {
    /// Text to display in chat (all content with `<voice>`/`</voice>` tags stripped).
    pub display_text: String,
    /// Complete sentences extracted from `<voice>` regions, ready for TTS.
    pub voice_sentences: Vec<String>,
}

/// Streaming parser that separates `<voice>`-tagged content (for TTS) from
/// the rest (display-only). Handles tags split across multiple deltas.
pub(crate) struct VoiceTagParser {
    /// Pending characters that might be part of an incomplete tag.
    pending: String,
    /// Whether we're currently inside a `<voice>` region.
    in_voice: bool,
    /// Accumulates voice-tagged text for sentence splitting.
    voice_buffer: String,
}

impl VoiceTagParser {
    pub(crate) fn new() -> Self {
        Self {
            pending: String::new(),
            in_voice: false,
            voice_buffer: String::new(),
        }
    }

    /// Push a streaming delta. Returns display text (tags stripped) and any
    /// complete voice sentences ready for TTS.
    pub(crate) fn push(&mut self, delta: &str) -> VoiceParseResult {
        self.pending.push_str(delta);

        let mut display = String::new();
        let mut voice_sentences = Vec::new();

        loop {
            if let Some(tag_start) = self.pending.find('<') {
                // Emit everything before the `<`.
                let before = &self.pending[..tag_start];
                if !before.is_empty() {
                    display.push_str(before);
                    if self.in_voice {
                        self.voice_buffer.push_str(before);
                    }
                }

                let rest = &self.pending[tag_start..];

                // Try to match a complete tag.
                if let Some(tag_end) = rest.find('>') {
                    let tag = &rest[..=tag_end];
                    let tag_lower = tag.to_ascii_lowercase();

                    if tag_lower == "<voice>" {
                        self.in_voice = true;
                        // Strip the tag from display output; prefix voice
                        // content with a speaker icon so the user can tell
                        // which parts were spoken aloud.
                        display.push_str("🔊 ");
                        self.pending = rest[tag_end + 1..].to_string();
                    } else if tag_lower == "</voice>" {
                        self.in_voice = false;
                        // Closing tag ends a spoken region — flush the voice
                        // buffer as a complete sentence for TTS.
                        let text = self.voice_buffer.trim().to_string();
                        if !text.is_empty() {
                            voice_sentences.push(text);
                        }
                        self.voice_buffer.clear();
                        // Strip the tag from display output.
                        self.pending = rest[tag_end + 1..].to_string();
                    } else {
                        // Not a voice tag — emit it as regular text.
                        display.push_str(tag);
                        if self.in_voice {
                            self.voice_buffer.push_str(tag);
                        }
                        self.pending = rest[tag_end + 1..].to_string();
                    }
                } else {
                    // `<` found but no closing `>` yet.
                    // Check if this could be the start of a voice tag.
                    if is_voice_tag_prefix(rest) {
                        // Buffer it — wait for more data.
                        self.pending = rest.to_string();
                        break;
                    } else {
                        // Not a voice tag prefix (e.g., "< 5") — flush as text.
                        let ch = &rest[..1];
                        display.push_str(ch);
                        if self.in_voice {
                            self.voice_buffer.push_str(ch);
                        }
                        self.pending = rest[1..].to_string();
                    }
                }
            } else {
                // No `<` — emit everything.
                if !self.pending.is_empty() {
                    display.push_str(&self.pending);
                    if self.in_voice {
                        self.voice_buffer.push_str(&self.pending);
                    }
                    self.pending.clear();
                }
                break;
            }
        }

        // Extract complete sentences from voice_buffer.
        self.extract_sentences(&mut voice_sentences);

        VoiceParseResult {
            display_text: display,
            voice_sentences,
        }
    }

    /// Flush remaining voice buffer content as a final sentence (on turn complete).
    pub(crate) fn flush(&mut self) -> Option<String> {
        // Flush any pending text first.
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            if self.in_voice {
                self.voice_buffer.push_str(&pending);
            }
        }

        let text = self.voice_buffer.trim().to_string();
        self.voice_buffer.clear();
        self.in_voice = false;
        if text.is_empty() { None } else { Some(text) }
    }

    /// Clear all state (for interruption/barge-in).
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.voice_buffer.clear();
        self.in_voice = false;
    }

    /// Extract complete sentences from voice_buffer into the output vec.
    fn extract_sentences(&mut self, out: &mut Vec<String>) {
        loop {
            if let Some(pos) = find_sentence_boundary(&self.voice_buffer) {
                let sentence = self.voice_buffer[..pos].trim().to_string();
                self.voice_buffer = self.voice_buffer[pos..].trim_start().to_string();
                if !sentence.is_empty() {
                    out.push(sentence);
                }
            } else {
                break;
            }
        }
    }
}

/// Check if `s` (starting with `<`) could be the prefix of `<voice>` or `</voice>`.
fn is_voice_tag_prefix(s: &str) -> bool {
    let s_lower: String = s.to_ascii_lowercase();
    "<voice>".starts_with(&s_lower) || "</voice>".starts_with(&s_lower)
}

/// Find a sentence boundary (`. `, `! `, `? `, or `\n\n`).
fn find_sentence_boundary(text: &str) -> Option<usize> {
    for (i, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let after = i + ch.len_utf8();
            if after < text.len() {
                let next = text[after..].chars().next();
                if matches!(next, Some(' ' | '\n')) {
                    return Some(after);
                }
            }
        }
        if ch == '\n' && text[i + 1..].starts_with('\n') {
            return Some(i);
        }
    }
    None
}

// ─── Voice mode UI state ─────────────────────────────────────────────────────

/// Minimum recording duration (ms) — recordings shorter than this are discarded
/// as accidental taps rather than intentional speech.
#[allow(dead_code)]
const MIN_RECORDING_MS: u64 = 600;

/// Holds all voice mode runtime state for a `ChatWidget`.
pub(crate) struct VoiceModeState {
    pub(crate) phase: VoiceModePhase,
    pub(crate) sentence_buffer: SentenceBuffer,
    pub(crate) voice_tag_parser: VoiceTagParser,
    pub(crate) output: VoiceOutput,
    pub(crate) auto_submit: bool,

    // Audio capture (when recording).
    pub(crate) capture: Option<crate::voice::VoiceCapture>,
    pub(crate) audio_player: Option<crate::voice::RealtimeAudioPlayer>,
    pub(crate) meter_state: Option<crate::voice::RecordingMeterState>,

    // TTS WebSocket handle (spawned async).
    pub(crate) tts_cancel: Option<tokio::sync::oneshot::Sender<()>>,

    /// When the current recording phase started (for min duration check).
    pub(crate) recording_started_at: Option<Instant>,

    /// Set on barge-in to suppress new TTS for the rest of the agent turn.
    /// Cleared when the next user message is submitted.
    pub(crate) tts_suppressed: bool,

    // ─── PTT (push-to-talk) fields ──────────────────────────────────────

    /// Set `true` on the first `KeyEventKind::Release` we receive.
    /// Terminals that don't support Release events will never set this,
    /// and we fall back to a timeout-based end-of-recording.
    pub(crate) key_release_supported: bool,

    /// Updated on every `KeyEventKind::Repeat` while recording. Used by the
    /// timeout poller to detect when repeats stop (key was released in a
    /// terminal that doesn't emit Release events).
    pub(crate) last_ptt_repeat_at: Option<Instant>,

    /// Cancel flag for the PTT timeout poller task.
    pub(crate) ptt_timeout_cancel: Option<Arc<AtomicBool>>,

    /// When Space was first pressed (pending state). Recording only starts
    /// when key repeats confirm a hold. On Release before recording, this
    /// is treated as a normal space tap.
    pub(crate) ptt_pending_at: Option<Instant>,

    /// Cancel flag for the PTT volume meter polling thread.
    pub(crate) ptt_meter_cancel: Option<Arc<AtomicBool>>,

    /// Ref-count of in-flight TTS tasks. `VoiceModeTtsFinished` is only
    /// sent when this drops to zero.
    pub(crate) tts_in_flight: Arc<AtomicUsize>,
}

impl VoiceModeState {
    pub(crate) fn new(config: &VoiceModeToml) -> Self {
        let output = config.output.unwrap_or_default();
        let auto_submit = config.auto_submit.unwrap_or(true);

        Self {
            phase: VoiceModePhase::Off,
            sentence_buffer: SentenceBuffer::new(),
            voice_tag_parser: VoiceTagParser::new(),
            output,
            auto_submit,
            capture: None,
            audio_player: None,
            meter_state: None,
            tts_cancel: None,
            recording_started_at: None,
            tts_suppressed: false,
            key_release_supported: false,
            last_ptt_repeat_at: None,
            ptt_timeout_cancel: None,
            ptt_pending_at: None,
            ptt_meter_cancel: None,
            tts_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.phase.is_active()
    }

    /// Should we send text deltas to TTS?
    pub(crate) fn should_tts(&self) -> bool {
        self.is_active()
            && !self.tts_suppressed
            && matches!(self.output, VoiceOutput::Voice | VoiceOutput::Both)
    }

    /// Should we suppress text streaming in the chat?
    #[allow(dead_code)]
    pub(crate) fn suppress_text(&self) -> bool {
        self.is_active() && matches!(self.output, VoiceOutput::Voice)
    }

    /// Stop TTS playback and clear buffers (for interruption).
    pub(crate) fn interrupt_tts(&mut self) {
        self.sentence_buffer.clear();
        self.voice_tag_parser.clear();
        self.tts_in_flight.store(0, Ordering::SeqCst);
        if let Some(cancel) = self.tts_cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(ref player) = self.audio_player {
            player.clear();
        }
    }

    /// Cancel the PTT timeout poller task.
    fn cancel_ptt_timeout_poller(&mut self) {
        if let Some(cancel) = self.ptt_timeout_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Cancel the PTT volume meter polling thread.
    fn cancel_ptt_meter(&mut self) {
        if let Some(cancel) = self.ptt_meter_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Full reset when toggling off.
    pub(crate) fn reset(&mut self) {
        self.interrupt_tts();
        self.cancel_ptt_timeout_poller();
        self.cancel_ptt_meter();
        self.tts_in_flight.store(0, Ordering::SeqCst);
        self.phase = VoiceModePhase::Off;
        self.capture = None;
        self.audio_player = None;
        self.meter_state = None;
        self.recording_started_at = None;
        self.last_ptt_repeat_at = None;
        self.ptt_pending_at = None;
    }
}

// ─── ChatWidget voice mode integration ───────────────────────────────────────

use crate::app_event::AppEvent;
use crate::history_cell;

/// Extract `VoiceModeToml` from the merged effective config (which is a raw `toml::Value`).
fn voice_mode_config(config: &codex_core::config::Config) -> VoiceModeToml {
    config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|t| t.get("voice_mode"))
        .and_then(|v| v.clone().try_into::<VoiceModeToml>().ok())
        .unwrap_or_default()
}

impl super::ChatWidget {
    /// Sync the composer placeholder text to reflect the current voice mode phase.
    /// Also syncs the voice status indicator in the reading view (if active).
    fn sync_voice_placeholder(&mut self) {
        let (label, phase) = match &self.voice_mode_state {
            Some(s) if s.phase.is_active() => (s.phase.status_label(), s.phase),
            _ => return,
        };
        self.bottom_pane
            .set_placeholder_text(label.to_string());
        // Also update the reading view's voice status indicator.
        let reading_status = if phase == VoiceModePhase::Idle {
            Some("Hold Space to ask".to_string())
        } else {
            Some(label.to_string())
        };
        self.bottom_pane
            .set_document_reader_voice_status(reading_status);
    }

    /// Restore the default placeholder text when voice mode turns off.
    fn restore_default_placeholder(&mut self) {
        use rand::Rng;
        let placeholders = super::PLACEHOLDERS;
        let idx = rand::rng().random_range(0..placeholders.len());
        self.bottom_pane
            .set_placeholder_text(placeholders[idx].to_string());
        // Clear reading view voice status.
        self.bottom_pane
            .set_document_reader_voice_status(None);
    }

    /// Toggle voice mode on/off (Ctrl+M or /voice).
    pub(crate) fn toggle_voice_mode(&mut self) {
        if !self.config.features.enabled(codex_core::features::Feature::VoiceMode) {
            self.add_info_message(
                "Voice mode is not enabled. Use /experimental to enable it.".to_string(),
                None,
            );
            return;
        }

        if let Some(ref mut state) = self.voice_mode_state {
            if state.is_active() {
                // Turn off.
                state.reset();
                self.app_event_tx
                    .send(AppEvent::PersistVoiceModeEnabled(false));
                self.add_info_message("Voice mode off.".to_string(), None);
                self.restore_default_placeholder();
                self.bottom_pane.set_force_hide_cursor(false);
                self.request_redraw();
                return;
            }
        }

        // Mutual exclusion: stop realtime mode if active.
        if self.realtime_conversation.is_live() {
            self.request_realtime_conversation_close(Some(
                "Stopped realtime mode to start voice mode.".to_string(),
            ));
        }

        // Initialize voice mode state from config.
        let voice_config = voice_mode_config(&self.config);

        let mut state = VoiceModeState::new(&voice_config);

        // Start audio player for TTS output.
        match crate::voice::RealtimeAudioPlayer::start() {
            Ok(player) => {
                state.audio_player = Some(player);
            }
            Err(e) => {
                tracing::error!("failed to start audio player: {e}");
                self.add_to_history(history_cell::new_error_event(format!(
                    "Failed to start audio: {e}"
                )));
                return;
            }
        }

        // PTT mode: don't start capture yet — it starts on Space press.
        state.phase = VoiceModePhase::Idle;
        self.voice_mode_state = Some(state);

        self.app_event_tx
            .send(AppEvent::PersistVoiceModeEnabled(true));

        self.add_info_message(
            "Voice mode on. Hold Space to speak. Ctrl+M to stop.".to_string(),
            None,
        );

        self.bottom_pane.set_force_hide_cursor(true);
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    // ─── Push-to-talk handlers ──────────────────────────────────────────

    /// Called on Space key press while voice mode is active.
    ///
    /// For terminals with key release support: start recording immediately
    /// on press. A quick release (<200ms) discards the recording and types
    /// a space instead.
    ///
    /// For terminals without release: enter a "pending" state — recording
    /// only starts once key repeats prove the user is *holding* Space.
    pub(crate) fn on_ptt_press(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if !state.is_active() {
            return;
        }

        // If agent is speaking, barge-in: interrupt TTS first.
        if state.phase == VoiceModePhase::Speaking {
            state.interrupt_tts();
            state.tts_suppressed = true;
            // Fall through to start recording / enter pending state.
        }

        // Already recording — treat repeated Press as a Repeat (terminals
        // without keyboard enhancement emit Press for every key repeat,
        // never Repeat or Release).
        if state.phase == VoiceModePhase::Recording {
            if !state.key_release_supported {
                state.last_ptt_repeat_at = Some(Instant::now());
            }
            return;
        }

        // Already pending (repeated Press before recording started) —
        // the key is being held, so transition to recording now.
        if state.ptt_pending_at.is_some() {
            let _ = state;
            self.start_ptt_recording();
            return;
        }

        // Only start from Idle or Speaking (barge-in).
        if state.phase != VoiceModePhase::Idle && state.phase != VoiceModePhase::Speaking {
            return;
        }

        // If the active view's embedded composer has focus (e.g. reading view
        // question input after pressing Tab), Space should type into the
        // composer — skip PTT entirely.
        if self.bottom_pane.is_view_composer_focused() {
            let _ = state;
            self.type_space_in_composer();
            return;
        }

        // Release-capable terminals: skip pending, start recording now.
        // Quick release (<200ms) will discard and type a space.
        if state.key_release_supported {
            let _ = state;
            self.start_ptt_recording();
            return;
        }

        // Non-release terminals: enter pending state — don't start recording yet.
        state.ptt_pending_at = Some(Instant::now());

        // Start a timeout poller so we can detect taps (no repeats
        // arrive) and release (repeats stop).
        let _ = state;
        self.start_ptt_timeout_poller();
    }

    /// Transition from pending to actual recording.
    fn start_ptt_recording(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        state.ptt_pending_at = None;

        // Start voice capture.
        let last_peak_arc;
        match crate::voice::VoiceCapture::start() {
            Ok(capture) => {
                last_peak_arc = capture.last_peak_arc();
                state.meter_state = Some(crate::voice::RecordingMeterState::new());
                state.capture = Some(capture);
            }
            Err(e) => {
                tracing::error!("failed to start voice capture: {e}");
                self.add_to_history(history_cell::new_error_event(format!(
                    "Failed to start mic: {e}"
                )));
                return;
            }
        }

        if let Some(ref mut state) = self.voice_mode_state {
            state.phase = VoiceModePhase::Recording;
            state.recording_started_at = Some(Instant::now());
            state.last_ptt_repeat_at = Some(Instant::now());
        }

        // Start the volume meter polling thread.
        self.start_ptt_meter(last_peak_arc);

        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Start a background thread that polls mic volume and sends meter ticks.
    fn start_ptt_meter(&mut self, last_peak: std::sync::Arc<std::sync::atomic::AtomicU16>) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // Cancel any previous meter thread.
        state.cancel_ptt_meter();

        let cancel = Arc::new(AtomicBool::new(false));
        state.ptt_meter_cancel = Some(cancel.clone());
        let tx = self.app_event_tx.clone();

        std::thread::spawn(move || {
            let mut meter = crate::voice::RecordingMeterState::new();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let text = meter.next_text(last_peak.load(Ordering::Relaxed));
                tx.send(AppEvent::VoiceModeMeterTick { text });
                std::thread::sleep(Duration::from_millis(80));
            }
        });
    }

    /// Called from the app event loop on `VoiceModeMeterTick`.
    pub(crate) fn on_voice_meter_tick(&mut self, text: String) {
        let Some(ref state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Recording {
            return;
        }
        self.bottom_pane
            .set_placeholder_text(format!("\u{25CF}  {text}  Recording..."));
    }

    /// Called on Space key release while voice mode is active.
    pub(crate) fn on_ptt_release(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // If still pending (never started recording) → it was a quick tap.
        // Type a normal space into the composer.
        if state.ptt_pending_at.is_some() {
            state.ptt_pending_at = None;
            state.cancel_ptt_timeout_poller();
            let _ = state;
            self.type_space_in_composer();
            return;
        }

        if state.phase != VoiceModePhase::Recording {
            return;
        }

        // Release-capable terminals: if held < 200ms, it was a quick tap.
        // Discard the recording and type a space instead.
        if state.key_release_supported {
            if let Some(started) = state.recording_started_at {
                if started.elapsed() < Duration::from_millis(200) {
                    // Discard recording — stop capture without transcribing.
                    let capture = state.capture.take();
                    if let Some(c) = capture {
                        let _ = c.stop();
                    }
                    state.phase = VoiceModePhase::Idle;
                    state.recording_started_at = None;
                    state.cancel_ptt_meter();
                    let _ = state;
                    self.sync_voice_placeholder();
                    self.type_space_in_composer();
                    return;
                }
            }
        }

        // Cancel timeout poller and meter.
        state.cancel_ptt_timeout_poller();
        state.cancel_ptt_meter();

        state.phase = VoiceModePhase::Transcribing;
        state.recording_started_at = None;
        state.last_ptt_repeat_at = None;

        // Stop capture and get audio on the current thread (VoiceCapture is !Send).
        let capture = state.capture.take();
        let Some(capture) = capture else {
            state.phase = VoiceModePhase::Idle;
            return;
        };

        let audio = match capture.stop() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("failed to stop capture: {e}");
                state.phase = VoiceModePhase::Idle;
                return;
            }
        };

        let wav_bytes = match crate::voice::encode_wav_for_voice_mode(&audio) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("WAV encode failed: {e}");
                state.phase = VoiceModePhase::Idle;
                return;
            }
        };

        let tx = self.app_event_tx.clone();
        let voice_config = voice_mode_config(&self.config);

        // Spawn STT transcription on a background thread (network I/O).
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tx.send(AppEvent::VoiceModeTranscriptionFailed {
                        error: format!("runtime error: {e}"),
                    });
                    return;
                }
            };

            // Resolve ElevenLabs API key.
            let api_key = voice_config
                .elevenlabs
                .as_ref()
                .and_then(|e| e.api_key.clone())
                .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok());

            let Some(api_key) = api_key else {
                tx.send(AppEvent::VoiceModeTranscriptionFailed {
                    error: "Missing ElevenLabs API key. Set ELEVENLABS_API_KEY or configure voice_mode.elevenlabs.api_key".to_string(),
                });
                return;
            };

            let mut config = codex_elevenlabs::ElevenLabsConfig::new(api_key);
            if let Some(ref el) = voice_config.elevenlabs {
                if let Some(ref vid) = el.voice_id {
                    config = config.with_voice_id(vid.clone());
                }
                if let Some(ref mid) = el.model_id {
                    config = config.with_model_id(mid.clone());
                }
            }

            let result = rt.block_on(codex_elevenlabs::stt::transcribe(&config, wav_bytes));
            match result {
                Ok(text) => {
                    tx.send(AppEvent::VoiceModeTranscriptionComplete { text });
                }
                Err(e) => {
                    tx.send(AppEvent::VoiceModeTranscriptionFailed {
                        error: format!("{e}"),
                    });
                }
            }
        });

        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Called on Space key repeat while voice mode is active.
    pub(crate) fn on_ptt_repeat(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // If pending → key is being held. Start recording.
        if state.ptt_pending_at.is_some() {
            let _ = state;
            self.start_ptt_recording();
            return;
        }

        // Update timestamp so the timeout poller knows the key is still held.
        if state.phase == VoiceModePhase::Recording && !state.key_release_supported {
            state.last_ptt_repeat_at = Some(Instant::now());
        }
    }

    /// Forward a space character to the composer (for quick-tap passthrough).
    fn type_space_in_composer(&mut self) {
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyModifiers;
        let space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
        let _ = self.bottom_pane.handle_key_event(space);
    }

    /// Start a tokio task that polls for PTT state changes in terminals that
    /// don't emit `KeyEventKind::Release`. Sends `VoiceModePttTimeoutCheck`
    /// every 200ms; the handler calls `check_ptt_timeout()`.
    fn start_ptt_timeout_poller(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // Cancel any previous poller.
        state.cancel_ptt_timeout_poller();

        let cancel = Arc::new(AtomicBool::new(false));
        state.ptt_timeout_cancel = Some(cancel.clone());
        let tx = self.app_event_tx.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));
            loop {
                interval.tick().await;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                tx.send(AppEvent::VoiceModePttTimeoutCheck);
            }
        });
    }

    /// Called from the app event loop on `VoiceModePttTimeoutCheck`.
    /// Handles two cases for terminals without key release:
    /// 1. Pending state with no repeats → was a tap, type space.
    /// 2. Recording with no repeats → key was released, stop recording.
    pub(crate) fn check_ptt_timeout(&mut self) {
        let Some(ref state) = self.voice_mode_state else {
            return;
        };

        // If we've since learned the terminal supports release, stop polling.
        if state.key_release_supported {
            return;
        }

        // Case 1: pending (no recording started yet).
        // If no repeat arrived within 500ms of the press, it was a tap.
        if let Some(pending_at) = state.ptt_pending_at {
            if Instant::now().duration_since(pending_at) > Duration::from_millis(500) {
                if let Some(ref mut state) = self.voice_mode_state {
                    state.ptt_pending_at = None;
                    state.cancel_ptt_timeout_poller();
                }
                self.type_space_in_composer();
            }
            return;
        }

        // Case 2: recording — check if repeats stopped.
        if state.phase != VoiceModePhase::Recording {
            return;
        }
        if let Some(last_repeat) = state.last_ptt_repeat_at {
            if Instant::now().duration_since(last_repeat) > Duration::from_millis(250) {
                self.on_ptt_release();
            }
        }
    }

    // ─── Agent delta / TTS / transcription handlers ─────────────────────

    /// Called when agent streaming delta arrives — parse `<voice>` tags, send
    /// tagged content to TTS, and return filtered display text (tags stripped).
    ///
    /// Returns `Some(display_text)` when voice mode is active (caller should use
    /// this instead of the raw delta), or `None` when voice mode is inactive.
    pub(crate) fn on_voice_mode_agent_delta(&mut self, delta: &str) -> Option<String> {
        let Some(ref mut state) = self.voice_mode_state else {
            return None;
        };
        if !state.is_active() {
            return None;
        }

        // In reading view mode, narrate ALL text (no <voice> tags needed).
        // The tag parser still runs for display (strips any tags the agent
        // might still emit), but TTS receives everything.
        let reading_view = self.bottom_pane.is_document_reader_active();

        // Always parse tags for display (strip <voice> markers), even if TTS
        // is suppressed due to barge-in.
        let result = state.voice_tag_parser.push(delta);

        // Determine what to send to TTS.
        let tts_sentences = if reading_view {
            // In reading view: send ALL display text to TTS via sentence buffer.
            let mut sentences = state.sentence_buffer.push(&result.display_text);
            // Also include any voice-tagged sentences (in case agent used tags).
            sentences.extend(result.voice_sentences);
            sentences
        } else {
            result.voice_sentences
        };

        // Only dispatch to TTS if not suppressed.
        if state.should_tts() && !tts_sentences.is_empty() {
            if state.phase != VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Speaking;
            }

            // Send each complete sentence to TTS.
            let vc = voice_mode_config(&self.config);
            let tx = self.app_event_tx.clone();
            let in_flight = state.tts_in_flight.clone();
            for sentence in tts_sentences {
                in_flight.fetch_add(1, Ordering::SeqCst);
                let tx = tx.clone();
                let config = vc.clone();
                let counter = in_flight.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_sentence_to_tts(&config, &sentence, tx, counter).await {
                        tracing::error!("TTS error: {e}");
                    }
                });
            }
        }

        Some(result.display_text)
    }

    /// Called when agent turn completes — flush remaining buffer to TTS.
    pub(crate) fn on_voice_mode_turn_complete(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if !state.is_active() {
            return;
        }

        if state.should_tts() {
            let vc = voice_mode_config(&self.config);
            let tx = self.app_event_tx.clone();
            let in_flight = state.tts_in_flight.clone();

            // Flush voice tag parser.
            if let Some(remaining) = state.voice_tag_parser.flush() {
                in_flight.fetch_add(1, Ordering::SeqCst);
                let vc = vc.clone();
                let tx = tx.clone();
                let in_flight = in_flight.clone();
                tokio::spawn(async move {
                    if let Err(e) = send_sentence_to_tts(&vc, &remaining, tx, in_flight).await {
                        tracing::error!("TTS flush error: {e}");
                    }
                });
            }

            // In reading view, also flush the sentence buffer (used for
            // narrate-all mode where all text goes to TTS).
            if let Some(remaining) = state.sentence_buffer.flush() {
                in_flight.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    if let Err(e) = send_sentence_to_tts(&vc, &remaining, tx, in_flight).await {
                        tracing::error!("TTS flush error: {e}");
                    }
                });
            }

            if state.phase != VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Speaking;
            }
        } else {
            // TTS suppressed (barge-in) or output mode is text-only.
            state.voice_tag_parser.clear();
            state.sentence_buffer.clear();
            state.phase = VoiceModePhase::Idle;
        }
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Called when a TTS audio chunk is received.
    pub(crate) fn on_voice_tts_audio_chunk(&mut self, pcm: Vec<i16>) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            state.phase = VoiceModePhase::Speaking;
        }
        if let Some(ref player) = state.audio_player {
            player.enqueue_pcm(&pcm, 24000, 1);
        }
    }

    /// Called when TTS playback is finished.
    pub(crate) fn on_voice_tts_finished(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            return;
        }

        // Ready for next PTT press.
        state.phase = VoiceModePhase::Idle;
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Called when ElevenLabs STT returns transcribed text.
    pub(crate) fn on_voice_transcription_complete(&mut self, text: String) {
        // Check phase and read auto_submit before borrowing self for other calls.
        let auto_submit = match self.voice_mode_state {
            Some(ref state) if state.phase == VoiceModePhase::Transcribing => state.auto_submit,
            _ => return,
        };

        // Update phase and clear per-turn state for the new turn.
        if let Some(ref mut state) = self.voice_mode_state {
            state.phase = VoiceModePhase::Idle;
            state.tts_suppressed = false;
        }

        if auto_submit {
            // Check if a reading view is active — if so, route through the
            // reading-view-aware voice path so the agent explains rather than
            // recites and writes a summary into the document.
            if let Some(rv_ctx) = self.bottom_pane.reading_view_voice_context() {
                self.submit_reading_view_voice_message(text, rv_ctx);
            } else {
                let mut msg = super::UserMessage::from(text);
                msg.voice_input = true;
                self.submit_user_message(msg);
            }
        } else {
            self.set_composer_text(text, Vec::new(), Vec::new());
        }

        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Submit a voice transcription routed through reading view context.
    ///
    /// Builds simplified instructions: voice instruction + section context +
    /// append_to_section tool call, then submits via `Op::UserInput`.
    pub(crate) fn submit_reading_view_voice_message(
        &mut self,
        text: String,
        ctx: crate::bottom_pane::ReadingViewVoiceContext,
    ) {
        use codex_protocol::protocol::Op;
        use codex_protocol::user_input::UserInput;

        let selection_hint = if let Some(ref sel) = ctx.selection {
            format!(
                "\nThe user selected this text and is asking about it:\n\
                 [Selected text:]\n{sel}\n"
            )
        } else {
            String::new()
        };

        let context = format!(
            "[VOICE MODE — READING VIEW]\n\
             The user is reading \"{title}\", section \"{heading}\". They asked:\n\
             {text}\n\
             {selection_hint}\n\
             Speak your explanation conversationally. Reference what they see. \
             Your entire response will be read aloud — do NOT use <voice> tags. \
             Do NOT include LaTeX, code blocks, or markdown formatting in your spoken answer \
             since this is a terminal that cannot render them. Use plain language instead \
             (e.g. say \"the attention equation\" instead of writing $\\text{{Attention}}(Q,K,V)$).\n\n\
             Then write a concise summary using EXACTLY ONE tool call:\n\
             append_to_section(document_id=\"{doc_id}\", section_index={idx}, \
             content=\"your written summary\", foldable=true, \
             summary=\"Descriptive topic label\")\n\n\
             Rules:\n\
             - Do NOT use <voice> tags — everything is narrated automatically\n\
             - Make exactly ONE append_to_section call\n\
             - Set foldable=true always\n\
             - Write prose, not Q:/A: format\n\
             - No LaTeX, no code blocks in your spoken response\n\
             - The summary should describe the topic (e.g. \"Dropout as regularization\", \
             \"Why gradients vanish\")\n\
             - Do NOT rewrite the section or make multiple tool calls",
            title = ctx.title,
            heading = ctx.heading,
            doc_id = ctx.document_id,
            idx = ctx.section_index,
        );

        self.app_event_tx.send(AppEvent::CodexOp(Op::UserInput {
            items: vec![UserInput::Text {
                text: context,
                text_elements: vec![],
            }],
            final_output_json_schema: None,
        }));
    }

    /// Called when STT transcription fails.
    pub(crate) fn on_voice_transcription_failed(&mut self, error: String) {
        if self.voice_mode_state.is_none() {
            return;
        }
        tracing::error!("voice mode STT failed: {error}");
        self.add_to_history(history_cell::new_error_event(format!(
            "Transcription failed: {error}"
        )));

        // Go back to idle.
        if let Some(ref mut state) = self.voice_mode_state {
            state.phase = VoiceModePhase::Idle;
        }
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Return `true` when voice mode is active and currently playing TTS audio.
    pub(crate) fn is_voice_speaking(&self) -> bool {
        self.voice_mode_state
            .as_ref()
            .is_some_and(|s| s.phase == VoiceModePhase::Speaking)
    }

    /// Interrupt TTS playback (e.g. user navigated to a different section).
    pub(crate) fn on_voice_interrupt_tts(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase == VoiceModePhase::Speaking {
            state.interrupt_tts();
            state.phase = VoiceModePhase::Idle;
            self.sync_voice_placeholder();
            self.request_redraw();
        }
    }

    /// Auto-narrate a reading view section via TTS.
    ///
    /// Called when the user navigates to a new section or when the reading view
    /// first opens. If voice mode is inactive or TTS is disabled, this is a no-op.
    pub(crate) fn on_voice_narrate_section(&mut self, raw_text: String) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if !state.should_tts() {
            return;
        }

        // Interrupt any ongoing TTS first.
        state.interrupt_tts();

        let cleaned = clean_for_tts(&raw_text);
        if cleaned.is_empty() {
            return;
        }

        state.phase = VoiceModePhase::Speaking;

        // Split into sentences and send each to TTS.
        let mut sentence_buf = SentenceBuffer::new();
        let mut sentences = sentence_buf.push(&cleaned);
        if let Some(remaining) = sentence_buf.flush() {
            sentences.push(remaining);
        }

        let vc = voice_mode_config(&self.config);
        let tx = self.app_event_tx.clone();
        let in_flight = state.tts_in_flight.clone();

        // Process sentences sequentially in a single task so they play
        // in order. Each sentence completes its TTS stream before the
        // next one begins.
        let total = sentences.len();
        in_flight.fetch_add(total, Ordering::SeqCst);
        tokio::spawn(async move {
            for sentence in sentences {
                let _ = send_sentence_to_tts(&vc, &sentence, tx.clone(), in_flight.clone()).await;
            }
        });

        self.sync_voice_placeholder();
    }
}

/// Strip markdown formatting from text to make it suitable for TTS narration.
///
/// Removes code blocks, inline code backticks, heading markers, bold/italic
/// markers, link syntax, LaTeX blocks, and image markers. Collapses whitespace
/// and caps length at ~2000 chars on a sentence boundary.
pub(crate) fn clean_for_tts(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();

    while let Some(ch) = chars.next() {
        // Fenced code blocks (```...```)
        if ch == '`' && chars.peek() == Some(&'`') {
            chars.next(); // second `
            if chars.peek() == Some(&'`') {
                chars.next(); // third `
                // Skip until closing ```
                // First skip the optional language tag (rest of current line)
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c == '\n' {
                        break;
                    }
                }
                // Now skip until closing ```
                let mut backtick_count = 0;
                for c in chars.by_ref() {
                    if c == '`' {
                        backtick_count += 1;
                        if backtick_count >= 3 {
                            break;
                        }
                    } else {
                        backtick_count = 0;
                    }
                }
                out.push_str(" (code block) ");
                continue;
            }
            // Just two backticks — treat as inline code start
            out.push_str("``");
            continue;
        }

        // Inline code (`...`)
        if ch == '`' {
            // Skip until closing backtick, keep the text inside
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                out.push(c);
            }
            continue;
        }

        // LaTeX display blocks ($$...$$)
        if ch == '$' && chars.peek() == Some(&'$') {
            chars.next(); // second $
            // Skip until closing $$
            let mut prev = ' ';
            for c in chars.by_ref() {
                if c == '$' && prev == '$' {
                    break;
                }
                prev = c;
            }
            out.push_str(" (equation) ");
            continue;
        }

        // Inline LaTeX ($...$) — but not "costs $5"
        if ch == '$' {
            let rest: String = chars.clone().take(50).collect();
            if let Some(end) = rest.find('$') {
                // Only treat as LaTeX if content looks non-numeric
                let inner = &rest[..end];
                if inner.contains('\\') || inner.contains('{') || inner.contains('^') {
                    for _ in 0..=end {
                        chars.next();
                    }
                    out.push_str(" (equation) ");
                    continue;
                }
            }
            out.push(ch);
            continue;
        }

        // Image markers ![alt](url)
        if ch == '!' && chars.peek() == Some(&'[') {
            chars.next(); // [
            // Skip alt text
            for c in chars.by_ref() {
                if c == ']' {
                    break;
                }
            }
            // Skip (url)
            if chars.peek() == Some(&'(') {
                chars.next();
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
            }
            continue;
        }

        // Links [text](url) — keep text, drop url
        if ch == '[' {
            let mut link_text = String::new();
            for c in chars.by_ref() {
                if c == ']' {
                    break;
                }
                link_text.push(c);
            }
            if chars.peek() == Some(&'(') {
                chars.next();
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
            }
            out.push_str(&link_text);
            continue;
        }

        // Markdown headings at start of line (strip # markers)
        if ch == '#' && (out.is_empty() || out.ends_with('\n')) {
            while chars.peek() == Some(&'#') {
                chars.next();
            }
            if chars.peek() == Some(&' ') {
                chars.next();
            }
            continue;
        }

        // Bold/italic markers (** * __ _) — strip them
        if ch == '*' || ch == '_' {
            // Consume consecutive same markers
            while chars.peek() == Some(&ch) {
                chars.next();
            }
            continue;
        }

        out.push(ch);
    }

    // Collapse multiple newlines into single newlines
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_newline = false;
    for ch in out.chars() {
        if ch == '\n' {
            if !prev_newline {
                collapsed.push('\n');
            }
            prev_newline = true;
        } else {
            prev_newline = false;
            collapsed.push(ch);
        }
    }

    let trimmed = collapsed.trim().to_string();

    // Cap at ~2000 chars at a sentence boundary
    if trimmed.len() <= 2000 {
        return trimmed;
    }

    // Find last sentence end before 2000
    let search_region = &trimmed[..2000];
    let last_sentence_end = search_region
        .rfind(". ")
        .or_else(|| search_region.rfind("! "))
        .or_else(|| search_region.rfind("? "))
        .map(|i| i + 1)
        .unwrap_or(2000);

    trimmed[..last_sentence_end].trim().to_string()
}

/// Send a sentence to ElevenLabs TTS and forward audio chunks as AppEvents.
///
/// The `in_flight` counter tracks concurrent TTS tasks. Only the last task
/// to finish (counter drops to zero) sends `VoiceModeTtsFinished`.
async fn send_sentence_to_tts(
    voice_config: &codex_core::config::types::VoiceModeToml,
    sentence: &str,
    tx: crate::app_event_sender::AppEventSender,
    in_flight: Arc<AtomicUsize>,
) -> Result<(), codex_elevenlabs::ElevenLabsError> {
    // Decrement counter on exit (success or error) via drop guard.
    let result = send_sentence_to_tts_inner(voice_config, sentence, &tx).await;

    // Regardless of success/failure, decrement the in-flight counter.
    // Only signal finished when this was the last in-flight TTS task.
    if in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
        tx.send(AppEvent::VoiceModeTtsFinished);
    }

    result
}

async fn send_sentence_to_tts_inner(
    voice_config: &codex_core::config::types::VoiceModeToml,
    sentence: &str,
    tx: &crate::app_event_sender::AppEventSender,
) -> Result<(), codex_elevenlabs::ElevenLabsError> {
    let api_key = voice_config
        .elevenlabs
        .as_ref()
        .and_then(|e| e.api_key.clone())
        .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok())
        .ok_or(codex_elevenlabs::ElevenLabsError::MissingApiKey)?;

    let mut config = codex_elevenlabs::ElevenLabsConfig::new(api_key);
    if let Some(ref el) = voice_config.elevenlabs {
        if let Some(ref vid) = el.voice_id {
            config = config.with_voice_id(vid.clone());
        }
        if let Some(ref mid) = el.model_id {
            config = config.with_model_id(mid.clone());
        }
    }

    let mut stream = codex_elevenlabs::tts::TtsStream::connect(&config).await?;
    stream.send_text(sentence).await?;
    stream.flush().await?;

    while let Some(pcm) = stream.recv_audio().await {
        tx.send(AppEvent::VoiceModeTtsAudioChunk { pcm });
    }

    stream.close().await;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_buffer_splits_on_punctuation() {
        let mut buf = SentenceBuffer::new();

        let sentences = buf.push("Hello world. This is a test! Are you ready? ");
        assert_eq!(
            sentences,
            vec![
                "Hello world.",
                "This is a test!",
                "Are you ready?",
            ]
        );
    }

    #[test]
    fn sentence_buffer_accumulates_until_boundary() {
        let mut buf = SentenceBuffer::new();

        assert!(buf.push("Hello ").is_empty());
        assert!(buf.push("world").is_empty());
        let sentences = buf.push(". Next sentence. ");
        assert_eq!(sentences, vec!["Hello world.", "Next sentence."]);
    }

    #[test]
    fn sentence_buffer_flush() {
        let mut buf = SentenceBuffer::new();
        buf.push("partial text without ending");
        assert_eq!(
            buf.flush(),
            Some("partial text without ending".to_string())
        );
        assert_eq!(buf.flush(), None);
    }

    #[test]
    fn sentence_buffer_double_newline() {
        let mut buf = SentenceBuffer::new();
        let sentences = buf.push("First paragraph\n\nSecond paragraph");
        assert_eq!(sentences, vec!["First paragraph"]);
        assert_eq!(buf.flush(), Some("Second paragraph".to_string()));
    }

    // ─── VoiceTagParser tests ────────────────────────────────────────────

    #[test]
    fn voice_tag_basic_parsing() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>Hello world.</voice> Some code here.");
        // marks spoken start, marks spoken end.
        assert_eq!(r.display_text, "🔊 Hello world. Some code here.");
        assert_eq!(r.voice_sentences, vec!["Hello world."]);
    }

    #[test]
    fn voice_tag_streaming_split() {
        let mut parser = VoiceTagParser::new();

        // Tag split across two deltas.
        let r1 = parser.push("<vo");
        assert_eq!(r1.display_text, "");
        assert!(r1.voice_sentences.is_empty());

        let r2 = parser.push("ice>Hello.</voice>");
        assert_eq!(r2.display_text, "🔊 Hello.");
        assert_eq!(r2.voice_sentences, vec!["Hello."]);
    }

    #[test]
    fn voice_tag_multiple_regions() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>First.</voice> code <voice>Second.</voice>");
        assert_eq!(r.display_text, "🔊 First. code 🔊 Second.");
        assert_eq!(r.voice_sentences, vec!["First.", "Second."]);
    }

    #[test]
    fn voice_tag_no_tags_fallback() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("Just regular text without any tags.");
        assert_eq!(r.display_text, "Just regular text without any tags.");
        // No voice tags → nothing goes to TTS.
        assert!(r.voice_sentences.is_empty());
    }

    #[test]
    fn voice_tag_flush() {
        let mut parser = VoiceTagParser::new();
        // Voice content without closing tag.
        let r = parser.push("<voice>Partial content");
        assert_eq!(r.display_text, "🔊 Partial content");
        assert!(r.voice_sentences.is_empty()); // No sentence boundary yet.

        // Flush on turn complete returns the partial voice content.
        assert_eq!(parser.flush(), Some("Partial content".to_string()));
        // Second flush is empty.
        assert_eq!(parser.flush(), None);
    }

    #[test]
    fn voice_tag_sentence_splitting_across_deltas() {
        let mut parser = VoiceTagParser::new();

        let r1 = parser.push("<voice>Hello ");
        assert_eq!(r1.display_text, "🔊 Hello ");
        assert!(r1.voice_sentences.is_empty());

        let r2 = parser.push("world. How are ");
        assert_eq!(r2.display_text, "world. How are ");
        assert_eq!(r2.voice_sentences, vec!["Hello world."]);

        let r3 = parser.push("you?</voice> ```rust\nfn main() {}```");
        assert_eq!(r3.display_text, "you? ```rust\nfn main() {}```");
        // </voice> flushes the remaining voice buffer as a sentence.
        assert_eq!(r3.voice_sentences, vec!["How are you?"]);
        // Nothing left to flush.
        assert_eq!(parser.flush(), None);
    }

    #[test]
    fn voice_tag_clear_resets_state() {
        let mut parser = VoiceTagParser::new();
        parser.push("<voice>Some text");
        parser.clear();
        assert_eq!(parser.flush(), None);

        // After clear, parser should work fresh.
        let r = parser.push("<voice>Fresh start.</voice>");
        assert_eq!(r.display_text, "🔊 Fresh start.");
        assert_eq!(r.voice_sentences, vec!["Fresh start."]);
    }

    #[test]
    fn voice_tag_closing_tag_split() {
        let mut parser = VoiceTagParser::new();

        // The closing </voice> tag is split: "</vo" in first delta, "ice>" in second.
        let r1 = parser.push("<voice>Done.</vo");
        assert_eq!(r1.display_text, "🔊 Done.");
        // "Done." is in voice_buffer but </voice> hasn't closed yet and there's
        // no sentence boundary (period needs trailing space), so no sentence yet.
        assert!(r1.voice_sentences.is_empty());

        let r2 = parser.push("ice> Next text.");
        assert_eq!(r2.display_text, " Next text.");
        // </voice> closes the region and flushes "Done." as a sentence.
        assert_eq!(r2.voice_sentences, vec!["Done."]);
    }
}
