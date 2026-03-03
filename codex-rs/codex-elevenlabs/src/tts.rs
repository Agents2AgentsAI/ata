//! Streaming TTS client using the ElevenLabs WebSocket API.
//!
//! Opens a WebSocket to `wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input`
//! with `output_format=pcm_24000` to avoid MP3 decode overhead. Text is streamed
//! in chunks and PCM audio is received back as 24kHz mono signed 16-bit LE samples.

use base64::Engine;
use futures::SinkExt;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::debug;
use tracing::error;
use tracing::trace;

use crate::ElevenLabsConfig;
use crate::ElevenLabsError;
use crate::types::GenerationConfig;
use crate::types::TtsAlignment;
use crate::types::TtsBosMessage;
use crate::types::TtsResponse;
use crate::types::TtsTextMessage;
use crate::types::VoiceSettings;

/// A single TTS audio chunk with optional alignment data.
#[derive(Debug, Clone)]
pub struct TtsChunk {
    /// PCM audio samples (24kHz mono i16).
    pub pcm: Vec<i16>,
    /// Character-level alignment for this chunk (times relative to chunk start).
    pub alignment: Option<TtsAlignment>,
}

/// Streaming TTS handle. Send text, receive PCM audio chunks.
pub struct TtsStream {
    text_tx: mpsc::Sender<TtsCommand>,
    audio_rx: mpsc::Receiver<TtsChunk>,
}

enum TtsCommand {
    Text(String),
    Flush,
    /// Send EOS (empty text) without closing the WebSocket.
    /// The server finishes generating audio and sends `is_final`.
    Eos,
    Close,
}

impl TtsStream {
    /// Open a WebSocket connection to ElevenLabs TTS streaming endpoint.
    pub async fn connect(config: &ElevenLabsConfig) -> Result<Self, ElevenLabsError> {
        // Ensure rustls has a crypto provider before any TLS handshake.
        codex_utils_rustls_provider::ensure_rustls_crypto_provider();

        let mut url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id={}&output_format=pcm_24000&sync_alignment=true",
            config.voice_id, config.model_id
        );
        if let Some(ref lang) = config.language_code {
            url.push_str(&format!("&language_code={lang}"));
        }

        let request = tokio_tungstenite::tungstenite::http::Request::builder()
            .uri(&url)
            .header("xi-api-key", &config.api_key)
            .header("Host", "api.elevenlabs.io")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tokio_tungstenite::tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .map_err(|e| ElevenLabsError::Api(format!("failed to build request: {e}")))?;

        let (ws_stream, _response) = connect_async(request).await?;
        let (mut ws_write, mut ws_read) = ws_stream.split();

        let (text_tx, mut text_rx) = mpsc::channel::<TtsCommand>(32);
        let (audio_tx, audio_rx) = mpsc::channel::<TtsChunk>(64);

