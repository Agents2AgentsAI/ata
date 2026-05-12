## TTS/Voice Analysis (Agent 6)

**Summary:** Both upstream and the local fork have voice/audio infrastructure, but they target very different use cases. Upstream provides only the OpenAI **Realtime API** path (microphone/speaker capture + streaming via `realtime-webrtc` crate, behind `/realtime`). The local fork layers a complete, self-contained **ElevenLabs-based "Voice Mode"** on top — a `mic → STT → agent → TTS → speaker` pipeline with karaoke-style word highlighting, reading-view narration, push-to-talk, VAD, and persistent settings. Almost every voice/TTS feature listed below is fork-only; only the low-level `cpal`-based audio capture/playback infrastructure (and the `RealtimeAudioPlayer`/`VoiceCapture` skeleton in `tui/src/voice.rs`) is shared with upstream.

### 1. `codex-elevenlabs` crate (entire crate)
- **Type:** Local-only (no upstream equivalent — upstream uses OpenAI Realtime via `realtime-webrtc`)
- **Description:** Self-contained ElevenLabs SDK: WebSocket streaming TTS (PCM 24kHz output, character-level alignment), HTTP STT (`scribe_v1` model), proxy fallback, request/response types.
- **Implementation:** `codex-rs/codex-elevenlabs/Cargo.toml` (deps: `tokio-tungstenite`, `reqwest`, `base64`, `codex-utils-rustls-provider`); `src/lib.rs`, `src/tts.rs` (495 LOC, WS streaming), `src/stt.rs` (128 LOC), `src/types.rs` (157 LOC, `TtsAlignment`, `ElevenLabsConfig`, `VoiceSettings`, `GenerationConfig`), `src/error.rs`. Workspace registration: `codex-rs/Cargo.toml` lines 73, 115. Tests: `tests/record_fixtures.rs`.
- **Merge plan:** Preserve crate intact. Upstream cannot conflict (no `codex-elevenlabs` upstream). Re-add workspace member entries if `Cargo.toml` is overwritten.

### 2. `VoiceModeToml` / `ElevenLabsToml` / `VoiceVerbosity` / `VoiceOutput` config types
- **Type:** Local-only
- **Description:** Persistent `[voice_mode]` config block (enabled, output mode voice|text|both, auto_submit, vad_threshold, silence_duration_ms, tts_enabled, stt_enabled, verbosity concise|verbose) and nested `[voice_mode.elevenlabs]` (api_key, voice_id, model_id, language_code, speed).
- **Implementation:** `codex-rs/core/src/config/types.rs` lines 957–1031; `codex-rs/core/src/config/mod.rs` line 1340 (`pub voice_mode: Option<VoiceModeToml>`). Upstream `core/src/config/types.rs` has zero matches for voice/tts/elevenlabs.
- **Merge plan:** Preserve. Re-apply the additions if upstream rewrites `types.rs`/`config/mod.rs`.

### 3. Voice config edit helpers (8 functions)
- **Type:** Local-only
- **Description:** Setters that produce `ConfigEdit` records for persisting voice settings to `config.toml`.
- **Implementation:** `codex-rs/core/src/config/edit.rs` lines 72–141 (`voice_mode_enabled_edit`, `voice_mode_tts_edit`, `voice_mode_stt_edit`, `voice_mode_elevenlabs_api_key_edit`, `voice_mode_elevenlabs_language_edit`, `voice_mode_elevenlabs_language_clear`, `voice_mode_elevenlabs_speed_edit`, `voice_mode_verbosity_edit`).
- **Merge plan:** Preserve. Re-apply.

### 4. `Feature::VoiceMode` and `Feature::VoiceTranscription` flags
- **Type:** Local-only feature flags (upstream features.rs has no voice entries)
- **Description:** Two gated features: `VoiceTranscription` (composer push-to-talk only) and `VoiceMode` (full pipeline).
- **Implementation:** `codex-rs/core/src/features.rs` lines 185–210, 936–1025.
- **Merge plan:** Preserve.

### 5. `chatwidget/voice_mode.rs` — full Voice Mode state machine (6,572 LOC)
- **Type:** Local-only
- **Description:** The heart of the local voice pipeline. Implements:
  - `VoiceModePhase` (Off / Idle / Listening / Thinking / Speaking) and `VoiceModeState`
  - `<voice>` tag streaming parser (`VoiceTagParser`) — separates voice-tagged content for TTS from chat text, with sentence boundary detection (`SentenceBuffer`)
  - Equation marker handling (`<eq latex="..." speak="...">`) for math TTS
  - `voice_mode_instruction()` system prompts (concise + verbose variants — agent-facing)
  - TTS task ref-counting, in-flight session management, prefetch/cache
  - Karaoke alignment timeline (`AlignmentEntry`, `build_alignment_entries`, `repair_timeline_monotonicity`, `find_active_word`)
  - `clean_for_tts_preserving_equation_markers`
  - ElevenLabs API key resolution (config → env var fallback)
  - PTT timeout, finalize-on-tick, narration / prefetch helpers
