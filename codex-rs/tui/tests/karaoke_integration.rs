//! Integration tests for the karaoke (TTS alignment) pipeline.
//!
//! These tests exercise the real production `build_alignment_entries` and
//! `find_active_word` functions from `codex_tui` using synthetic TTS fixture
//! files, without requiring an ElevenLabs API key.

// Voice/TTS is only available on non-Linux platforms with the voice-input feature.
#![cfg(all(not(target_os = "linux"), feature = "voice-input"))]

mod support;

use codex_tui::find_active_word;
use support::recorded_tts::fixture_to_timeline;
use support::recorded_tts::load_fixture;
use support::sync_oracle::verify_karaoke_sync;

// ─── Test 24: timeline_from_fixtures_matches_text ────────────────────────────

/// Build timeline from fixture -> word sequence matches input text words.
#[test]
fn karaoke_timeline_from_fixtures_matches_text() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);

        // Collect words from the input text (split on whitespace).
        let expected_words: Vec<&str> = fixture.input_text.split_whitespace().collect();
        let timeline_words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();

        assert_eq!(
            timeline_words, expected_words,
            "fixture '{name}': timeline words should match input text words"
        );
    }
}

// ─── Test 25: timeline_word_indices_monotonic ────────────────────────────────

/// Timeline word start_ms values are monotonically increasing.
#[test]
fn karaoke_timeline_word_indices_monotonic() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);

        for window in timeline.windows(2) {
            assert!(
                window[1].start_ms > window[0].start_ms,
                "fixture '{name}': start_ms should be strictly increasing, \
                 got {} then {}",
                window[0].start_ms,
                window[1].start_ms,
            );
        }
    }
}

// ─── Test 26: timeline_gaps_no_regression ────────────────────────────────────

/// Step through at 10ms intervals, active word index never decreases.
#[test]
fn karaoke_timeline_gaps_no_regression() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);

        // Find the last meaningful timestamp.
        let max_ms = timeline
            .last()
            .map(|e| e.start_ms + e.duration_ms + 100)
            .unwrap_or(0);

        let mut last_idx: Option<usize> = None;
        let mut t = 0u64;
        while t <= max_ms {
            let current = find_active_word(&timeline, t);
            if let (Some(prev), Some(cur)) = (last_idx, current) {
                assert!(
                    cur >= prev,
                    "fixture '{name}': active word index regressed from {prev} \
                     to {cur} at t={t}ms"
                );
            }
            if current.is_some() {
                last_idx = current;
            }
            t += 10;
        }
    }
}

// ─── Test 27: simulated_pause_resume ─────────────────────────────────────────

/// Simulate pause/resume by verifying `find_active_word` returns correct words
/// at specific playback positions, and that pausing (holding a position) then
/// resuming yields consistent results.
#[test]
fn karaoke_simulated_pause_resume() {
    let fixture = load_fixture("medium");
    let timeline = fixture_to_timeline(&fixture);

    // Verify we get a word at a known position inside the timeline.
    let first_word_start = timeline[0].start_ms;
    let first_word_mid = first_word_start + timeline[0].duration_ms / 2;
    let idx_at_first = find_active_word(&timeline, first_word_mid);
    assert_eq!(
        idx_at_first,
        Some(0),
        "midpoint of first word should map to index 0"
    );

    // Pick a pause time that falls within the third word.
    let pause_time = timeline[2].start_ms + timeline[2].duration_ms / 2;
    let word_at_pause = find_active_word(&timeline, pause_time);
    assert_eq!(
        word_at_pause,
        Some(2),
        "midpoint of third word should map to index 2"
    );

    // Simulate pause: calling find_active_word at the same position should
    // return the same result (idempotency).
    let word_after_pause = find_active_word(&timeline, pause_time);
    assert_eq!(
        word_at_pause, word_after_pause,
        "same position after pause/resume should yield the same word"
    );

    // Simulate resume: advance past the third word into the fourth.
    let resume_time = timeline[3].start_ms + timeline[3].duration_ms / 2;
    let word_at_resume = find_active_word(&timeline, resume_time);
    assert_eq!(
        word_at_resume,
        Some(3),
        "after resuming past third word, should find fourth word"
    );

    // Verify the word index never goes backward when resuming.
    if let (Some(pause_idx), Some(resume_idx)) = (word_at_pause, word_at_resume) {
        assert!(
            resume_idx >= pause_idx,
            "word index should not regress after resume: \
             pause_idx={pause_idx}, resume_idx={resume_idx}"
        );
    }
}

// ─── Extra: cross-chunk word merging ─────────────────────────────────────────

/// The medium fixture splits "jumps" across two chunks ("ju" + "mps").
/// Verify the timeline correctly merges this into a single "jumps" word.
#[test]
fn karaoke_cross_chunk_word_merging() {
    let fixture = load_fixture("medium");
    let timeline = fixture_to_timeline(&fixture);

    let words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();
    assert!(
        words.contains(&"jumps"),
        "expected 'jumps' in timeline (merged across chunks), got: {words:?}"
    );
}

/// Every word in the timeline has a positive duration.
#[test]
fn karaoke_all_words_have_positive_duration() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);

        for entry in &timeline {
            assert!(
                entry.duration_ms > 0,
                "fixture '{name}': word '{}' has zero duration",
                entry.word
            );
        }
    }
}

/// Words don't overlap in time.
#[test]
fn karaoke_words_do_not_overlap() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);

        for window in timeline.windows(2) {
            let end_of_first = window[0].start_ms + window[0].duration_ms;
            assert!(
                end_of_first <= window[1].start_ms,
                "fixture '{name}': word '{}' (end={end_of_first}ms) overlaps \
                 with '{}' (start={}ms)",
                window[0].word,
                window[1].word,
                window[1].start_ms,
            );
        }
    }
}

// ─── Sync oracle validation ─────────────────────────────────────────────────

/// Run the sync verification oracle on both synthetic fixtures.
/// All checks should pass.
#[test]
fn karaoke_sync_oracle_validates_fixtures() {
    for name in &["short", "medium"] {
        let fixture = load_fixture(name);
        let timeline = fixture_to_timeline(&fixture);
        let result = verify_karaoke_sync(&timeline, &fixture.input_text, 50);

        assert!(
            result.all_ok(),
            "fixture '{name}': sync oracle failed: {:?}",
            result.errors,
        );
    }
}
