//! Cross-platform helpers for the TTS playback speed ladder.
//!
//! The reading view's `+`/`-` keys step through a fixed ladder of speed
//! multipliers ([`TTS_SPEED_LADDER`]). Both the macOS / ElevenLabs path in
//! `chatwidget::voice_mode` and the Linux-only `tts_linux` path consume the
//! same ladder so the user-visible behaviour matches across platforms.

/// Discrete speed tiers cycled through by `+`/`-` in the reading view.
/// Each press of `+` moves to the next-higher tier; `-` moves to the
/// next-lower tier. Out-of-ladder starting values snap to the nearest tier
/// before the step is applied.
pub(crate) const TTS_SPEED_LADDER: &[f64] =
    &[0.5, 0.8, 0.9, 1.0, 1.1, 1.2, 1.5, 1.7, 2.0, 2.5, 3.0];

/// Snap an arbitrary speed to the nearest tier on [`TTS_SPEED_LADDER`].
pub(crate) fn snap_to_ladder(speed: f64) -> f64 {
    if !speed.is_finite() {
        return 1.0;
    }
    let mut best = TTS_SPEED_LADDER[0];
    let mut best_diff = (speed - best).abs();
    for &tier in TTS_SPEED_LADDER.iter().skip(1) {
        let diff = (speed - tier).abs();
        if diff < best_diff {
            best_diff = diff;
            best = tier;
        }
    }
    best
}

/// Returns the smallest tier on the ladder strictly greater than `current`,
/// or the top of the ladder if `current` is already at or above the top.
/// Off-ladder values cleanly fall to the next tier above them — e.g. 1.05
/// steps up to 1.1, not 1.2.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn step_speed_up(current: f64) -> f64 {
    for &tier in TTS_SPEED_LADDER {
        if tier > current + 1e-6 {
            return tier;
        }
    }
    *TTS_SPEED_LADDER.last().unwrap_or(&1.0)
}

/// Returns the largest tier on the ladder strictly less than `current`, or
/// the bottom of the ladder if `current` is already at or below the bottom.
/// Off-ladder values cleanly fall to the next tier below them — e.g. 1.05
/// steps down to 1.0, not 0.9.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn step_speed_down(current: f64) -> f64 {
    for &tier in TTS_SPEED_LADDER.iter().rev() {
        if tier < current - 1e-6 {
            return tier;
        }
    }
    *TTS_SPEED_LADDER.first().unwrap_or(&1.0)
}

/// Format a speed as it appears in the voice status line: `1×`, `1.5×`,
/// `0.5×` — drops trailing `.0` so whole-number speeds aren't padded.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn format_speed(speed: f64) -> String {
    let rounded = (speed * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < 1e-6 {
        format!("{}\u{00D7}", rounded.round() as i64)
    } else {
        format!("{rounded:.1}\u{00D7}")
    }
}

/// Build the reading-view "Speaking..." status string with the current speed
/// appended when non-default. Returns `"▶️  Speaking..."` at 1.0× and
/// `"▶️  Speaking (1.5×)"` otherwise.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn speaking_status_text(speed: f64) -> String {
    if (speed - 1.0).abs() < 1e-6 {
        "\u{25B6}\u{FE0F}  Speaking...".to_string()
    } else {
        format!("\u{25B6}\u{FE0F}  Speaking ({})", format_speed(speed))
    }
}
