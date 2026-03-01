//! ElevenLabs API client for TTS (WebSocket streaming) and STT (HTTP).
//!
//! Designed for the ATA voice mode pipeline. The TTS client opens a
//! persistent WebSocket to the ElevenLabs streaming input endpoint and
//! produces PCM 24kHz mono i16 chunks suitable for direct cpal playback.

pub mod error;
pub mod stt;
pub mod tts;
pub mod types;

pub use error::ElevenLabsError;
pub use tts::TtsChunk;
pub use types::ElevenLabsConfig;
pub use types::TtsAlignment;
