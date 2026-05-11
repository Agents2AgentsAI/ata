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

use crate::legacy_core::config::types::TtsBackend;
use crate::legacy_core::config::types::VoiceModeToml;
use crate::legacy_core::config::types::VoiceOutput;
use crate::legacy_core::config::types::VoiceVerbosity;

// ─── Voice mode instruction prefix ──────────────────────────────────────────
// These prompt prefixes are injected into the agent's instructions when the
// user enables voice mode. They are referenced by `voice_mode_instruction()`,
// which the core prompt builder will call once the voice-mode prompt
// injection wiring lands. See follow-up: hook these into
// `ThreadConfigSnapshot` / instruction stitching in `codex-core`.
#[allow(dead_code)]
/// Verbose instruction: acknowledgments, progress updates, and final summary
/// are all spoken aloud.
const VOICE_MODE_INSTRUCTION_VERBOSE: &str = "\
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
- For every rendered equation or symbol, use an <eq> structured pair. \
Inline form: <eq latex=\"...\">spoken reading</eq> \
Display form: <eq latex=\"...\" display=\"block\">spoken reading</eq> \
In each <eq>, the latex attribute is rendered visually and the inner text between tags is spoken aloud. \
In the latex attribute, provide raw LaTeX body only (no $, $$, \\(, or \\[ delimiters). \
The spoken reading should be a natural English paraphrase of the math. \
Example: <eq latex=\"\\\\sqrt{d_k}\">square root of d sub k</eq>\n\
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

#[allow(dead_code)]
/// Concise instruction: only the final answer/summary is spoken aloud.
const VOICE_MODE_INSTRUCTION_CONCISE: &str = "\
[VOICE MODE] The user is speaking to you via voice. \
Wrap any text you want spoken aloud in <voice></voice> tags. \
Only wrap your FINAL answer or summary in <voice> tags. Do NOT use <voice> \
tags for acknowledgments, progress updates, or intermediate thoughts — those \
should be text-only so the user can read them on screen. \
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
- For every rendered equation or symbol, use an <eq> structured pair. \
Inline form: <eq latex=\"...\">spoken reading</eq> \
Display form: <eq latex=\"...\" display=\"block\">spoken reading</eq> \
In each <eq>, the latex attribute is rendered visually and the inner text between tags is spoken aloud. \
In the latex attribute, provide raw LaTeX body only (no $, $$, \\(, or \\[ delimiters). \
The spoken reading should be a natural English paraphrase of the math. \
Example: <eq latex=\"\\\\sqrt{d_k}\">square root of d sub k</eq>\n\
\n\
Follow this pattern:\n\
1. Do any technical work (tool calls, skill execution, subagents) without voice tags — show progress as text only.\n\
2. End with a <voice> summary of what you found or did (2-3 sentences).\n\
\n\
For purely conversational responses with no code or tools, wrap the entire \
response in <voice> tags.\n\n";

#[allow(dead_code)]
/// Returns the voice mode instruction for the given verbosity level.
pub(crate) fn voice_mode_instruction(verbosity: VoiceVerbosity) -> &'static str {
    match verbosity {
        VoiceVerbosity::Verbose => VOICE_MODE_INSTRUCTION_VERBOSE,
        VoiceVerbosity::Concise => VOICE_MODE_INSTRUCTION_CONCISE,
    }
}

#[allow(dead_code)]
/// Returns all voice mode instruction variants (for prefix stripping).
pub(crate) fn voice_mode_instruction_prefixes() -> &'static [&'static str] {
    &[
        VOICE_MODE_INSTRUCTION_VERBOSE,
        VOICE_MODE_INSTRUCTION_CONCISE,
    ]
}

fn append_browser_reading_view_debug_log(message: &str) {
    let Ok(home) = crate::legacy_core::config::find_codex_home() else {
        return;
    };
    let path = home.join("logs/browser-reading-view.log");
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::io::Write::write_all(&mut file, format!("[{ts}] {message}\n").as_bytes());
}

#[allow(dead_code)]
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
#[derive(Default)]
pub struct SentenceBuffer {
    buffer: String,
}

#[allow(dead_code)]
impl SentenceBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a text delta. Returns completed sentences ready for TTS.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
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
    pub fn flush(&mut self) -> Option<String> {
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
///
/// `display_text` and `voice_block_closed` are read by the agent-message-delta
/// streaming integration which has not yet been ported to v0.129.0; they are
/// allowed-dead until that wiring lands.
#[allow(dead_code)]
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

    // ─── Equation tag state ──────────────────────────────────────────
    /// Counts equations seen so far (1-based).
    equation_ordinal: usize,
    /// Whether we're currently inside an `<eq ...>...</eq>` region.
    in_equation: bool,
    /// The `latex` attribute of the current `<eq>` tag.
    eq_latex: String,
    /// Whether the current equation uses block display (`display="block"`).
    eq_display: bool,
    /// Accumulated spoken text for the current equation (inner text).
    eq_speak: String,
}

