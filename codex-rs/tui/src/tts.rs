//! ElevenLabs TTS playback for agent text deltas.
//!
//! Activated when `[elevenlabs] api_key = "..."` is set in
//! `~/.codex/config.toml`. When active, every agent message delta is also
//! streamed into an ElevenLabs WebSocket TTS session; the resulting PCM is
//! played through the same `cpal`-backed `RealtimeAudioPlayer` upstream
//! voice uses, so we share one speaker pipeline regardless of TTS source.
//!
//! Architecture note: this is the "primary TTS" path on this branch.
//! Upstream's OpenAI-realtime audio output is preserved as code in
//! `voice.rs::RealtimeAudioPlayer::enqueue_frame` so a future PR can swap
//! the source back if/when ElevenLabs becomes a fallback.

// All `.expect(...)` calls in this file are on `std::sync::Mutex::lock()`
// results — `Mutex::lock` returns `Err` only on poison, which itself
// represents a panic in another holder of the lock. We intentionally
// propagate that panic rather than degrading silently.
#![allow(clippy::expect_used)]

use crate::legacy_core::config::Config;
use crate::voice::RealtimeAudioPlaybackHandle;
use crate::voice::RealtimeAudioPlayer;
use codex_elevenlabs::ElevenLabsConfig;
use codex_elevenlabs::TtsChunk;
use codex_elevenlabs::tts::TtsStream;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tracing::warn;

/// Sample rate ElevenLabs returns when the request asks for `pcm_24000`
/// output (see `codex_elevenlabs::tts::TtsStream::connect`). Mono i16.
const ELEVENLABS_OUTPUT_SAMPLE_RATE: u32 = 24_000;
const ELEVENLABS_OUTPUT_CHANNELS: u16 = 1;

/// Read `[elevenlabs]` config and produce a fully-formed ElevenLabs config
/// when `api_key` is set. Returns `None` when ElevenLabs is not configured —
/// callers treat that as "TTS disabled, use upstream voice path."
pub(crate) fn elevenlabs_config_from(config: &Config) -> Option<ElevenLabsConfig> {
    let toml = config.elevenlabs.as_ref()?;
    let api_key = toml.api_key.as_ref()?.trim().to_string();
    if api_key.is_empty() {
        return None;
    }
    let mut cfg = ElevenLabsConfig::new(api_key);
    if let Some(voice) = toml.voice_id.as_ref().filter(|s| !s.trim().is_empty()) {
        cfg = cfg.with_voice_id(voice.trim().to_string());
    }
    if let Some(model) = toml.model_id.as_ref().filter(|s| !s.trim().is_empty()) {
        cfg = cfg.with_model_id(model.trim().to_string());
    }
    if let Some(language) = toml.language_code.as_ref().filter(|s| !s.trim().is_empty()) {
        cfg.language_code = Some(language.trim().to_string());
    }
    cfg.speed = toml.speed;
    Some(cfg)
}

struct ActiveSession {
    text_tx: mpsc::UnboundedSender<TextCommand>,
    interrupt: Arc<AtomicBool>,
}

enum TextCommand {
    Text(String),
    Eos,
    Close,
}

/// Public manager held by `ChatWidget`. Owns the cpal player on the UI
/// thread (so the cpal stream stays where it was built) and exposes a
/// Send-safe `RealtimeAudioPlaybackHandle` to driver tasks running on the
/// chat widget's tokio runtime.
pub(crate) struct ElevenLabsTtsManager {
    config: ElevenLabsConfig,
    runtime: Handle,
    audio_config: Arc<Config>,
    /// Lazily initialized — we don't open the speaker until the first
    /// agent text actually arrives. `_player` is held to keep the cpal
    /// stream alive; `playback` is the Send view used by driver tasks.
    player: Mutex<Option<RealtimeAudioPlayer>>,
    playback: Mutex<Option<RealtimeAudioPlaybackHandle>>,
    active: Arc<Mutex<Option<ActiveSession>>>,
}

impl ElevenLabsTtsManager {
    pub(crate) fn try_new(config: &Config) -> Option<Self> {
        let elevenlabs = elevenlabs_config_from(config)?;
        let runtime = Handle::try_current().ok()?;
        Some(Self {
            config: elevenlabs,
            runtime,
            audio_config: Arc::new(config.clone()),
            player: Mutex::new(None),
            playback: Mutex::new(None),
            active: Arc::new(Mutex::new(None)),
        })
    }

