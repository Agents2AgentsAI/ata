//! Sync verification oracle for karaoke alignment timelines.
//!
//! Validates that a word-level timeline produced from TTS alignment data
//! is consistent with the original input text and has reasonable timing
//! properties.

use codex_tui::AlignmentEntry;

/// Result of running the sync verification oracle on a timeline.
#[derive(Debug)]
pub struct SyncVerification {
    /// Words appear in the same order as the original text.
    pub word_order_ok: bool,
    /// Word start times are monotonically increasing.
    pub timing_monotonic: bool,
    /// No single word exceeds 5000ms duration.
    pub no_excessive_duration: bool,
    /// Total duration is within 50% of expected (~60ms per char).
    pub total_duration_reasonable: bool,
    /// Detailed error descriptions for any failed checks.
    pub errors: Vec<String>,
}

impl SyncVerification {
    /// Returns `true` if all checks passed.
    pub fn all_ok(&self) -> bool {
        self.word_order_ok
            && self.timing_monotonic
            && self.no_excessive_duration
            && self.total_duration_reasonable
    }
}

/// Strip punctuation from a word for fuzzy comparison.
fn strip_punctuation(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Verify karaoke sync properties of a word timeline against original text.
///
/// Checks:
/// 1. Word order matches (fuzzy: strip punctuation, case-insensitive)
/// 2. Timing is monotonically increasing
/// 3. No word exceeds `max_duration_ms` (default 5000ms)
/// 4. Total duration within 50% of expected (~60ms per char)
pub fn verify_karaoke_sync(
    timeline: &[AlignmentEntry],
    original_text: &str,
    _tolerance_ms: u64,
) -> SyncVerification {
    let mut errors: Vec<String> = Vec::new();

    // ── 1. Word order ────────────────────────────────────────────────────
    let expected_words: Vec<&str> = original_text.split_whitespace().collect();
    let timeline_words: Vec<&str> = timeline.iter().map(|e| e.word.as_str()).collect();

    let word_order_ok = if expected_words.len() != timeline_words.len() {
        errors.push(format!(
            "word count mismatch: expected {}, got {}",
            expected_words.len(),
            timeline_words.len(),
        ));
        false
    } else {
        let mut order_ok = true;
        for (i, (expected, actual)) in expected_words.iter().zip(timeline_words.iter()).enumerate()
        {
            let exp_clean = strip_punctuation(expected).to_lowercase();
            let act_clean = strip_punctuation(actual).to_lowercase();
            if exp_clean != act_clean {
                errors.push(format!(
                    "word {i}: expected '{expected}' (normalized: '{exp_clean}'), \
                     got '{actual}' (normalized: '{act_clean}')",
                ));
                order_ok = false;
            }
        }
        order_ok
    };

    // ── 2. Timing monotonicity ───────────────────────────────────────────
    let timing_monotonic = {
        let mut monotonic = true;
        for window in timeline.windows(2) {
            if window[1].start_ms <= window[0].start_ms {
                errors.push(format!(
                    "non-monotonic timing: '{}' at {}ms followed by '{}' at {}ms",
                    window[0].word, window[0].start_ms, window[1].word, window[1].start_ms,
                ));
                monotonic = false;
            }
        }
        monotonic
    };

    // ── 3. No excessive duration ─────────────────────────────────────────
    let max_duration_ms = 5000u64;
    let no_excessive_duration = {
        let mut ok = true;
        for entry in timeline {
            if entry.duration_ms > max_duration_ms {
                errors.push(format!(
                    "excessive duration: '{}' has duration {}ms (max {max_duration_ms}ms)",
                    entry.word, entry.duration_ms,
                ));
                ok = false;
            }
        }
        ok
    };

    // ── 4. Total duration reasonable ─────────────────────────────────────
    let total_duration_reasonable = if timeline.is_empty() {
        errors.push("empty timeline".to_string());
        false
    } else {
        let last = &timeline[timeline.len() - 1];
        let actual_duration = last.start_ms + last.duration_ms;
        let char_count = original_text.len() as u64;
        let expected_duration = char_count * 60; // ~60ms per char
        let lower = expected_duration / 2;
        let upper = expected_duration + expected_duration / 2;
        if actual_duration < lower || actual_duration > upper {
            errors.push(format!(
                "total duration {actual_duration}ms outside expected range \
                 [{lower}ms, {upper}ms] (expected ~{expected_duration}ms for {char_count} chars)",
            ));
            false
        } else {
            true
        }
    };

    SyncVerification {
        word_order_ok,
        timing_monotonic,
        no_excessive_duration,
        total_duration_reasonable,
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timeline(words: &[(&str, u64, u64)]) -> Vec<AlignmentEntry> {
        words
            .iter()
            .map(|(word, start, dur)| AlignmentEntry {
                start_ms: *start,
                duration_ms: *dur,
                word: word.to_string(),
            })
            .collect()
    }

    #[test]
    fn oracle_passes_valid_timeline() {
        let timeline = make_timeline(&[("Hello,", 0, 225), ("world.", 315, 285)]);
        let result = verify_karaoke_sync(&timeline, "Hello, world.", 50);
        assert!(result.all_ok(), "errors: {:?}", result.errors);
    }

    #[test]
    fn oracle_detects_wrong_word_order() {
        let timeline = make_timeline(&[("world.", 0, 225), ("Hello,", 315, 285)]);
        let result = verify_karaoke_sync(&timeline, "Hello, world.", 50);
        assert!(!result.word_order_ok);
    }

    #[test]
    fn oracle_detects_non_monotonic_timing() {
        let timeline = make_timeline(&[("Hello,", 315, 225), ("world.", 100, 285)]);
        let result = verify_karaoke_sync(&timeline, "Hello, world.", 50);
        assert!(!result.timing_monotonic);
    }

    #[test]
    fn oracle_detects_excessive_duration() {
        let timeline = make_timeline(&[("Hello,", 0, 6000), ("world.", 7000, 285)]);
        let result = verify_karaoke_sync(&timeline, "Hello, world.", 50);
        assert!(!result.no_excessive_duration);
    }
}