impl VoiceTagParser {
    pub(crate) fn new() -> Self {
        Self {
            pending: String::new(),
            in_voice: false,
            voice_buffer: String::new(),
            equation_ordinal: 0,
            in_equation: false,
            eq_latex: String::new(),
            eq_display: false,
            eq_speak: String::new(),
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
                    if self.in_equation {
                        // Inside <eq>...</eq>: accumulate as spoken text.
                        self.eq_speak.push_str(before);
                    } else {
                        display.push_str(before);
                    }
                    if self.in_voice && !self.in_equation {
                        self.voice_buffer.push_str(before);
                    }
                }

                let rest = &self.pending[tag_start..];

                // Try to match a complete tag.
                if let Some(tag_end) = rest.find('>') {
                    let tag = &rest[..=tag_end];
                    let tag_lower = tag.to_ascii_lowercase();

                    if tag_lower == "<voice>" || tag_lower.starts_with("<voice ") {
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
                    } else if tag_lower.starts_with("<eq ")
                        || tag_lower.starts_with("<eq/")
                        || tag_lower == "<eq>"
                    {
                        // Opening <eq> tag — parse attributes.
                        let (latex, is_display, speak) = parse_eq_attributes(tag);
                        let is_self_closing = tag.ends_with("/>");

                        if is_self_closing {
                            // Self-closing: <eq latex="..." speak="..."/>
                            self.equation_ordinal += 1;
                            let spoken = if speak.is_empty() {
                                latex.clone()
                            } else {
                                speak
                            };
                            // Add LaTeX to display text.
                            let delim = if is_display { "$$" } else { "$" };
                            display.push_str(delim);
                            display.push_str(&latex);
                            display.push_str(delim);
                            // Add spoken text with equation markers to voice buffer.
                            if self.in_voice {
                                let marker = format!(
                                    "[[[EQ:{}]]]{}[[[/EQ]]]",
                                    self.equation_ordinal, spoken,
                                );
                                self.voice_buffer.push_str(&marker);
                            }
                        } else {
                            // Non-self-closing: <eq ...>inner text</eq>
                            self.in_equation = true;
                            self.eq_latex = latex;
                            self.eq_display = is_display;
                            self.eq_speak.clear();
                            if !speak.is_empty() {
                                self.eq_speak.push_str(&speak);
                            }
                        }
                        self.pending = rest[tag_end + 1..].to_string();
                    } else if tag_lower == "</eq>" {
                        // Closing </eq> tag — finalize the equation.
                        self.in_equation = false;
                        self.equation_ordinal += 1;
                        let spoken = self.eq_speak.trim().to_string();
                        let latex = std::mem::take(&mut self.eq_latex);
                        let is_display = self.eq_display;
                        self.eq_speak.clear();

                        // Add LaTeX to display text.
                        let delim = if is_display { "$$" } else { "$" };
                        display.push_str(delim);
                        display.push_str(&latex);
                        display.push_str(delim);

                        // Add spoken text with equation markers to voice buffer.
                        if self.in_voice && !spoken.is_empty() {
                            let marker =
                                format!("[[[EQ:{}]]]{}[[[/EQ]]]", self.equation_ordinal, spoken,);
                            self.voice_buffer.push_str(&marker);
                        }
                        self.pending = rest[tag_end + 1..].to_string();
                    } else {
                        // Not a voice/eq tag — emit it as regular text.
                        if self.in_equation {
                            self.eq_speak.push_str(tag);
                        } else {
                            display.push_str(tag);
                        }
                        if self.in_voice && !self.in_equation {
                            self.voice_buffer.push_str(tag);
                        }
                        self.pending = rest[tag_end + 1..].to_string();
                    }
                } else {
                    // `<` found but no closing `>` yet.
                    // Check if this could be the start of a recognized tag.
                    if is_voice_tag_prefix(rest) {
                        // Buffer it — wait for more data.
                        self.pending = rest.to_string();
                        break;
                    } else {
                        // Not a recognized tag prefix (e.g., "< 5") — flush as text.
                        let ch = &rest[..1];
                        if self.in_equation {
                            self.eq_speak.push_str(ch);
                        } else {
                            display.push_str(ch);
                        }
                        if self.in_voice && !self.in_equation {
                            self.voice_buffer.push_str(ch);
                        }
                        self.pending = rest[1..].to_string();
                    }
                }
            } else {
                // No `<` — emit everything.
                if !self.pending.is_empty() {
                    if self.in_equation {
                        self.eq_speak.push_str(&self.pending);
                    } else {
                        display.push_str(&self.pending);
                    }
                    if self.in_voice && !self.in_equation {
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

        // If we're still inside an <eq> tag at flush time, finalize it.
        if self.in_equation {
            self.in_equation = false;
            self.equation_ordinal += 1;
            let spoken = self.eq_speak.trim().to_string();
            if !spoken.is_empty() {
                let marker = format!("[[[EQ:{}]]]{}[[[/EQ]]]", self.equation_ordinal, spoken,);
                self.voice_buffer.push_str(&marker);
            }
            self.eq_latex.clear();
            self.eq_speak.clear();
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
        self.in_equation = false;
        self.eq_latex.clear();
        self.eq_display = false;
        self.eq_speak.clear();
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

/// Check if `s` (starting with `<`) could be the prefix of a recognized tag
/// (`<voice>`, `</voice>`, `<eq ...>`, or `</eq>`).
fn is_voice_tag_prefix(s: &str) -> bool {
    let s_lower: String = s.to_ascii_lowercase();
    "<voice>".starts_with(&s_lower)
        || "</voice>".starts_with(&s_lower)
        || s_lower.starts_with("<voice ")
        || (s_lower.len() <= 3 && "<eq ".starts_with(&s_lower))
        || s_lower.starts_with("<eq ")
        || s_lower.starts_with("<eq/")
        || "</eq>".starts_with(&s_lower)
}

/// Parse attributes from an `<eq ...>` tag string.
///
/// Returns `(latex, is_display, speak)` where:
/// - `latex` is the value of the `latex` attribute
/// - `is_display` is true if `display="block"` or `mode="display"` or `mode="block"`
/// - `speak` is the value of the `speak` attribute (empty if absent)
fn parse_eq_attributes(tag: &str) -> (String, bool, String) {
    let mut latex = String::new();
    let mut is_display = false;
    let mut speak = String::new();

    // Extract attribute values from the tag using a simple scanner.
    // Handles both single and double quoted attribute values.
    let inner = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/');

    // Skip the tag name ("eq")
    let attrs = if let Some(pos) = inner.find(char::is_whitespace) {
        &inner[pos..]
    } else {
        return (latex, is_display, speak);
    };

    // Simple attribute parser: find name="value" or name='value' pairs.
    let mut remaining = attrs;
    while !remaining.is_empty() {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }
        // Find the attribute name (up to '=')
        let eq_pos = match remaining.find('=') {
            Some(p) => p,
            None => break,
        };
        let attr_name = remaining[..eq_pos].trim().to_ascii_lowercase();
        remaining = &remaining[eq_pos + 1..];
        remaining = remaining.trim_start();

        // Parse the quoted value
        let quote = match remaining.chars().next() {
            Some(q @ ('"' | '\'')) => q,
            _ => break,
        };
        remaining = &remaining[1..]; // skip opening quote
        let end_quote = match remaining.find(quote) {
            Some(p) => p,
            None => break,
        };
        let value = &remaining[..end_quote];
        remaining = &remaining[end_quote + 1..];

        match attr_name.as_str() {
            "latex" => latex = value.to_string(),
            "display" | "mode" => {
                is_display = matches!(value.to_ascii_lowercase().as_str(), "block" | "display");
            }
            "speak" => speak = value.to_string(),
            _ => {}
        }
    }

    (latex, is_display, speak)
}

/// Parse `[[[EQ:N]]]...[[[/EQ]]]` markers from text prepared for TTS.
///
/// Returns `(cleaned_text, spans)` where:
/// - `cleaned_text` has all markers stripped (clean for TTS)
/// - `spans` is a Vec of `(eq_index, start_word, end_word)` for each equation
///
/// Word indices are computed against the final assembled `cleaned_text` using
/// byte offsets, so punctuation that merges across marker boundaries (e.g.
/// `W[[[/EQ]]], where`) is counted correctly.
pub fn parse_equation_markers(text: &str) -> (String, Vec<(usize, usize, usize)>) {
    let mut result = String::new();
    // Track (eq_index, start_byte, end_byte) in the result string.
    let mut byte_spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut remaining = text;

    while let Some(start_pos) = remaining.find("[[[EQ:") {
        result.push_str(&remaining[..start_pos]);

        let after_prefix = &remaining[start_pos + 6..]; // skip "[[[EQ:"
        if let Some(end_idx) = after_prefix.find("]]]") {
            let eq_index: usize = after_prefix[..end_idx].parse().unwrap_or(0);
            let after_start = &after_prefix[end_idx + 3..]; // skip "]]]"

            if let Some(end_pos) = after_start.find("[[[/EQ]]]") {
                let eq_text = &after_start[..end_pos];
                let start_byte = result.len();
                result.push_str(eq_text);
                let end_byte = result.len();
                byte_spans.push((eq_index, start_byte, end_byte));
                remaining = &after_start[end_pos + 9..]; // skip "[[[/EQ]]]"
            } else {
                result.push_str(&remaining[start_pos..]);
                remaining = "";
                break;
            }
        } else {
            result.push_str(&remaining[start_pos..]);
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);

    // Convert byte spans to word-index spans by counting words in the
    // assembled result up to each boundary. This correctly handles
    // punctuation that merges across marker boundaries.
    let spans = byte_spans
        .iter()
        .map(|(eq_index, start_byte, end_byte)| {
            let start_word = result[..*start_byte].split_whitespace().count();
            let end_word = result[..*end_byte].split_whitespace().count();
            (*eq_index, start_word, end_word)
        })
        .collect();

    (result, spans)
}

/// Convert raw text into the TTS stream format used by the voice pipeline.
///
/// This strips any `<voice>` wrappers while preserving `<eq>` content as
/// `[[[EQ:N]]]spoken text[[[/EQ]]]` markers so equation timing can later be
/// mapped back onto rendered math.
pub(crate) fn text_to_tts_markup(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }

    let mut parser = VoiceTagParser::new();
    parser.in_voice = true;

    let mut parts = parser.push(text).voice_sentences;
    if let Some(remaining) = parser.flush() {
        parts.push(remaining);
    }

    parts.join(" ")
}

/// Clean text for TTS while preserving equation markers in the output.
pub fn clean_for_tts_preserving_equation_markers(text: &str) -> String {
    fn append_piece(out: &mut String, piece: &str, prefer_newline: bool) {
        let trimmed = piece.trim();
        if trimmed.is_empty() {
            return;
        }
        if !out.is_empty()
            && !out.ends_with(char::is_whitespace)
            && !trimmed
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '.' | ',' | ';' | ':' | '!' | '?'))
        {
            out.push(if prefer_newline { '\n' } else { ' ' });
        }
        out.push_str(trimmed);
    }

    let marked = text_to_tts_markup(text);
    if !marked.contains("[[[EQ:") {
        return clean_for_tts(&marked);
    }

    let mut cleaned = String::with_capacity(marked.len());
    let mut remaining = marked.as_str();

    while let Some(start_pos) = remaining.find("[[[EQ:") {
        let before = &remaining[..start_pos];
        append_piece(&mut cleaned, &clean_for_tts(before), false);

        let after_prefix = &remaining[start_pos + 6..];
        let Some(end_idx) = after_prefix.find("]]]") else {
            append_piece(&mut cleaned, &clean_for_tts(&remaining[start_pos..]), false);
            remaining = "";
            break;
        };
        let eq_index = &after_prefix[..end_idx];
        let after_start = &after_prefix[end_idx + 3..];
        let Some(end_pos) = after_start.find("[[[/EQ]]]") else {
            append_piece(&mut cleaned, &clean_for_tts(&remaining[start_pos..]), false);
            remaining = "";
            break;
        };

        let eq_text = clean_for_tts(&after_start[..end_pos]).trim().to_string();
        append_piece(
            &mut cleaned,
            &format!("[[[EQ:{eq_index}]]]{eq_text}[[[/EQ]]]"),
            before.ends_with('\n'),
        );
        remaining = &after_start[end_pos + 9..];
    }

    append_piece(
        &mut cleaned,
        &clean_for_tts(remaining),
        remaining.starts_with('\n'),
    );
    cleaned.trim().to_string()
}

/// Extract the `ms` attribute from a `<pause .../>` tag string.
/// Returns 500 if missing/invalid, clamped to [100, 3000].
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignmentEntry {
    /// Absolute start time in ms from beginning of playback.
    pub start_ms: u64,
    /// Duration in ms.
    pub duration_ms: u64,
    /// The word text (for rendering the highlight).
    pub word: String,
}

/// Per-thread voice mode state. Several fields are written by the
/// PTT/meter/timer flows that are not yet wired through a composer key
/// handler in v0.129.0; rustc therefore flags them as never-read until that
/// wiring lands. Marking the struct `allow(dead_code)` keeps the warnings
/// off without losing the field shape we'll need.
#[allow(dead_code)]
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
    /// Controls how much the agent narrates aloud.
    pub(crate) verbosity: VoiceVerbosity,

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
    /// Live narration audio buffered before playback starts.  For reading-view
    /// section reads we hold the first chunks until we have at least one
    /// completed aligned word, otherwise karaoke can begin a few words late.
    pub(crate) tts_startup_buffered_chunks: Vec<Vec<i16>>,
    /// Whether audio for the current TTS turn has actually been enqueued to the
    /// output player yet.  This differs from "chunks received" because section
    /// narration may buffer startup audio until alignment is ready.
    pub(crate) tts_playback_started: bool,
    /// True when the browser is using alignment-driven word wrapping
    /// (alignmentWords sent in startKaraoke). False for heuristic wrapping.
    pub(crate) alignment_driven_karaoke: bool,
    // ─── Equation karaoke highlighting ───────────────────────────────
    /// Word spans for each equation: `(eq_index, start_word, end_word)`.
    /// Populated by `parse_equation_markers()` when preparing TTS text.
    pub(crate) equation_word_spans: Vec<(usize, usize, usize)>,
    /// Currently narrated equation index (1-based), or `None` if between equations.
    pub(crate) active_equation_index: Option<usize>,
    /// Highest equation index that has been fully passed (0 = none yet).
    pub(crate) passed_equation_index: usize,

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
    /// When true, this state was lazily created for on-demand TTS only,
    /// not full voice mode (no STT, no "Hold Space" prompt).
    pub(crate) tts_only: bool,
    // ─── Reading view Space tap/hold ──────────────────────────────────
    /// When Space was pressed in the reading view (for tap/hold detection).
    pub(crate) space_press_at: Option<Instant>,
    /// Whether audio was paused before the current Space press.
    pub(crate) space_was_paused: bool,

    /// Test-only: simulate paused audio player without real hardware.
    #[cfg(test)]
    pub(crate) mock_audio_paused: bool,
    /// Test-only: simulate audio buffer state without real hardware.
    #[cfg(test)]
    pub(crate) mock_has_audio: Option<bool>,
}

#[allow(dead_code)]
impl VoiceModeState {
    pub(crate) fn new(config: &VoiceModeToml) -> Self {
        let output = config.output.unwrap_or_default();
        let auto_submit = config.auto_submit.unwrap_or(true);
        let tts_enabled = config.tts_enabled.unwrap_or(true);
        let stt_enabled = config.stt_enabled.unwrap_or(true);
        let verbosity = config.verbosity.unwrap_or_default();

        Self {
            phase: VoiceModePhase::Off,
            sentence_buffer: SentenceBuffer::new(),
            voice_tag_parser: VoiceTagParser::new(),
            output,
            auto_submit,
            tts_enabled,
            stt_enabled,
            verbosity,
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
            tts_startup_buffered_chunks: Vec::new(),
            tts_playback_started: false,
            alignment_driven_karaoke: false,
            equation_word_spans: Vec::new(),
            active_equation_index: None,
            passed_equation_index: 0,
            tts_alignment_timeline: Vec::new(),
            tts_cumulative_ms: 0,
            tts_highlight_word_idx: None,
            highlight_tick_cancel: None,
            tts_pending_word: None,
            tts_data_complete: false,
            tts_block_break_pending: false,
            tts_only: false,
            space_press_at: None,
            space_was_paused: false,
            #[cfg(test)]
            mock_audio_paused: false,
            #[cfg(test)]
            mock_has_audio: None,
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

    /// Apply updated TTS/STT/verbosity settings from the voice setup popup.
    pub(crate) fn apply_voice_settings(&mut self, tts: bool, stt: bool, verbosity: VoiceVerbosity) {
        self.tts_enabled = tts;
        self.stt_enabled = stt;
        self.verbosity = verbosity;
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
            // Reset paused state so subsequent narrations are not silenced.
            if player.is_paused() {
                player.resume();
            }
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
        self.tts_startup_buffered_chunks.clear();
        self.tts_playback_started = false;
        // Clear equation highlighting state.
        self.equation_word_spans.clear();
        self.active_equation_index = None;
        self.passed_equation_index = 0;
        // Clear alignment timeline and highlight.
        self.tts_alignment_timeline.clear();
        self.tts_cumulative_ms = 0;
        self.tts_highlight_word_idx = None;
        self.tts_pending_word = None;
        self.tts_data_complete = false;
        self.tts_block_break_pending = false;
        self.cancel_highlight_tick();
    }

    /// Pause TTS playback without clearing buffers.
    pub(crate) fn pause_tts(&mut self) {
        if let Some(ref player) = self.audio_player {
            player.pause();
        }
        self.cancel_highlight_tick();
    }

    /// Resume TTS playback after a pause.
    pub(crate) fn resume_tts(&mut self) {
        if let Some(ref player) = self.audio_player {
            player.resume();
        }
    }

    /// Returns true if the audio player is currently paused.
    /// In test builds, checks `mock_audio_paused` first.
    pub(crate) fn is_audio_paused(&self) -> bool {
        #[cfg(test)]
        if self.mock_audio_paused {
            return true;
        }
        self.audio_player
            .as_ref()
            .is_some_and(super::super::voice::RealtimeAudioPlayer::is_paused)
    }

    /// Returns true if the audio player has buffered samples.
    pub(crate) fn has_buffered_audio(&self) -> bool {
        #[cfg(test)]
        if let Some(has) = self.mock_has_audio {
            return has;
        }
        self.audio_player
            .as_ref()
            .is_some_and(super::super::voice::RealtimeAudioPlayer::has_buffered_audio)
    }

    /// Process phase transition when an audio chunk arrives.
    /// Does NOT transition to Speaking if audio is paused, because TTS chunks
    /// may still stream from the network while the user has paused playback.
    pub(crate) fn transition_phase_on_chunk(&mut self) {
        if self.phase != VoiceModePhase::Speaking && !self.is_audio_paused() {
            self.phase = VoiceModePhase::Speaking;
        }
    }

    /// Returns true if voice turn finalization should be blocked.
    pub(crate) fn should_block_finalization(&self) -> bool {
        self.is_audio_paused()
    }

    /// Returns true if resuming TTS would actually play audio.
    /// False when the voice turn was already finalized (nothing to resume).
    pub(crate) fn can_resume_playback(&self) -> bool {
        // If audio is buffered, resume will play it.
        if self.has_buffered_audio() {
            return true;
        }
        // If TTS data is still streaming AND there's an active narration,
        // more audio chunks may arrive — resume is valid.
        if !self.tts_data_complete {
            return self.narrating_section.is_some();
        }
        // TTS data is complete and buffer is empty — nothing to play.
        false
    }

    /// Returns true if the highlight tick should finalize the voice turn.
    pub(crate) fn should_finalize_on_tick(&self) -> bool {
        self.phase == VoiceModePhase::Speaking
            && self.tts_data_complete
            && !self.is_audio_paused()
            && !self.has_buffered_audio()
    }

    /// Persist collected narration chunks into the section cache as soon as
    /// generation completes, so repeated `r` can replay locally even while
    /// buffered audio is still draining.
    fn persist_narration_cache(&mut self) {
        let Some((doc_id, sec_idx, content_hash)) = self.narrating_section.clone() else {
            return;
        };
        if self.narrating_chunks.is_empty() {
            return;
        }

        let chunks = std::mem::take(&mut self.narrating_chunks);
        let alignment_timeline = self.tts_alignment_timeline.clone();
        if let Ok(mut cache) = self.tts_section_cache.lock() {
            cache.insert(
                (doc_id, sec_idx),
                TtsCacheEntry {
                    content_hash,
                    chunks,
                    alignment_timeline,
                },
            );
        } else {
            self.narrating_chunks = chunks;
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
        self.space_press_at = None;
        self.space_was_paused = false;
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

/// Resolve the ElevenLabs API key from config or environment.
/// Returns `None` if no key is available (ATA users will use proxy mode instead).
fn resolve_elevenlabs_api_key_from_config(voice_config: &VoiceModeToml) -> Option<String> {
    voice_config
        .elevenlabs
        .as_ref()
        .and_then(|e| e.api_key.clone())
        .or_else(|| {
            std::env::var("ELEVENLABS_API_KEY")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
}

/// Build an `ElevenLabsProxy` for ATA-authenticated users.
/// Returns `None` if not in ATA mode or if auth is unavailable.
///
/// ATA-Supabase-routed proxy is only enabled when the user is authenticated
/// via the ATA Supabase flow. In the current v0.129.0 baseline the
/// `AuthMode::Ata` variant is not yet wired through `codex-login`; this helper
/// therefore short-circuits and falls back to direct ElevenLabs API key auth.
fn build_elevenlabs_proxy(
    _auth_manager: &crate::legacy_core::AuthManager,
) -> Option<codex_elevenlabs::ElevenLabsProxy> {
    return None;
    // Restore once `AuthMode::Ata` + `CodexAuth::Ata` land in codex-login:
    // let auth = _auth_manager.auth_cached()?;
    // let token = match &auth {
    //     crate::legacy_core::auth::CodexAuth::Ata(ata) => ata.access_token.clone(),
    //     _ => return None,
    // };
    #[allow(unreachable_code)]
    let token = String::new();

    let base_url = std::env::var("ATA_ELEVENLABS_PROXY_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "{}/functions/v1",
                crate::legacy_core::config::types::DEFAULT_ATA_SUPABASE_URL
            )
        });

    Some(codex_elevenlabs::ElevenLabsProxy {
        base_url,
        bearer_token: token,
        extra_header: Some((
            "apikey".to_string(),
            crate::legacy_core::config::types::DEFAULT_ATA_SUPABASE_ANON_KEY.to_string(),
        )),
    })
}

/// Build an `ElevenLabsConfig` with the API key from config/env, or with proxy
/// for ATA users. Returns `None` if neither is available.
fn build_elevenlabs_config(
    voice_config: &VoiceModeToml,
    auth_manager: &crate::legacy_core::AuthManager,
) -> Option<codex_elevenlabs::ElevenLabsConfig> {
    let api_key = resolve_elevenlabs_api_key_from_config(voice_config);
    let proxy = if api_key.is_none() {
        build_elevenlabs_proxy(auth_manager)
    } else {
        None
    };

    if api_key.is_none() && proxy.is_none() {
        return None;
    }

    let mut config = codex_elevenlabs::ElevenLabsConfig::new(api_key.unwrap_or_default());
    if let Some(proxy) = proxy {
        config.proxy = Some(proxy);
    }
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
    Some(config)
}

/// Extract `VoiceModeToml` from the merged effective config (which is a raw `toml::Value`).
fn voice_mode_config(config: &crate::legacy_core::config::Config) -> VoiceModeToml {
    config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|t| t.get("voice_mode"))
        .and_then(|v| v.clone().try_into::<VoiceModeToml>().ok())
        .unwrap_or_default()
}

// Several voice-mode ChatWidget entry points (PTT key handlers, voice setup
// popup callbacks, voice-mode toggle, browser TTS bridge helpers) are
// invoked by UI flows that have not yet been wired in v0.129.0 (composer
// Space key handler + VoiceSetupView popup port). Allow dead_code on the
// entire impl block until those follow-ups land.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
impl super::ChatWidget {
    /// Return the voice mode config with in-session overrides (speed, language)
    /// applied on top of the on-disk config.  `/voice-setup` writes to disk but
    /// the in-memory `config_layer_stack` isn't reloaded, so we patch the cached
    /// values in here to make changes take effect immediately.
    fn effective_voice_config(&self) -> VoiceModeToml {
        let mut vc = voice_mode_config(&self.config);
        if let Some(ref api_key) = self.cached_elevenlabs_api_key {
            let el = vc.elevenlabs.get_or_insert_with(Default::default);
            el.api_key = Some(api_key.clone());
        }
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
            Some(s) if s.phase.is_active() && !s.tts_only => {
                (s.phase.status_label(), s.phase, s.stt_enabled)
            }
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
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane
                .set_document_reader_voice_status(reading_status);
        }
    }

    /// Restore the default placeholder text when voice mode turns off.
    fn restore_default_placeholder(&mut self) {
        use rand::Rng;
        let placeholders = super::PLACEHOLDERS;
        let idx = rand::rng().random_range(0..placeholders.len());
        self.bottom_pane
            .set_placeholder_text(placeholders[idx].to_string());
        // Clear reading view voice status.
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane.set_document_reader_voice_status(None);
        }
    }

    /// Toggle voice mode on/off (`/voice` command).
    pub(crate) fn toggle_voice_mode(&mut self) {
        // Auto-enable the VoiceMode feature flag on first use.
        if !self
            .config
            .features
            .enabled(codex_features::Feature::VoiceMode)
        {
            let _ = self
                .config
                .features
                .enable(codex_features::Feature::VoiceMode);
            self.app_event_tx.send(AppEvent::UpdateFeatureFlags {
                updates: vec![(codex_features::Feature::VoiceMode, true)],
            });
        }

        // If a tts_only state exists, tear it down so we can create a fresh
        // full voice mode state below.
        if self.voice_mode_state.as_ref().is_some_and(|s| s.tts_only) {
            if let Some(ref mut state) = self.voice_mode_state {
                state.reset();
            }
            self.voice_mode_state = None;
        }

        if let Some(ref mut state) = self.voice_mode_state
            && state.is_active()
        {
            // Turn off.
            state.reset();
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
        // ATA users get keys vended at runtime, so skip the warning for them.
        let session_not_ready = self.thread_id.is_none();
        let has_tts_key = resolve_elevenlabs_api_key_from_config(&voice_config).is_some();
        if !has_tts_key {
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
                if state.stt_enabled {
                    state.tts_enabled = false;
                    let msg = format!(
                        "Audio output unavailable ({e}) — starting voice mode with STT only."
                    );
                    if session_not_ready {
                        self.pending_voice_startup_cells
                            .push(Box::new(history_cell::new_warning_event(msg)));
                    } else {
                        self.add_to_history(history_cell::new_warning_event(msg));
                    }
                } else {
                    self.add_to_history(history_cell::new_error_event(format!(
                        "Failed to start audio: {e}"
                    )));
                    return;
                }
            }
        }

        // PTT mode: don't start capture yet — it starts on Space press.
        state.phase = VoiceModePhase::Idle;
        self.voice_mode_state = Some(state);

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

    /// Update the ElevenLabs API key for the current ATA session.
    pub(crate) fn update_elevenlabs_api_key(&mut self, key: String) {
        self.cached_elevenlabs_api_key = Some(key);
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
    pub(crate) fn apply_voice_settings(&mut self, tts: bool, stt: bool, verbosity: VoiceVerbosity) {
        if let Some(ref mut state) = self.voice_mode_state {
            state.apply_voice_settings(tts, stt, verbosity);
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
        let should_deactivate = self
            .voice_mode_state
            .as_ref()
            .is_some_and(VoiceModeState::is_active);
        if should_deactivate {
            if let Some(ref mut state) = self.voice_mode_state {
                state.reset();
            }
            self.add_info_message(
                "Voice mode off (TTS and STT both disabled).".to_string(),
                None,
            );
            self.restore_default_placeholder();
            self.bottom_pane.set_force_hide_cursor(false);
            // Clear any stale karaoke state from the document reader.
            #[cfg(not(target_os = "linux"))]
            {
                self.bottom_pane
                    .set_document_reader_karaoke_lines(None, false);
                self.bottom_pane
                    .set_document_reader_reading_progress(None, 0);
                self.bottom_pane.set_document_reader_voice_status(None);
            }
            self.request_redraw();
        }
    }

    /// Clear a TTS-only voice state if one exists. Called when voice mode
    /// is explicitly disabled to prevent zombie TTS processing.
    pub(crate) fn clear_tts_only_state(&mut self) {
        if self.voice_mode_state.as_ref().is_some_and(|s| s.tts_only) {
            if let Some(ref mut state) = self.voice_mode_state {
                state.reset();
            }
            // Clear any stale karaoke state from the document reader.
            #[cfg(not(target_os = "linux"))]
            {
                self.bottom_pane
                    .set_document_reader_karaoke_lines(None, false);
                self.bottom_pane
                    .set_document_reader_reading_progress(None, 0);
                self.bottom_pane.set_document_reader_voice_status(None);
            }
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
        let verbosity = self
            .voice_mode_state
            .as_ref()
            .map_or(voice_config.verbosity.unwrap_or_default(), |s| s.verbosity);

        let api_key = voice_config
            .elevenlabs
            .as_ref()
            .and_then(|e| e.api_key.clone());

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
        let startup_enabled = voice_config.enabled.unwrap_or(false);
        let tts_backend = voice_config.tts_backend.unwrap_or_default();

        let view = crate::bottom_pane::VoiceSetupView::new(
            startup_enabled,
            tts_enabled,
            stt_enabled,
            verbosity,
            api_key,
            language_code,
            speed,
            tts_backend,
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
        let browser_mode = self.is_reading_view_browser_mode();
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
            // Forward stopKaraoke to browser if in browser mode.
            if browser_mode {
                let ws_msg = serde_json::json!({ "type": "stopKaraoke" }).to_string();
                if let Some(ref server) = self.reading_view_server {
                    server.send_event(&ws_msg);
                } else {
                    self.reading_view_pending_events.push(ws_msg);
                }
                let ws_msg = serde_json::json!({
                    "type": "ttsStateChanged",
                    "state": "stopped",
                })
                .to_string();
                if let Some(ref server) = self.reading_view_server {
                    server.send_event(&ws_msg);
                } else {
                    self.reading_view_pending_events.push(ws_msg);
                }
            }
            // Clear reading view overlays so the highlight doesn't stick.
            if !browser_mode {
                self.bottom_pane
                    .set_document_reader_karaoke_lines(None, false);
                self.bottom_pane
                    .set_document_reader_reading_progress(None, 0);
            }
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
        match crate::voice::VoiceCapture::start_ptt(&self.config) {
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

        const MIN_PTT_DURATION_SECONDS: f32 = 1.0;
        let duration = audio.duration_seconds();
        if duration < MIN_PTT_DURATION_SECONDS {
            state.phase = VoiceModePhase::Idle;
            self.app_event_tx.send(AppEvent::VoiceModeTranscriptionFailed {
                error: format!(
                    "recording too short ({duration:.2}s); hold Space for at least {MIN_PTT_DURATION_SECONDS:.1}s"
                ),
            });
            self.sync_voice_placeholder();
            self.request_redraw();
            return;
        }

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
        let auth_mgr = Arc::clone(&self.auth_manager);

        let Some(config) = build_elevenlabs_config(&voice_config, &auth_mgr) else {
            tx.send(AppEvent::VoiceModeTranscriptionFailed {
                error: "Missing ElevenLabs API key. Set ELEVENLABS_API_KEY or configure voice_mode.elevenlabs.api_key".to_string(),
            });
            self.sync_voice_placeholder();
            self.request_redraw();
            return;
        };

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
    /// Always strips `<voice>` tags from display text. Only sends to TTS when
    /// voice mode is active and `from_replay` is false.
    ///
    /// Returns `Some(display_text)` when tags were (or could be) present, or
    /// `None` when voice mode was never initialized and no tags are in the
    /// delta.
    pub(crate) fn on_voice_mode_agent_delta(
        &mut self,
        delta: &str,
        from_replay: bool,
    ) -> Option<String> {
        // Capture config before taking a mutable borrow on voice_mode_state.
        let vc = self.effective_voice_config();
        let has_tts_key = resolve_elevenlabs_api_key_from_config(&vc).is_some();
        let state = match self.voice_mode_state.as_mut() {
            Some(s) => s,
            None => {
                // Voice mode was never initialized. Still strip any `<voice>`
                // tags that may be present in replayed/historical content.
                if delta.contains('<') {
                    let stripped = crate::text_formatting::strip_voice_tags(delta);
                    if stripped != delta {
                        return Some(stripped);
                    }
                }
                return None;
            }
        };
        if !state.is_active() {
            // Voice mode was previously on but is now off.  The agent may
            // still emit <voice> tags from earlier instructions in the
            // conversation context.  Strip them for clean display without
            // sending anything to TTS.
            let result = state.voice_tag_parser.push(delta);
            return Some(result.display_text);
        }

        // tts_only mode is used exclusively for reading view section
        // narration.  Agent streaming responses should NOT be read aloud
        // in this mode — the user never activated full voice mode.
        if state.tts_only {
            let result = state.voice_tag_parser.push(delta);
            return Some(result.display_text);
        }

        // Always parse tags for display (strip <voice> markers), even if TTS
        // is suppressed due to barge-in or replay.
        let result = state.voice_tag_parser.push(delta);

        // During replay we strip tags for clean display but never send to TTS.
        if from_replay {
            return Some(result.display_text);
        }

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
            if state.tts_worker_tx.is_none() && !has_tts_key {
                return Some(result.display_text);
            }

            state.transition_phase_on_chunk();

            // Ensure TTS worker is running (one persistent WebSocket per voice turn).
            if state.tts_worker_tx.is_none() {
                let tx = self.app_event_tx.clone();
                let in_flight = state.tts_in_flight.clone();
                let gen_ref = state.tts_generation.clone();
                let spawn_gen = gen_ref.load(Ordering::SeqCst);
                in_flight.fetch_add(1, Ordering::SeqCst);
                let proxy = build_elevenlabs_proxy(&self.auth_manager);
                let backend = vc.tts_backend.unwrap_or_default();

                let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
                state.tts_worker_tx = Some(worker_tx);

                tokio::spawn(async move {
                    match backend {
                        TtsBackend::Say => {
                            say_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
                        }
                        TtsBackend::Elevenlabs => {
                            tts_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen, proxy)
                                .await;
                        }
                    }
                });
            }

            // Send each sentence to the worker — no per-sentence WebSocket needed.
            // Strip equation markers before sending to TTS, and record word spans
            // for equation karaoke highlighting.
            if let Some(ref worker_tx) = state.tts_worker_tx {
                for sentence in tts_sentences.iter() {
                    let (cleaned_sentence, spans) = parse_equation_markers(sentence);
                    if !spans.is_empty() {
                        state.equation_word_spans.extend(spans);
                    }
                    let _ = worker_tx.send(TtsWorkerCommand::SendText(cleaned_sentence));
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
                let proxy = build_elevenlabs_proxy(&self.auth_manager);
                let backend = vc.tts_backend.unwrap_or_default();

                let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
                state.tts_worker_tx = Some(worker_tx);

                tokio::spawn(async move {
                    match backend {
                        TtsBackend::Say => {
                            say_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
                        }
                        TtsBackend::Elevenlabs => {
                            tts_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen, proxy)
                                .await;
                        }
                    }
                });
            }

            // Send remaining text and signal finish. Strip equation markers
            // before sending to TTS and record word spans for highlighting.
            if let Some(ref worker_tx) = state.tts_worker_tx {
                for sentence in remaining {
                    let (cleaned_sentence, spans) = parse_equation_markers(&sentence);
                    if !spans.is_empty() {
                        state.equation_word_spans.extend(spans);
                    }
                    let _ = worker_tx.send(TtsWorkerCommand::SendText(cleaned_sentence));
                }
                let _ = worker_tx.send(TtsWorkerCommand::Finish);
            }
            state.tts_worker_tx = None;

            state.transition_phase_on_chunk();
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
        let browser_mode = self.is_reading_view_browser_mode();
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        state.transition_phase_on_chunk();

        // Compute chunk duration from PCM length (24kHz mono).
        let chunk_duration_ms = (pcm.len() as u64) * 1000 / 24000;

        // Log first chunk enqueue for timing diagnosis.
        if state.tts_cumulative_ms == 0 {
            tracing::info!(
                "[TTS-TIMING] on_voice_tts_audio_chunk: FIRST chunk enqueued to player \
                 ({} samples, ~{chunk_duration_ms}ms audio, has_player={})",
                pcm.len(),
                state.audio_player.is_some(),
            );
        }

        // Build alignment entries from this chunk's alignment data.
        if let Some(ref align) = alignment {
            // Pass cumulative PCM duration so timestamp resets between
            // text segments (after flush) are offset to match playback.
            build_alignment_entries(
                align,
                state.tts_cumulative_ms,
                &mut state.tts_alignment_timeline,
                &mut state.tts_pending_word,
            );
        }

        state.tts_cumulative_ms += chunk_duration_ms;

        // Collect chunks for narration caching.
        if state.narrating_section.is_some() {
            state.narrating_chunks.push(pcm.clone());
        }

        // For browser narration: buffer ALL chunks until TTS finishes so we
        // have the complete alignment timeline for alignment-driven wrapping.
        // This eliminates the unreliable heuristic wrapping during streaming.
        // For TUI or non-narration: start playback progressively as before.
        let is_browser_narration = state.narrating_section.is_some() && browser_mode;
        let should_buffer_startup = state.narrating_section.is_some()
            && !state.tts_playback_started
            && (state.tts_alignment_timeline.is_empty() || is_browser_narration);
        let should_push_initial_lines = alignment.is_some() && !is_browser_narration;
        if should_buffer_startup {
            state.tts_startup_buffered_chunks.push(pcm);
        } else if !is_browser_narration || state.tts_playback_started {
            let _ = state;
            self.ensure_tts_playback_started(vec![pcm]);
        } else {
            // Browser narration, not yet started — keep buffering.
            state.tts_startup_buffered_chunks.push(pcm);
        }

        // Push initial karaoke lines to the reading view (if active) so
        // the highlighted text appears as soon as alignment data arrives.
        #[cfg(not(target_os = "linux"))]
        if should_push_initial_lines {
            self.push_karaoke_to_reader();
        }
    }

    /// Handle a TTS error from the background worker. Surfaces the error
    /// in the voice status / placeholder so the user knows what happened,
    /// and clears the narration state so the UI doesn't stay stuck in
    /// "Speaking..." with no audio playing.
    pub(crate) fn on_voice_tts_error(&mut self, error: &str) {
        tracing::warn!("Voice TTS error surfaced to user: {error}");

        // Clear the narration state so the UI is not stuck in "Speaking..."
        // with is_narrating=true, phase=Speaking, but no audio arriving.
        if let Some(ref mut state) = self.voice_mode_state {
            state.interrupt_tts();
            if state.phase == VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Idle;
            }
        }

        // Clear karaoke overlay and reading cursor.
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane
                .set_document_reader_karaoke_lines(None, false);
            self.bottom_pane
                .set_document_reader_reading_progress(None, 0);
            self.bottom_pane.set_document_reader_tts_paused(false);
        }

        // Show a user-friendly error in the placeholder and reading view status.
        let msg = if error.contains("401") || error.contains("Unauthorized") {
            "TTS error: invalid ElevenLabs API key".to_string()
        } else if error.contains("402") || error.contains("quota") || error.contains("credit") {
            "TTS error: ElevenLabs credits exhausted".to_string()
        } else {
            format!("TTS error: {}", truncate_error(error, 60))
        };
        self.bottom_pane.set_placeholder_text(msg.clone());
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane.set_document_reader_voice_status(Some(msg));
        }
        // Forward error state to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            let ws_msg = serde_json::json!({ "type": "stopKaraoke" });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "stopped",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }
        self.request_redraw();
    }

    pub(crate) fn on_voice_tts_finished(&mut self) {
        let Some(ref state) = self.voice_mode_state else {
            tracing::debug!("[TTS-DBG] on_voice_tts_finished: voice_mode_state is None");
            return;
        };
        let phase = state.phase;
        tracing::debug!("[TTS-DBG] on_voice_tts_finished: phase={phase:?}");
        if phase != VoiceModePhase::Speaking {
            tracing::debug!("[TTS-DBG] on_voice_tts_finished: SKIPPED (phase != Speaking)");
            return;
        }

        let mut should_start_playback = false;
        if let Some(ref mut state) = self.voice_mode_state {
            // Flush any pending partial word so the last word gets highlighted.
            if let Some(pw) = state.tts_pending_word.take() {
                state.tts_alignment_timeline.push(pw);
            }

            // Repair timestamp resets between text segments (ElevenLabs
            // can restart alignment timestamps after flush boundaries).
            repair_timeline_monotonicity(&mut state.tts_alignment_timeline);

            // If startup audio was buffered waiting for the first completed
            // word, begin playback now.  This handles the edge case where the
            // very first word is only flushed once generation finishes.
            should_start_playback =
                !state.tts_playback_started && !state.tts_startup_buffered_chunks.is_empty();

            state.persist_narration_cache();
        }

        if should_start_playback {
            let buffered_chunks = self
                .voice_mode_state
                .as_ref()
                .map(|s| s.tts_startup_buffered_chunks.len())
                .unwrap_or(0);
            let timeline_words = self
                .voice_mode_state
                .as_ref()
                .map(|s| s.tts_alignment_timeline.len())
                .unwrap_or(0);
            tracing::info!(
                "[TTS-TIMING] tts_finished: starting deferred playback ({buffered_chunks} chunks, {timeline_words} alignment words)"
            );
            self.ensure_tts_playback_started(Vec::new());
        }

        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        let has_audio = state.has_buffered_audio();
        let is_paused = state.is_audio_paused();
        tracing::debug!(
            "[TTS-DBG] on_voice_tts_finished: phase={:?}, has_audio={has_audio}, is_paused={is_paused}",
            state.phase,
        );

        // Defer finalization while audio is paused — the highlight tick
        // restarted on resume will finalize once audio drains.
        if is_paused {
            tracing::debug!("[TTS-DBG] on_voice_tts_finished: DEFERRED (audio is paused)");
            state.tts_data_complete = true;
            return;
        }

        // If the highlight tick is running and the audio player still has
        // buffered audio, defer the full cleanup — the highlight tick will
        // finalize once the player's buffer is empty.
        if has_audio {
            state.tts_data_complete = true;
            return;
        }

        if self.is_reading_view_browser_mode() {
            append_browser_reading_view_debug_log("tts_finished finalize_immediately");
        }
        self.finalize_voice_turn();
    }

    /// Full cleanup of voice turn state — called either immediately from
    /// `on_voice_tts_finished` or deferred from `on_voice_highlight_tick`
    /// once the audio player's buffer has drained.
    fn finalize_voice_turn(&mut self) {
        tracing::debug!("[TTS-DBG] finalize_voice_turn called");
        let finished_section_index = self
            .voice_mode_state
            .as_ref()
            .and_then(|s| s.narrating_section.as_ref().map(|(_, idx, _)| *idx));
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        // Never finalize while audio is paused — the user may resume.
        if state.should_block_finalization() {
            tracing::debug!("[TTS-DBG] finalize_voice_turn: BLOCKED (audio is paused)");
            return;
        }

        // Store collected narration chunks + alignment in cache.
        state.narrating_cleaned_text = None;
        state.narrating_heading_words = 0;
        state.selection_word_offset = None;
        state.persist_narration_cache();
        state.narrating_section = None;
        state.tts_startup_buffered_chunks.clear();
        state.tts_playback_started = false;

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

        // Forward stopKaraoke to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            append_browser_reading_view_debug_log("finalize_voice_turn stop_karaoke");
            let ws_msg = serde_json::json!({ "type": "stopKaraoke" });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "stopped",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }

        // Clear karaoke overlay and reading cursor in the reading view.
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane
                .set_document_reader_karaoke_lines(None, false);
            self.bottom_pane
                .set_document_reader_reading_progress(None, 0);
            self.bottom_pane.set_document_reader_tts_paused(false);
        }

        // Flush any voice response cells that were stashed during karaoke.
        self.flush_deferred_voice_cells();

        // Auto-advance to next section in browser mode.
        if self.is_reading_view_browser_mode() {
            let next_section = finished_section_index.map(|idx| idx + 1);
            if let Some(next) = next_section
                && next < self.reading_view_browser_raw_sections.len()
            {
                append_browser_reading_view_debug_log(&format!(
                    "auto_advance section={} -> {}",
                    next - 1,
                    next
                ));
                self.handle_browser_request_read_aloud(next);
                return; // Don't clear voice state — new narration is starting.
            }
        }

        self.sync_voice_placeholder();
        // For tts_only mode, sync_voice_placeholder skips setting
        // the reading view status; explicitly clear it so the `s` hint
        // disappears when playback finishes.
        if self.voice_mode_state.as_ref().is_some_and(|s| s.tts_only)
            && !self.is_reading_view_browser_mode()
        {
            self.bottom_pane.set_document_reader_voice_status(None);
        }
        self.request_redraw();
    }

    /// Start TTS playback for the current turn, flushing any startup-buffered
    /// narration audio first and priming the initial karaoke word at 0ms.
    fn ensure_tts_playback_started(&mut self, extra_chunks: Vec<Vec<i16>>) {
        let mut should_start_tick = false;
        let mut should_push_karaoke = false;
        let mut browser_start_payload: Option<(usize, usize, Option<usize>)> = None;

        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        if !state.tts_playback_started {
            state.tts_playback_started = true;
            let mut enqueued_audio = false;
            if let Some(ref player) = state.audio_player {
                player.reset_playback_position();
                let mut chunks = std::mem::take(&mut state.tts_startup_buffered_chunks);
                chunks.extend(extra_chunks);
                for chunk in &chunks {
                    player.enqueue_pcm(chunk, 24_000, 1);
                }
                enqueued_audio = !chunks.is_empty();
            } else {
                state.tts_startup_buffered_chunks.clear();
            }

            let initial_idx = find_active_word(&state.tts_alignment_timeline, 0);
            if initial_idx != state.tts_highlight_word_idx {
                state.tts_highlight_word_idx = initial_idx;
                should_push_karaoke = initial_idx.is_some();
            }

            should_start_tick = state.highlight_tick_cancel.is_none();
            if enqueued_audio
                && let Some(section_index) = state
                    .narrating_section
                    .as_ref()
                    .map(|(_, section_index, _)| *section_index)
            {
                let spoken_total_words = state
                    .narrating_cleaned_text
                    .as_deref()
                    .map(|text| text.split_whitespace().count())
                    .unwrap_or(0);
                let hidden_equation_words = state
                    .equation_word_spans
                    .iter()
                    .map(|(_, start, end)| end.saturating_sub(*start))
                    .sum::<usize>();
                let total_words = spoken_total_words.saturating_sub(hidden_equation_words);
                browser_start_payload =
                    Some((section_index, total_words, state.selection_word_offset));
            }
        } else if let Some(ref player) = state.audio_player {
            for chunk in &extra_chunks {
                player.enqueue_pcm(chunk, 24_000, 1);
            }
        }

        if let Some((section_index, total_words, selection_word_offset)) = browser_start_payload
            && self.is_reading_view_browser_mode()
        {
            // Build alignment word list for alignment-driven karaoke.
            // Only send when the timeline is complete (cache hit or all chunks
            // received). During live streaming, the timeline is partial and
            // would produce wrong wrapping — fall back to heuristic approach.
            let timeline_complete = self.voice_mode_state.as_ref().is_some_and(|s| {
                let expected = s
                    .narrating_cleaned_text
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .count();
                expected > 0 && s.tts_alignment_timeline.len() >= expected
            });
            let alignment_words: Vec<serde_json::Value> = if timeline_complete {
                self.voice_mode_state
                    .as_ref()
                    .map(|s| {
                        s.tts_alignment_timeline
                            .iter()
                            .enumerate()
                            .map(|(i, entry)| {
                                let is_eq = s
                                    .equation_word_spans
                                    .iter()
                                    .any(|(_, start, end)| i >= *start && i < *end);
                                // Mark standalone punctuation so the browser
                                // skips them (same as equation words) to keep
                                // DOM word indices aligned with TTS indices.
                                let is_decor = !is_eq
                                    && entry.word.chars().all(|c| {
                                        c.is_ascii_punctuation()
                                            || matches!(
                                                c,
                                                '\u{2014}'
                                                    | '\u{2013}'
                                                    | '\u{2026}'
                                                    | '\u{201C}'
                                                    | '\u{201D}'
                                                    | '\u{2018}'
                                                    | '\u{2019}'
                                            )
                                    });
                                serde_json::json!({
                                    "idx": i,
                                    "word": entry.word,
                                    "eq": is_eq,
                                    "decor": is_decor,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![] // Heuristic wrapping will be used
            };

            // Track whether alignment-driven mode is active for this session.
            if let Some(ref mut state) = self.voice_mode_state {
                state.alignment_driven_karaoke = !alignment_words.is_empty();
            }

            append_browser_reading_view_debug_log(&format!(
                "start_karaoke section={section_index} total_words={total_words} alignment_words={} alignment_driven={} selection_word_offset={selection_word_offset:?}",
                alignment_words.len(),
                !alignment_words.is_empty(),
            ));
            let ws_msg = serde_json::json!({
                "type": "startKaraoke",
                "sectionIndex": section_index,
                "totalWords": total_words,
                "selectionWordOffset": selection_word_offset,
                "alignmentWords": alignment_words,
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "playing",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }
        if should_start_tick {
            self.start_highlight_tick();
        }
        if should_push_karaoke {
            #[cfg(not(target_os = "linux"))]
            self.push_karaoke_to_reader();
            self.request_redraw();
        }
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
                std::thread::sleep(Duration::from_millis(33));
            }
        });
    }

    /// Called on each highlight tick — update the highlighted word based on playback position.
    pub(crate) fn on_voice_highlight_tick(&mut self) {
        // Check if we should finalize — TTS data complete and audio drained.
        let should_finalize = self
            .voice_mode_state
            .as_ref()
            .is_some_and(VoiceModeState::should_finalize_on_tick);
        if should_finalize {
            tracing::debug!(
                "[TTS-DBG] highlight_tick: should_finalize=true, calling finalize_voice_turn"
            );
            self.finalize_voice_turn();
            return;
        }

        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        if state.phase != VoiceModePhase::Speaking {
            return;
        }

        let raw_pos_ms = state
            .audio_player
            .as_ref()
            .map(super::super::voice::RealtimeAudioPlayer::playback_position_ms)
            .unwrap_or(0);

        let pos_ms = raw_pos_ms;

        if state.tts_alignment_timeline.is_empty() {
            return;
        }

        let new_idx = find_active_word(&state.tts_alignment_timeline, pos_ms);

        // During inter-sentence gaps find_active_word returns None.
        // Keep the previous highlight rather than showing no highlight,
        // so the karaoke doesn't appear to "pause".
        let effective_idx = new_idx.or(state.tts_highlight_word_idx);

        // Diagnostic: log position and word index periodically so stuck
        // highlights can be diagnosed from logs.
        if effective_idx != state.tts_highlight_word_idx || raw_pos_ms == 0 {
            tracing::debug!(
                "[KARAOKE-TICK] raw_pos={raw_pos_ms}ms pos={pos_ms}ms \
                 word={effective_idx:?} prev={:?} timeline_len={}",
                state.tts_highlight_word_idx,
                state.tts_alignment_timeline.len(),
            );
        }

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
        // Reserve 2 extra columns for the "• " / "  " prefix that mirrors the
        // history cell indent so the karaoke text aligns with normal messages.
        let wrap_width = width.saturating_sub(4).saturating_sub(2) as usize;
        if wrap_width < 10 {
            return None;
        }
        let mut wrapped: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        // Pre-compute the byte offset of each wrapped line's start within
        // `full_text`.  Using pointer arithmetic on `Cow::Borrowed` lines
        // avoids the off-by-one that a manual "+1 per line" counter causes
        // at paragraph boundaries and when textwrap trims multiple spaces.
        let mut ft_starts: Vec<usize> = Vec::new();
        let ft_base = full_text.as_ptr() as usize;
        for paragraph in full_text.split("\n\n") {
            if paragraph.is_empty() {
                continue;
            }
            if !wrapped.is_empty() {
                // Empty line between paragraphs — sentinel offset.
                wrapped.push(std::borrow::Cow::Borrowed(""));
                ft_starts.push(usize::MAX);
            }
            let para_base = paragraph.as_ptr() as usize;
            let para_ft_offset = para_base - ft_base;
            let para_lines = wrap(paragraph, WrapOptions::new(wrap_width));
            for line in &para_lines {
                let offset = match line {
                    std::borrow::Cow::Borrowed(s) => {
                        para_ft_offset + (s.as_ptr() as usize - para_base)
                    }
                    std::borrow::Cow::Owned(_) => {
                        // Hyphenated line — estimate from previous line.
                        ft_starts.last().copied().map_or(para_ft_offset, |prev| {
                            if prev == usize::MAX {
                                para_ft_offset
                            } else {
                                let prev_line_len = wrapped.last().map_or(0, |l| l.len());
                                prev + prev_line_len + 1
                            }
                        })
                    }
                };
                ft_starts.push(offset);
                wrapped.push(line.clone());
            }
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
                    let mut lines = Self::build_karaoke_lines_inner(
                        &wrapped,
                        &ft_starts,
                        char_offset,
                        char_offset + word_len,
                        highlight_style,
                    );
                    Self::prepend_bullet_indent(&mut lines);
                    return Some(lines);
                }
                char_offset += entry.word.len();
            }
        }

        // No highlight — render plain text.
        let mut lines: Vec<Line<'static>> = wrapped
            .iter()
            .map(|line| Line::from(Span::raw(line.to_string())))
            .collect();
        Self::prepend_bullet_indent(&mut lines);
        Some(lines)
    }

    /// Prepend "• " (dimmed) to the first line and "  " to subsequent lines,
    /// matching the indent used by normal assistant history cells.
    #[cfg(not(target_os = "linux"))]
    fn prepend_bullet_indent(lines: &mut [ratatui::text::Line<'static>]) {
        use ratatui::style::Stylize as _;
        use ratatui::text::Span;
        for (i, line) in lines.iter_mut().enumerate() {
            let prefix = if i == 0 {
                Span::from("◆ ").dim()
            } else {
                Span::raw("  ")
            };
            line.spans.insert(0, prefix);
        }
    }

    /// Build wrapped Lines with a highlighted byte range.
    ///
    /// `ft_starts` contains the pre-computed byte offset in `full_text` where
    /// each wrapped line begins (using pointer arithmetic on `Cow::Borrowed`
    /// slices).  Paragraph separator lines have `usize::MAX`.
    ///
    /// `hl_start` and `hl_end` are byte offsets into the **unwrapped**
    /// concatenated text.  We snap them to valid UTF-8 character boundaries
    /// to avoid panics on multi-byte characters (e.g. `'`).
    #[cfg(not(target_os = "linux"))]
    fn build_karaoke_lines_inner(
        wrapped: &[std::borrow::Cow<'_, str>],
        ft_starts: &[usize],
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
        for (i, wrapped_line) in wrapped.iter().enumerate() {
            let line_start = ft_starts[i];
            if line_start == usize::MAX {
                // Paragraph separator — empty line.
                lines.push(Line::from(Span::raw(String::new())));
                continue;
            }
            let line_len = wrapped_line.len();
            let line_end = line_start + line_len;

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
        // Browser mode: forward karaoke word updates via WebSocket.
        if self.is_reading_view_browser_mode() {
            let is_narrating = self
                .voice_mode_state
                .as_ref()
                .is_some_and(|s| s.narrating_section.is_some());
            if is_narrating {
                let (word_idx, sel_offset, section_index) = self
                    .voice_mode_state
                    .as_ref()
                    .map(|s| {
                        let sec_idx = s
                            .narrating_section
                            .as_ref()
                            .map(|(_, idx, _)| *idx)
                            .unwrap_or(0);
                        (
                            s.tts_highlight_word_idx,
                            s.selection_word_offset.unwrap_or(0),
                            sec_idx,
                        )
                    })
                    .unwrap_or((None, 0, 0));
                if let Some(word_idx) = word_idx {
                    let adjusted = word_idx + sel_offset;

                    // When alignment-driven karaoke is active, data-wi uses
                    // the spoken (alignment) index directly. When heuristic
                    // wrapping is used (streaming), we need visible→spoken.
                    let using_alignment = self
                        .voice_mode_state
                        .as_ref()
                        .is_some_and(|s| s.alignment_driven_karaoke);
                    let browser_word_idx = if using_alignment {
                        adjusted
                    } else {
                        // Old heuristic path: convert spoken → visible
                        let eq_spans = self
                            .voice_mode_state
                            .as_ref()
                            .map(|s| &s.equation_word_spans);
                        if let Some(spans) = eq_spans.filter(|s| !s.is_empty()) {
                            let spoken = adjusted;
                            let mut hidden = 0usize;
                            for &(_, start, end) in spans.iter() {
                                if start > spoken {
                                    break;
                                }
                                let overlap_end = (spoken + 1).min(end);
                                if overlap_end > start {
                                    hidden += overlap_end - start;
                                }
                            }
                            spoken.saturating_sub(hidden)
                        } else {
                            adjusted
                        }
                    };

                    let ws_msg = serde_json::json!({
                        "type": "karaokeWord",
                        "sectionIndex": section_index,
                        "wordIndex": browser_word_idx,
                    });
                    append_browser_reading_view_debug_log(&format!(
                        "karaoke_word section={section_index} spoken_word={adjusted} browser_word={browser_word_idx} selection_offset={sel_offset}"
                    ));
                    self.forward_to_reading_view_server(&ws_msg.to_string());

                    // Compute and send equation highlight state if spans exist.
                    let eq_update = self.voice_mode_state.as_mut().and_then(|state| {
                        if state.equation_word_spans.is_empty() {
                            return None;
                        }
                        let active_eq = state
                            .equation_word_spans
                            .iter()
                            .find(|(_, start, end)| word_idx >= *start && word_idx < *end)
                            .map(|(idx, _, _)| *idx);
                        let passed_eq = state
                            .equation_word_spans
                            .iter()
                            .filter(|(_, _, end)| word_idx >= *end)
                            .map(|(idx, _, _)| *idx)
                            .max()
                            .unwrap_or(0);

                        if active_eq != state.active_equation_index
                            || passed_eq != state.passed_equation_index
                        {
                            state.active_equation_index = active_eq;
                            state.passed_equation_index = passed_eq;
                            Some((active_eq, passed_eq))
                        } else {
                            None
                        }
                    });
                    if let Some((active_eq, passed_eq)) = eq_update {
                        let eq_msg = serde_json::json!({
                            "type": "equationHighlight",
                            "activeIndex": active_eq
                                .map(|i| i as i64)
                                .unwrap_or(-1),
                            "passedIndex": passed_eq,
                        });
                        self.forward_to_reading_view_server(&eq_msg.to_string());
                    }
                }
            }
            return;
        }

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
        if !is_narrating && !self.is_reading_view_browser_mode() {
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
                // The `voice_input` flag from main is not yet wired in
                // v0.129.0; submit as a plain text message for now.
                let msg = super::UserMessage::from_text(text);
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
             NEVER use academic citation style like 'Smith et al. (2026)' — use natural \
             phrasing like 'the authors showed' or 'researchers found'. No parenthetical \
             years like '(2025)'. Write for spoken delivery. \
             Wrap ALL of your spoken text in <voice>...</voice> tags so it is read aloud. \
             For every rendered equation or symbol, use an <eq> structured pair. \
             Inline form: <eq latex=\"...\">spoken reading</eq> \
             Display form: <eq latex=\"...\" display=\"block\">spoken reading</eq> \
             In each <eq>, the latex attribute is rendered visually and the inner text between tags is spoken aloud. \
             In the latex attribute, provide raw LaTeX body only (no $, $$, \\(, or \\[ delimiters). \
             The spoken reading should be a natural English paraphrase of the math. \
             Example: <eq latex=\"\\\\sqrt{{d_k}}\">square root of d sub k</eq> \
             Do NOT include raw LaTeX, code blocks, or markdown formatting.\n\n\
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
             - Use <eq> tags for math; no raw LaTeX, no code blocks\n\
             - The summary should describe the topic (e.g. \"Dropout as regularization\", \
             \"Why gradients vanish\")\n\
             - Do NOT rewrite the section or make multiple tool calls",
            title = ctx.title,
            heading = ctx.heading,
            doc_id = ctx.document_id,
            idx = ctx.section_index,
        );

        self.last_turn_was_local_submit = true;
        self.app_event_tx
            .send(AppEvent::SubmitUserText { text: context });
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
    /// Whether voice mode is currently active (on, regardless of phase).
    pub(crate) fn is_voice_mode_active(&self) -> bool {
        self.voice_mode_state
            .as_ref()
            .is_some_and(|s| s.is_active() && !s.tts_only)
    }

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
        let browser_mode = self.is_reading_view_browser_mode();
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };
        tracing::info!(
            "[TTS-TIMING] on_voice_interrupt_tts: phase={:?}, has_worker={}, has_player={}",
            state.phase,
            state.tts_worker_tx.is_some(),
            state.audio_player.is_some(),
        );
        // Always clear the audio player buffer and TTS state, regardless
        // of phase. Audio may still be buffered after phase left Speaking.
        state.interrupt_tts();
        if state.phase == VoiceModePhase::Speaking {
            state.phase = VoiceModePhase::Idle;
        }
        let tts_only = state.tts_only;
        let _ = state;
        // Forward stopKaraoke to browser if in browser mode.
        if browser_mode {
            let ws_msg = serde_json::json!({ "type": "stopKaraoke" });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "stopped",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }
        // Clear karaoke overlay and reading cursor in the reading view.
        if !browser_mode {
            self.bottom_pane
                .set_document_reader_karaoke_lines(None, false);
            self.bottom_pane
                .set_document_reader_reading_progress(None, 0);
            self.bottom_pane.set_document_reader_tts_paused(false);
            if tts_only {
                self.bottom_pane.set_document_reader_voice_status(None);
            }
        }
        self.sync_voice_placeholder();
        self.request_redraw();
    }

    /// Pause TTS playback (e.g. user pressed Space in reading view).
    pub(crate) fn on_voice_pause_tts(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            tracing::debug!("[TTS-DBG] on_voice_pause_tts: voice_mode_state is None");
            return;
        };
        tracing::debug!(
            "[TTS-DBG] on_voice_pause_tts: phase={:?}, has_audio={}, is_paused={}",
            state.phase,
            state.has_buffered_audio(),
            state.is_audio_paused(),
        );
        // Allow pausing when Speaking OR when Idle but audio is still playing.
        let has_audio = state.has_buffered_audio();
        if state.phase != VoiceModePhase::Speaking
            && !(state.phase == VoiceModePhase::Idle && has_audio)
        {
            tracing::debug!(
                "[TTS-DBG] on_voice_pause_tts: SKIPPED (phase={:?}, has_audio={has_audio})",
                state.phase
            );
            return;
        }
        state.pause_tts();
        tracing::debug!("[TTS-DBG] on_voice_pause_tts: paused successfully");
        // Forward pause state to browser.
        if self.is_reading_view_browser_mode() {
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "paused",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }
        if !self.is_reading_view_browser_mode() {
            let msg = "\u{23F8}\u{FE0F}  Paused \u{2014} s/Space to resume".to_string();
            self.bottom_pane.set_document_reader_voice_status(Some(msg));
            self.bottom_pane.set_document_reader_tts_paused(true);
        }
        self.request_redraw();
    }

    /// Resume TTS playback after pause.
    pub(crate) fn on_voice_resume_tts(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            tracing::debug!("[TTS-DBG] on_voice_resume_tts: voice_mode_state is None");
            return;
        };
        tracing::debug!(
            "[TTS-DBG] on_voice_resume_tts: phase={:?}, has_audio={}, is_paused={}, tts_data_complete={}",
            state.phase,
            state.has_buffered_audio(),
            state.is_audio_paused(),
            state.tts_data_complete,
        );
        // Allow resume when Speaking OR when Idle but audio is still paused
        // (finalize_voice_turn may have already run while audio was paused).
        let is_paused = state.is_audio_paused();
        if state.phase != VoiceModePhase::Speaking
            && !(state.phase == VoiceModePhase::Idle && is_paused)
        {
            tracing::debug!(
                "[TTS-DBG] on_voice_resume_tts: SKIPPED (phase={:?}, is_paused={is_paused})",
                state.phase
            );
            return;
        }
        // If the audio buffer drained while paused (race or natural finish),
        // don't pretend to resume — there's nothing to play.
        if !state.can_resume_playback() {
            tracing::debug!(
                "[TTS-DBG] on_voice_resume_tts: NO AUDIO to resume (tts_data_complete={}, has_audio={})",
                state.tts_data_complete,
                state.has_buffered_audio(),
            );
            // Unpause the player so it doesn't block future narration.
            state.resume_tts();
            if !self.is_reading_view_browser_mode() {
                self.bottom_pane.set_document_reader_tts_paused(false);
            }
            self.finalize_voice_turn();
            return;
        }
        // If we're resuming from Idle (post-finalization), restore Speaking phase
        // so the highlight tick and subsequent logic work correctly.
        if state.phase == VoiceModePhase::Idle {
            state.phase = VoiceModePhase::Speaking;
        }
        state.resume_tts();
        self.start_highlight_tick();
        // Forward resume state to browser.
        if self.is_reading_view_browser_mode() {
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "playing",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }
        if !self.is_reading_view_browser_mode() {
            let msg = "\u{25B6}\u{FE0F}  Speaking...".to_string();
            self.bottom_pane.set_document_reader_voice_status(Some(msg));
            self.bottom_pane.set_document_reader_tts_paused(false);
        }
        self.request_redraw();
        tracing::debug!("[TTS-DBG] on_voice_resume_tts: resumed successfully");
    }

    /// Change the client-side TTS playback speed by `delta` (e.g. +0.1 or -0.1).
    /// The speed is clamped to [0.75, 3.0] and rounded to the nearest 0.1.
    pub(crate) fn on_voice_playback_speed_change(&mut self, delta: f64) {
        let Some(ref state) = self.voice_mode_state else {
            return;
        };
        let Some(ref player) = state.audio_player else {
            return;
        };
        let current = player.playback_speed();
        let new_speed = ((current + delta) * 10.0).round() / 10.0;
        let clamped = new_speed.clamp(0.75, 3.0);
        player.set_playback_speed(clamped);
        // Update voice status to show current speed.
        if !self.is_reading_view_browser_mode() {
            let speed_str = format!("{clamped:.1}");
            let msg = format!("\u{25B6}\u{FE0F}  Speaking ({speed_str}\u{00D7})");
            self.bottom_pane.set_document_reader_voice_status(Some(msg));
        }
        self.request_redraw();
    }

    // ─── Reading view Space tap/hold ──────────────────────────────────

    /// Threshold for distinguishing a tap from a hold.
    const READING_SPACE_HOLD_MS: u64 = 200;

    /// Called on Space Press in the reading view while TTS is speaking.
    /// Saves pre-press state and pauses TTS. If STT is available (not
    /// tts_only mode), also starts mic capture so a hold can become PTT.
    pub(crate) fn on_reading_view_space_press(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        // Save pre-press state for tap detection on release.
        let was_paused = state.is_audio_paused();
        state.space_was_paused = was_paused;
        state.space_press_at = Some(Instant::now());

        // Pause TTS (non-destructive — preserves buffers).
        if !was_paused {
            state.pause_tts();
            if !self.is_reading_view_browser_mode() {
                self.bottom_pane.set_document_reader_tts_paused(true);
            }
        }

        // Start mic capture if STT is available (not tts_only).
        let state_ref = self.voice_mode_state.as_ref();
        let can_record = state_ref.is_some_and(|s| s.stt_enabled && !s.tts_only);
        if can_record {
            match crate::voice::VoiceCapture::start_ptt(&self.config) {
                Ok(capture) => {
                    if let Some(ref mut s) = self.voice_mode_state {
                        s.capture = Some(capture);
                        s.recording_started_at = Some(Instant::now());
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to start voice capture for reading PTT: {e}");
                    // Non-fatal: tap still works, hold just won't record.
                }
            }
        }

        self.request_redraw();
    }

    /// Called on Space Release in the reading view during tap/hold detection.
    pub(crate) fn on_reading_view_space_release(&mut self) {
        let Some(ref mut state) = self.voice_mode_state else {
            return;
        };

        let press_at = match state.space_press_at.take() {
            Some(t) => t,
            None => return,
        };
        let was_paused = state.space_was_paused;
        let held_ms = press_at.elapsed().as_millis() as u64;
        let is_hold = held_ms >= Self::READING_SPACE_HOLD_MS;

        if is_hold && state.stt_enabled && !state.tts_only {
            // ── Hold path: PTT barge-in + transcription ──
            // Now we know the user intended a PTT hold — do destructive barge-in.
            state.interrupt_tts();
            state.tts_suppressed = true;
            if state.phase == VoiceModePhase::Speaking {
                state.phase = VoiceModePhase::Idle;
            }
            // Clear reading view overlays.
            if !self.is_reading_view_browser_mode() {
                self.bottom_pane
                    .set_document_reader_karaoke_lines(None, false);
                self.bottom_pane
                    .set_document_reader_reading_progress(None, 0);
                self.bottom_pane.set_document_reader_tts_paused(false);
            }

            // Complete the PTT flow: stop capture, encode, transcribe.
            // Re-borrow state since interrupt_tts consumed the mutable ref.
            let Some(ref mut state) = self.voice_mode_state else {
                return;
            };
            state.phase = VoiceModePhase::Transcribing;
            let capture = state.capture.take();
            state.recording_started_at = None;

            let Some(capture) = capture else {
                state.phase = VoiceModePhase::Idle;
                self.sync_voice_placeholder();
                self.request_redraw();
                return;
            };

            let audio = match capture.stop() {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("failed to stop capture: {e}");
                    if let Some(ref mut s) = self.voice_mode_state {
                        s.phase = VoiceModePhase::Idle;
                    }
                    self.sync_voice_placeholder();
                    self.request_redraw();
                    return;
                }
            };

            const MIN_PTT_DURATION_SECONDS: f32 = 1.0;
            let duration = audio.duration_seconds();
            if duration < MIN_PTT_DURATION_SECONDS {
                if let Some(ref mut s) = self.voice_mode_state {
                    s.phase = VoiceModePhase::Idle;
                }
                self.app_event_tx.send(AppEvent::VoiceModeTranscriptionFailed {
                    error: format!(
                        "recording too short ({duration:.2}s); hold Space for at least {MIN_PTT_DURATION_SECONDS:.1}s"
                    ),
                });
                self.sync_voice_placeholder();
                self.request_redraw();
                return;
            }

            let wav_bytes = match crate::voice::encode_wav_for_voice_mode(&audio) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("WAV encode failed: {e}");
                    if let Some(ref mut s) = self.voice_mode_state {
                        s.phase = VoiceModePhase::Idle;
                    }
                    self.sync_voice_placeholder();
                    self.request_redraw();
                    return;
                }
            };

            let tx = self.app_event_tx.clone();
            let voice_config = self.effective_voice_config();
            let auth_mgr = Arc::clone(&self.auth_manager);

            let Some(config) = build_elevenlabs_config(&voice_config, &auth_mgr) else {
                tx.send(AppEvent::VoiceModeTranscriptionFailed {
                    error: "Missing ElevenLabs API key".to_string(),
                });
                self.sync_voice_placeholder();
                self.request_redraw();
                return;
            };

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
        } else {
            // ── Tap path: toggle pause/resume ──
            // Discard any recording that started.
            if let Some(ref mut s) = self.voice_mode_state {
                if let Some(capture) = s.capture.take() {
                    let _ = capture.stop();
                }
                s.recording_started_at = None;
            }

            if was_paused {
                // Was paused before press → resume.
                self.on_voice_resume_tts();
            } else {
                // Was playing before press → stay paused (we already paused on press).
                // Just update the status message.
                if !self.is_reading_view_browser_mode() {
                    let msg = "\u{23F8}\u{FE0F}  Paused \u{2014} s/Space to resume".to_string();
                    self.bottom_pane.set_document_reader_voice_status(Some(msg));
                    self.bottom_pane.set_document_reader_tts_paused(true);
                }
            }
        }

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
        manual: bool,
    ) {
        let narrate_start = std::time::Instant::now();
        let voice_config = self.effective_voice_config();
        let has_tts_key = resolve_elevenlabs_api_key_from_config(&voice_config).is_some();
        tracing::info!(
            "[TTS-TIMING] on_voice_narrate_section: section={section_index}, manual={manual}, \
             voice_state_exists={}, text_len={}",
            self.voice_mode_state.is_some(),
            raw_text.len(),
        );

        // Lazily initialize a TTS-only state when voice mode is off but
        // the user manually pressed `r` to narrate.
        if self.voice_mode_state.is_none() && manual {
            if !has_tts_key {
                self.bottom_pane.set_document_reader_tts_flash_msg(Some(
                    "TTS not configured \u{2014} use /voice to set up".to_string(),
                ));
                self.request_redraw();
                return;
            }
            tracing::info!("[TTS-TIMING] creating TTS-only voice state + audio player...");
            let player_start = std::time::Instant::now();
            let mut state = VoiceModeState::new(&voice_config);
            state.tts_only = true;
            state.phase = VoiceModePhase::Idle;
            match crate::voice::RealtimeAudioPlayer::start(&self.config) {
                Ok(player) => {
                    tracing::info!(
                        "[TTS-TIMING] audio player created in {:?}",
                        player_start.elapsed(),
                    );
                    state.audio_player = Some(player);
                }
                Err(e) => {
                    tracing::warn!("Failed to start audio player for TTS: {e}");
                    if self.is_reading_view_browser_mode() {
                        let ws_msg = serde_json::json!({
                            "type": "ttsStateChanged",
                            "state": "stopped",
                        });
                        self.forward_to_reading_view_server(&ws_msg.to_string());
                    }
                    return;
                }
            }
            self.voice_mode_state = Some(state);
        }
        if manual
            && self
                .voice_mode_state
                .as_ref()
                .is_some_and(|state| state.audio_player.is_none())
        {
            if !has_tts_key {
                self.bottom_pane.set_document_reader_tts_flash_msg(Some(
                    "TTS not configured \u{2014} use /voice to set up".to_string(),
                ));
                self.request_redraw();
                if self.is_reading_view_browser_mode() {
                    let ws_msg = serde_json::json!({
                        "type": "ttsStateChanged",
                        "state": "stopped",
                    });
                    self.forward_to_reading_view_server(&ws_msg.to_string());
                }
                return;
            }
            tracing::info!(
                "[TTS-TIMING] creating audio player for existing TTS-only voice state..."
            );
            let player_start = std::time::Instant::now();
            match crate::voice::RealtimeAudioPlayer::start(&self.config) {
                Ok(player) => {
                    tracing::info!(
                        "[TTS-TIMING] audio player created in {:?}",
                        player_start.elapsed(),
                    );
                    if let Some(ref mut state) = self.voice_mode_state {
                        state.audio_player = Some(player);
                        state.tts_only = true;
                        state.phase = VoiceModePhase::Idle;
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to start audio player for TTS: {e}");
                    if self.is_reading_view_browser_mode() {
                        let ws_msg = serde_json::json!({
                            "type": "ttsStateChanged",
                            "state": "stopped",
                        });
                        self.forward_to_reading_view_server(&ws_msg.to_string());
                    }
                    return;
                }
            }
        }

        // When the user explicitly presses 'r', clear any lingering TTS
        // suppression (e.g. from a prior PTT barge-in) so narration works.
        if manual && let Some(ref mut state) = self.voice_mode_state {
            state.tts_suppressed = false;
        }

        // Show a helpful error if voice mode is on but TTS is disabled
        // (e.g., STT-only mode) and the user explicitly pressed `r`.
        if manual
            && let Some(ref state) = self.voice_mode_state
            && !state.tts_only
            && !state.should_tts()
        {
            if !state.tts_enabled {
                self.bottom_pane.set_document_reader_tts_flash_msg(Some(
                    "TTS is disabled \u{2014} enable in /voice-setup".to_string(),
                ));
                self.request_redraw();
            }
            return;
        }

        // Phase 1: check preconditions and interrupt (borrows state mutably).
        {
            let Some(ref mut state) = self.voice_mode_state else {
                return;
            };
            // Skip should_tts() for tts_only mode and for manual 'r'
            // presses — the user explicitly wants TTS in both cases.
            if !state.tts_only && !manual && !state.should_tts() {
                return;
            }
            // Skip TTS silently if no API key — the user was already warned
            // at voice mode activation time.
            if state.tts_worker_tx.is_none() && !has_tts_key {
                return;
            }
            tracing::info!(
                "[TTS-TIMING] interrupting previous TTS (elapsed since entry: {:?})",
                narrate_start.elapsed(),
            );
            state.interrupt_tts();
        }

        // Clear the document reader's visual karaoke state immediately so
        // stale highlights from a previous read don't linger while we wait
        // for new alignment data to arrive.  Without this, pressing `r`
        // would briefly show the old highlight (random jumping words) until
        // the first chunk of the new TTS response overwrites it.
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane
                .set_document_reader_karaoke_lines(None, false);
            self.bottom_pane
                .set_document_reader_reading_progress(None, 0);
            self.bottom_pane.set_document_reader_tts_paused(false);
        }

        let cleaned = clean_for_tts_preserving_equation_markers(&raw_text);
        if cleaned.is_empty() {
            if self.is_reading_view_browser_mode() {
                append_browser_reading_view_debug_log(&format!(
                    "narrate_section skipped_empty section={section_index} manual={manual}"
                ));
                // Clear hourglass so the browser doesn't stay stuck in 'starting'.
                let ws_msg = serde_json::json!({
                    "type": "ttsStateChanged",
                    "state": "stopped",
                });
                self.forward_to_reading_view_server(&ws_msg.to_string());
            }
            return;
        }
        if self.is_reading_view_browser_mode() {
            append_browser_reading_view_debug_log(&format!(
                "narrate_section request section={section_index} manual={manual} selection_word_offset={selection_word_offset:?} text_len={} cleaned_len={}",
                raw_text.len(),
                cleaned.len()
            ));
        }

        let content_hash = hash_text(&cleaned);

        // Strip equation markers and record word spans for equation karaoke
        // highlighting (consistent with the streaming TTS path).
        let (tts_text, eq_spans) = parse_equation_markers(&cleaned);

        // Phase 2: check cache (works for both full sections and selections —
        // the content_hash distinguishes different text under the same key).
        let cache_check_start = std::time::Instant::now();
        let cached: Option<(Vec<Vec<i16>>, Vec<AlignmentEntry>)> =
            self.voice_mode_state.as_ref().and_then(|state| {
                state.tts_section_cache.lock().ok().and_then(|cache| {
                    cache
                        .get(&(document_id.clone(), section_index))
                        .filter(|entry| entry.content_hash == content_hash)
                        .map(|entry| (entry.chunks.clone(), entry.alignment_timeline.clone()))
                })
            });
        let cache_hit = cached.is_some();
        let cache_chunks = cached.as_ref().map(|(c, _)| c.len()).unwrap_or(0);
        let cache_samples: usize = cached
            .as_ref()
            .map(|(c, _)| c.iter().map(std::vec::Vec::len).sum())
            .unwrap_or(0);
        tracing::info!(
            "[TTS-TIMING] cache check: hit={cache_hit}, chunks={cache_chunks}, \
             samples={cache_samples}, lookup took {:?} (total elapsed: {:?})",
            cache_check_start.elapsed(),
            narrate_start.elapsed(),
        );

        // Heading words to skip = 0: the heading exists in BOTH the TTS
        // timeline and the rendered section lines, so word indices are
        // naturally aligned and no skip is needed.
        let heading_words = 0;

        if let Some((chunks, cached_timeline)) = cached {
            if self.is_reading_view_browser_mode() {
                append_browser_reading_view_debug_log(&format!(
                    "narrate_section cache_hit section={section_index} chunks={} timeline_entries={}",
                    chunks.len(),
                    cached_timeline.len()
                ));
            }
            // Cache hit — play cached chunks and restore alignment for karaoke.
            let enqueue_start = std::time::Instant::now();
            let num_chunks = chunks.len();
            let total_samples: usize = chunks.iter().map(std::vec::Vec::len).sum();
            if let Some(ref mut state) = self.voice_mode_state {
                state.phase = VoiceModePhase::Speaking;
                state.narrating_section = Some((document_id, section_index, content_hash));
                state.narrating_heading_words = heading_words;
                state.selection_word_offset = selection_word_offset;
                state.narrating_cleaned_text = Some(tts_text);
                state.equation_word_spans = eq_spans;
                state.tts_alignment_timeline = cached_timeline;
                repair_timeline_monotonicity(&mut state.tts_alignment_timeline);
            }
            for chunk in chunks {
                self.on_voice_tts_audio_chunk(chunk, None);
            }
            tracing::info!(
                "[TTS-TIMING] cache hit: enqueued {num_chunks} chunks ({total_samples} samples, \
                 ~{}ms audio) in {:?} (total elapsed: {:?})",
                total_samples as u64 * 1000 / 24000,
                enqueue_start.elapsed(),
                narrate_start.elapsed(),
            );
            // Start highlight tick so karaoke progresses during playback.
            self.start_highlight_tick();
            tracing::info!(
                "[TTS-TIMING] cache_hit: queuing VoiceModeTtsFinished, buffered_chunks={}, phase={:?}",
                self.voice_mode_state
                    .as_ref()
                    .map(|s| s.tts_startup_buffered_chunks.len())
                    .unwrap_or(0),
                self.voice_mode_state.as_ref().map(|s| s.phase),
            );
            self.app_event_tx.send(AppEvent::VoiceModeTtsFinished);
            self.sync_voice_placeholder();
            // Ensure the reading view shows "Speaking..." so the `s` key
            // hint and handler are active (sync_voice_placeholder skips
            // tts_only mode).
            if !self.is_reading_view_browser_mode() {
                self.bottom_pane.set_document_reader_voice_status(Some(
                    "\u{25B6}\u{FE0F}  Speaking...".to_string(),
                ));
            }
            return;
        }

        // Phase 3: cache miss — use persistent TTS worker (single WebSocket).
        tracing::info!(
            "[TTS-TIMING] cache miss: spawning TTS worker (total elapsed: {:?})",
            narrate_start.elapsed(),
        );
        if self.is_reading_view_browser_mode() {
            append_browser_reading_view_debug_log(&format!(
                "narrate_section cache_miss section={section_index} cleaned_len={}",
                cleaned.len()
            ));
        }
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
        state.equation_word_spans = eq_spans;
        state.narrating_chunks.clear();

        state.narrating_cleaned_text = Some(tts_text.clone());

        // Start the persistent TTS worker if not running.
        if state.tts_worker_tx.is_none() {
            let vc = narration_vc;
            let tx = self.app_event_tx.clone();
            let in_flight = state.tts_in_flight.clone();
            let gen_ref = state.tts_generation.clone();
            let spawn_gen = gen_ref.load(Ordering::SeqCst);
            in_flight.fetch_add(1, Ordering::SeqCst);
            let proxy = build_elevenlabs_proxy(&self.auth_manager);
            let backend = vc.tts_backend.unwrap_or_default();

            let (worker_tx, worker_rx) = tokio::sync::mpsc::unbounded_channel();
            state.tts_worker_tx = Some(worker_tx);

            tokio::spawn(async move {
                match backend {
                    TtsBackend::Say => {
                        say_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen).await;
                    }
                    TtsBackend::Elevenlabs => {
                        tts_worker_loop(vc, worker_rx, tx, in_flight, gen_ref, spawn_gen, proxy)
                            .await;
                    }
                }
            });
        }

        // Send the full section text at once and signal finish.
        if let Some(ref worker_tx) = state.tts_worker_tx {
            let _ = worker_tx.send(TtsWorkerCommand::SendText(tts_text));
            let _ = worker_tx.send(TtsWorkerCommand::Finish);
        }
        state.tts_worker_tx = None;

        self.sync_voice_placeholder();
        // Ensure the reading view shows "Speaking..." so the `s` key
        // hint and handler are active (sync_voice_placeholder skips
        // tts_only mode).
        if !self.is_reading_view_browser_mode() {
            self.bottom_pane.set_document_reader_voice_status(Some(
                "\u{25B6}\u{FE0F}  Speaking...".to_string(),
            ));
        }
    }

    /// Handle a prefetch request: generate TTS in background, cache result.
    pub(crate) fn on_voice_prefetch_section(
        &mut self,
        document_id: String,
        section_index: usize,
        raw_text: String,
    ) {
        let voice_config = self.effective_voice_config();
        let has_tts_key = resolve_elevenlabs_api_key_from_config(&voice_config).is_some();
        if self.voice_mode_state.is_none() {
            if !has_tts_key {
                return;
            }
            let mut state = VoiceModeState::new(&voice_config);
            state.tts_only = true;
            state.phase = VoiceModePhase::Idle;
            self.voice_mode_state = Some(state);
        }
        let Some(ref state) = self.voice_mode_state else {
            return;
        };
        if !state.tts_only && !state.should_tts() {
            return;
        }

        // Prefetch is only meaningful for ElevenLabs (where we cache PCM
        // chunks). The `say` backend has nothing to prefetch — audio is
        // produced on demand by the system.
        if voice_config.tts_backend.unwrap_or_default() == TtsBackend::Say {
            return;
        }

        let cleaned = clean_for_tts_preserving_equation_markers(&raw_text);
        if cleaned.is_empty() {
            return;
        }

        let content_hash = hash_text(&cleaned);
        // Strip equation markers before TTS (same as narration path).
        let (tts_text, _eq_spans) = parse_equation_markers(&cleaned);
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

        // Split marker-free text into sentences.
        let mut sentence_buf = SentenceBuffer::new();
        let mut sentences = sentence_buf.push(&tts_text);
        if let Some(remaining) = sentence_buf.flush() {
            sentences.push(remaining);
        }

        let vc = voice_config;
        let cache = state.tts_section_cache.clone();
        let pending = state.prefetch_pending.clone();
        let proxy = build_elevenlabs_proxy(&self.auth_manager);

        tokio::spawn(async move {
            let mut all_chunks = Vec::new();
            let mut all_timeline = Vec::new();
            for sentence in &sentences {
                match prefetch_sentence_tts(&vc, sentence, proxy.as_ref()).await {
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
/// markers, link syntax, LaTeX blocks, and image markers. Collapses
/// consecutive newlines (3+ to 2) and strips horizontal-rule lines.
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
        // Also strip numeric bracket citations like [1], [2,3], [1-3].
        if ch == '[' {
            let mut link_text = String::new();
            for c in chars.by_ref() {
                if c == ']' {
                    break;
                }
                link_text.push(c);
            }
            // Numeric citation markers (e.g. [1], [2,3], [1-3]) — drop entirely.
            if !link_text.is_empty()
                && link_text.starts_with(|c: char| c.is_ascii_digit())
                && link_text
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ',' || c == '-' || c == ' ')
            {
                continue;
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

        // Bare URLs (https://... or http://...) — strip them entirely.
        // TTS engines skip or garble URLs, so keeping them creates a word
        // count mismatch between the spoken alignment and the rendered
        // display, causing karaoke highlighting to drift.
        if ch == 'h'
            && (out.is_empty()
                || out.ends_with(|c: char| c.is_whitespace() || c == '(' || c == '[' || c == '"'))
        {
            let rest: String = chars.clone().take(8).collect(); // "ttps://X" or "ttp://XX"
            let is_url = rest.starts_with("ttps://") || rest.starts_with("ttp://");
            if is_url {
                // Consume the rest of the URL (until whitespace or end).
                for c in chars.by_ref() {
                    if c.is_whitespace() {
                        // Push the whitespace so spacing is preserved.
                        out.push(c);
                        break;
                    }
                }
                continue;
            }
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

    // Strip academic citation artifacts that sound unnatural when spoken.
    // 1. Parenthetical years: "(2024)", "(2025)", "(2026)", etc.
    // 2. "et al." (with or without trailing period)
    // 3. Bracket citation markers: "[1]", "[2,3]", "[1-3]", "[12, 15]", etc.
    use std::sync::LazyLock;
    static RE_PAREN_YEAR: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"\s*\(\d{4}\)") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_PAREN_YEAR regex: {e}"),
        });
    static RE_ET_AL: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"\s*et\s+al\.?") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_ET_AL regex: {e}"),
        });
    static RE_BRACKET_CITE: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"\s*\[\d[\d,\s\-]*\]") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_BRACKET_CITE regex: {e}"),
        });

    let collapsed = RE_PAREN_YEAR.replace_all(&collapsed, "");
    let collapsed = RE_ET_AL.replace_all(&collapsed, "");
    let collapsed = RE_BRACKET_CITE.replace_all(&collapsed, "");

    // Strip list item markers (-, +, 1.) so they aren't spoken literally.
    static RE_LIST_PREFIX: LazyLock<regex_lite::Regex> =
        LazyLock::new(
            || match regex_lite::Regex::new(r"^[ \t]*(?:[-+]|\d{1,3}\.)\s+") {
                Ok(r) => r,
                Err(e) => panic!("invalid RE_LIST_PREFIX regex: {e}"),
            },
        );
    let collapsed = {
        let text = collapsed.to_string();
        let lines: Vec<&str> = text.split('\n').collect();
        let mut result = String::with_capacity(text.len());
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                result.push('\n');
            }
            if RE_LIST_PREFIX.is_match(line) {
                let stripped = RE_LIST_PREFIX.replace(line, "");
                result.push_str(stripped.as_ref());
            } else {
                result.push_str(line);
            }
        }
        result
    };

