//! Fixture loader and replayer for TTS alignment data.
//!
//! Uses the real `AlignmentEntry`, `build_alignment_entries`, and
//! `find_active_word` from the production TUI code rather than
//! reimplementing them.

use codex_elevenlabs::TtsAlignment;
use codex_tui::AlignmentEntry;
use codex_tui::build_alignment_entries;
use serde::Deserialize;

// ─── Fixture types ───────────────────────────────────────────────────────────

/// Top-level fixture file format.
#[derive(Debug, Deserialize)]
pub struct RecordedFixture {
    pub input_text: String,
    pub chunks: Vec<RecordedChunk>,
}

/// A single TTS chunk from the fixture.
#[derive(Debug, Deserialize)]
pub struct RecordedChunk {
    /// Number of PCM samples (stored for documentation; actual audio omitted).
    #[allow(dead_code)]
    pub pcm_samples: usize,
    pub alignment: RecordedAlignment,
}

/// Character-level alignment data (mirrors `TtsAlignment`).
#[derive(Debug, Deserialize)]
pub struct RecordedAlignment {
    pub chars: Vec<String>,
    pub char_start_times_ms: Vec<u64>,
    pub char_durations_ms: Vec<u64>,
}

// ─── Fixture loading ─────────────────────────────────────────────────────────

/// Load a fixture by short name (e.g. "short" loads `tts_short.json`).
pub fn load_fixture(name: &str) -> RecordedFixture {
    let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let path = fixture_dir.join(format!("tts_{name}.json"));
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()));
    serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("failed to parse fixture {}: {e}", path.display()))
}

// ─── Timeline builder ────────────────────────────────────────────────────────

/// Build a word-level timeline from all chunks in a fixture using the real
/// production `build_alignment_entries` function.
pub fn fixture_to_timeline(fixture: &RecordedFixture) -> Vec<AlignmentEntry> {
    let mut timeline: Vec<AlignmentEntry> = Vec::new();
    let mut pending_word: Option<AlignmentEntry> = None;

    for chunk in &fixture.chunks {
        let tts_align = TtsAlignment {
            chars: chunk.alignment.chars.clone(),
            char_start_times_ms: chunk.alignment.char_start_times_ms.clone(),
            char_durations_ms: chunk.alignment.char_durations_ms.clone(),
        };
        build_alignment_entries(&tts_align, 0, &mut timeline, &mut pending_word);
    }

    // Flush any remaining pending word.
    if let Some(pw) = pending_word.take() {
        timeline.push(pw);
    }

    timeline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_short_fixture() {
        let fixture = load_fixture("short");
        assert_eq!(fixture.input_text, "Hello, world.");
        assert_eq!(fixture.chunks.len(), 1);
        assert_eq!(fixture.chunks[0].alignment.chars.len(), 13);
    }

    #[test]
    fn load_medium_fixture() {
        let fixture = load_fixture("medium");
        assert_eq!(
            fixture.input_text,
            "The quick brown fox jumps over the lazy dog."
        );
        assert_eq!(fixture.chunks.len(), 2);
    }

    #[test]
    fn timeline_from_short_fixture() {
        let fixture = load_fixture("short");
        let timeline = fixture_to_timeline(&fixture);
        // "Hello," and "world." => 2 words
        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].word, "Hello,");
        assert_eq!(timeline[1].word, "world.");
    }

    #[test]
    fn timeline_from_medium_fixture() {
        let fixture = load_fixture("medium");
        let timeline = fixture_to_timeline(&fixture);
        let words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(
            words,
            vec![
                "The", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog."
            ]
        );
    }
}
