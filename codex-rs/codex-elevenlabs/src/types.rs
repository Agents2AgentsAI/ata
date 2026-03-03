//! Shared types for the ElevenLabs API client.

use serde::Deserialize;
use serde::Serialize;

/// Configuration for connecting to the ElevenLabs API.
#[derive(Debug, Clone)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    /// Voice ID (defaults to "21m00Tcm4TlvDq8ikWAM" — Rachel).
    pub voice_id: String,
    /// TTS model ID (defaults to "eleven_turbo_v2_5").
    pub model_id: String,
    /// ISO 639-1 language code. None = auto-detect.
    pub language_code: Option<String>,
    /// Speech speed: 0.7–1.2 (default 1.0).
    pub speed: Option<f64>,
}

impl ElevenLabsConfig {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            voice_id: "21m00Tcm4TlvDq8ikWAM".to_string(),
            model_id: "eleven_flash_v2_5".to_string(),
            language_code: None,
            speed: None,
        }
    }

    pub fn with_voice_id(mut self, voice_id: String) -> Self {
        self.voice_id = voice_id;
        self
    }

    pub fn with_model_id(mut self, model_id: String) -> Self {
        self.model_id = model_id;
        self
    }
}

// ─── TTS WebSocket messages ──────────────────────────────────────────────────

/// Message sent to the TTS WebSocket to stream text.
#[derive(Serialize)]
pub(crate) struct TtsTextMessage {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush: Option<bool>,
}

/// Message sent as the initial handshake (beginning of stream).
#[derive(Serialize)]
pub(crate) struct TtsBosMessage {
    pub text: String,
    pub voice_settings: VoiceSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
}

#[derive(Serialize)]
pub(crate) struct VoiceSettings {
    pub stability: f64,
    pub similarity_boost: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct GenerationConfig {
    pub chunk_length_schedule: Vec<u32>,
}

/// Per-chunk alignment data returned by the ElevenLabs TTS WebSocket.
///
/// Times are **relative to the returned chunk**, not the full audio stream.
/// Each chunk's alignment starts from 0ms.
#[derive(Deserialize, Debug, Clone)]
pub struct TtsAlignment {
    pub chars: Vec<String>,
    #[serde(rename = "charStartTimesMs")]
    pub char_start_times_ms: Vec<u64>,
    #[serde(rename = "charDurationsMs")]
    pub char_durations_ms: Vec<u64>,
}

/// Response message from the TTS WebSocket.
#[derive(Deserialize, Debug)]
pub(crate) struct TtsResponse {
    /// Base64-encoded audio chunk (PCM bytes).
    pub audio: Option<String>,
    /// True when the stream is complete.
    #[serde(default)]
    pub is_final: Option<bool>,
    /// Character-level alignment data for this chunk.
    #[serde(default)]
    pub alignment: Option<TtsAlignment>,
    /// Normalized alignment (post-normalization text). Falls back to `alignment`.
    #[serde(default, rename = "normalizedAlignment")]
    pub normalized_alignment: Option<TtsAlignment>,
}

// ─── STT types ───────────────────────────────────────────────────────────────

/// Response from the ElevenLabs STT endpoint.
#[derive(Deserialize, Debug)]
pub(crate) struct SttResponse {
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = ElevenLabsConfig::new("test-key".to_string());
        assert_eq!(config.voice_id, "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(config.model_id, "eleven_flash_v2_5");
    }

    #[test]
    fn tts_text_message_serialization() {
        let msg = TtsTextMessage {
            text: "hello".to_string(),
            flush: None,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(!json.contains("flush"));

        let msg_flush = TtsTextMessage {
            text: " ".to_string(),
            flush: Some(true),
        };
        let json_flush = serde_json::to_string(&msg_flush).expect("serialize");
        assert!(json_flush.contains("\"flush\":true"));
    }
}
