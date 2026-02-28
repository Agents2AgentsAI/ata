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
use crate::types::TtsBosMessage;
use crate::types::TtsResponse;
use crate::types::TtsTextMessage;
use crate::types::VoiceSettings;

/// Streaming TTS handle. Send text, receive PCM audio chunks.
pub struct TtsStream {
    text_tx: mpsc::Sender<TtsCommand>,
    audio_rx: mpsc::Receiver<Vec<i16>>,
}

enum TtsCommand {
    Text(String),
    Flush,
    Close,
}

impl TtsStream {
    /// Open a WebSocket connection to ElevenLabs TTS streaming endpoint.
    pub async fn connect(config: &ElevenLabsConfig) -> Result<Self, ElevenLabsError> {
        // Ensure rustls has a crypto provider before any TLS handshake.
        codex_utils_rustls_provider::ensure_rustls_crypto_provider();

        let url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input?model_id={}&output_format=pcm_24000",
            config.voice_id, config.model_id
        );

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
        let (audio_tx, audio_rx) = mpsc::channel::<Vec<i16>>(64);

        // Send BOS (beginning of stream) message.
        let bos = TtsBosMessage {
            text: " ".to_string(),
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: 0.75,
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
                        let m = TtsTextMessage {
                            text,
                            flush: None,
                        };
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
                                                if audio_tx.send(pcm).await.is_err() {
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

    /// Gracefully close the WebSocket.
    pub async fn close(self) {
        let _ = self.text_tx.send(TtsCommand::Close).await;
    }

    /// Receive the next PCM audio chunk (24kHz mono i16).
    /// Returns `None` when the stream is complete.
    pub async fn recv_audio(&mut self) -> Option<Vec<i16>> {
        self.audio_rx.recv().await
    }
}

/// Convert raw LE bytes to i16 PCM samples.
fn bytes_to_pcm_i16(bytes: &[u8]) -> Vec<i16> {
    if bytes.len() % 2 != 0 {
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
}