        // Send BOS (beginning of stream) message.
        // Clamp speed to the ElevenLabs API range (0.7–1.2). Omit if
        // default (1.0) to maximize compatibility.
        let clamped_speed = config
            .speed
            .map(|s| s.clamp(0.7, 1.2))
            .filter(|&s| (s - 1.0).abs() > f64::EPSILON);
        let bos = TtsBosMessage {
            text: " ".to_string(),
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: 0.75,
                speed: clamped_speed,
            },
            generation_config: Some(GenerationConfig {
                chunk_length_schedule: vec![120, 160, 250, 290],
            }),
        };
        let bos_json = serde_json::to_string(&bos)?;
        ws_write.send(Message::Text(bos_json.into())).await?;
        debug!("TTS WebSocket connected");

        // Writer task: forwards text commands to the WebSocket.
        tokio::spawn(async move {
            while let Some(cmd) = text_rx.recv().await {
                let msg = match cmd {
                    TtsCommand::Text(text) => {
                        let m = TtsTextMessage { text, flush: None };
                        match serde_json::to_string(&m) {
                            Ok(json) => Message::Text(json.into()),
                            Err(e) => {
                                error!("TTS serialize error: {e}");
                                continue;
                            }
                        }
                    }
                    TtsCommand::Flush => {
                        let m = TtsTextMessage {
                            text: " ".to_string(),
                            flush: Some(true),
                        };
                        match serde_json::to_string(&m) {
                            Ok(json) => Message::Text(json.into()),
                            Err(e) => {
                                error!("TTS flush serialize error: {e}");
                                continue;
                            }
                        }
                    }
                    TtsCommand::Eos => {
                        // Send EOS (empty text) to signal end of input.
                        // Don't close the WebSocket — let the reader drain
                        // remaining audio until the server sends `is_final`.
                        let m = TtsTextMessage {
                            text: String::new(),
                            flush: None,
                        };
                        if let Ok(json) = serde_json::to_string(&m) {
                            let _ = ws_write.send(Message::Text(json.into())).await;
                        }
                        continue;
                    }
                    TtsCommand::Close => {
                        // Send EOS (empty text) to signal end of input.
                        let m = TtsTextMessage {
                            text: String::new(),
                            flush: None,
                        };
                        if let Ok(json) = serde_json::to_string(&m) {
                            let _ = ws_write.send(Message::Text(json.into())).await;
                        }
                        let _ = ws_write.close().await;
                        break;
                    }
                };
                if let Err(e) = ws_write.send(msg).await {
                    error!("TTS WebSocket send error: {e}");
                    break;
                }
            }
        });

        // Reader task: receives audio chunks and decodes PCM.
        tokio::spawn(async move {
            while let Some(msg_result) = ws_read.next().await {
                match msg_result {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<TtsResponse>(&text) {
                            Ok(resp) => {
                                if let Some(audio_b64) = resp.audio {
                                    match base64::engine::general_purpose::STANDARD
                                        .decode(&audio_b64)
                                    {
                                        Ok(bytes) => {
                                            let pcm = bytes_to_pcm_i16(&bytes);
                                            if !pcm.is_empty() {
                                                // Prefer per-chunk alignment over normalizedAlignment.
                                                // normalizedAlignment has session-absolute timestamps,
                                                // but our consumer accumulates cumulative_ms from PCM
                                                // duration, so per-chunk (relative) times are needed.
                                                let alignment =
                                                    resp.alignment.or(resp.normalized_alignment);
                                                let chunk = TtsChunk { pcm, alignment };
                                                if audio_tx.send(chunk).await.is_err() {
                                                    trace!("TTS audio receiver dropped");
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!("TTS base64 decode error: {e}");
                                        }
                                    }
                                }
                                if resp.is_final == Some(true) {
                                    debug!("TTS stream complete (is_final)");
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("TTS response parse error: {e}");
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        debug!("TTS WebSocket closed by server");
                        break;
                    }
                    Err(e) => {
                        error!("TTS WebSocket read error: {e}");
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(Self { text_tx, audio_rx })
    }

    /// Send a text chunk for synthesis.
    pub async fn send_text(&self, text: &str) -> Result<(), ElevenLabsError> {
        self.text_tx
            .send(TtsCommand::Text(text.to_string()))
            .await
            .map_err(|_| ElevenLabsError::ConnectionClosed)
    }

    /// Flush remaining audio from the server.
    pub async fn flush(&self) -> Result<(), ElevenLabsError> {
        self.text_tx
            .send(TtsCommand::Flush)
            .await
            .map_err(|_| ElevenLabsError::ConnectionClosed)
    }

    /// Send EOS (end-of-stream) without closing the WebSocket.
    /// The server finishes generating audio for all flushed text and sends
    /// `is_final`, causing `recv_audio()` to return `None`.
    pub async fn send_eos(&self) {
        let _ = self.text_tx.send(TtsCommand::Eos).await;
    }

    /// Gracefully close the WebSocket.
    pub async fn close(self) {
        let _ = self.text_tx.send(TtsCommand::Close).await;
    }

    /// Send the close command without consuming self, allowing further
    /// `recv_audio()` calls to drain remaining audio until `None`.
    pub async fn request_close(&self) {
        let _ = self.text_tx.send(TtsCommand::Close).await;
    }

    /// Receive the next TTS chunk (PCM audio + optional alignment).
    /// Returns `None` when the stream is complete.
    pub async fn recv_audio(&mut self) -> Option<TtsChunk> {
        self.audio_rx.recv().await
    }
}

/// Convert raw LE bytes to i16 PCM samples.
fn bytes_to_pcm_i16(bytes: &[u8]) -> Vec<i16> {
    if !bytes.len().is_multiple_of(2) {
        return Vec::new();
    }
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_pcm_roundtrip() {
        let samples: Vec<i16> = vec![0, 1000, -1000, i16::MAX, i16::MIN];
        let mut bytes = Vec::new();
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let decoded = bytes_to_pcm_i16(&bytes);
        assert_eq!(decoded, samples);
    }

    #[test]
    fn bytes_to_pcm_odd_length_returns_empty() {
        let decoded = bytes_to_pcm_i16(&[1, 2, 3]);
        assert!(decoded.is_empty());
    }

    /// Integration test: verify ElevenLabs returns alignment data with sync_alignment=true.
    /// Requires ELEVENLABS_API_KEY env var. Skipped if not set.
    #[tokio::test]
    async fn alignment_data_returned() {
        let api_key = match std::env::var("ELEVENLABS_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                eprintln!("ELEVENLABS_API_KEY not set, skipping");
                return;
            }
        };

        let config = crate::ElevenLabsConfig::new(api_key);
        let mut stream = TtsStream::connect(&config).await.expect("connect");

        stream
            .send_text("Hello world, this is a test.")
            .await
            .expect("send");
        stream.flush().await.expect("flush");
        stream.send_eos().await;

        let mut got_alignment = false;
        let mut total_chunks = 0;
        while let Some(chunk) = stream.recv_audio().await {
            total_chunks += 1;
            if let Some(a) = chunk.alignment.as_ref() {
                got_alignment = true;
                eprintln!(
                    "chunk {total_chunks}: alignment with {} chars",
                    a.chars.len()
                );
            } else {
                eprintln!("chunk {total_chunks}: NO alignment");
            }
        }

        eprintln!("total chunks: {total_chunks}, got_alignment: {got_alignment}");
        assert!(total_chunks > 0, "expected at least one audio chunk");
        assert!(
            got_alignment,
            "expected alignment data with sync_alignment=true"
        );
    }
}