- **Implementation:** `codex-rs/tui/src/chatwidget/voice_mode.rs`
- **Merge plan:** Preserve as-is. File is fork-specific and lives in `chatwidget/` subdir; upstream will not touch it. Wired into `ChatWidget` via `voice_mode_state: Option<voice_mode::VoiceModeState>`.

### 6. `tui/src/voice.rs` — extended audio capture/playback (1,198 LOC vs upstream 486)
- **Type:** Both (skeleton shared, ATA extends ~712 LOC)
- **Description:** Upstream provides realtime mic capture + speaker playback (`VoiceCapture::start_realtime`, `RealtimeAudioPlayer` with `enqueue_frame` for `ThreadRealtimeAudioChunk`). Fork adds:
  - `VoiceCapture::start()` (non-realtime push-to-talk recording into `Vec<i16>`)
  - WAV encoding (`encode_wav_for_voice_mode`, `encode_wav_normalized`) using `hound`
  - `transcribe_async` against OpenAI `gpt-4o-mini-transcribe` (auth-aware via `codex_login::AuthMode`)
  - `RealtimeAudioPlayer::enqueue_pcm`, `seek_to_sample`, `seek_to_ms`, `set_playback_speed`, `pause`, `resume`, `is_paused`, `playback_position_ms`, `reset_playback_position` — needed to drive ElevenLabs TTS PCM and karaoke
  - `feature = "voice-input"` cfg gating with no-op fallback module in `lib.rs` (lines 161–277) and Linux disablement
- **Implementation:** `codex-rs/tui/src/voice.rs`; upstream stub at same path.
- **Merge plan:** This is a **mixed file**. After upstream merge, re-apply the fork-only methods on `RealtimeAudioPlayer`, the `VoiceCapture::start()` push-to-talk path, the OpenAI transcription helpers, and the WAV encoders. Keep `start_realtime` aligned with upstream signature.

### 7. `tui/src/lib.rs` cfg-gated voice module + stubs (lines 73–277)
- **Type:** Local-only scaffolding
- **Description:** `voice-input` cargo feature controls compilation; Linux excluded. Provides public re-exports of fork-specific helpers and a no-op fallback module so non-voice builds compile.
- **Implementation:** `codex-rs/tui/src/lib.rs` lines 73–102, 159–277.
- **Merge plan:** Re-apply when upstream changes `lib.rs` module declarations.

### 8. `tui/Cargo.toml` voice-input feature gating + ElevenLabs dep
- **Type:** Local-only
- **Description:** Adds `default = ["voice-input"]`, `voice-input = ["dep:cpal", "dep:hound"]`, optional `cpal`/`hound` deps, and `codex-elevenlabs = { workspace = true }` (regular + dev-dep). Upstream's `cpal` is unconditional.
- **Implementation:** `codex-rs/tui/Cargo.toml` lines 16, 21, 118–120, 142.
- **Merge plan:** Re-apply feature flag definition + ElevenLabs dep entries after upstream Cargo.toml updates.

### 9. `vad.rs` — Energy-threshold Voice Activity Detection
- **Type:** Local-only
- **Description:** RMS-based VAD with onset frame counting and TTS-suppression multiplier.
- **Implementation:** `codex-rs/tui/src/vad.rs` (312 LOC); only included via `feature = "voice-input"`.
- **Merge plan:** Preserve as-is.

### 10. `bottom_pane/voice_setup_view.rs` — `/voice-setup` popup UI
- **Type:** Local-only
- **Description:** Bottom-pane setup view (882 LOC) for configuring voice defaults. Persists via `UpdateVoiceSettings` event handled in `app.rs` (lines 3890–3940).
- **Implementation:** `codex-rs/tui/src/bottom_pane/voice_setup_view.rs` and event handler in `tui/src/app.rs`.
- **Merge plan:** Preserve.

