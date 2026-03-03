//! Energy-threshold Voice Activity Detection (VAD).
//!
//! Uses a simple peak energy approach with configurable threshold.
//! Designed for the voice mode pipeline where low latency matters more
//! than perfect accuracy.

use std::sync::Arc;
use std::sync::atomic::AtomicU16;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

/// Voice activity detector based on peak energy levels.
///
/// Uses a fixed base threshold (configurable, default 0.04) with an echo
/// suppression multiplier for TTS playback. The threshold is shared across
/// threads via `SharedVadThreshold`.
pub(crate) struct VoiceActivityDetector {
    /// Base threshold (0.0–1.0). Peaks above this are considered speech.
    base_threshold: f64,
    /// Echo suppression multiplier (1.0 = normal, >1.0 = suppressed).
    echo_multiplier: f64,
    /// How long silence must last before declaring end of speech.
    silence_gate: Duration,
    /// When speech was last detected above threshold.
    last_speech_at: Option<Instant>,
    /// Whether we're currently in the "speaking" state.
    is_speaking: bool,
    /// Consecutive above-threshold frames needed to trigger speech onset.
    onset_frames_required: u32,
    /// Current consecutive above-threshold frame count.
    onset_frame_count: u32,
    /// Maximum duration of continuous speech before auto-silence.
    max_speech_duration: Duration,
    /// When speech onset was first detected (for max duration tracking).
    speech_onset_at: Option<Instant>,
}

/// Events emitted by the VAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VadEvent {
    /// Speech onset detected.
    SpeechDetected,
    /// Silence gate expired — speech is over.
    SilenceDetected,
}

/// Shared threshold that can be updated from the main thread and read by the
/// spawned VAD polling task. Stores `f32` bits in an `AtomicU32`.
#[derive(Clone)]
pub(crate) struct SharedVadThreshold {
    inner: Arc<AtomicU32>,
}

impl SharedVadThreshold {
    pub(crate) fn new(threshold: f64) -> Self {
        Self {
            inner: Arc::new(AtomicU32::new((threshold as f32).to_bits())),
        }
    }

    pub(crate) fn load(&self) -> f64 {
        f32::from_bits(self.inner.load(Ordering::Relaxed)) as f64
    }

    pub(crate) fn store(&self, value: f64) {
        self.inner
            .store((value as f32).to_bits(), Ordering::Relaxed);
    }
}

impl VoiceActivityDetector {
    /// Default onset frames — 3 consecutive frames (150ms at 50ms polling).
    const DEFAULT_ONSET_FRAMES: u32 = 3;

    /// Default max speech duration — 30 seconds.
    const DEFAULT_MAX_SPEECH: Duration = Duration::from_secs(30);

    pub(crate) fn new(base_threshold: f64, silence_duration: Duration) -> Self {
        Self::with_onset_frames(base_threshold, silence_duration, Self::DEFAULT_ONSET_FRAMES)
    }

    /// Create a VAD that requires `onset_frames` consecutive above-threshold
    /// frames before declaring speech onset.
    pub(crate) fn with_onset_frames(
        base_threshold: f64,
        silence_duration: Duration,
        onset_frames: u32,
    ) -> Self {
        Self {
            base_threshold,
            echo_multiplier: 1.0,
            silence_gate: silence_duration,
            last_speech_at: None,
            is_speaking: false,
            onset_frames_required: onset_frames,
            onset_frame_count: 0,
            max_speech_duration: Self::DEFAULT_MAX_SPEECH,
            speech_onset_at: None,
        }
    }

    /// Current effective threshold: `base_threshold × echo_multiplier`.
    fn effective_threshold(&self) -> f64 {
        (self.base_threshold * self.echo_multiplier).min(1.0)
    }

    /// Feed a peak sample (from `AtomicU16`) and return any state-change event.
    pub(crate) fn process_peak(&mut self, peak: &AtomicU16) -> Option<VadEvent> {
        let raw = peak.load(Ordering::Relaxed);
        let normalized = raw as f64 / i16::MAX as f64;
        self.process(normalized)
    }

    /// Feed a normalized peak value (0.0–1.0).
    fn process(&mut self, normalized_peak: f64) -> Option<VadEvent> {
        let threshold = self.effective_threshold();
        let now = Instant::now();

        // Max speech duration guard — force silence if speaking too long.
        // Prevents runaway recordings from ambient audio (podcasts, TV).
        if self.is_speaking
            && let Some(onset) = self.speech_onset_at
            && now.duration_since(onset) >= self.max_speech_duration
        {
            self.is_speaking = false;
            self.last_speech_at = None;
            self.speech_onset_at = None;
            self.onset_frame_count = 0;
            return Some(VadEvent::SilenceDetected);
        }

        if normalized_peak >= threshold {
            self.onset_frame_count += 1;
            self.last_speech_at = Some(now);

            if !self.is_speaking && self.onset_frame_count >= self.onset_frames_required {
                self.is_speaking = true;
                self.speech_onset_at = Some(now);
                return Some(VadEvent::SpeechDetected);
            }
        } else {
            // Reset onset counter on any below-threshold frame.
            self.onset_frame_count = 0;

            if self.is_speaking
                && let Some(last) = self.last_speech_at
                && now.duration_since(last) >= self.silence_gate
            {
                self.is_speaking = false;
                self.last_speech_at = None;
                self.speech_onset_at = None;
                return Some(VadEvent::SilenceDetected);
            }
        }

        None
    }

