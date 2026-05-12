//! ATA-private config types (Supabase auth, relay routing).
//!
//! Surfaced through `codex_core::config::types::*` so the TUI and any other
//! consumer can import them via the standard config types path.

use serde::Deserialize;
use serde::Serialize;

/// ATA Supabase project URL.
pub const DEFAULT_ATA_SUPABASE_URL: &str = "https://natbqqfawsmcoeutsogu.supabase.co";

/// ATA Supabase publishable (anon) key.
pub const DEFAULT_ATA_SUPABASE_ANON_KEY: &str = "sb_publishable_MopvwXLh_k866kZvplSGGQ_IBircspG";

/// Configuration for ATA account features (Supabase-backed auth and relay).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AtaAccountConfig {
    #[serde(default = "default_ata_supabase_url")]
    pub supabase_url: String,
    #[serde(default = "default_ata_supabase_anon_key")]
    pub supabase_anon_key: String,
    #[serde(default)]
    pub relay_mode: RelayMode,
}

impl Default for AtaAccountConfig {
    fn default() -> Self {
        Self {
            supabase_url: default_ata_supabase_url(),
            supabase_anon_key: default_ata_supabase_anon_key(),
            relay_mode: RelayMode::default(),
        }
    }
}

fn default_ata_supabase_url() -> String {
    DEFAULT_ATA_SUPABASE_URL.to_string()
}

fn default_ata_supabase_anon_key() -> String {
    DEFAULT_ATA_SUPABASE_ANON_KEY.to_string()
}

/// Relay connection mode for ATA accounts.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum RelayMode {
    #[default]
    Cloud,
    Local,
    Custom {
        relay_url: String,
    },
}

// ─── Voice Mode ──────────────────────────────────────────────────────────────

/// Controls how much the agent narrates aloud via `<voice>` tags.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VoiceVerbosity {
    /// Only the final answer/summary is spoken aloud.
    #[default]
    Concise,
    /// Intermediate progress updates and final answer are spoken aloud.
    Verbose,
}

/// Which TTS backend voice mode should use.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TtsBackend {
    /// ElevenLabs cloud TTS via WebSocket — high quality, requires API key.
    #[default]
    Elevenlabs,
    /// macOS built-in `say` command — no network, no API key, no karaoke.
    Say,
}

/// Output mode for the voice mode pipeline.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VoiceOutput {
    /// Only play TTS audio, suppress text streaming.
    Voice,
    /// Only show text, no TTS playback.
    Text,
    /// Show text AND play TTS audio (default).
    #[default]
    Both,
}

/// Voice mode configuration nested under `[voice_mode]` in config.toml.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct VoiceModeToml {
    /// When true, voice mode is automatically enabled on startup.
    pub enabled: Option<bool>,
    /// Controls what output is produced: "voice", "text", or "both".
    #[serde(default)]
    pub output: Option<VoiceOutput>,
    /// When true, transcribed speech is auto-submitted to the agent.
    pub auto_submit: Option<bool>,
    /// RMS energy threshold for voice activity detection (0.0–1.0).
    pub vad_threshold: Option<f64>,
    /// Milliseconds of silence before a recording is finalized.
    pub silence_duration_ms: Option<u64>,
    /// When false, TTS (text-to-speech) is disabled even when voice mode is on.
    pub tts_enabled: Option<bool>,
    /// When false, STT is disabled even when voice mode is on.
    pub stt_enabled: Option<bool>,
    /// Controls how much the agent narrates: "concise" or "verbose".
    pub verbosity: Option<VoiceVerbosity>,
    /// Which TTS backend to use: "elevenlabs" or "say".
    pub tts_backend: Option<TtsBackend>,
    /// ElevenLabs API settings (api_key, voice_id, language, speed).
    #[serde(default)]
    pub elevenlabs: Option<codex_config::config_toml::ElevenLabsToml>,
}
