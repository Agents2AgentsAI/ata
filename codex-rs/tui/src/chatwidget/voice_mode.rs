//! Voice mode state machine and TTS sentence buffer.
//!
//! Data flow (push-to-talk):
//! ```text
//! INPUT:  Space hold → VoiceCapture → Space release → WAV → STT → text → agent
//! OUTPUT: AgentMessageDelta → VoiceTagParser → <voice> content → TTS → PCM → speaker
//! ```

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
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
IMPORTANT: Voice mode ONLY changes how you format output (with <voice> tags). \
Everything else stays identical to non-voice mode:\n\
- Trigger skills exactly as you would without voice mode. If a request would \
normally activate a skill (paper synthesis, research, etc.), activate it now.\n\
- Use all tools normally — spawn subagents, check KB, read files, fetch URLs, \
search code, call APIs, and use any other capabilities you have.\n\
- Follow multi-step skill workflows completely. Do NOT take shortcuts or skip \
steps (like KB checks) just because the user is speaking.\n\
Do not answer from memory when you have tools and skills that can do it better.\n\
- Never use LaTeX math notation (\\mathbb, \\frac, etc.) — the terminal cannot \
render it. Use plain text or Unicode for math (e.g. \"E[|h_j(x)|]\" not \
\"\\\\mathbb{E}[|h_j(x)|]\").\n\
\n\
Follow this pattern:\n\
1. Start with a brief <voice> acknowledgment (1-2 sentences).\n\
2. Do any technical work (tool calls, skill execution, subagents) without voice tags.\n\
3. End with a <voice> summary of what you found or did (2-3 sentences).\n\
\n\
For multi-step tasks, add brief <voice> progress updates between steps \
so the user knows what is happening (e.g. \"Found the issue, fixing it now.\" \
or \"Checking a few more files.\"). Keep these to one sentence.\n\
\n\
For purely conversational responses with no code or tools, wrap the entire \
response in <voice> tags.\n\n";

/// Instruction prepended to the first user message after voice mode is
/// turned off, so the agent stops using `<voice>` tags.
pub(crate) const VOICE_MODE_OFF_INSTRUCTION: &str = "\
[VOICE MODE OFF] Voice mode has been turned off. \
Do NOT use <voice></voice> tags in your responses. \
Respond normally with plain text.\n\n";

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
            Self::Speaking => "\u{25B6}\u{FE0F}  Speaking...",
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

        while let Some(split_pos) = self.find_sentence_boundary() {
            let sentence: String = self.buffer[..split_pos].trim().to_string();
            self.buffer = self.buffer[split_pos..].trim_start().to_string();
            if !sentence.is_empty() {
                sentences.push(sentence);
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
    /// Whether a `</voice>` closing tag was processed in this delta (voice block boundary).
    pub voice_block_closed: bool,
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
        let mut voice_block_closed = false;

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
                        // Strip the tag from display output.
                        self.pending = rest[tag_end + 1..].to_string();
                    } else if tag_lower == "</voice>" {
                        self.in_voice = false;
                        voice_block_closed = true;
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
            voice_block_closed,
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
        while let Some(pos) = find_sentence_boundary(&self.voice_buffer) {
            let sentence = self.voice_buffer[..pos].trim().to_string();
            self.voice_buffer = self.voice_buffer[pos..].trim_start().to_string();
            if !sentence.is_empty() {
                out.push(sentence);
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

/// Cached TTS audio for a reading view section.
pub(crate) struct TtsCacheEntry {
    pub(crate) content_hash: u64,
    pub(crate) chunks: Vec<Vec<i16>>,
    /// Cached alignment timeline so karaoke works on replay.
    pub(crate) alignment_timeline: Vec<AlignmentEntry>,
}

/// A word-level entry in the alignment timeline.
/// Maps an absolute playback time range to a word in the TTS output.
#[derive(Debug, Clone)]
pub(crate) struct AlignmentEntry {
    /// Absolute start time in ms from beginning of playback.
    pub(crate) start_ms: u64,
    /// Duration in ms.
    pub(crate) duration_ms: u64,
    /// The word text (for rendering the highlight).
    pub(crate) word: String,
}

pub(crate) struct VoiceModeState {
    pub(crate) phase: VoiceModePhase,
    pub(crate) sentence_buffer: SentenceBuffer,
    pub(crate) voice_tag_parser: VoiceTagParser,
    pub(crate) output: VoiceOutput,
    pub(crate) auto_submit: bool,
    /// When false, TTS is disabled (no `<voice>` tag injection, no audio playback).
    pub(crate) tts_enabled: bool,
    /// When false, STT/push-to-talk is disabled (Space key not intercepted).
    pub(crate) stt_enabled: bool,

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

    /// Ordering lock: each spawned TTS task acquires this before streaming
    /// audio so sentences play in the order they were spawned.
    pub(crate) tts_ordering_lock: Arc<tokio::sync::Mutex<()>>,

    /// Generation counter: incremented on interrupt so stale tasks know to
    /// discard their audio instead of playing it.
    pub(crate) tts_generation: Arc<AtomicUsize>,

    /// Channel to the persistent TTS worker task. `None` when no worker is running.
    /// The worker maintains a single ElevenLabs WebSocket, eliminating per-sentence
    /// connection overhead.
    pub(crate) tts_worker_tx: Option<tokio::sync::mpsc::UnboundedSender<TtsWorkerCommand>>,

    /// Cache of pre-generated TTS audio. Key: (document_id, section_index).
    pub(crate) tts_section_cache:
        Arc<std::sync::Mutex<std::collections::HashMap<(String, usize), TtsCacheEntry>>>,
    /// Sections currently being prefetched (to avoid duplicates).
    pub(crate) prefetch_pending: Arc<std::sync::Mutex<std::collections::HashSet<(String, usize)>>>,

    // ─── Reading view narration cache collection ───────────────────────
    /// When narrating a section, tracks the (document_id, section_index)
    /// and content hash so chunks can be collected for caching.
    pub(crate) narrating_section: Option<(String, usize, u64)>,
    /// Number of heading words in the narration text. The TTS audio includes
    /// the heading but the reading view renders it separately, so karaoke
    /// must skip this many words to stay in sync.
    pub(crate) narrating_heading_words: usize,
    /// When narrating a visual selection, holds the word offset of the
    /// selection start within the section's rendered lines.  The offset
    /// is added to TTS word indices so karaoke highlights the correct
    /// word in-place.  `None` means full-section narration.
    pub(crate) selection_word_offset: Option<usize>,
    /// Cleaned text sent to TTS for the current narration (preserves newlines).
    /// Used by the karaoke builder to maintain line structure in the display.
    pub(crate) narrating_cleaned_text: Option<String>,
    /// Chunks collected during narration for cache storage.
    pub(crate) narrating_chunks: Vec<Vec<i16>>,

    // ─── Word-level alignment highlighting ──────────────────────────────
    /// Timeline of word-level alignment entries for the current voice turn.
    pub(crate) tts_alignment_timeline: Vec<AlignmentEntry>,
    /// Cumulative audio duration in ms (converts per-chunk relative times to absolute).
    pub(crate) tts_cumulative_ms: u64,
    /// Index into `tts_alignment_timeline` for the currently highlighted word.
    pub(crate) tts_highlight_word_idx: Option<usize>,
    /// Cancel flag for the highlight tick timer.
    pub(crate) highlight_tick_cancel: Option<Arc<AtomicBool>>,
    /// Partial word carried across chunk boundaries in alignment data.
    pub(crate) tts_pending_word: Option<AlignmentEntry>,
    /// Set when TTS worker has finished sending all audio to the player,
    /// but the player may still be playing buffered audio.
    pub(crate) tts_data_complete: bool,
    /// Set when a `</voice>` block closes; cleared when the next block's
    /// sentences are about to be sent to TTS.  Used to insert paragraph
    /// break sentinels in the alignment timeline between voice blocks.
    pub(crate) tts_block_break_pending: bool,
}

impl VoiceModeState {
    pub(crate) fn new(config: &VoiceModeToml) -> Self {
        let output = config.output.unwrap_or_default();
        let auto_submit = config.auto_submit.unwrap_or(true);
        let tts_enabled = config.tts_enabled.unwrap_or(true);
        let stt_enabled = config.stt_enabled.unwrap_or(true);

        Self {
            phase: VoiceModePhase::Off,
            sentence_buffer: SentenceBuffer::new(),
            voice_tag_parser: VoiceTagParser::new(),
            output,
            auto_submit,
            tts_enabled,
            stt_enabled,
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
            tts_ordering_lock: Arc::new(tokio::sync::Mutex::new(())),
            tts_generation: Arc::new(AtomicUsize::new(0)),
            tts_worker_tx: None,
            tts_section_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            prefetch_pending: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            narrating_section: None,
            narrating_heading_words: 0,
            selection_word_offset: None,
            narrating_cleaned_text: None,
            narrating_chunks: Vec::new(),
            tts_alignment_timeline: Vec::new(),
            tts_cumulative_ms: 0,
            tts_highlight_word_idx: None,
            highlight_tick_cancel: None,
            tts_pending_word: None,
            tts_data_complete: false,
            tts_block_break_pending: false,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.phase.is_active()
    }

    /// Should we send text deltas to TTS?
    pub(crate) fn should_tts(&self) -> bool {
        self.is_active()
            && self.tts_enabled
            && !self.tts_suppressed
            && matches!(self.output, VoiceOutput::Voice | VoiceOutput::Both)
    }

    /// Apply updated TTS/STT settings from the voice setup popup.
    pub(crate) fn apply_voice_settings(&mut self, tts: bool, stt: bool) {
        self.tts_enabled = tts;
        self.stt_enabled = stt;
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
        // Shut down the TTS worker (if running) by dropping the sender.
        self.tts_worker_tx = None;
        if let Some(cancel) = self.tts_cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(ref player) = self.audio_player {
            player.clear();
        }
        // Bump generation so any in-flight tasks discard their audio.
        self.tts_generation.fetch_add(1, Ordering::SeqCst);
        // Replace the ordering lock so new tasks don't queue behind stale ones.
        self.tts_ordering_lock = Arc::new(tokio::sync::Mutex::new(()));
        // Clear narration collection state.
        self.narrating_section = None;
        self.narrating_heading_words = 0;
        self.selection_word_offset = None;
        self.narrating_cleaned_text = None;
        self.narrating_chunks.clear();
        // Clear alignment timeline and highlight.
        self.tts_alignment_timeline.clear();
        self.tts_cumulative_ms = 0;
        self.tts_highlight_word_idx = None;
        self.tts_pending_word = None;
        self.tts_data_complete = false;
        self.tts_block_break_pending = false;
        self.cancel_highlight_tick();
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

    /// Cancel the highlight tick timer.
    fn cancel_highlight_tick(&mut self) {
        if let Some(cancel) = self.highlight_tick_cancel.take() {
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
        // Clear prefetch cache and pending set on full reset.
        if let Ok(mut cache) = self.tts_section_cache.lock() {
            cache.clear();
        }
        if let Ok(mut pending) = self.prefetch_pending.lock() {
            pending.clear();
        }
    }
}

// ─── ChatWidget voice mode integration ───────────────────────────────────────

use crate::app_event::AppEvent;
use crate::history_cell;

/// Check if the ElevenLabs API key is available (config or environment).
fn has_elevenlabs_api_key(config: &codex_core::config::Config) -> bool {
    let vc = voice_mode_config(config);
    vc.elevenlabs
        .as_ref()
        .and_then(|e| e.api_key.as_ref())
        .is_some()
        || std::env::var("ELEVENLABS_API_KEY").is_ok()
}

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
    /// Return the voice mode config with in-session overrides (speed, language)
    /// applied on top of the on-disk config.  `/voice-setup` writes to disk but
    /// the in-memory `config_layer_stack` isn't reloaded, so we patch the cached
    /// values in here to make changes take effect immediately.
    fn effective_voice_config(&self) -> VoiceModeToml {
        let mut vc = voice_mode_config(&self.config);
        if let Some(speed) = self.cached_elevenlabs_speed {
            let el = vc.elevenlabs.get_or_insert_with(Default::default);
            el.speed = Some(speed);
        }
        if let Some(ref lang) = self.cached_elevenlabs_language {
            let el = vc.elevenlabs.get_or_insert_with(Default::default);
            el.language_code = lang.clone();
        }
        vc
    }

    /// Sync the composer placeholder text to reflect the current voice mode phase.
    /// Also syncs the voice status indicator in the reading view (if active).
    fn sync_voice_placeholder(&mut self) {
        let (label, phase, stt_enabled) = match &self.voice_mode_state {
            Some(s) if s.phase.is_active() => (s.phase.status_label(), s.phase, s.stt_enabled),
            _ => return,
        };
        // When Idle and STT is disabled, show a different placeholder since
        // Space-to-speak won't work.
        let placeholder = if phase == VoiceModePhase::Idle && !stt_enabled {
            "\u{1F3A4}  Voice mode on (TTS only)"
        } else {
            label
        };
        self.bottom_pane
            .set_placeholder_text(placeholder.to_string());
        // Also update the reading view's voice status indicator.
        let reading_status = if phase == VoiceModePhase::Idle {
            if stt_enabled {
                Some("Hold Space to ask".to_string())
            } else {
                Some("Voice mode on (TTS only)".to_string())
            }
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
        self.bottom_pane.set_document_reader_voice_status(None);
    }

    /// Toggle voice mode on/off (`/voice` command).
    pub(crate) fn toggle_voice_mode(&mut self) {
        // Auto-enable the VoiceMode feature flag on first use.
        if !self
            .config
            .features
            .enabled(codex_core::features::Feature::VoiceMode)
        {
            self.config
                .features
                .enable(codex_core::features::Feature::VoiceMode);
            self.app_event_tx.send(AppEvent::UpdateFeatureFlags {
                updates: vec![(codex_core::features::Feature::VoiceMode, true)],
            });
        }

        if let Some(ref mut state) = self.voice_mode_state
            && state.is_active()
        {
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

        // Mutual exclusion: stop realtime mode if active.
        if self.realtime_conversation.is_live() {
            self.request_realtime_conversation_close(Some(
                "Stopped realtime mode to start voice mode.".to_string(),
            ));
        }

        // Initialize voice mode state from config.
        let voice_config = self.effective_voice_config();

        // Show a friendly API key hint if TTS won't work (STT still works
        // without ElevenLabs because it uses the built-in Whisper path).
        let api_key = voice_config
            .elevenlabs
            .as_ref()
            .and_then(|e| e.api_key.clone())
            .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok());
        let session_not_ready = self.thread_id.is_none();
        if api_key.is_none() {
            let warning_cell = history_cell::new_warning_event(
                "ElevenLabs API key not found — TTS will not work.\n\
                 Set ELEVENLABS_API_KEY or run /voice-setup to paste your key."
                    .to_string(),
            );
            if session_not_ready {
                self.pending_voice_startup_cells
                    .push(Box::new(warning_cell));
            } else {
                self.add_to_history(warning_cell);
            }
            self.request_redraw();
        }

        let mut state = VoiceModeState::new(&voice_config);

        // If both TTS and STT are disabled, don't activate — nothing useful
        // would happen. Point the user to /voice-setup instead.
        if !state.tts_enabled && !state.stt_enabled {
            self.add_info_message(
                "Both TTS and STT are disabled. Use /voice-setup to enable at least one."
                    .to_string(),
                None,
            );
            return;
        }

        // Start audio player for TTS output.
        match crate::voice::RealtimeAudioPlayer::start(&self.config) {
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

        if session_not_ready {
            self.pending_voice_startup_cells
                .push(Box::new(history_cell::new_info_event(
                    "Voice mode on. Hold Space to speak. /voice to stop.".to_string(),
                    None,
                )));
        } else {
            self.add_info_message(
                "Voice mode on. Hold Space to speak. /voice to stop.".to_string(),
                None,
            );
        }

        self.bottom_pane.set_force_hide_cursor(true);
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Update the ElevenLabs API key for the current process.
    pub(crate) fn update_elevenlabs_api_key(&mut self, key: String) {
        // SAFETY: single-threaded TUI — no other threads read this concurrently.
        unsafe {
            std::env::set_var("ELEVENLABS_API_KEY", &key);
        }
        self.add_info_message("ElevenLabs API key saved.".to_string(), None);
    }

    /// Cache the last-saved ElevenLabs language and speed so re-opening
    /// `/voice-setup` reflects the latest values without a config reload.
    pub(crate) fn update_elevenlabs_voice_settings(
        &mut self,
        language_code: Option<Option<String>>,
        speed: Option<f64>,
    ) {
        if let Some(lang) = language_code {
            self.cached_elevenlabs_language = Some(lang);
        }
        if let Some(s) = speed {
            self.cached_elevenlabs_speed = Some(s);
        }
    }

    /// Apply voice settings from the setup popup.
    pub(crate) fn apply_voice_settings(&mut self, tts: bool, stt: bool) {
        if let Some(ref mut state) = self.voice_mode_state {
            state.apply_voice_settings(tts, stt);
            // Re-sync placeholder to reflect new STT state.
            if state.is_active() {
                let _ = state;
                self.sync_voice_placeholder();
                self.request_redraw();
            }
        }
    }

    /// Deactivate voice mode if it's currently active (called when both
    /// TTS and STT are turned off via the setup popup).
    pub(crate) fn deactivate_voice_mode_if_active(&mut self) {
        if let Some(ref mut state) = self.voice_mode_state
            && state.is_active()
        {
            state.reset();
            self.app_event_tx
                .send(AppEvent::PersistVoiceModeEnabled(false));
            self.add_info_message(
                "Voice mode off (TTS and STT both disabled).".to_string(),
                None,
            );
            self.restore_default_placeholder();
            self.bottom_pane.set_force_hide_cursor(false);
            self.request_redraw();
        }
    }

    /// Open the voice setup popup.
    pub(crate) fn open_voice_setup_popup(&mut self) {
        let voice_config = self.effective_voice_config();

        let tts_enabled = self
            .voice_mode_state
            .as_ref()
            .map_or(voice_config.tts_enabled.unwrap_or(true), |s| s.tts_enabled);
        let stt_enabled = self
            .voice_mode_state
            .as_ref()
            .map_or(voice_config.stt_enabled.unwrap_or(true), |s| s.stt_enabled);

        let api_key_available = voice_config
            .elevenlabs
            .as_ref()
            .and_then(|e| e.api_key.as_ref())
            .is_some()
            || std::env::var("ELEVENLABS_API_KEY").is_ok();

        // Prefer cached values (set from the last save) over the stale in-memory config.
        let language_code = self.cached_elevenlabs_language.clone().unwrap_or_else(|| {
            voice_config
                .elevenlabs
                .as_ref()
                .and_then(|e| e.language_code.clone())
        });
        let speed = self
            .cached_elevenlabs_speed
            .or_else(|| voice_config.elevenlabs.as_ref().and_then(|e| e.speed));

        let view = crate::bottom_pane::VoiceSetupView::new(
            tts_enabled,
            stt_enabled,
            api_key_available,
            language_code,
            speed,
            self.app_event_tx.clone(),
        );
        self.bottom_pane.show_view(Box::new(view));
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

        // If agent is speaking (or audio is still buffered), barge-in:
        // interrupt TTS first.
        if state.phase == VoiceModePhase::Speaking
            || state
                .audio_player
                .as_ref()
                .is_some_and(super::super::voice::RealtimeAudioPlayer::has_buffered_audio)
        {
            state.interrupt_tts();
            state.tts_suppressed = true;
            if state.phase == VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Idle;
            }
            // Clear reading view overlays so the highlight doesn't stick.
            self.bottom_pane
                .set_document_reader_karaoke_lines(None, false);
            self.bottom_pane
                .set_document_reader_reading_progress(None, 0);
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

        // Clear any whitespace-only content from the composer (stale spaces
        // left by previous quick-tap PTT attempts).
        if !self.bottom_pane.composer_is_empty()
            && !self
                .bottom_pane
                .composer_text()
                .chars()
                .any(|c| !c.is_whitespace())
        {
            self.bottom_pane
                .set_composer_text(String::new(), Vec::new(), Vec::new());
        }

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
        if state.key_release_supported
            && let Some(started) = state.recording_started_at
            && started.elapsed() < Duration::from_millis(200)
        {
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
        let voice_config = self.effective_voice_config();

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
                config.language_code = el.language_code.clone();
                config.speed = el.speed;
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
        if let Some(last_repeat) = state.last_ptt_repeat_at
            && Instant::now().duration_since(last_repeat) > Duration::from_millis(250)
        {
            self.on_ptt_release();
        }
    }

    // ─── Agent delta / TTS / transcription handlers ─────────────────────

    /// Called when agent streaming delta arrives — parse `<voice>` tags, send
    /// tagged content to TTS, and return filtered display text (tags stripped).
    ///
    /// Returns `Some(display_text)` when voice mode is active (caller should use
    /// this instead of the raw delta), or `None` when voice mode is inactive.
    pub(crate) fn on_voice_mode_agent_delta(&mut self, delta: &str) -> Option<String> {
        let vc = self.effective_voice_config();
        let state = self.voice_mode_state.as_mut()?;
        if !state.is_active() {
            // Voice mode was previously on but is now off.  The agent may
            // still emit <voice> tags from earlier instructions in the
            // conversation context.  Strip them for clean display without
            // sending anything to TTS.
            let result = state.voice_tag_parser.push(delta);
            return Some(result.display_text);
        }

        // Always parse tags for display (strip <voice> markers), even if TTS
        // is suppressed due to barge-in.
        let result = state.voice_tag_parser.push(delta);

        let block_closed = result.voice_block_closed;

        // Determine what to send to TTS.
        // When a section is being auto-narrated, skip sending agent delta
        // text to TTS — the section narration already owns the audio stream
        // and mixing in the agent's conversational response would corrupt
        // the alignment timeline and karaoke overlay.
        let is_narrating = state.narrating_section.is_some();
        let tts_sentences = if is_narrating {
            // Auto-narration active — don't send anything to TTS.
            vec![]
        } else {
            // Normal voice mode (whether in reading view or not): only
            // <voice>-tagged content goes to TTS. The agent's response
            // appears in chat history, not in the reading view overlay.
            result.voice_sentences
        };

        // Only dispatch to TTS if not suppressed.
        if state.should_tts() && !tts_sentences.is_empty() {
            // Insert a paragraph break sentinel in the alignment timeline
            // when a new voice block begins after a previous one closed.
            // This gives the karaoke display paragraph structure matching
            // the history cells.
            if state.tts_block_break_pending && !state.tts_alignment_timeline.is_empty() {
                state.tts_alignment_timeline.push(AlignmentEntry {
                    start_ms: state.tts_cumulative_ms,
                    duration_ms: 0,
                    word: "\n\n".to_string(),
                });
                state.tts_block_break_pending = false;
            }

            // Skip TTS silently if no API key — the user was already warned
            // at voice mode activation time.
            if state.tts_worker_tx.is_none() && !has_elevenlabs_api_key(&self.config) {
                return Some(result.display_text);
            }

            if state.phase != VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Speaking;
            }

            // Ensure TTS worker is running (one persistent WebSocket per voice turn).
            if state.tts_worker_tx.is_none() {
                let worker_vc = vc.clone();
                let tx = self.app_event_tx.clone();
                let in_flight = state.tts_in_flight.clone();
                let gen_ref = state.tts_generation.clone();
                let spawn_gen = gen_ref.load(Ordering::SeqCst);
                in_flight.fetch_add(1, Ordering::SeqCst);

                let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
                state.tts_worker_tx = Some(worker_tx);

                tokio::spawn(async move {
                    tts_worker_loop(worker_vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
                });
            }

            // Send each sentence to the worker — no per-sentence WebSocket needed.
            if let Some(ref worker_tx) = state.tts_worker_tx {
                for sentence in tts_sentences.iter() {
                    let _ = worker_tx.send(TtsWorkerCommand::SendText(sentence.clone()));
                }
            }
        }

        // Mark block boundary AFTER dispatching the closing block's sentences,
        // so the break sentinel is inserted before the NEXT block, not this one.
        if block_closed {
            state.tts_block_break_pending = true;
        }

        Some(result.display_text)
    }

    /// Called when agent turn completes — flush remaining buffer to TTS.
    pub(crate) fn on_voice_mode_turn_complete(&mut self) {
        let vc = self.effective_voice_config();
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if !state.is_active() {
            return;
        }

        if state.should_tts() {
            // Collect remaining text from both parsers.
            let mut remaining = Vec::new();
            if let Some(r) = state.voice_tag_parser.flush() {
                remaining.push(r);
            }
            if let Some(r) = state.sentence_buffer.flush() {
                remaining.push(r);
            }

            let has_remaining = !remaining.is_empty();

            // If there's remaining text but no worker, start one.
            if has_remaining && state.tts_worker_tx.is_none() {
                let tx = self.app_event_tx.clone();
                let in_flight = state.tts_in_flight.clone();
                let gen_ref = state.tts_generation.clone();
                let spawn_gen = gen_ref.load(Ordering::SeqCst);
                in_flight.fetch_add(1, Ordering::SeqCst);

                let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
                state.tts_worker_tx = Some(worker_tx);

                tokio::spawn(async move {
                    tts_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
                });
            }

            // Send remaining text and signal finish.
            if let Some(ref worker_tx) = state.tts_worker_tx {
                for sentence in remaining {
                    let _ = worker_tx.send(TtsWorkerCommand::SendText(sentence));
                }
                let _ = worker_tx.send(TtsWorkerCommand::Finish);
            }
            state.tts_worker_tx = None;

            if state.phase != VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Speaking;
            }
        } else {
            // TTS suppressed (barge-in) or output mode is text-only.
            state.voice_tag_parser.clear();
            state.sentence_buffer.clear();
            state.phase = VoiceModePhase::Idle;
            // Clear suppression so the next turn's TTS works.
            state.tts_suppressed = false;
        }
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Called when a TTS audio chunk is received.
    pub(crate) fn on_voice_tts_audio_chunk(
        &mut self,
        pcm: Vec<i16>,
        alignment: Option<codex_elevenlabs::TtsAlignment>,
    ) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            state.phase = VoiceModePhase::Speaking;
        }

        // Compute chunk duration from PCM length (24kHz mono).
        let chunk_duration_ms = (pcm.len() as u64) * 1000 / 24000;

        // Reset playback position counter on the first chunk of a turn
        // so alignment timing stays in sync with audio output.
        if state.tts_cumulative_ms == 0
            && let Some(ref player) = state.audio_player
        {
            player.reset_playback_position();
        }

        // Build alignment entries from this chunk's alignment data.
        if let Some(ref align) = alignment {
            // ElevenLabs alignment timestamps are session-absolute (not
            // per-chunk), so pass 0 as offset — no cumulative adjustment needed.
            build_alignment_entries(
                align,
                0,
                &mut state.tts_alignment_timeline,
                &mut state.tts_pending_word,
            );
        }

        state.tts_cumulative_ms += chunk_duration_ms;

        // Collect chunks for narration caching.
        if state.narrating_section.is_some() {
            state.narrating_chunks.push(pcm.clone());
        }

        if let Some(ref player) = state.audio_player {
            player.enqueue_pcm(&pcm, 24000, 1);
        }

        // Start highlight tick if not already running.
        let needs_tick = self.voice_mode_state.as_ref().is_some_and(|s| {
            s.highlight_tick_cancel.is_none() && !s.tts_alignment_timeline.is_empty()
        });
        if needs_tick {
            self.start_highlight_tick();
        }

        // Push initial karaoke lines to the reading view (if active) so
        // the highlighted text appears as soon as alignment data arrives.
        #[cfg(not(target_os = "linux"))]
        if alignment.is_some() {
            self.push_karaoke_to_reader();
        }
    }

    /// Called when TTS playback is finished.
    /// Handle a TTS error from the background worker. Surfaces the error
    /// in the voice status / placeholder so the user knows what happened.
    pub(crate) fn on_voice_tts_error(&mut self, error: &str) {
        tracing::warn!("Voice TTS error surfaced to user: {error}");
        // Show a user-friendly error in the placeholder and reading view status.
        let msg = if error.contains("401") || error.contains("Unauthorized") {
            "TTS error: invalid ElevenLabs API key".to_string()
        } else if error.contains("402") || error.contains("quota") || error.contains("credit") {
            "TTS error: ElevenLabs credits exhausted".to_string()
        } else {
            format!("TTS error: {}", truncate_error(error, 60))
        };
        self.bottom_pane.set_placeholder_text(msg.clone());
        self.bottom_pane.set_document_reader_voice_status(Some(msg));
    }

    pub(crate) fn on_voice_tts_finished(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            return;
        }

        // Flush any pending partial word so the last word gets highlighted.
        if let Some(pw) = state.tts_pending_word.take() {
            state.tts_alignment_timeline.push(pw);
        }

        // If the highlight tick is running and the audio player still has
        // buffered audio, defer the full cleanup — the highlight tick will
        // finalize once the player's buffer is empty.
        let has_audio = state
            .audio_player
            .as_ref()
            .is_some_and(super::super::voice::RealtimeAudioPlayer::has_buffered_audio);
        if has_audio && !state.tts_alignment_timeline.is_empty() {
            state.tts_data_complete = true;
            return;
        }

        self.finalize_voice_turn();
    }

    /// Full cleanup of voice turn state — called either immediately from
    /// `on_voice_tts_finished` or deferred from `on_voice_highlight_tick`
    /// once the audio player's buffer has drained.
    fn finalize_voice_turn(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // Store collected narration chunks + alignment in cache.
        state.narrating_cleaned_text = None;
        state.narrating_heading_words = 0;
        state.selection_word_offset = None;
        if let Some((doc_id, sec_idx, content_hash)) = state.narrating_section.take() {
            let chunks = std::mem::take(&mut state.narrating_chunks);
            let alignment_timeline = state.tts_alignment_timeline.clone();
            if !chunks.is_empty()
                && let Ok(mut cache) = state.tts_section_cache.lock()
            {
                cache.insert(
                    (doc_id, sec_idx),
                    TtsCacheEntry {
                        content_hash,
                        chunks,
                        alignment_timeline,
                    },
                );
            }
        }

        // Clear alignment state.
        state.tts_alignment_timeline.clear();
        state.tts_cumulative_ms = 0;
        state.tts_highlight_word_idx = None;
        state.tts_pending_word = None;
        state.tts_data_complete = false;
        state.tts_block_break_pending = false;
        state.cancel_highlight_tick();

        // Ready for next PTT press.
        state.phase = VoiceModePhase::Idle;

        // Clear karaoke overlay and reading cursor in the reading view.
        self.bottom_pane
            .set_document_reader_karaoke_lines(None, false);
        self.bottom_pane
            .set_document_reader_reading_progress(None, 0);

        // Flush any voice response cells that were stashed during karaoke.
        self.flush_deferred_voice_cells();

        self.sync_voice_placeholder();
        self.request_redraw();
    }

    // ─── Highlight tick timer ──────────────────────────────────────────

    /// Start a periodic tick that updates the TTS word highlight.
    fn start_highlight_tick(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // Don't start if already running.
        if state.highlight_tick_cancel.is_some() {
            return;
        }

        tracing::debug!(
            "Starting highlight tick timer, timeline has {} entries",
            state.tts_alignment_timeline.len(),
        );

        let cancel = Arc::new(AtomicBool::new(false));
        state.highlight_tick_cancel = Some(cancel.clone());
        let tx = self.app_event_tx.clone();

        std::thread::spawn(move || {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                tx.send(AppEvent::VoiceModeHighlightTick);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }

    /// Called on each highlight tick — update the highlighted word based on playback position.
    pub(crate) fn on_voice_highlight_tick(&mut self) {
        // Check if we should finalize — TTS data complete and audio drained.
        let should_finalize = self.voice_mode_state.as_ref().is_some_and(|s| {
            s.phase == VoiceModePhase::Speaking
                && s.tts_data_complete
                && !s
                    .audio_player
                    .as_ref()
                    .is_some_and(super::super::voice::RealtimeAudioPlayer::has_buffered_audio)
        });
        if should_finalize {
            self.finalize_voice_turn();
            return;
        }

        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            return;
        }

        let pos_ms = state
            .audio_player
            .as_ref()
            .map(super::super::voice::RealtimeAudioPlayer::playback_position_ms)
            .unwrap_or(0);

        if state.tts_alignment_timeline.is_empty() {
            return;
        }

        let new_idx = find_active_word(&state.tts_alignment_timeline, pos_ms);

        // During inter-sentence gaps find_active_word returns None.
        // Keep the previous highlight rather than showing no highlight,
        // so the karaoke doesn't appear to "pause".
        let effective_idx = new_idx.or(state.tts_highlight_word_idx);

        if effective_idx != state.tts_highlight_word_idx {
            state.tts_highlight_word_idx = effective_idx;
            #[cfg(not(target_os = "linux"))]
            self.push_karaoke_to_reader();
            self.request_redraw();
        }
    }

    /// Build styled Lines for the voice playback karaoke display.
    ///
    /// During TTS playback the response text lives in terminal scrollback
    /// (outside the ratatui buffer), so we can't use a post-render overlay.
    /// Instead we render the text directly in the viewport with the current
    /// word styled as bold+underline.
    #[cfg(not(target_os = "linux"))]
    pub(crate) fn voice_karaoke_lines(
        &self,
        width: u16,
    ) -> Option<Vec<ratatui::text::Line<'static>>> {
        use ratatui::style::Modifier;
        use ratatui::style::Style;
        use ratatui::text::Line;
        use ratatui::text::Span;
        use textwrap::Options as WrapOptions;
        use textwrap::wrap;

        let state = self.voice_mode_state.as_ref()?;
        if state.phase != VoiceModePhase::Speaking {
            return None;
        }
        let timeline = &state.tts_alignment_timeline;
        if timeline.is_empty() {
            return None;
        }
        let word_idx = state.tts_highlight_word_idx;

        // Build the full text from the timeline words.
        // Voice block boundaries ("\n\n" sentinels) become paragraph breaks.
        // Use `need_space` (not `i > 0`) so that skipped sentinels don't
        // inject a spurious leading space — mirrors the offset calculation below.
        let mut full_text = String::new();
        let mut ft_need_space = false;
        for entry in timeline.iter() {
            if entry.word == "\n" {
                continue; // legacy sentinel
            }
            if entry.word == "\n\n" {
                // Voice block boundary — paragraph break.
                full_text.push_str("\n\n");
                ft_need_space = false;
                continue;
            }
            if ft_need_space {
                full_text.push(' ');
            }
            ft_need_space = true;
            full_text.push_str(&entry.word);
        }

        // Word-wrap each paragraph independently to respect block boundaries.
        let wrap_width = width.saturating_sub(4) as usize;
        if wrap_width < 10 {
            return None;
        }
        let mut wrapped: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        for paragraph in full_text.split("\n\n") {
            if paragraph.is_empty() {
                continue;
            }
            if !wrapped.is_empty() {
                // Empty line between paragraphs.
                wrapped.push(std::borrow::Cow::Borrowed(""));
            }
            wrapped.extend(wrap(paragraph, WrapOptions::new(wrap_width)));
        }

        // Now build styled Lines. We need to find which characters belong to
        // the highlighted word and style them differently.
        let highlight_style = Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        // Calculate character offsets for the highlighted word.
        // Track offsets mirroring how full_text was built above.
        if let Some(idx) = word_idx {
            let mut char_offset = 0;
            let mut need_space = false;
            for (i, entry) in timeline.iter().enumerate() {
                if entry.word == "\n" {
                    continue; // legacy sentinel
                }
                if entry.word == "\n\n" {
                    // Block boundary — account for "\n\n" in full_text.
                    char_offset += "\n\n".len();
                    need_space = false;
                    continue;
                }
                if need_space {
                    char_offset += 1; // space separator
                }
                need_space = true;
                if i == idx {
                    let word_len = entry.word.len();
                    return Some(Self::build_karaoke_lines_inner(
                        &wrapped,
                        char_offset,
                        char_offset + word_len,
                        highlight_style,
                    ));
                }
                char_offset += entry.word.len();
            }
        }

        // No highlight — render plain text.
        Some(
            wrapped
                .iter()
                .map(|line| Line::from(Span::raw(line.to_string())))
                .collect(),
        )
    }

    /// Build wrapped Lines with a highlighted byte range.
    ///
    /// `hl_start` and `hl_end` are byte offsets into the **unwrapped**
    /// concatenated text. We snap them to valid UTF-8 character boundaries
    /// to avoid panics on multi-byte characters (e.g. `'`).
    #[cfg(not(target_os = "linux"))]
    fn build_karaoke_lines_inner(
        wrapped: &[std::borrow::Cow<'_, str>],
        hl_start: usize,
        hl_end: usize,
        highlight_style: ratatui::style::Style,
    ) -> Vec<ratatui::text::Line<'static>> {
        use ratatui::text::Line;
        use ratatui::text::Span;

        /// Snap a byte offset to the nearest valid character boundary
        /// (scanning forward). Returns `s.len()` if `pos >= s.len()`.
        fn snap_forward(s: &str, pos: usize) -> usize {
            let mut p = pos.min(s.len());
            while p < s.len() && !s.is_char_boundary(p) {
                p += 1;
            }
            p
        }

        /// Snap a byte offset to the nearest valid character boundary
        /// (scanning backward). Returns 0 if already at or before start.
        fn snap_backward(s: &str, pos: usize) -> usize {
            let mut p = pos.min(s.len());
            while p > 0 && !s.is_char_boundary(p) {
                p -= 1;
            }
            p
        }

        let mut lines = Vec::new();
        let mut global_offset = 0usize;
        for wrapped_line in wrapped {
            let line_len = wrapped_line.len();
            let line_start = global_offset;
            let line_end = global_offset + line_len;

            if hl_start >= line_end || hl_end <= line_start {
                // No overlap — plain line.
                lines.push(Line::from(Span::raw(wrapped_line.to_string())));
            } else {
                // Overlap — split into before/highlight/after spans.
                let mut spans = Vec::new();
                let rel_start = snap_backward(wrapped_line, hl_start.saturating_sub(line_start));
                let rel_end = snap_forward(wrapped_line, hl_end.min(line_end) - line_start);

                if rel_start > 0 {
                    spans.push(Span::raw(wrapped_line[..rel_start].to_string()));
                }
                if rel_start < rel_end {
                    spans.push(Span::styled(
                        wrapped_line[rel_start..rel_end].to_string(),
                        highlight_style,
                    ));
                }
                if rel_end < line_len {
                    spans.push(Span::raw(wrapped_line[rel_end..].to_string()));
                }
                lines.push(Line::from(spans));
            }

            // +1 for the newline/wrap boundary.
            global_offset = line_end + 1;
        }
        lines
    }

    /// Push reading progress to the reading view if the document reader is active.
    ///
    /// For section auto-narration (`narrating_section` is Some): sends the
    /// current word index so the view can highlight the corresponding
    /// rendered line (preserving markdown formatting).
    /// For Q&A responses: appends karaoke-highlighted lines after the
    /// existing section content (word-level highlight in plain text).
    #[cfg(not(target_os = "linux"))]
    fn push_karaoke_to_reader(&mut self) {
        if !self.bottom_pane.is_document_reader_active() {
            return;
        }
        let is_narrating = self
            .voice_mode_state
            .as_ref()
            .is_some_and(|s| s.narrating_section.is_some());

        if is_narrating {
            // Narration mode (full section or selection): send word index to
            // the view which maps it to a rendered line, preserving markdown.
            // For selections, add the word offset so karaoke highlights the
            // correct position within the full rendered content.
            let (word_idx, sel_offset, heading_words) = self
                .voice_mode_state
                .as_ref()
                .map(|s| {
                    let hw = s.narrating_heading_words;
                    (
                        s.tts_highlight_word_idx,
                        s.selection_word_offset.unwrap_or(0),
                        hw,
                    )
                })
                .unwrap_or((None, 0, 0));
            let adjusted = word_idx.map(|w| w + sel_offset);
            self.bottom_pane
                .set_document_reader_reading_progress(adjusted, heading_words);
        }
        // Non-narrating Q&A responses: push karaoke into the reading view
        // in append mode so the user sees the spoken text below the section
        // content, even though chat streaming is suppressed.
        if !is_narrating {
            let width = self.last_rendered_width.get().unwrap_or(80) as u16;
            let lines = self.voice_karaoke_lines(width);
            self.bottom_pane
                .set_document_reader_karaoke_lines(lines, true);
        }
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

        let mut reading_view_question: Option<String> = None;
        if auto_submit {
            // Check if a reading view is active — if so, route through the
            // reading-view-aware voice path so the agent explains rather than
            // recites and writes a summary into the document.
            if let Some(rv_ctx) = self.bottom_pane.reading_view_voice_context() {
                let question = text.clone();
                self.submit_reading_view_voice_message(text, rv_ctx);
                reading_view_question = Some(question);
            } else {
                let mut msg = super::UserMessage::from(text);
                msg.voice_input = true;
                self.submit_user_message(msg);
            }
        } else {
            self.set_composer_text(text, Vec::new(), Vec::new());
        }

        self.sync_voice_placeholder();
        // Show the inline "You asked: ... • thinking..." indicator in the
        // reading view — same style as text questions.
        if let Some(question) = reading_view_question
            && let Some(ctx) = self.bottom_pane.reading_view_voice_context()
        {
            self.bottom_pane
                .set_document_reader_pending_voice_question(ctx.section_index, question);
        }
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
             Write your explanation as plain conversational prose. \
             Your text response will be read aloud AND saved as a note, so write \
             it exactly as you want it to sound and appear. \
             Wrap ALL of your spoken text in <voice>...</voice> tags so it is read aloud. \
             Do NOT include LaTeX, code blocks, or markdown formatting \
             since this is a terminal TTS context. Use plain language instead \
             (e.g. say \"the attention equation\" instead of writing $\\text{{Attention}}(Q,K,V)$).\n\n\
             After your spoken explanation, save it using EXACTLY ONE tool call:\n\
             append_to_section(document_id=\"{doc_id}\", section_index={idx}, \
             content=\"<same text you just spoke, without the voice tags>\", foldable=true, \
             summary=\"Descriptive topic label\")\n\n\
             IMPORTANT: The content in append_to_section MUST be the same text you \
             spoke above — do NOT write a separate summary. Copy your spoken \
             response verbatim into the content field (without voice tags).\n\n\
             Rules:\n\
             - Wrap your spoken response in <voice>...</voice> tags\n\
             - Make exactly ONE append_to_section call\n\
             - Set foldable=true always\n\
             - The content must match what you said (verbatim, without voice tags)\n\
             - No LaTeX, no code blocks\n\
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

    /// Return `true` when voice mode is active and TTS audio is playing.
    ///
    /// Checks both the phase (Speaking) AND whether the audio player still
    /// has buffered samples, since `on_voice_tts_finished` transitions the
    /// phase to Idle as soon as all TTS chunks are enqueued — the player
    /// may still be playing buffered audio.
    pub(crate) fn is_voice_speaking(&self) -> bool {
        self.voice_mode_state.as_ref().is_some_and(|s| {
            s.phase == VoiceModePhase::Speaking
                || (s.is_active()
                    && s.audio_player
                        .as_ref()
                        .is_some_and(super::super::voice::RealtimeAudioPlayer::has_buffered_audio))
        })
    }

    /// Interrupt TTS playback (e.g. user pressed Escape or navigated away).
    pub(crate) fn on_voice_interrupt_tts(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        // Always clear the audio player buffer and TTS state, regardless
        // of phase. Audio may still be buffered after phase left Speaking.
        state.interrupt_tts();
        if state.phase == VoiceModePhase::Speaking {
            state.phase = VoiceModePhase::Idle;
        }
        // Clear karaoke overlay and reading cursor in the reading view.
        self.bottom_pane
            .set_document_reader_karaoke_lines(None, false);
        self.bottom_pane
            .set_document_reader_reading_progress(None, 0);
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Auto-narrate a reading view section via TTS.
    ///
    /// Called when the user navigates to a new section or when the reading view
    /// first opens. If voice mode is inactive or TTS is disabled, this is a no-op.
    pub(crate) fn on_voice_narrate_section(
        &mut self,
        document_id: String,
        section_index: usize,
        raw_text: String,
        selection_word_offset: Option<usize>,
    ) {
        // Phase 1: check preconditions and interrupt (borrows state mutably).
        {
            let Some(ref mut state) = self.voice_mode_state else {
                return;
            };
            if !state.should_tts() {
                return;
            }
            // Skip TTS silently if no API key — the user was already warned
            // at voice mode activation time.
            if state.tts_worker_tx.is_none() && !has_elevenlabs_api_key(&self.config) {
                return;
            }
            state.interrupt_tts();
        }

        let cleaned = clean_for_tts(&raw_text);
        if cleaned.is_empty() {
            return;
        }

        let content_hash = hash_text(&cleaned);

        // Phase 2: check cache (works for both full sections and selections —
        // the content_hash distinguishes different text under the same key).
        let cached: Option<(Vec<Vec<i16>>, Vec<AlignmentEntry>)> =
            self.voice_mode_state.as_ref().and_then(|state| {
                state.tts_section_cache.lock().ok().and_then(|cache| {
                    cache
                        .get(&(document_id.clone(), section_index))
                        .filter(|entry| entry.content_hash == content_hash)
                        .map(|entry| (entry.chunks.clone(), entry.alignment_timeline.clone()))
                })
            });

        // Heading words to skip = 0: the heading exists in BOTH the TTS
        // timeline and the rendered section lines, so word indices are
        // naturally aligned and no skip is needed.
        let heading_words = 0;

        if let Some((chunks, cached_timeline)) = cached {
            // Cache hit — play cached chunks and restore alignment for karaoke.
            if let Some(ref mut state) = self.voice_mode_state {
                state.phase = VoiceModePhase::Speaking;
                state.narrating_section = Some((document_id, section_index, content_hash));
                state.narrating_heading_words = heading_words;
                state.selection_word_offset = selection_word_offset;
                state.narrating_cleaned_text = Some(cleaned);
                state.tts_alignment_timeline = cached_timeline;
            }
            for chunk in chunks {
                self.on_voice_tts_audio_chunk(chunk, None);
            }
            // Start highlight tick so karaoke progresses during playback.
            self.start_highlight_tick();
            self.app_event_tx.send(AppEvent::VoiceModeTtsFinished);
            self.sync_voice_placeholder();
            return;
        }

        // Phase 3: cache miss — use persistent TTS worker (single WebSocket).
        let narration_vc = self.effective_voice_config();
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        state.phase = VoiceModePhase::Speaking;
        tracing::debug!(
            "Narrate section: cache miss, starting TTS worker for text ({} chars)",
            cleaned.len()
        );

        // Track narration for chunk collection / caching.
        state.narrating_section = Some((document_id, section_index, content_hash));
        state.narrating_heading_words = heading_words;
        state.selection_word_offset = selection_word_offset;
        state.narrating_cleaned_text = Some(cleaned.clone());
        state.narrating_chunks.clear();

        let mut sentence_buf = SentenceBuffer::new();
        let mut sentences = sentence_buf.push(&cleaned);
        if let Some(remaining) = sentence_buf.flush() {
            sentences.push(remaining);
        }

        // Start the persistent TTS worker if not running.
        if state.tts_worker_tx.is_none() {
            let vc = narration_vc;
            let tx = self.app_event_tx.clone();
            let in_flight = state.tts_in_flight.clone();
            let gen_ref = state.tts_generation.clone();
            let spawn_gen = gen_ref.load(Ordering::SeqCst);
            in_flight.fetch_add(1, Ordering::SeqCst);

            let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
            state.tts_worker_tx = Some(worker_tx);

            tokio::spawn(async move {
                tts_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
            });
        }

        // Send all sentences and signal finish.
        if let Some(ref worker_tx) = state.tts_worker_tx {
            for sentence in sentences {
                let _ = worker_tx.send(TtsWorkerCommand::SendText(sentence));
            }
            let _ = worker_tx.send(TtsWorkerCommand::Finish);
        }
        state.tts_worker_tx = None;

        self.sync_voice_placeholder();
    }

    /// Handle a prefetch request: generate TTS in background, cache result.
    pub(crate) fn on_voice_prefetch_section(
        &mut self,
        document_id: String,
        section_index: usize,
        raw_text: String,
    ) {
        let Some(ref state) = self.voice_mode_state else {
            return;
        };
        if !state.should_tts() {
            return;
        }

        let cleaned = clean_for_tts(&raw_text);
        if cleaned.is_empty() {
            return;
        }

        let content_hash = hash_text(&cleaned);
        let key = (document_id, section_index);

        // Already cached with matching hash?
        if let Ok(cache) = state.tts_section_cache.lock()
            && let Some(entry) = cache.get(&key)
            && entry.content_hash == content_hash
        {
            return; // Already cached.
        }

        // Already being prefetched?
        if let Ok(mut pending) = state.prefetch_pending.lock()
            && !pending.insert(key.clone())
        {
            return; // Prefetch already in progress.
        }

        // Split into sentences.
        let mut sentence_buf = SentenceBuffer::new();
        let mut sentences = sentence_buf.push(&cleaned);
        if let Some(remaining) = sentence_buf.flush() {
            sentences.push(remaining);
        }

        let vc = self.effective_voice_config();
        let cache = state.tts_section_cache.clone();
        let pending = state.prefetch_pending.clone();

        tokio::spawn(async move {
            let mut all_chunks = Vec::new();
            let mut all_timeline = Vec::new();
            for sentence in &sentences {
                match prefetch_sentence_tts(&vc, sentence).await {
                    Ok((chunks, timeline)) => {
                        all_chunks.extend(chunks);
                        all_timeline.extend(timeline);
                    }
                    Err(e) => {
                        tracing::error!("TTS prefetch error: {e}");
                        // Remove from pending on failure.
                        if let Ok(mut p) = pending.lock() {
                            p.remove(&key);
                        }
                        return;
                    }
                }
            }
            // Write to cache with alignment for karaoke on replay.
            if let Ok(mut c) = cache.lock() {
                c.insert(
                    key.clone(),
                    TtsCacheEntry {
                        content_hash,
                        chunks: all_chunks,
                        alignment_timeline: all_timeline,
                    },
                );
            }
            // Remove from pending.
            if let Ok(mut p) = pending.lock() {
                p.remove(&key);
            }
        });
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
        // Citation annotations (\u{e200}...\u{e201}) — strip them entirely.
        // The reading view's render pipeline also strips these, so omitting them
        // here keeps the TTS word sequence aligned with the rendered word counter.
        if ch == '\u{e200}' {
            for inner in chars.by_ref() {
                if inner == '\u{e201}' {
                    break;
                }
            }
            continue;
        }
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

    // Collapse 3+ consecutive newlines to 2 (preserve paragraph breaks).
    let mut collapsed = String::with_capacity(out.len());
    let mut consecutive_newlines = 0u32;
    for ch in out.chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                collapsed.push('\n');
            }
        } else {
            consecutive_newlines = 0;
            collapsed.push(ch);
        }
    }

    // Strip horizontal rule lines (--- or more dashes) so they don't create
    // extra TTS words that have no counterpart in the rendered view (where
    // horizontal rules render as "———" which the word counter skips).
    let collapsed = collapsed
        .lines()
        .filter(|line| {
            let t = line.trim();
            // Keep empty lines and lines that aren't just dashes.
            t.is_empty() || !t.chars().all(|c| c == '-')
        })
        .collect::<Vec<_>>()
        .join("\n");

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

/// Generate TTS for a sentence without sending audio events (for prefetching).
/// Collects PCM chunks and alignment timeline entries.
async fn prefetch_sentence_tts(
    voice_config: &codex_core::config::types::VoiceModeToml,
    sentence: &str,
) -> Result<(Vec<Vec<i16>>, Vec<AlignmentEntry>), codex_elevenlabs::ElevenLabsError> {
    let mut rx = start_tts_generation(voice_config, sentence)?;
    let mut chunks = Vec::new();
    let mut timeline = Vec::new();
    let mut pending_word: Option<AlignmentEntry> = None;
    while let Some(chunk) = rx.recv().await {
        if let Some(ref align) = chunk.alignment {
            build_alignment_entries(align, 0, &mut timeline, &mut pending_word);
        }
        chunks.push(chunk.pcm);
    }
    if let Some(pw) = pending_word {
        timeline.push(pw);
    }
    Ok((chunks, timeline))
}

/// Start TTS generation in a background task, returning a channel that
/// receives `TtsChunk`s (PCM + alignment) as they arrive from ElevenLabs.
fn start_tts_generation(
    voice_config: &codex_core::config::types::VoiceModeToml,
    sentence: &str,
) -> Result<
    tokio::sync::mpsc::UnboundedReceiver<codex_elevenlabs::TtsChunk>,
    codex_elevenlabs::ElevenLabsError,
> {
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
        config.language_code = el.language_code.clone();
        config.speed = el.speed;
    }

    let sentence = sentence.to_string();
    let (chunk_tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::spawn(async move {
        let stream_result = codex_elevenlabs::tts::TtsStream::connect(&config).await;
        let mut stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("TTS connect error: {e}");
                return;
            }
        };

        let send_result = async {
            stream.send_text(&sentence).await?;
            stream.flush().await?;
            Ok::<(), codex_elevenlabs::ElevenLabsError>(())
        }
        .await;

        if let Err(e) = send_result {
            tracing::error!("TTS send error: {e}");
            return;
        }

        while let Some(chunk) = stream.recv_audio().await {
            if chunk_tx.send(chunk).is_err() {
                break; // Receiver dropped (interrupted).
            }
        }

        // Drop sender BEFORE closing the WebSocket so the consumer
        // unblocks immediately and can start the next sentence.
        // The WebSocket close handshake can take hundreds of ms.
        drop(chunk_tx);
        stream.close().await;
    });

    Ok(chunk_rx)
}

/// Truncate an error message for display, keeping the first `max_len` chars.
fn truncate_error(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

/// Compute a simple hash of text for cache invalidation.
fn hash_text(text: &str) -> u64 {
    use std::hash::Hash;
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

// ─── Persistent TTS worker (single WebSocket per voice turn) ─────────────────

/// Commands sent to the persistent TTS worker task.
#[derive(Debug)]
pub(crate) enum TtsWorkerCommand {
    /// Send a sentence to TTS via the existing WebSocket.
    SendText(String),
    /// Flush remaining audio and shut down the connection.
    Finish,
}

/// Long-lived TTS task that maintains a single ElevenLabs WebSocket connection.
/// Sentences are sent through the same connection via `send_text` + `flush`,
/// eliminating per-sentence connection overhead (DNS + TLS + WS handshake).
async fn tts_worker_loop(
    voice_config: codex_core::config::types::VoiceModeToml,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TtsWorkerCommand>,
    event_tx: crate::app_event_sender::AppEventSender,
    in_flight: Arc<AtomicUsize>,
    gen_ref: Arc<AtomicUsize>,
    my_gen: usize,
) {
    let api_key = voice_config
        .elevenlabs
        .as_ref()
        .and_then(|e| e.api_key.clone())
        .or_else(|| std::env::var("ELEVENLABS_API_KEY").ok());
    let Some(api_key) = api_key else {
        tracing::error!("TTS worker: missing API key");
        event_tx.send(AppEvent::VoiceModeTtsError {
            error: "Missing ElevenLabs API key".to_string(),
        });
        if in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            event_tx.send(AppEvent::VoiceModeTtsFinished);
        }
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
        config.language_code = el.language_code.clone();
        config.speed = el.speed;
    }

    let mut stream = match codex_elevenlabs::tts::TtsStream::connect(&config).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("TTS worker connect: {e}");
            event_tx.send(AppEvent::VoiceModeTtsError {
                error: format!("TTS connection failed: {e}"),
            });
            if in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
                event_tx.send(AppEvent::VoiceModeTtsFinished);
            }
            return;
        }
    };

    tracing::debug!("TTS worker started (gen={my_gen})");
    let mut finishing = false;

    loop {
        if finishing {
            // Drain remaining audio after close request.
            match stream.recv_audio().await {
                Some(chunk) if gen_ref.load(Ordering::SeqCst) == my_gen => {
                    event_tx.send(AppEvent::VoiceModeTtsAudioChunk {
                        pcm: chunk.pcm,
                        alignment: chunk.alignment,
                    });
                }
                Some(_) => {} // stale generation
                None => break,
            }
        } else {
            tokio::select! {
                biased;

                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(TtsWorkerCommand::SendText(text)) => {
                            if gen_ref.load(Ordering::SeqCst) != my_gen { break; }
                            if let Err(e) = stream.send_text(&text).await {
                                tracing::error!("TTS worker send: {e}");
                                break;
                            }
                            if let Err(e) = stream.flush().await {
                                tracing::error!("TTS worker flush: {e}");
                                break;
                            }
                        }
                        Some(TtsWorkerCommand::Finish) => {
                            let _ = stream.flush().await;
                            // Send EOS without closing — the server finishes
                            // generating audio and sends is_final, which causes
                            // recv_audio() to return None.
                            stream.send_eos().await;
                            finishing = true;
                        }
                        None => {
                            // Sender dropped (interrupted) — exit immediately.
                            break;
                        }
                    }
                }

                chunk = stream.recv_audio() => {
                    match chunk {
                        Some(chunk) if gen_ref.load(Ordering::SeqCst) == my_gen => {
                            event_tx.send(AppEvent::VoiceModeTtsAudioChunk {
                                pcm: chunk.pcm,
                                alignment: chunk.alignment,
                            });
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            }
        }
    }

    tracing::debug!("TTS worker exiting (gen={my_gen})");
    if gen_ref.load(Ordering::SeqCst) == my_gen && in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
        event_tx.send(AppEvent::VoiceModeTtsFinished);
    }
}

// ─── Alignment timeline builder ──────────────────────────────────────────────

/// Build word-level `AlignmentEntry`s from a chunk's alignment data.
///
/// Groups consecutive characters into words (splitting on whitespace),
/// using the alignment's absolute timestamps directly (ElevenLabs
/// `sync_alignment` timestamps are session-absolute).
///
/// Words that span chunk boundaries are handled via `pending_word`: if the
/// chunk ends mid-word the partial entry is stored there, and on the next
/// call the continuation is merged in.
fn build_alignment_entries(
    align: &codex_elevenlabs::TtsAlignment,
    _cumulative_ms: u64,
    timeline: &mut Vec<AlignmentEntry>,
    pending_word: &mut Option<AlignmentEntry>,
) {
    let n = align
        .chars
        .len()
        .min(align.char_start_times_ms.len())
        .min(align.char_durations_ms.len());
    if n == 0 {
        return;
    }

    // Group characters into words, tracking the first and last char index per word.
    let mut word_start_idx: Option<usize> = None;
    let mut word_end_idx: usize = 0;
    let mut word_chars: Vec<&str> = Vec::new();
    let mut is_first_word = true;

    let flush_word = |start_idx: usize,
                      end_idx: usize,
                      chars: &mut Vec<&str>,
                      timeline: &mut Vec<AlignmentEntry>,
                      pending: &mut Option<AlignmentEntry>,
                      first: &mut bool| {
        let abs_start = align.char_start_times_ms[start_idx];
        let last_start = align.char_start_times_ms[end_idx];
        let last_dur = align.char_durations_ms[end_idx];
        let abs_end = last_start + last_dur;
        let word: String = chars.iter().copied().collect();
        chars.clear();

        // If this is the first word in the chunk and there's a pending
        // partial word from the previous chunk, merge them.
        if *first {
            *first = false;
            if let Some(mut prev) = pending.take() {
                prev.word.push_str(&word);
                prev.duration_ms = abs_end.saturating_sub(prev.start_ms);
                timeline.push(prev);
                return;
            }
        }

        timeline.push(AlignmentEntry {
            start_ms: abs_start,
            duration_ms: abs_end.saturating_sub(abs_start),
            word,
        });
    };

    // Track whether we've seen any non-whitespace character in this chunk.
    // This distinguishes true leading whitespace (pending word is complete)
    // from whitespace that follows the first word (pending word was continued).
    let mut seen_non_ws = false;

    for i in 0..n {
        let ch = align.chars[i].as_str();
        if ch.trim().is_empty() {
            // Leading whitespace (before any non-ws char) means the pending
            // word from the previous chunk is complete — flush it standalone.
            if is_first_word && !seen_non_ws {
                is_first_word = false;
                if let Some(prev) = pending_word.take() {
                    timeline.push(prev);
                }
            }
            // Whitespace: flush current word if any.
            if let Some(ws) = word_start_idx.take() {
                flush_word(
                    ws,
                    word_end_idx,
                    &mut word_chars,
                    timeline,
                    pending_word,
                    &mut is_first_word,
                );
            }
        } else {
            seen_non_ws = true;
            if word_start_idx.is_none() {
                word_start_idx = Some(i);
            }
            word_end_idx = i;
            word_chars.push(ch);
        }
    }

    // Final partial word: might span into the next chunk.
    // Check if the chunk ended mid-word (last char was not whitespace).
    if let Some(ws) = word_start_idx {
        let last_char = align.chars.last().map(String::as_str).unwrap_or("");
        let ends_mid_word = !last_char.trim().is_empty();

        if ends_mid_word {
            // Save as pending — might continue in the next chunk.
            let abs_start = align.char_start_times_ms[ws];
            let last_start = align.char_start_times_ms[word_end_idx];
            let last_dur = align.char_durations_ms[word_end_idx];
            let abs_end = last_start + last_dur;
            let word: String = word_chars.iter().copied().collect();

            // Merge with existing pending if this is the first (and only) word.
            if is_first_word && let Some(prev) = pending_word.as_mut() {
                prev.word.push_str(&word);
                prev.duration_ms = abs_end.saturating_sub(prev.start_ms);
                return;
            }

            *pending_word = Some(AlignmentEntry {
                start_ms: abs_start,
                duration_ms: abs_end.saturating_sub(abs_start),
                word,
            });
        } else {
            // Chunk ends with whitespace after this word — flush it.
            flush_word(
                ws,
                word_end_idx,
                &mut word_chars,
                timeline,
                pending_word,
                &mut is_first_word,
            );
        }
    } else if is_first_word {
        // Chunk was all whitespace but we have a pending word — flush it now.
        if let Some(prev) = pending_word.take() {
            timeline.push(prev);
        }
    }
}

/// Find the alignment entry active at the given playback position.
/// Returns `Some(idx)` — the index into the timeline — if a word is active.
fn find_active_word(timeline: &[AlignmentEntry], pos_ms: u64) -> Option<usize> {
    if timeline.is_empty() {
        return None;
    }

    // Binary search: find the last entry whose start_ms <= pos_ms.
    let idx = match timeline.binary_search_by_key(&pos_ms, |e| e.start_ms) {
        Ok(i) => i,
        Err(0) => return None, // Before first word.
        Err(i) => i - 1,
    };

    let entry = &timeline[idx];
    // Check that we're within this word's duration.
    if pos_ms <= entry.start_ms + entry.duration_ms {
        Some(idx)
    } else {
        None // In the gap between words.
    }
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
            vec!["Hello world.", "This is a test!", "Are you ready?",]
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
        assert_eq!(buf.flush(), Some("partial text without ending".to_string()));
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
        assert_eq!(r.display_text, "Hello world. Some code here.");
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
        assert_eq!(r2.display_text, "Hello.");
        assert_eq!(r2.voice_sentences, vec!["Hello."]);
    }

    #[test]
    fn voice_tag_multiple_regions() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>First.</voice> code <voice>Second.</voice>");
        assert_eq!(r.display_text, "First. code Second.");
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
        assert_eq!(r.display_text, "Partial content");
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
        assert_eq!(r1.display_text, "Hello ");
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
        assert_eq!(r.display_text, "Fresh start.");
        assert_eq!(r.voice_sentences, vec!["Fresh start."]);
    }

    #[test]
    fn voice_tag_closing_tag_split() {
        let mut parser = VoiceTagParser::new();

        // The closing </voice> tag is split: "</vo" in first delta, "ice>" in second.
        let r1 = parser.push("<voice>Done.</vo");
        assert_eq!(r1.display_text, "Done.");
        // "Done." is in voice_buffer but </voice> hasn't closed yet and there's
        // no sentence boundary (period needs trailing space), so no sentence yet.
        assert!(r1.voice_sentences.is_empty());

        let r2 = parser.push("ice> Next text.");
        assert_eq!(r2.display_text, " Next text.");
        // </voice> closes the region and flushes "Done." as a sentence.
        assert_eq!(r2.voice_sentences, vec!["Done."]);
    }

    // ─── clean_for_tts tests ─────────────────────────────────────────────

    #[test]
    fn clean_for_tts_strips_citation_annotations() {
        let text = "Before \u{e200}F:/path.rs\u{2020}L1\u{e201} After";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Before  After");
    }

    #[test]
    fn clean_for_tts_strips_horizontal_rules() {
        let text = "Before\n\n---\n\nAfter";
        let cleaned = clean_for_tts(text);
        // The --- line is removed, leaving an extra blank line that the
        // newline collapser already capped at 2. The result has the
        // surrounding paragraph breaks preserved.
        assert!(!cleaned.contains("---"));
        assert!(cleaned.contains("Before"));
        assert!(cleaned.contains("After"));
    }

    #[test]
    fn clean_for_tts_keeps_dashes_in_words() {
        // Dashes within text (not standalone lines) should be kept.
        let text = "Llama-2-13B uses top-k=2";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Llama-2-13B uses top-k=2");
    }

    #[test]
    fn clean_for_tts_heading_after_newline() {
        // Markdown heading at the start of a new line should be stripped.
        let text = "Results.\n### General benchmarks\n\nThe model achieved";
        let cleaned = clean_for_tts(text);
        assert_eq!(
            cleaned,
            "Results.\nGeneral benchmarks\n\nThe model achieved"
        );
    }
}
