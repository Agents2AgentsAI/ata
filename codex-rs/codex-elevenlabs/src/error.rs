//! Error types for the ElevenLabs client.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ElevenLabsError {
    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("API error: {0}")]
    Api(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Missing API key: set ELEVENLABS_API_KEY or configure voice_mode.elevenlabs.api_key")]
    MissingApiKey,
}