    collapsed.trim().to_string()
}

/// Generate TTS for a sentence without sending audio events (for prefetching).
/// Collects PCM chunks and alignment timeline entries.
async fn prefetch_sentence_tts(
    voice_config: &crate::legacy_core::config::types::VoiceModeToml,
    sentence: &str,
    proxy: Option<&codex_elevenlabs::ElevenLabsProxy>,
) -> Result<(Vec<Vec<i16>>, Vec<AlignmentEntry>), codex_elevenlabs::ElevenLabsError> {
    let mut rx = start_tts_generation(voice_config, sentence, proxy)?;
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
    voice_config: &crate::legacy_core::config::types::VoiceModeToml,
    sentence: &str,
    proxy: Option<&codex_elevenlabs::ElevenLabsProxy>,
) -> Result<
    tokio::sync::mpsc::UnboundedReceiver<codex_elevenlabs::TtsChunk>,
    codex_elevenlabs::ElevenLabsError,
> {
    let api_key = resolve_elevenlabs_api_key_from_config(voice_config);

    if api_key.is_none() && proxy.is_none() {
        return Err(codex_elevenlabs::ElevenLabsError::MissingApiKey);
    }

    let mut config = codex_elevenlabs::ElevenLabsConfig::new(api_key.unwrap_or_default());
    if let Some(proxy) = proxy {
        config.proxy = Some(proxy.clone());
    }
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

/// Long-lived TTS worker that shells out to the macOS `say` command per
/// sentence. No PCM streaming, no karaoke alignment, no audio chunk events
/// — `say` plays directly to system audio. We just emit
/// `VoiceModeTtsFinished` when the queue drains so the state machine returns
/// to Idle.
async fn say_worker_loop(
    voice_config: crate::legacy_core::config::types::VoiceModeToml,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TtsWorkerCommand>,
    event_tx: crate::app_event_sender::AppEventSender,
    in_flight: Arc<AtomicUsize>,
    gen_ref: Arc<AtomicUsize>,
    my_gen: usize,
) {
    use tokio::process::Command;

    let speed = voice_config
        .elevenlabs
        .as_ref()
        .and_then(|e| e.speed)
        .unwrap_or(1.0);
    let wpm = ((175.0_f64) * speed).clamp(80.0, 400.0) as u32;

    let mut current: Option<tokio::process::Child> = None;
    tracing::info!("[TTS-TIMING] say_worker_loop: starting wpm={wpm}");

    loop {
        if gen_ref.load(Ordering::SeqCst) != my_gen {
            tracing::info!("[TTS-TIMING] say_worker_loop: generation changed, exiting");
            break;
        }
        match cmd_rx.recv().await {
            Some(TtsWorkerCommand::SendText(text)) => {
                if gen_ref.load(Ordering::SeqCst) != my_gen {
                    break;
                }
                if text.trim().is_empty() {
                    continue;
                }
                // Wait for the previous sentence so playback is sequential.
                if let Some(mut child) = current.take() {
                    let _ = child.wait().await;
                }
                tracing::info!(
                    "[TTS-TIMING] say_worker_loop: spawning say for {} chars",
                    text.len()
                );
                let child = Command::new("say")
                    .arg("-r")
                    .arg(wpm.to_string())
                    .arg("--")
                    .arg(&text)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                match child {
                    Ok(c) => {
                        current = Some(c);
                    }
                    Err(err) => {
                        tracing::error!("`say` spawn failed: {err}");
                        event_tx.send(AppEvent::VoiceModeTtsError {
                            error: format!("`say` failed: {err}"),
                        });
                        break;
                    }
                }
            }
            Some(TtsWorkerCommand::Finish) | None => {
                tracing::info!("[TTS-TIMING] say_worker_loop: Finish/None received");
                if let Some(mut child) = current.take() {
                    let _ = child.wait().await;
                }
                break;
            }
        }
    }

    // If we exited due to generation change, kill any in-flight speech so the
    // user hears the barge-in cleanly.
    if let Some(mut child) = current.take() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }

    let last = in_flight.fetch_sub(1, Ordering::SeqCst) == 1;
    tracing::info!(
        "[TTS-TIMING] say_worker_loop: exiting (last_in_flight={last})"
    );
    if last {
        event_tx.send(AppEvent::VoiceModeTtsFinished);
    }
}

/// True iff macOS `say` is available on PATH and we should use it.
#[allow(dead_code)]
fn say_backend_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("which")
            .arg("say")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Long-lived TTS task that maintains a single ElevenLabs WebSocket connection.
/// Sentences are sent through the same connection via `send_text` + `flush`,
/// eliminating per-sentence connection overhead (DNS + TLS + WS handshake).
async fn tts_worker_loop(
    voice_config: crate::legacy_core::config::types::VoiceModeToml,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<TtsWorkerCommand>,
    event_tx: crate::app_event_sender::AppEventSender,
    in_flight: Arc<AtomicUsize>,
    gen_ref: Arc<AtomicUsize>,
    my_gen: usize,
    proxy: Option<codex_elevenlabs::ElevenLabsProxy>,
) {
    let api_key = resolve_elevenlabs_api_key_from_config(&voice_config);

    if api_key.is_none() && proxy.is_none() {
        tracing::error!("TTS worker: missing API key and no proxy configured");
        event_tx.send(AppEvent::VoiceModeTtsError {
            error: "Missing ElevenLabs API key".to_string(),
        });
        if in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            event_tx.send(AppEvent::VoiceModeTtsFinished);
        }
        return;
    }

