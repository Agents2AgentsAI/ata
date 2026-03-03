//! Speech-to-text client using the ElevenLabs HTTP API.
//!
//! Sends WAV audio to `POST https://api.elevenlabs.io/v1/speech-to-text`
//! and returns the transcribed text.

use reqwest::Client;
use tracing::debug;
use tracing::trace;

use crate::ElevenLabsConfig;
use crate::ElevenLabsError;
use crate::types::SttResponse;

/// Transcribe WAV audio bytes using the ElevenLabs STT API.
///
/// The `wav_bytes` should be a complete WAV file (header + PCM data).
pub async fn transcribe(
    config: &ElevenLabsConfig,
    wav_bytes: Vec<u8>,
) -> Result<String, ElevenLabsError> {
    let audio_kib = wav_bytes.len() as f32 / 1024.0;
    trace!("sending STT request: audio={audio_kib:.1}KiB");

    let part = reqwest::multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| ElevenLabsError::Api(format!("failed to set mime: {e}")))?;

    let form = reqwest::multipart::Form::new()
        .text("model_id", "scribe_v1")
        .part("file", part);

    let client = Client::new();
    let resp = client
        .post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", &config.api_key)
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|_| "<failed to read body>".to_string());
        return Err(ElevenLabsError::Api(format!(
            "STT request failed: {status} {body}"
        )));
    }

    let stt_resp: SttResponse = resp.json().await?;
    debug!("STT transcription succeeded: {} chars", stt_resp.text.len());

    if stt_resp.text.is_empty() {
        Err(ElevenLabsError::Api(
            "empty transcription result".to_string(),
        ))
    } else {
        Ok(stt_resp.text)
    }
}