    /// Set the echo suppression multiplier (1.0 = normal, 5.0 = suppressed).
    pub(crate) fn set_echo_multiplier(&mut self, multiplier: f64) {
        self.echo_multiplier = multiplier;
    }

    /// Clear echo suppression (restore multiplier to 1.0).
    pub(crate) fn clear_echo_multiplier(&mut self) {
        self.echo_multiplier = 1.0;
    }

    /// Current effective threshold (for sharing with the poll task).
    pub(crate) fn threshold(&self) -> f64 {
        self.effective_threshold()
    }

    /// Sync echo multiplier from a shared threshold value.
    /// The poll task receives the pre-computed effective threshold and derives
    /// what echo multiplier to use locally.
    pub(crate) fn set_threshold_from_shared(&mut self, shared_threshold: f64) {
        if self.base_threshold > 0.0 {
            self.echo_multiplier = (shared_threshold / self.base_threshold).max(1.0);
        }
    }

    pub(crate) fn is_speaking(&self) -> bool {
        self.is_speaking
    }

    /// Force-reset to idle state (used on mode exit).
    pub(crate) fn reset(&mut self) {
        self.is_speaking = false;
        self.last_speech_at = None;
        self.speech_onset_at = None;
        self.onset_frame_count = 0;
        self.echo_multiplier = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_onset_requires_consecutive_frames() {
        let mut vad = VoiceActivityDetector::with_onset_frames(0.04, Duration::from_millis(100), 3);

        // Single frame above threshold — not enough.
        assert_eq!(vad.process(0.10), None);
        assert!(!vad.is_speaking());

        // Second consecutive frame — still not enough.
        assert_eq!(vad.process(0.10), None);

        // Third consecutive frame — triggers speech onset.
        assert_eq!(vad.process(0.10), Some(VadEvent::SpeechDetected));
        assert!(vad.is_speaking());
    }

    #[test]
    fn transient_noise_does_not_trigger() {
        let mut vad = VoiceActivityDetector::with_onset_frames(0.04, Duration::from_millis(100), 3);

        // Single spike (keyboard click) then silence.
        assert_eq!(vad.process(0.15), None);
        assert_eq!(vad.process(0.01), None); // below threshold resets counter
        assert_eq!(vad.process(0.15), None); // starts counting again from 1
        assert!(!vad.is_speaking());
    }

    #[test]
    fn speech_onset_and_silence() {
        let mut vad = VoiceActivityDetector::with_onset_frames(0.04, Duration::from_millis(100), 1);

        // Below threshold — nothing.
        assert_eq!(vad.process(0.02), None);
        assert!(!vad.is_speaking());

        // Above threshold — speech onset.
        assert_eq!(vad.process(0.10), Some(VadEvent::SpeechDetected));
        assert!(vad.is_speaking());

        // Still above — nothing new.
        assert_eq!(vad.process(0.08), None);

        // Drop below, but silence gate hasn't expired.
        assert_eq!(vad.process(0.01), None);
        assert!(vad.is_speaking());

        // Simulate time passing by resetting last_speech_at.
        vad.last_speech_at = Some(Instant::now() - Duration::from_millis(200));
        assert_eq!(vad.process(0.01), Some(VadEvent::SilenceDetected));
        assert!(!vad.is_speaking());
    }

    #[test]
    fn echo_suppression_raises_threshold() {
        let mut vad = VoiceActivityDetector::with_onset_frames(0.04, Duration::from_millis(100), 1);

        // Normal: 0.10 triggers.
        assert_eq!(vad.process(0.10), Some(VadEvent::SpeechDetected));
        vad.reset();

        // 5x echo suppression: threshold = 0.04*5 = 0.20.
        vad.set_echo_multiplier(5.0);
        assert_eq!(vad.process(0.10), None); // below 0.20

        // Loud speech (0.30) still triggers.
        assert_eq!(vad.process(0.30), Some(VadEvent::SpeechDetected));
        vad.reset();

        // Restore echo suppression.
        vad.clear_echo_multiplier();
        assert_eq!(vad.process(0.10), Some(VadEvent::SpeechDetected));
    }

    #[test]
    fn max_speech_duration_forces_silence() {
        let mut vad = VoiceActivityDetector::with_onset_frames(0.04, Duration::from_millis(100), 1);
        vad.max_speech_duration = Duration::from_millis(50);

        // Trigger speech onset.
        assert_eq!(vad.process(0.10), Some(VadEvent::SpeechDetected));
        assert!(vad.is_speaking());

        // Simulate time passing beyond max speech duration.
        vad.speech_onset_at = Some(Instant::now() - Duration::from_millis(100));
        assert_eq!(vad.process(0.10), Some(VadEvent::SilenceDetected));
        assert!(!vad.is_speaking());
    }

    #[test]
    fn shared_threshold_roundtrip() {
        let shared = SharedVadThreshold::new(0.05);
        assert!((shared.load() - 0.05).abs() < 0.001);

        shared.store(0.25);
        assert!((shared.load() - 0.25).abs() < 0.001);
    }

    #[test]
    fn set_threshold_from_shared_derives_echo_multiplier() {
        let mut vad = VoiceActivityDetector::new(0.04, Duration::from_millis(100));

        // Shared value = 0.20 → echo_multiplier = 0.20/0.04 = 5.0
        vad.set_threshold_from_shared(0.20);
        assert!((vad.threshold() - 0.20).abs() < 0.001);

        // Reset shared back to base → echo_multiplier = 1.0
        vad.set_threshold_from_shared(0.04);
        assert!((vad.threshold() - 0.04).abs() < 0.001);
    }
}