### 11. AppEvent voice variants (~25 events)
- **Type:** Local-only
- **Description:** Voice-mode-specific events in `AppEvent`: `UpdateVoiceSettings`, `VoiceModePttTimeoutCheck`, `VoiceModeTtsAudioChunk` (carries `codex_elevenlabs::TtsAlignment`), `VoiceModeMeterTick`, `VoiceModeTtsFinished`, `VoiceModeTtsError`, `VoiceModeHighlightTick`, `VoiceModeTranscriptionComplete`, `VoiceModeTranscriptionFailed`, `VoiceModeInterruptTts`, `VoiceModePauseTts`, `VoiceModeResumeTts`, `VoiceModePlaybackSpeedChange`, `VoiceModeNarrateSection`, `VoiceModePrefetchSection`, plus transcription placeholder events for the composer push-to-talk path.
- **Implementation:** `codex-rs/tui/src/app_event.rs` lines 220–598; dispatched in `tui/src/app.rs` lines 4025–4048.
- **Merge plan:** Preserve.

### 12. `/voice` and `/voice-setup` slash commands
- **Type:** Local-only (upstream has only `/realtime`)
- **Description:** `/voice` toggles the ATA voice pipeline; `/voice-setup` opens the configuration popup.
- **Implementation:** `codex-rs/tui/src/slash_command.rs` lines 62–64, 108–109, 196–197.
- **Merge plan:** Re-add.

### 13. Reading-view TTS integration (karaoke + auto-narration)
- **Type:** Local-only (no upstream reading view at all)
- **Description:** Voice Mode auto-narrates each section as the user navigates the reading view; pre-fetches adjacent sections; karaoke-highlights words synced to TTS alignment; `r` key to manually narrate; pause/resume on space.
- **Implementation:**
  - `codex-rs/tui/src/chatwidget_document_reader.rs` (1,399 LOC)
  - `codex-rs/tui/src/bottom_pane/document_reader/mod.rs` — `voice_status`, `voice_karaoke_lines`, etc.
  - `codex-rs/tui/src/bottom_pane/document_reader/render.rs`
  - `codex-rs/reading-view-server/src/assets/LivingReadingView.html` — browser-side TTS controls
- **Merge plan:** Preserve.

### 14. `<voice>` tag stripping/handling in text formatting
- **Type:** Local-only
- **Description:** Robust HTML-attribute-aware stripping of `<voice>` / `</voice>` (and `<voice name="alloy">`) so they never appear in chat history, including inside `<eq>` LaTeX wrappers. Comprehensive test suite (lines 1286–1457).
- **Implementation:** `codex-rs/tui/src/text_formatting.rs` lines 967–1457.
- **Merge plan:** Preserve.

### 15. TTS end-to-end & sync test harness
- **Type:** Local-only
- **Description:** Integration tests that hit ElevenLabs (gated by env var), record fixtures, and validate TTS↔text alignment for karaoke.
- **Implementation:**
  - `codex-rs/tui/tests/tts_e2e.rs`
  - `codex-rs/tui/tests/tts_sync_report.rs`
  - `codex-rs/tui/tests/support/recorded_tts.rs`
  - `codex-rs/codex-elevenlabs/tests/record_fixtures.rs`
- **Merge plan:** Preserve.

### Features both have (shared infrastructure)
- **`tui/src/audio_device.rs`** (176 LOC, identical line count)
- **`VoiceCapture::start_realtime`**
- **`RealtimeAudioPlayer`** core
- **`RealtimeAudioFrame` / `ConversationAudioParams` / `Op::RealtimeConversationAudio` / `EventMsg::AudioOut`** in `protocol/src/protocol.rs`
- **`/realtime` and `/settings` slash commands**
- **`cpal = "0.15"` dependency**

### Notes for the merge
- **No upstream `codex-realtime-webrtc` crate is imported by the fork**. If upstream's voice path is ever merged, decide whether to bring `realtime-webrtc/` over or continue routing realtime through the existing `RealtimeAudioPlayer`/`VoiceCapture` shim.
- Tag the following as **mixed files** for `just sync-release`: `tui/src/voice.rs`, `tui/src/lib.rs`, `tui/src/app_event.rs`, `tui/src/app.rs`, `tui/src/slash_command.rs`, `tui/src/text_formatting.rs`, `tui/Cargo.toml`, `core/src/features.rs`, `core/src/config/types.rs`, `core/src/config/edit.rs`, `core/src/config/mod.rs`, `protocol/src/protocol.rs`.
- All `<voice>` instructions in `chatwidget/voice_mode.rs` are agent-facing prompts — ensure they remain registered with the Prompt Inspector (`just check-prompts`) per `codex-rs/CLAUDE.md`.