    /// Enqueue an agent text fragment for TTS. The first call lazily opens
    /// the ElevenLabs WebSocket and the cpal speaker. Subsequent calls
    /// append to the same utterance until `flush()` is called.
    pub(crate) fn enqueue_text(&self, fragment: &str) {
        if fragment.is_empty() {
            return;
        }
        if !self.ensure_player() {
            return;
        }
        let text = fragment.to_string();
        let Some(session) = self.ensure_session() else {
            return;
        };
        if let Err(err) = session.text_tx.send(TextCommand::Text(text)) {
            warn!("ElevenLabs TTS text channel closed: {err}");
            *self.active.lock().expect("ElevenLabs active session lock") = None;
        }
    }

    /// Mark the end of the current utterance so the server flushes
    /// remaining audio. Safe to call repeatedly.
    pub(crate) fn flush(&self) {
        let guard = self.active.lock().expect("ElevenLabs active session lock");
        if let Some(session) = guard.as_ref() {
            let _ = session.text_tx.send(TextCommand::Eos);
        }
    }

    /// Hard-stop: clear playback buffer and close the active TTS session
    /// (next `enqueue_text` opens a fresh one). Used when the user
    /// interrupts the agent or starts a new turn.
    pub(crate) fn interrupt(&self) {
        if let Some(playback) = self
            .playback
            .lock()
            .expect("ElevenLabs playback handle lock")
            .as_ref()
        {
            playback.clear();
        }
        let mut guard = self.active.lock().expect("ElevenLabs active session lock");
        if let Some(session) = guard.take() {
            session.interrupt.store(true, Ordering::SeqCst);
            let _ = session.text_tx.send(TextCommand::Close);
        }
    }

    fn ensure_player(&self) -> bool {
        let mut guard = self.player.lock().expect("ElevenLabs player lock");
        if guard.is_some() {
            return true;
        }
        match RealtimeAudioPlayer::start(&self.audio_config) {
            Ok(player) => {
                *self
                    .playback
                    .lock()
                    .expect("ElevenLabs playback handle lock") = Some(player.playback_handle());
                *guard = Some(player);
                true
            }
            Err(err) => {
                warn!("ElevenLabs TTS could not open the speaker: {err}");
                false
            }
        }
    }

    fn ensure_session(&self) -> Option<ActiveSession> {
        let mut guard = self.active.lock().expect("ElevenLabs active session lock");
        if let Some(existing) = guard.as_ref() {
            return Some(ActiveSession {
                text_tx: existing.text_tx.clone(),
                interrupt: existing.interrupt.clone(),
            });
        }
        let playback = self
            .playback
            .lock()
            .expect("ElevenLabs playback handle lock")
            .clone()?;
        let elevenlabs = self.config.clone();
        let interrupt = Arc::new(AtomicBool::new(false));
        let interrupt_for_task = interrupt.clone();
        let (text_tx, mut text_rx) = mpsc::unbounded_channel::<TextCommand>();

        self.runtime.spawn(async move {
            let mut stream = match TtsStream::connect(&elevenlabs).await {
                Ok(stream) => stream,
                Err(err) => {
                    warn!("ElevenLabs TTS connect failed: {err}");
                    return;
                }
            };

            loop {
                tokio::select! {
                    cmd = text_rx.recv() => {
                        match cmd {
                            Some(TextCommand::Text(text)) => {
                                if interrupt_for_task.load(Ordering::SeqCst) {
                                    break;
                                }
                                if let Err(err) = stream.send_text(&text).await {
                                    warn!("ElevenLabs TTS send_text failed: {err}");
                                    break;
                                }
                            }
                            Some(TextCommand::Eos) => {
                                stream.send_eos().await;
                            }
                            Some(TextCommand::Close) | None => break,
                        }
                    }
                    chunk = stream.recv_audio() => {
                        match chunk {
                            Some(TtsChunk { pcm, .. }) => {
                                if interrupt_for_task.load(Ordering::SeqCst) {
                                    break;
                                }
                                playback.enqueue_pcm(
                                    &pcm,
                                    ELEVENLABS_OUTPUT_SAMPLE_RATE,
                                    ELEVENLABS_OUTPUT_CHANNELS,
                                );
                            }
                            None => break,
                        }
                    }
                }
            }
            stream.close().await;
        });

        let session = ActiveSession {
            text_tx,
            interrupt,
        };
        let snapshot = ActiveSession {
            text_tx: session.text_tx.clone(),
            interrupt: session.interrupt.clone(),
        };
        *guard = Some(session);
        Some(snapshot)
    }
}
