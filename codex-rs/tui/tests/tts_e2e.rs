//! Live TTS E2E tests for the karaoke alignment pipeline.
//!
//! All tests are `#[ignore]` by default — they require a valid
//! `ELEVENLABS_API_KEY` environment variable and make real API calls.
//!
//! Run manually:
//! ```sh
//! ELEVENLABS_API_KEY=sk-... cargo test -p codex-tui --test tts_e2e -- --ignored
//! ```

// codex-elevenlabs is only available on non-Linux platforms.
#![cfg(not(target_os = "linux"))]
#![allow(clippy::expect_used)]

mod support;

use codex_elevenlabs::ElevenLabsConfig;
use codex_elevenlabs::TtsChunk;
use codex_elevenlabs::tts::TtsStream;

use codex_tui::find_active_word;
use support::recorded_tts::RecordedAlignment;
use support::recorded_tts::RecordedChunk;
use support::recorded_tts::RecordedFixture;
use support::recorded_tts::fixture_to_timeline;

/// Helper: collect all TTS chunks from a stream for the given text.
async fn collect_tts_chunks(text: &str) -> Vec<TtsChunk> {
    let api_key =
        std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY must be set for E2E tests");
    assert!(!api_key.is_empty(), "ELEVENLABS_API_KEY is empty");

    let config = ElevenLabsConfig::new(api_key);
    let mut stream = TtsStream::connect(&config)
        .await
        .expect("failed to connect to ElevenLabs");

    stream.send_text(text).await.expect("failed to send text");
    stream.flush().await.expect("failed to flush");
    stream.send_eos().await;

    let mut chunks = Vec::new();
    while let Some(chunk) = stream.recv_audio().await {
        chunks.push(chunk);
    }
    chunks
}

/// Convert live TTS chunks into a `RecordedFixture` for use with the
/// fixture-based timeline builder.
fn chunks_to_fixture(input_text: &str, chunks: &[TtsChunk]) -> RecordedFixture {
    let recorded_chunks: Vec<RecordedChunk> = chunks
        .iter()
        .filter_map(|chunk| {
            chunk.alignment.as_ref().map(|a| RecordedChunk {
                pcm_samples: chunk.pcm.len(),
                alignment: RecordedAlignment {
                    chars: a.chars.clone(),
                    char_start_times_ms: a.char_start_times_ms.clone(),
                    char_durations_ms: a.char_durations_ms.clone(),
                },
            })
        })
        .collect();

    RecordedFixture {
        input_text: input_text.to_string(),
        chunks: recorded_chunks,
    }
}

// ─── Test 47: tts_stream_has_alignment_data ──────────────────────────────────

/// Connect to real ElevenLabs, send "Hello, world.", verify at least one
/// chunk has alignment data and the combined chars match the input text.
#[tokio::test]
#[ignore]
async fn tts_stream_has_alignment_data() {
    let input_text = "Hello, world.";
    let chunks = collect_tts_chunks(input_text).await;

    assert!(!chunks.is_empty(), "expected at least one audio chunk");

    // Verify at least one chunk has alignment data.
    let has_alignment = chunks.iter().any(|c| c.alignment.is_some());
    assert!(
        has_alignment,
        "expected at least one chunk with alignment data"
    );

    // Combine all alignment chars and verify they match input text characters.
    let combined_chars: String = chunks
        .iter()
        .filter_map(|c| c.alignment.as_ref())
        .flat_map(|a| a.chars.iter())
        .cloned()
        .collect::<String>();

    // The combined chars should contain all characters from the input text.
    // ElevenLabs may insert extra spaces or normalize slightly, so we compare
    // after stripping whitespace.
    let expected_no_ws: String = input_text.chars().filter(|c| !c.is_whitespace()).collect();
    let actual_no_ws: String = combined_chars
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    assert_eq!(
        actual_no_ws, expected_no_ws,
        "combined alignment chars (sans whitespace) should match input text: \
         expected '{expected_no_ws}', got '{actual_no_ws}'"
    );
}

// ─── Test 48: tts_alignment_to_timeline_matches_text ─────────────────────────

/// Send "The quick brown fox." to real ElevenLabs, build timeline from
/// alignment data, verify word lookup and word sequence.
#[tokio::test]
#[ignore]
async fn tts_alignment_to_timeline_matches_text() {
    let input_text = "The quick brown fox.";
    let chunks = collect_tts_chunks(input_text).await;

    assert!(!chunks.is_empty(), "expected at least one audio chunk");

    let fixture = chunks_to_fixture(input_text, &chunks);
    let timeline = fixture_to_timeline(&fixture);

    // Verify word sequence matches input text words.
    let expected_words: Vec<&str> = input_text.split_whitespace().collect();
    let timeline_words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();

    assert_eq!(
        timeline_words, expected_words,
        "timeline words should match input text words"
    );

    // Verify find_active_word at key timestamps returns correct words.
    // The first word should be active near time 0.
    if let Some(idx) = find_active_word(&timeline, timeline[0].start_ms) {
        assert_eq!(timeline[idx].word, "The", "first word should be 'The'");
    }

    // The last word should be active near its start time.
    let last = &timeline[timeline.len() - 1];
    if let Some(idx) = find_active_word(&timeline, last.start_ms) {
        assert_eq!(timeline[idx].word, "fox.", "last word should be 'fox.'");
    }
}

// ─── Test 49: tts_alignment_word_count_matches_input ─────────────────────────

/// Send a longer text, build timeline, verify word count matches.
#[tokio::test]
#[ignore]
async fn tts_alignment_word_count_matches_input() {
    let input_text = "In the beginning, there was only darkness. Then a spark of light emerged.";
    let chunks = collect_tts_chunks(input_text).await;

    assert!(!chunks.is_empty(), "expected at least one audio chunk");

    let fixture = chunks_to_fixture(input_text, &chunks);
    let timeline = fixture_to_timeline(&fixture);

    let expected_word_count = input_text.split_whitespace().count();
    assert_eq!(
        timeline.len(),
        expected_word_count,
        "timeline word count ({}) should match input text word count ({expected_word_count})",
        timeline.len(),
    );
}