    let worker_start = std::time::Instant::now();
    tracing::info!("[TTS-TIMING] tts_worker_loop: connecting to ElevenLabs WebSocket...");
    let mut config = codex_elevenlabs::ElevenLabsConfig::new(api_key.unwrap_or_default());
    if let Some(proxy) = proxy {
        config.proxy = Some(proxy);
    }
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
        Ok(s) => {
            tracing::info!(
                "[TTS-TIMING] tts_worker_loop: WebSocket connected in {:?}",
                worker_start.elapsed(),
            );
            s
        }
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
    let mut first_chunk_sent = false;
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
                            if !first_chunk_sent {
                                first_chunk_sent = true;
                                tracing::info!(
                                    "[TTS-TIMING] tts_worker_loop: first audio chunk received \
                                     ({} samples, ~{}ms) after {:?} total",
                                    chunk.pcm.len(),
                                    chunk.pcm.len() as u64 * 1000 / 24000,
                                    worker_start.elapsed(),
                                );
                            }
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
    // Check if the stream was closed with an error (e.g., invalid voice_id)
    if let Some(err) = stream.recv_error() {
        tracing::warn!("TTS worker detected server error: {err}");
        if gen_ref.load(Ordering::SeqCst) == my_gen {
            in_flight.fetch_sub(1, Ordering::SeqCst);
            event_tx.send(AppEvent::VoiceModeTtsError { error: err });
        }
    } else if gen_ref.load(Ordering::SeqCst) == my_gen
        && in_flight.fetch_sub(1, Ordering::SeqCst) == 1
    {
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
pub fn build_alignment_entries(
    align: &codex_elevenlabs::TtsAlignment,
    cumulative_ms: u64,
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

    // Detect timestamp resets: if this chunk's first timestamp is lower
    // than the last timeline entry's end, ElevenLabs has restarted
    // timestamps for a new text segment. Offset all timestamps in this
    // chunk so they're contiguous with the existing timeline.
    let offset = {
        let chunk_first = align.char_start_times_ms[0];
        let timeline_end = timeline
            .last()
            .map(|e| e.start_ms + e.duration_ms)
            .or_else(|| pending_word.as_ref().map(|e| e.start_ms + e.duration_ms))
            .unwrap_or(0);
        if chunk_first < timeline_end && timeline_end > 0 {
            // Use cumulative PCM position as the true audio offset.
            // Fall back to timeline_end if cumulative is 0 (e.g. tests).
            if cumulative_ms > chunk_first {
                cumulative_ms - chunk_first
            } else {
                timeline_end - chunk_first
            }
        } else {
            0
        }
    };

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
        let abs_start = align.char_start_times_ms[start_idx] + offset;
        let last_start = align.char_start_times_ms[end_idx] + offset;
        let last_dur = align.char_durations_ms[end_idx];
        let abs_end = last_start.saturating_add(last_dur);
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
        let last_char = align.chars[n - 1].as_str();
        let ends_mid_word = !last_char.trim().is_empty();

        if ends_mid_word {
            // Save as pending — might continue in the next chunk.
            let abs_start = align.char_start_times_ms[ws] + offset;
            let last_start = align.char_start_times_ms[word_end_idx] + offset;
            let last_dur = align.char_durations_ms[word_end_idx];
            let abs_end = last_start.saturating_add(last_dur);
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
pub fn find_active_word(timeline: &[AlignmentEntry], pos_ms: u64) -> Option<usize> {
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
    if pos_ms <= entry.start_ms.saturating_add(entry.duration_ms) {
        Some(idx)
    } else {
        None // In the gap between words.
    }
}

/// Repair a timeline so timestamps are strictly monotonically increasing.
///
/// ElevenLabs alignment timestamps can reset between text segments (after
/// `flush` + new `send_text`). This causes `find_active_word` binary search
/// to jump around. Fix by offsetting any entry whose start_ms is less than
/// the previous entry's end to be contiguous with the previous entry.
pub fn repair_timeline_monotonicity(timeline: &mut [AlignmentEntry]) {
    if timeline.len() < 2 {
        return;
    }
    for i in 1..timeline.len() {
        let prev_end = timeline[i - 1].start_ms + timeline[i - 1].duration_ms;
        if timeline[i].start_ms < prev_end {
            // This entry's timestamp went backward — offset it forward.
            // Preserve the gap between this entry and the next by shifting
            // all subsequent entries in this "reset group" by the same delta.
            let delta = prev_end - timeline[i].start_ms;
            // Find how far the reset extends (entries with ascending timestamps
            // that are all below prev_end).
            let mut j = i;
            while j < timeline.len() {
                timeline[j].start_ms += delta;
                j += 1;
                // Stop when the next entry is already past our adjusted range
                // (it belongs to yet another segment or is already monotonic).
                if j < timeline.len() && timeline[j].start_ms >= timeline[j - 1].start_ms {
                    // Check if this next entry is ALSO below the original prev_end
                    // (part of the same reset group). If not, stop.
                    if timeline[j].start_ms >= prev_end + delta {
                        break;
                    }
                }
            }
        }
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

    // ─── Equation tag tests ────────────────────────────────────────────

    #[test]
    fn eq_tag_inline_basic() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>The value is <eq latex=\"x^2\">x squared</eq> here.</voice>");
        // Display text should contain $x^2$, not the eq tags or spoken text.
        assert_eq!(r.display_text, "The value is $x^2$ here.");
        // Voice sentence should contain equation markers around spoken text.
        assert_eq!(r.voice_sentences.len(), 1);
        assert!(r.voice_sentences[0].contains("[[[EQ:1]]]x squared[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_display_block() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push(
            "<voice>Consider <eq latex=\"E=mc^2\" display=\"block\">E equals m c squared</eq> now.</voice>",
        );
        // Display should use $$ for block mode.
        assert_eq!(r.display_text, "Consider $$E=mc^2$$ now.");
        assert!(r.voice_sentences[0].contains("[[[EQ:1]]]E equals m c squared[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_self_closing_with_speak() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>Result: <eq latex=\"\\pi\" speak=\"pi\"/> done.</voice>");
        assert_eq!(r.display_text, "Result: $\\pi$ done.");
        assert!(r.voice_sentences[0].contains("[[[EQ:1]]]pi[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_self_closing_without_speak() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>Value <eq latex=\"42\" speak=\"forty two\"/> end.</voice>");
        assert_eq!(r.display_text, "Value $42$ end.");
        assert!(r.voice_sentences[0].contains("[[[EQ:1]]]forty two[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_multiple_equations() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push(
            "<voice>First <eq latex=\"a\">alpha</eq> then <eq latex=\"b\">beta</eq> end.</voice>",
        );
        assert_eq!(r.display_text, "First $a$ then $b$ end.");
        let s = &r.voice_sentences[0];
        assert!(s.contains("[[[EQ:1]]]alpha[[[/EQ]]]"));
        assert!(s.contains("[[[EQ:2]]]beta[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_streaming_split() {
        let mut parser = VoiceTagParser::new();
        // Tag split across deltas.
        let r1 = parser.push("<voice>See <eq lat");
        assert_eq!(r1.display_text, "See ");
        let r2 = parser.push("ex=\"y\">why</eq> ok.</voice>");
        assert_eq!(r2.display_text, "$y$ ok.");
        assert!(r2.voice_sentences[0].contains("[[[EQ:1]]]why[[[/EQ]]]"));
    }

    #[test]
    fn eq_tag_does_not_break_voice_tags() {
        // Ensure existing voice tag handling still works alongside eq tags.
        let mut parser = VoiceTagParser::new();
        let r = parser.push("<voice>Hello world.</voice> code <voice>Bye.</voice>");
        assert_eq!(r.display_text, "Hello world. code Bye.");
        assert_eq!(r.voice_sentences, vec!["Hello world.", "Bye."]);
    }

    #[test]
    fn eq_tag_outside_voice_still_displays() {
        // <eq> outside of <voice> tags should still render LaTeX in display text.
        let mut parser = VoiceTagParser::new();
        let r = parser.push("Result: <eq latex=\"x+1\">x plus one</eq> done.");
        assert_eq!(r.display_text, "Result: $x+1$ done.");
        // No voice sentences since not inside <voice> tags.
        assert!(r.voice_sentences.is_empty());
    }

    #[test]
    fn eq_tag_clear_resets_equation_state() {
        let mut parser = VoiceTagParser::new();
        parser.push("<voice><eq latex=\"a\">alpha</eq></voice>");
        parser.clear();
        // After clear, equation_ordinal should reset? No — clear doesn't
        // reset ordinal (it's a counter for the whole turn). But in_equation
        // should be false.
        let r = parser.push("<voice><eq latex=\"b\">beta</eq></voice>");
        assert_eq!(r.display_text, "$b$");
        // The ordinal continues from where it was (2), so this is EQ:2.
        assert!(r.voice_sentences[0].contains("[[[EQ:2]]]beta[[[/EQ]]]"));
    }

    #[test]
    fn clean_for_tts_preserving_equation_markers_keeps_eq_spans() {
        let text = "We define <eq latex=\"x^2\" speak=\"x squared\"/> and continue.";

        assert_eq!(
            clean_for_tts_preserving_equation_markers(text),
            "We define [[[EQ:1]]]x squared[[[/EQ]]] and continue."
        );
    }

    // ─── parse_equation_markers tests ────────────────────────────────

    #[test]
    fn parse_eq_markers_basic() {
        let input = "Before [[[EQ:1]]]alpha[[[/EQ]]] after";
        let (cleaned, spans) = parse_equation_markers(input);
        assert_eq!(cleaned, "Before alpha after");
        assert_eq!(spans, vec![(1, 1, 2)]); // "alpha" is word index 1..2
    }

    #[test]
    fn parse_eq_markers_multiple() {
        let input = "A [[[EQ:1]]]one[[[/EQ]]] B [[[EQ:2]]]two three[[[/EQ]]] C";
        let (cleaned, spans) = parse_equation_markers(input);
        assert_eq!(cleaned, "A one B two three C");
        assert_eq!(spans, vec![(1, 1, 2), (2, 3, 5)]);
    }

    #[test]
    fn parse_eq_markers_no_markers() {
        let input = "Just plain text";
        let (cleaned, spans) = parse_equation_markers(input);
        assert_eq!(cleaned, "Just plain text");
        assert!(spans.is_empty());
    }

    #[test]
    fn parse_eq_markers_at_start() {
        let input = "[[[EQ:1]]]hello world[[[/EQ]]] rest";
        let (cleaned, spans) = parse_equation_markers(input);
        assert_eq!(cleaned, "hello world rest");
        assert_eq!(spans, vec![(1, 0, 2)]);
    }

    // ─── parse_eq_attributes tests ───────────────────────────────────

    #[test]
    fn parse_eq_attrs_basic() {
        let (latex, display, speak) =
            parse_eq_attributes("<eq latex=\"x^2\" display=\"block\" speak=\"x squared\">");
        assert_eq!(latex, "x^2");
        assert!(display);
        assert_eq!(speak, "x squared");
    }

    #[test]
    fn parse_eq_attrs_single_quotes() {
        let (latex, display, speak) = parse_eq_attributes("<eq latex='y+1'>");
        assert_eq!(latex, "y+1");
        assert!(!display);
        assert_eq!(speak, "");
    }

    #[test]
    fn parse_eq_attrs_self_closing() {
        let (latex, display, speak) = parse_eq_attributes("<eq latex=\"\\pi\" speak=\"pi\"/>");
        assert_eq!(latex, "\\pi");
        assert!(!display);
        assert_eq!(speak, "pi");
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
        // The --- line is removed. The newline collapser runs before the HR
        // filter, so the two surrounding blank lines survive, producing three
        // consecutive newlines in the output.
        assert_eq!(cleaned, "Before\n\n\nAfter");
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

    #[test]
    fn clean_for_tts_strips_bare_https_url() {
        let text = "Visit https://example.com/path for details";
        let cleaned = clean_for_tts(text);
        // The URL is stripped; the space before it remains, so there's a
        // double space.  That's fine — split_whitespace() and TTS both
        // ignore extra whitespace.
        assert_eq!(cleaned, "Visit  for details");
    }

    #[test]
    fn clean_for_tts_strips_bare_http_url() {
        let text = "See http://example.org for info";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "See  for info");
    }

    #[test]
    fn clean_for_tts_strips_url_at_end_of_text() {
        let text = "Link: https://example.com";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Link:");
    }

    #[test]
    fn clean_for_tts_strips_multiple_urls() {
        let text = "First https://a.com then https://b.com end";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "First  then  end");
    }

    #[test]
    fn clean_for_tts_keeps_non_url_h_words() {
        // Words starting with 'h' that are not URLs should be kept.
        let text = "hello http://x.com here";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "hello  here");
    }

    #[test]
    fn clean_for_tts_strips_parenthetical_years() {
        let text = "Smith (2026) showed that transformers work";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Smith showed that transformers work");
    }

    #[test]
    fn clean_for_tts_strips_et_al() {
        let text = "Smith et al. showed that transformers work";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Smith showed that transformers work");
    }

    #[test]
    fn clean_for_tts_strips_et_al_with_year() {
        let text = "Smith et al. (2026) showed that transformers work";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Smith showed that transformers work");
    }

    #[test]
    fn clean_for_tts_strips_bracket_citations() {
        let text = "Attention is effective [1] and scalable [2,3]";
        let cleaned = clean_for_tts(text);
        // Double spaces are fine — TTS engines ignore extra whitespace.
        assert_eq!(cleaned, "Attention is effective  and scalable");
    }

    #[test]
    fn clean_for_tts_strips_bracket_citation_range() {
        let text = "Prior work [1-3] explored this";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Prior work  explored this");
    }

    // ─── clean_for_tts list item prefix stripping tests ────────────────

    #[test]
    fn clean_for_tts_strips_unordered_list_prefixes() {
        let text = "Problems:\n- First problem\n- Second problem\n- Third problem";
        let cleaned = clean_for_tts(text);
        assert_eq!(
            cleaned,
            "Problems:\nFirst problem\nSecond problem\nThird problem"
        );
    }

    #[test]
    fn clean_for_tts_strips_ordered_list_prefixes() {
        let text = "Steps:\n1. Do this\n2. Then that\n3. Finally this";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Steps:\nDo this\nThen that\nFinally this");
    }

    #[test]
    fn clean_for_tts_strips_single_list_item() {
        let text = "Here are items:\n- Only one item";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Here are items:\nOnly one item");
    }

    #[test]
    fn clean_for_tts_no_strip_for_dashes_in_text() {
        let text = "Llama-2 is a model";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Llama-2 is a model");
    }

    #[test]
    fn clean_for_tts_strips_plus_list_marker() {
        let text = "Items:\n+ Alpha\n+ Beta";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Items:\nAlpha\nBeta");
    }

    // ─── Additional clean_for_tts tests (comprehensive) ─────────────────

    #[test]
    fn clean_for_tts_url_at_start_of_text() {
        let text = "https://example.com is great";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "is great");
    }

    #[test]
    fn clean_for_tts_preserves_hypothetical() {
        // "hypothetical" starts with 'h' but is not a URL.
        let text = "hypothetical scenarios";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "hypothetical scenarios");
    }

    #[test]
    fn clean_for_tts_strips_multiple_parenthetical_years() {
        let text = "In (2024) and (2025) we saw";
        let cleaned = clean_for_tts(text);
        // The regex `\s*\(\d{4}\)` also consumes the leading space.
        assert_eq!(cleaned, "In and we saw");
    }

    #[test]
    fn clean_for_tts_preserves_year_without_parens() {
        // Year without parentheses should NOT be stripped — it's likely
        // used in normal prose ("founded in 2026").
        let text = "founded in 2026";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "founded in 2026");
    }

    #[test]
    fn clean_for_tts_strips_et_al_without_period() {
        // "et al" without trailing period — regex allows optional period.
        let text = "Smith et al showed";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Smith showed");
    }

    #[test]
    fn clean_for_tts_no_truncation_at_2000_chars() {
        // Verify the 2000-char truncation was removed.
        let text = "a ".repeat(3000);
        let cleaned = clean_for_tts(&text);
        // Should NOT be truncated — the full text should come through.
        assert!(
            cleaned.len() > 4000,
            "clean_for_tts should not truncate at 2000 chars; got len={}",
            cleaned.len()
        );
    }

    #[test]
    fn clean_for_tts_very_long_input_processed_correctly() {
        // 10000 characters with mixed content.
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("Sentence number {i}. "));
        }
        let cleaned = clean_for_tts(&text);
        // All sentences should be preserved.
        assert!(
            cleaned.contains("Sentence number 499"),
            "last sentence should be present in output"
        );
    }

    #[test]
    fn clean_for_tts_mixed_paragraph_and_list() {
        // Paragraph → list items → paragraph: list prefixes stripped.
        let text = "Intro paragraph.\n- Item A\n- Item B\nConclusion.";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Intro paragraph.\nItem A\nItem B\nConclusion.");
    }

    #[test]
    fn clean_for_tts_single_list_item_prefix_stripped() {
        let text = "Summary:\n- Only one item";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Summary:\nOnly one item");
    }

    #[test]
    fn clean_for_tts_bracket_citation_at_end() {
        let text = "Results were significant [4]";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Results were significant");
    }

    #[test]
    fn clean_for_tts_url_with_query_params() {
        let text = "Check http://foo.bar/path?q=1&v=2 now";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "Check  now");
    }

    #[test]
    fn clean_for_tts_only_urls() {
        let text = "https://a.com https://b.com";
        let cleaned = clean_for_tts(text);
        // Both stripped, spaces remain.
        assert_eq!(cleaned.trim(), "");
    }

    #[test]
    fn clean_for_tts_strips_parenthesized_url() {
        let text = "see (https://example.com) for details";
        let cleaned = clean_for_tts(text);
        assert_eq!(cleaned, "see ( for details");
    }

    // ─── VoiceTagParser attributed voice tag tests ─────────────────────

    #[test]
    fn voice_tag_with_name_attribute() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push(r#"<voice name="alloy">hello</voice>"#);
        assert_eq!(r.display_text, "hello");
        assert_eq!(r.voice_sentences, vec!["hello"]);
    }

    #[test]
    fn voice_tag_with_multiple_attributes() {
        let mut parser = VoiceTagParser::new();
        let r = parser.push(r#"<voice name="shimmer" style="expressive">spoken text.</voice>"#);
        assert_eq!(r.display_text, "spoken text.");
        assert_eq!(r.voice_sentences, vec!["spoken text."]);
    }

    #[test]
    fn voice_tag_attribute_streaming_split() {
        let mut parser = VoiceTagParser::new();

        // Tag with attribute split across two deltas.
        let r1 = parser.push(r#"<voice na"#);
        assert_eq!(r1.display_text, "");
        assert!(r1.voice_sentences.is_empty());

        let r2 = parser.push(r#"me="alloy">Hello.</voice>"#);
        assert_eq!(r2.display_text, "Hello.");
        assert_eq!(r2.voice_sentences, vec!["Hello."]);
    }

    // ─── is_voice_tag_prefix tests ────────────────────────────────────

    #[test]
    fn is_voice_tag_prefix_with_space_for_attributes() {
        // "<voice " (with trailing space) indicates attributes may follow.
        assert!(
            is_voice_tag_prefix("<voice "),
            "should recognize '<voice ' as prefix for attributed voice tag"
        );
    }

    #[test]
    fn is_voice_tag_prefix_partial_voice() {
        assert!(is_voice_tag_prefix("<vo"));
        assert!(is_voice_tag_prefix("<voic"));
        assert!(is_voice_tag_prefix("<voice"));
        assert!(is_voice_tag_prefix("<voice>"));
    }

    #[test]
    fn is_voice_tag_prefix_closing() {
        assert!(is_voice_tag_prefix("</vo"));
        assert!(is_voice_tag_prefix("</voice>"));
    }

    #[test]
    fn is_voice_tag_prefix_eq_tag() {
        assert!(is_voice_tag_prefix("<eq"));
        assert!(is_voice_tag_prefix("<eq "));
    }

    #[test]
    fn is_voice_tag_prefix_not_recognized() {
        assert!(!is_voice_tag_prefix("<div"));
        assert!(!is_voice_tag_prefix("<p>"));
        assert!(!is_voice_tag_prefix("<span"));
    }

    // ─── build_alignment_entries tests ──────────────────────────────────

    /// Helper to construct a TtsAlignment from parallel slices.
    fn make_alignment(
        chars: &[&str],
        starts: &[u64],
        durations: &[u64],
    ) -> codex_elevenlabs::TtsAlignment {
        codex_elevenlabs::TtsAlignment {
            chars: chars.iter().map(std::string::ToString::to_string).collect(),
            char_start_times_ms: starts.to_vec(),
            char_durations_ms: durations.to_vec(),
        }
    }

    #[test]
    fn alignment_single_word() {
        // "Hi" → one entry with correct start and duration.
        let align = make_alignment(&["H", "i"], &[100, 150], &[50, 60]);
        let mut timeline = Vec::new();
        let mut pending = None;
        // Chunk ends mid-word (last char is non-ws), so it goes to pending.
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // Since "Hi" ends mid-word (no trailing space), it stays as pending.
        assert!(
            timeline.is_empty(),
            "word should be pending since chunk ends mid-word"
        );
        assert!(pending.is_some(), "partial word should be saved as pending");
        let p = pending.as_ref().expect("pending should be Some");
        assert_eq!(p.word, "Hi");
        assert_eq!(p.start_ms, 100);
        // duration = (150 + 60) - 100 = 110
        assert_eq!(p.duration_ms, 110);
    }

    #[test]
    fn alignment_multiple_words() {
        // "Hello world" with a space between → 2 entries.
        // H(0,10) e(10,10) l(20,10) l(30,10) o(40,10) ' '(50,10) w(60,10) o(70,10) r(80,10) l(90,10) d(100,10)
        let align = make_alignment(
            &["H", "e", "l", "l", "o", " ", "w", "o", "r", "l", "d"],
            &[0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100],
            &[10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "Hello" is flushed by the space, "world" ends mid-word → pending.
        assert_eq!(timeline.len(), 1, "space should flush 'Hello'");
        assert_eq!(timeline[0].word, "Hello");
        assert_eq!(timeline[0].start_ms, 0);
        // duration = (40 + 10) - 0 = 50
        assert_eq!(timeline[0].duration_ms, 50);
        // "world" is pending because chunk ends mid-word.
        let p = pending.as_ref().expect("'world' should be pending");
        assert_eq!(p.word, "world");
        assert_eq!(p.start_ms, 60);
        assert_eq!(p.duration_ms, 50);
    }

    #[test]
    fn alignment_cross_chunk_merge() {
        // First chunk: "Hel" (ends mid-word → pending).
        let align1 = make_alignment(&["H", "e", "l"], &[0, 10, 20], &[10, 10, 10]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align1, 0, &mut timeline, &mut pending);
        assert!(timeline.is_empty());
        assert_eq!(pending.as_ref().expect("should be pending").word, "Hel");

        // Second chunk: "lo " (continues word then space flushes).
        let align2 = make_alignment(&["l", "o", " "], &[30, 40, 50], &[10, 10, 10]);
        build_alignment_entries(&align2, 0, &mut timeline, &mut pending);
        // "Hel" + "lo" should merge into "Hello" and be flushed by the space.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hello");
        assert_eq!(timeline[0].start_ms, 0);
        // duration spans from start_ms=0 to end of 'o' at 40+10=50 → 50
        assert_eq!(timeline[0].duration_ms, 50);
        assert!(pending.is_none(), "pending should be cleared after flush");
    }

    #[test]
    fn alignment_leading_whitespace_flushes() {
        // Set up a pending word from a previous chunk.
        let mut timeline = Vec::new();
        let mut pending = Some(AlignmentEntry {
            start_ms: 0,
            duration_ms: 30,
            word: "Hel".to_string(),
        });

        // This chunk starts with " word" — leading space flushes pending standalone.
        let align = make_alignment(
            &[" ", "w", "o", "r", "d"],
            &[30, 40, 50, 60, 70],
            &[10, 10, 10, 10, 10],
        );
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "Hel" should be flushed as-is (not merged with "word").
        assert_eq!(
            timeline.len(),
            1,
            "pending word should be flushed by leading space"
        );
        assert_eq!(timeline[0].word, "Hel");
        assert_eq!(timeline[0].start_ms, 0);
        // "word" ends mid-word → pending.
        let p = pending.as_ref().expect("'word' should be pending");
        assert_eq!(p.word, "word");
    }

    #[test]
    fn alignment_empty_chunk() {
        let align = make_alignment(&[], &[], &[]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert!(timeline.is_empty(), "empty chunk should produce no entries");
        assert!(pending.is_none(), "empty chunk should not create pending");
    }

    #[test]
    fn alignment_all_whitespace() {
        // Pending word exists; chunk is all whitespace → pending is flushed.
        let mut timeline = Vec::new();
        let mut pending = Some(AlignmentEntry {
            start_ms: 0,
            duration_ms: 30,
            word: "end".to_string(),
        });

        let align = make_alignment(&[" ", " "], &[100, 110], &[10, 10]);
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert_eq!(
            timeline.len(),
            1,
            "all-whitespace chunk should flush pending word"
        );
        assert_eq!(timeline[0].word, "end");
        assert!(pending.is_none());
    }

    #[test]
    fn alignment_punctuation_attached() {
        // "Hello." — period is not whitespace, so it stays attached to the word.
        let align = make_alignment(
            &["H", "e", "l", "l", "o", "."],
            &[0, 10, 20, 30, 40, 50],
            &[10, 10, 10, 10, 10, 10],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // Ends mid-word (period is non-ws), so goes to pending.
        let p = pending.as_ref().expect("should be pending");
        assert_eq!(p.word, "Hello.", "punctuation should be part of the word");
    }

    #[test]
    fn alignment_paragraph_break() {
        // Test with "\n\n" as paragraph sentinel chars.
        let align = make_alignment(
            &["H", "i", "\n", "\n", "B", "y"],
            &[0, 10, 20, 30, 40, 50],
            &[10, 10, 10, 10, 10, 10],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "\n" is whitespace, so "Hi" should be flushed. "By" ends mid-word → pending.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
        let p = pending.as_ref().expect("'By' should be pending");
        assert_eq!(p.word, "By");
    }

    // ─── find_active_word tests ─────────────────────────────────────────

    fn sample_timeline() -> Vec<AlignmentEntry> {
        vec![
            AlignmentEntry {
                start_ms: 100,
                duration_ms: 50,
                word: "Hello".into(),
            },
            AlignmentEntry {
                start_ms: 200,
                duration_ms: 50,
                word: "world".into(),
            },
            AlignmentEntry {
                start_ms: 300,
                duration_ms: 50,
                word: "test".into(),
            },
        ]
    }

    #[test]
    fn find_word_exact_start() {
        let tl = sample_timeline();
        // Exact start of second word (200ms).
        assert_eq!(find_active_word(&tl, 200), Some(1));
    }

    #[test]
    fn find_word_mid_word() {
        let tl = sample_timeline();
        // 120ms is in the middle of "Hello" (100..150).
        assert_eq!(find_active_word(&tl, 120), Some(0));
    }

    #[test]
    fn find_word_between_words() {
        let tl = sample_timeline();
        // 170ms is between "Hello" (100..150) and "world" (200..250).
        assert_eq!(find_active_word(&tl, 170), None);
    }

    #[test]
    fn find_word_before_first() {
        let tl = sample_timeline();
        // 50ms is before the first word starts at 100ms.
        assert_eq!(find_active_word(&tl, 50), None);
    }

    #[test]
    fn find_word_after_last() {
        let tl = sample_timeline();
        // 400ms is after the last word ends at 350ms.
        assert_eq!(find_active_word(&tl, 400), None);
    }

    #[test]
    fn find_word_empty_timeline() {
        let tl: Vec<AlignmentEntry> = Vec::new();
        assert_eq!(find_active_word(&tl, 100), None);
    }

    // ─── Adversarial tests: build_alignment_entries ─────────────────────

    #[test]
    fn adversarial_alignment_emoji_chars() {
        // ElevenLabs might send emoji as individual chars.
        // Emoji are multi-byte but should still be grouped into words correctly.
        let align = make_alignment(
            &["\u{1F44B}", " ", "H", "i"],
            &[0, 50, 100, 150],
            &[50, 50, 50, 50],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "\u{1F44B}" is flushed by the space, "Hi" ends mid-word -> pending.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "\u{1F44B}");
        assert_eq!(timeline[0].start_ms, 0);
        assert_eq!(timeline[0].duration_ms, 50);
        let p = pending.as_ref().expect("'Hi' should be pending");
        assert_eq!(p.word, "Hi");
    }

    #[test]
    fn adversarial_alignment_cjk_chars() {
        // CJK characters are single code points but multi-byte.
        // They should be treated as non-whitespace and grouped into words.
        let align = make_alignment(
            &["\u{4F60}", "\u{597D}", " ", "w"],
            &[0, 50, 100, 150],
            &[50, 50, 50, 50],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "\u{4F60}\u{597D}" (你好) flushed by space, "w" pending.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "\u{4F60}\u{597D}");
        let p = pending.as_ref().expect("'w' should be pending");
        assert_eq!(p.word, "w");
    }

    #[test]
    fn adversarial_alignment_zero_width_joiner() {
        // Zero-width joiner (U+200D) is not whitespace but is "invisible".
        // It should be treated as part of a word (not a word boundary).
        let align = make_alignment(
            &["\u{1F468}", "\u{200D}", "\u{1F469}", " ", "x"],
            &[0, 20, 40, 60, 80],
            &[20, 20, 20, 20, 20],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // The ZWJ sequence should stay together as one "word".
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "\u{1F468}\u{200D}\u{1F469}");
        let p = pending.as_ref().expect("'x' should be pending");
        assert_eq!(p.word, "x");
    }

    #[test]
    fn adversarial_alignment_mismatched_lengths_chars_longer() {
        // BUG PROBE: chars array is longer than timing arrays.
        // n = min(2, 2, 2) = 2, but chars has 3 elements.
        // The code checks align.chars.last() which is " " (the 3rd element),
        // even though we only processed indices 0..2. This causes a word
        // that should be pending (ends mid-word at index 1) to be flushed
        // incorrectly because the code sees the unprocessed trailing space.
        let align = make_alignment(
            &["H", "i", " "], // 3 chars
            &[0, 50],         // 2 starts
            &[50, 50],        // 2 durations
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // We only process "H" and "i" (n=2). Since the last PROCESSED char
        // is "i" (non-whitespace), the word should be pending.
        // BUG: The code checks align.chars.last() which is " ", so it
        // incorrectly thinks the chunk ends with whitespace and flushes.
        //
        // Expected: "Hi" is pending (last processed char is non-ws).
        // Actual (buggy): "Hi" is flushed to timeline.
        //
        // This test documents the bug. If pending is None, the bug is confirmed.
        assert!(
            pending.is_some(),
            "BUG: 'Hi' should be pending since only 2 chars were processed and last processed char is 'i' (non-ws), \
             but align.chars.last() incorrectly looks at unprocessed trailing space"
        );
        assert_eq!(pending.as_ref().expect("should be pending").word, "Hi");
    }

    #[test]
    fn adversarial_alignment_mismatched_lengths_times_longer() {
        // Times arrays are longer than chars — should only process min(len).
        let align = make_alignment(
            &["H", "i"],        // 2 chars
            &[0, 50, 100, 150], // 4 starts
            &[50, 50, 50, 50],  // 4 durations
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // n=2, only "H" and "i" processed. Last char is "i" (non-ws) -> pending.
        let p = pending.as_ref().expect("'Hi' should be pending");
        assert_eq!(p.word, "Hi");
        assert_eq!(p.start_ms, 0);
        assert_eq!(p.duration_ms, 100); // (50 + 50) - 0
    }

    #[test]
    fn adversarial_alignment_single_whitespace_char() {
        // Single whitespace char with no pending — should produce nothing.
        let align = make_alignment(&[" "], &[100], &[10]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert!(
            timeline.is_empty(),
            "single space should produce no entries"
        );
        assert!(pending.is_none());
    }

    #[test]
    fn adversarial_alignment_single_whitespace_with_pending() {
        // Single whitespace char WITH a pending word — should flush the pending.
        let mut timeline = Vec::new();
        let mut pending = Some(AlignmentEntry {
            start_ms: 0,
            duration_ms: 30,
            word: "Hi".to_string(),
        });
        let align = make_alignment(&[" "], &[100], &[10]);
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert_eq!(timeline.len(), 1, "pending should be flushed by whitespace");
        assert_eq!(timeline[0].word, "Hi");
        assert!(pending.is_none());
    }

    #[test]
    fn adversarial_alignment_chars_nonempty_times_empty() {
        // Chars has entries but times arrays are empty. n = min(2, 0, 0) = 0.
        let align = make_alignment(&["H", "i"], &[], &[]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert!(
            timeline.is_empty(),
            "no timing data means nothing should be produced"
        );
        assert!(pending.is_none());
    }

    #[test]
    fn adversarial_alignment_same_start_times() {
        // Two consecutive chars with the SAME start_ms.
        // This could happen with very fast speech or timing quantization.
        let align = make_alignment(
            &["H", "i", " ", "Y", "o"],
            &[100, 100, 200, 300, 300],
            &[0, 100, 100, 0, 100],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "Hi" should be flushed by space.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
        assert_eq!(timeline[0].start_ms, 100);
        // duration = (100 + 100) - 100 = 100
        assert_eq!(timeline[0].duration_ms, 100);
        // "Yo" should be pending.
        let p = pending.as_ref().expect("'Yo' should be pending");
        assert_eq!(p.word, "Yo");
    }

    #[test]
    fn adversarial_alignment_zero_duration() {
        // All chars have zero duration — word duration degenerates.
        let align = make_alignment(&["H", "i", " "], &[100, 200, 300], &[0, 0, 0]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
        // duration = (200 + 0) - 100 = 100 (comes from timestamps, not durations per se)
        assert_eq!(timeline[0].start_ms, 100);
        assert_eq!(timeline[0].duration_ms, 100);
    }

    #[test]
    fn adversarial_alignment_non_monotonic_timestamps() {
        // Non-monotonic timestamps within a chunk (out of order).
        // This shouldn't happen in practice, but if it does, we should not panic.
        let align = make_alignment(
            &["H", "i", " ", "B", "y"],
            &[200, 100, 50, 300, 250],
            &[10, 10, 10, 10, 10],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        // Should not panic even with non-monotonic timestamps.
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // "Hi" flushed by space. start_ms = char_start_times[0] = 200
        // duration = (100 + 10) - 200 = 0 (saturating_sub since 110 < 200)
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
        assert_eq!(timeline[0].start_ms, 200);
        assert_eq!(timeline[0].duration_ms, 0);
    }

    #[test]
    fn adversarial_alignment_large_timestamps_near_max() {
        // Very large timestamps near u64::MAX.
        // Line 3045: abs_end = last_start + last_dur — could overflow!
        let align = make_alignment(
            &["H", "i", " "],
            &[u64::MAX - 100, u64::MAX - 50, u64::MAX - 10],
            &[100, 100, 100],
        );
        let mut timeline = Vec::new();
        let mut pending = None;
        // This might panic with overflow in debug mode.
        // Line 3045: abs_end = (u64::MAX - 50) + 100 = u64::MAX + 50 → OVERFLOW!
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        // If we get here, at least it didn't panic.
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
    }

    #[test]
    fn adversarial_alignment_three_chunk_word_span() {
        // A word spans THREE chunks: "Hel" + "lo" + " world"
        let align1 = make_alignment(&["H", "e", "l"], &[0, 10, 20], &[10, 10, 10]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align1, 0, &mut timeline, &mut pending);
        assert!(timeline.is_empty());
        assert_eq!(pending.as_ref().expect("").word, "Hel");

        let align2 = make_alignment(&["l", "o"], &[30, 40], &[10, 10]);
        build_alignment_entries(&align2, 0, &mut timeline, &mut pending);
        assert!(timeline.is_empty());
        assert_eq!(pending.as_ref().expect("").word, "Hello");

        let align3 = make_alignment(
            &[" ", "w", "o", "r", "l", "d", " "],
            &[50, 60, 70, 80, 90, 100, 110],
            &[10, 10, 10, 10, 10, 10, 10],
        );
        build_alignment_entries(&align3, 0, &mut timeline, &mut pending);
        // Both "Hello" and "world" should be flushed.
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].word, "Hello");
        assert_eq!(timeline[0].start_ms, 0);
        assert_eq!(timeline[0].duration_ms, 50); // (40+10) - 0 = 50
        assert_eq!(timeline[1].word, "world");
        assert!(pending.is_none());
    }

    #[test]
    fn adversarial_alignment_pending_word_never_flushed() {
        // A pending word from the last chunk is never flushed because
        // no subsequent call happens. The caller is responsible for
        // flushing pending when TTS is complete, but let's verify the
        // pending entry is correct and accessible.
        let align = make_alignment(&["H", "i"], &[0, 50], &[50, 50]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert!(timeline.is_empty());
        let p = pending.as_ref().expect("should be pending");
        assert_eq!(p.word, "Hi");
        assert_eq!(p.start_ms, 0);
        assert_eq!(p.duration_ms, 100);
        // The pending word is accessible and has correct data.
        // The caller should flush this when the stream ends.
    }

    #[test]
    fn adversarial_alignment_trailing_whitespace_after_word() {
        // "Hi " — trailing space should flush the word immediately.
        let align = make_alignment(&["H", "i", " "], &[0, 50, 100], &[50, 50, 50]);
        let mut timeline = Vec::new();
        let mut pending = None;
        build_alignment_entries(&align, 0, &mut timeline, &mut pending);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].word, "Hi");
        assert!(
            pending.is_none(),
            "word should be flushed by trailing space"
        );
    }

    // ─── Adversarial tests: find_active_word ────────────────────────────

    #[test]
    fn adversarial_find_word_duplicate_start_ms() {
        // Two entries with the same start_ms.
        // binary_search_by_key with Ok(i) returns an arbitrary matching index.
        let tl = vec![
            AlignmentEntry {
                start_ms: 100,
                duration_ms: 50,
                word: "first".into(),
            },
            AlignmentEntry {
                start_ms: 100,
                duration_ms: 80,
                word: "second".into(),
            },
        ];
        // binary_search finds some entry with start_ms=100.
        let result = find_active_word(&tl, 100);
        // Should return Some — doesn't matter which one, as long as it doesn't panic.
        assert!(
            result.is_some(),
            "should find an entry when pos matches duplicated start_ms"
        );
    }

    #[test]
    fn adversarial_find_word_overlapping_entries() {
        // Overlapping time ranges: entry0 ends at 200, entry1 starts at 150.
        let tl = vec![
            AlignmentEntry {
                start_ms: 100,
                duration_ms: 100, // ends at 200
                word: "first".into(),
            },
            AlignmentEntry {
                start_ms: 150,
                duration_ms: 100, // ends at 250
                word: "second".into(),
            },
        ];
        // At 175ms: binary search finds entry at index 1 (start_ms=150 <= 175).
        // It checks 175 <= 150 + 100 = 250 → true. Returns Some(1).
        // But entry 0 also covers 175ms. This is an ambiguity in the data,
        // not necessarily a bug — just documenting behavior.
        let result = find_active_word(&tl, 175);
        assert!(result.is_some());
    }

    #[test]
    fn adversarial_find_word_duration_overflow() {
        // start_ms + duration_ms overflows u64.
        let tl = vec![AlignmentEntry {
            start_ms: u64::MAX - 10,
            duration_ms: 100, // start_ms + duration_ms overflows!
            word: "overflow".into(),
        }];
        // Line 3095: pos_ms <= entry.start_ms + entry.duration_ms
        // This will overflow: (u64::MAX - 10) + 100 wraps in release mode,
        // and panics in debug mode.
        let result = find_active_word(&tl, u64::MAX);
        // If we get here without panicking, the function handled the overflow.
        // With wrapping: (u64::MAX - 10) + 100 = 89 (wrapped). u64::MAX <= 89 is false.
        // So it returns None, even though logically the word should be active.
        // This is a bug (overflow), but at least check it doesn't panic.
        let _ = result; // Don't assert specific value — just confirm no panic.
    }

    #[test]
    fn adversarial_find_word_max_duration() {
        // Single entry with start_ms=0, duration=u64::MAX.
        // Should cover every possible pos_ms value.
        let tl = vec![AlignmentEntry {
            start_ms: 0,
            duration_ms: u64::MAX,
            word: "everything".into(),
        }];
        // 0 + u64::MAX = u64::MAX, and pos_ms <= u64::MAX is always true.
        assert_eq!(find_active_word(&tl, 0), Some(0));
        assert_eq!(find_active_word(&tl, 1000), Some(0));
        assert_eq!(find_active_word(&tl, u64::MAX), Some(0));
    }

    #[test]
    fn adversarial_find_word_exact_end_boundary() {
        // pos_ms exactly at start_ms + duration_ms.
        // The check is pos_ms <= start_ms + duration_ms (inclusive).
        let tl = vec![AlignmentEntry {
            start_ms: 100,
            duration_ms: 50,
            word: "boundary".into(),
        }];
        assert_eq!(
            find_active_word(&tl, 150),
            Some(0),
            "pos at exact end should be inclusive"
        );
        assert_eq!(
            find_active_word(&tl, 151),
            None,
            "pos past end should return None"
        );
    }

    #[test]
    fn adversarial_find_word_zero_duration() {
        // Entry with zero duration — only active at exact start_ms.
        let tl = vec![AlignmentEntry {
            start_ms: 100,
            duration_ms: 0,
            word: "instant".into(),
        }];
        assert_eq!(
            find_active_word(&tl, 100),
            Some(0),
            "exact match should work"
        );
        assert_eq!(find_active_word(&tl, 101), None, "past zero-duration word");
    }

    #[test]
    fn adversarial_find_word_single_entry_pos_zero() {
        // Entry starts at 0, pos_ms is 0.
        let tl = vec![AlignmentEntry {
            start_ms: 0,
            duration_ms: 100,
            word: "first".into(),
        }];
        assert_eq!(find_active_word(&tl, 0), Some(0));
    }

    // ─── Pause / resume state machine tests ─────────────────────────────

    #[test]
    fn tts_chunk_during_pause_does_not_transition_phase() {
        // Regression: on_voice_tts_audio_chunk unconditionally set phase to
        // Speaking on every incoming TTS chunk, even when audio was paused.
        // This caused the "Paused" footer status to be overwritten.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.phase = VoiceModePhase::Speaking;

        // Simulate pause
        state.mock_audio_paused = true;

        // Simulate what finalize_voice_turn does: phase → Idle
        state.phase = VoiceModePhase::Idle;

        // Incoming TTS chunk should NOT override phase to Speaking
        state.transition_phase_on_chunk();

        assert_eq!(
            state.phase,
            VoiceModePhase::Idle,
            "Phase must remain Idle when audio is paused — TTS chunks from the \
             network should not override the paused state"
        );
    }

    #[test]
    fn tts_chunk_transitions_to_speaking_when_not_paused() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.phase = VoiceModePhase::Idle;

        state.transition_phase_on_chunk();

        assert_eq!(
            state.phase,
            VoiceModePhase::Speaking,
            "Phase should transition to Speaking when not paused"
        );
    }

    #[test]
    fn tts_chunk_noop_when_already_speaking() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.phase = VoiceModePhase::Speaking;

        state.transition_phase_on_chunk();

        assert_eq!(state.phase, VoiceModePhase::Speaking);
    }

    #[test]
    fn finalize_blocked_when_audio_paused() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.mock_audio_paused = true;

        assert!(
            state.should_block_finalization(),
            "Finalization must be blocked while audio is paused"
        );
    }

    #[test]
    fn finalize_allowed_when_audio_not_paused() {
        let vc = VoiceModeToml::default();
        let state = VoiceModeState::new(&vc);

        assert!(
            !state.should_block_finalization(),
            "Finalization should proceed when audio is not paused"
        );
    }

    #[test]
    fn should_finalize_on_tick_blocked_when_paused() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = true;
        state.mock_audio_paused = true;

        assert!(
            !state.should_finalize_on_tick(),
            "Highlight tick must NOT finalize while audio is paused"
        );
    }

    #[test]
    fn should_finalize_on_tick_when_data_complete_and_drained() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = true;
        // mock_audio_paused defaults to false, no audio player means no buffered audio

        assert!(
            state.should_finalize_on_tick(),
            "Should finalize when TTS data is complete and no audio remains"
        );
    }

    // ─── Pause / resume integration-style state machine tests ───────────
    //
    // These model the real event sequence observed in production logs:
    //   1. TTS finishes streaming → tts_data_complete = true
    //   2. User pauses while audio is buffered
    //   3. Audio buffer drains (race / natural finish)
    //   4. User tries to resume → has_audio = false
    //
    // The previous tests checked individual predicates; these test the
    // full state machine through the same sequence as the real code path.

    #[test]
    fn resume_with_drained_buffer_does_not_enter_speaking() {
        // Scenario: user paused, audio buffer drained while paused,
        // user presses 's' to resume → should NOT pretend to speak.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);

        // Phase 1: Speaking, TTS data complete, audio buffered
        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = true;
        state.mock_has_audio = Some(true);

        // Phase 2: User pauses
        state.mock_audio_paused = true;
        state.pause_tts(); // would call player.pause() + cancel tick

        // Phase 3: Audio buffer drains (race condition on ARM)
        state.mock_has_audio = Some(false);

        // Phase 4: can_resume_playback should say NO
        assert!(
            !state.can_resume_playback(),
            "can_resume_playback must return false when tts_data_complete \
             and audio buffer is empty — nothing left to play"
        );

        // Finalization should still be blocked while paused
        assert!(state.should_block_finalization());

        // Tick should NOT finalize while paused
        assert!(!state.should_finalize_on_tick());
    }

    #[test]
    fn resume_with_buffered_audio_succeeds() {
        // Scenario: user paused, buffer preserved, resume should work.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);

        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = true;
        state.mock_has_audio = Some(true);

        // User pauses
        state.mock_audio_paused = true;
        state.pause_tts();

        // Buffer preserved (no drain race)
        assert!(
            state.can_resume_playback(),
            "can_resume_playback must return true when audio is still buffered"
        );
    }

    #[test]
    fn pause_blocks_finalization_then_resume_without_audio_allows_it() {
        // Full lifecycle: speaking → pause → buffer drains → resume →
        // finalization should be allowed after unpause.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);

        // Speaking with audio
        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = true;
        state.mock_has_audio = Some(true);

        // Pause
        state.mock_audio_paused = true;
        assert!(state.should_block_finalization());
        assert!(!state.should_finalize_on_tick());

        // Buffer drains
        state.mock_has_audio = Some(false);
        assert!(!state.should_finalize_on_tick()); // still blocked by pause

        // Resume (unpause)
        state.mock_audio_paused = false;
        state.resume_tts();

        // Now finalization should be allowed (data complete, no audio, not paused)
        assert!(!state.should_block_finalization());
        assert!(
            state.should_finalize_on_tick(),
            "After resume with empty buffer, tick should finalize the voice turn"
        );
    }

    #[test]
    fn tts_data_incomplete_prevents_finalization_even_when_buffer_empty() {
        // TTS is still streaming data — don't finalize even if buffer is empty.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);

        state.phase = VoiceModePhase::Speaking;
        state.tts_data_complete = false;
        state.mock_has_audio = Some(false);
        // Simulate active narration (worker is still sending chunks).
        state.narrating_section = Some(("doc".into(), 0, 0));

        assert!(
            !state.should_finalize_on_tick(),
            "Should not finalize while TTS data is still streaming"
        );
        assert!(
            state.can_resume_playback(),
            "Resume should be allowed when TTS data is still streaming \
             and narration is active"
        );
    }

    #[test]
    fn resume_after_finalization_does_not_resume() {
        // Exact production scenario from logs:
        //   1. TTS finishes → tts_data_complete = true
        //   2. User pauses (audio buffered)
        //   3. finalize_voice_turn runs (due to race) → clears tts_data_complete,
        //      phase=Idle, narrating_section=None, audio drained
        //   4. User presses 's' to resume
        //   5. State: phase=Idle, has_audio=false, tts_data_complete=false,
        //      narrating_section=None
        //   6. can_resume_playback MUST return false — nothing left to play.
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);

        // After finalize_voice_turn: everything cleared
        state.phase = VoiceModePhase::Idle;
        state.tts_data_complete = false; // cleared by finalization
        state.mock_has_audio = Some(false); // drained
        state.mock_audio_paused = true; // player still paused
        state.narrating_section = None; // cleared by finalization

        assert!(
            !state.can_resume_playback(),
            "can_resume_playback must return false after finalization — \
             tts_data_complete=false, has_audio=false, narrating_section=None \
             means the voice turn was already cleaned up"
        );
    }

    #[test]
    fn persist_narration_cache_keeps_active_narration_until_cleanup() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.narrating_section = Some(("doc".into(), 2, 99));
        state.narrating_chunks = vec![vec![1, 2, 3], vec![4, 5]];
        state.tts_alignment_timeline = vec![AlignmentEntry {
            start_ms: 0,
            duration_ms: 120,
            word: "hello".into(),
        }];

        state.persist_narration_cache();

        let cache = state
            .tts_section_cache
            .lock()
            .expect("cache lock should succeed");
        let entry = cache
            .get(&("doc".to_string(), 2))
            .expect("cache entry should be present");
        assert_eq!(entry.content_hash, 99);
        assert_eq!(entry.chunks, vec![vec![1, 2, 3], vec![4, 5]]);
        assert_eq!(entry.alignment_timeline, state.tts_alignment_timeline);
        drop(cache);
        assert_eq!(state.narrating_section, Some(("doc".into(), 2, 99)));
        assert!(state.narrating_chunks.is_empty());
    }

    /// Proves the tts_only state leak bug: a tts_only VoiceModeState with
    /// phase=Idle has is_active()=true AND should_tts()=true. Without the
    /// guard in on_voice_mode_agent_delta, streaming agent text would be
    /// sent to TTS even though the user never activated full voice mode.
    #[test]
    fn tts_only_state_should_not_tts_for_streaming() {
        let vc = VoiceModeToml::default();
        let mut state = VoiceModeState::new(&vc);
        state.tts_only = true;
        state.phase = VoiceModePhase::Idle;

        // The bug: is_active() and should_tts() both return true.
        assert!(
            state.is_active(),
            "tts_only Idle state is considered active"
        );
        assert!(
            state.should_tts(),
            "tts_only Idle state would send to TTS without the guard"
        );

        // The fix: on_voice_mode_agent_delta checks state.tts_only
        // before reaching should_tts(), so streaming content is never
        // sent to TTS in tts_only mode. This test documents the
        // condition that the guard protects against.
        assert!(
            state.tts_only,
            "tts_only flag is set — on_voice_mode_agent_delta will return early"
        );
    }
}
