## TUI Analysis (Agent 1)

This catalogs TUI-related divergence between upstream `rust-v0.129.0` and our fork's HEAD on `merge_upstream_0.129.0`. Local TUI lib is `codex-rs/tui/` (1831-line `lib.rs`, 10518-line `chatwidget.rs`, 85 chatwidget snapshots, 1399-line `chatwidget_document_reader.rs`) plus the local-only sibling crate `codex-rs/reading-view-server/` (3885-line embedded HTML).

### 1. Reading View (TUI mode) — sectioned `DocumentReaderView`
- **Type**: Local-only.
- **Description**: Long agent-produced markdown is presented as a navigable, foldable, sectioned reader. The user can browse sections, ask follow-ups via an embedded composer, and exit to leave a transcript entry. Triggered by the agent calling `present_reading_view`.
- **Implementation**:
  - `codex-rs/tui/src/bottom_pane/document_reader/{mod.rs,render.rs}` — the `DocumentReaderView` `BottomPaneView` (view id `doc_reader`).
  - `codex-rs/tui/src/bottom_pane/document_reader_ext.rs` — `include!`-included into `bottom_pane/mod.rs` to keep fork-specific glue isolated.
  - `codex-rs/tui/src/chatwidget_document_reader.rs` — 1399 lines, `include!`-included into `chatwidget.rs` for the same reason. Contains markdown→sections pipeline, `<voice>`/`<eq>` tag handling, browser-mirror state, and karaoke offset bookkeeping.
  - `codex-rs/tui/src/chatwidget.rs` lines 1604, 5931 — `EventMsg::PresentDocument` dispatch.
  - Closed-document tracking on `BottomPane.closed_document_ids` (`bottom_pane/mod.rs:207, 1027, 1045`) so replayed `PresentDocument` events after agent switch don't re-open dismissed readers.
- **Merge plan**: Preserve and reapply on top of upstream. The fork-specific `include!` pattern was already designed to minimize merge conflict surface — keep it.

### 2. Reading View (Browser mode) + `codex-reading-view-server` crate
- **Type**: Local-only.
- **Description**: Alternate "browser" mode of reading view: spawns a local Axum WebSocket server that serves a single 3885-line HTML SPA, so the document streams into a browser tab with live section updates and TTS karaoke highlighting.
- **Implementation**:
  - `codex-rs/reading-view-server/` (Cargo: axum + tokio + tower-http; lib serves `LivingReadingView.html`).
  - `codex-rs/tui/src/app_event.rs:391-399`: events `ReadingViewServerStarted`, `ReadingViewBrowserMessage`, `ReadingViewModeChanged`; `ReadingViewMode` enum (`Tui`/`Browser`/`Disabled`) at line 632.
  - `codex-rs/tui/src/app.rs:1302-1305, 3658-3689`: handles server handoff and mode toggling.
  - `chatwidget.rs:861-884` holds `reading_view_server`, pending events, `reading_view_browser_*` mirror state.
  - Cargo dep `codex-reading-view-server` (line 46 of `tui/Cargo.toml`).
- **Merge plan**: Preserve.

### 3. TTS playback with karaoke word highlighting
- **Type**: Local-only (no upstream TTS support).
- **Description**: Streams ElevenLabs TTS audio and renders synchronized word highlighting in the reading view ("karaoke"). Supports interrupt, pause/resume, playback speed change (`+0.1`/`-0.1`), and prefetching of adjacent sections.
- **Implementation**: Events in `codex-rs/tui/src/app_event.rs:521-598`. Alignment data via `codex_elevenlabs::TtsAlignment`. Public re-exports for integration tests at `lib.rs:79-103`. Integration tests `codex-rs/tui/tests/karaoke_integration.rs`, `tts_e2e.rs`, `tts_sync_report.rs`; fixtures `tts_short.json`, `tts_medium.json`; helpers `support/recorded_tts.rs`, `support/sync_oracle.rs`.
- **Merge plan**: Preserve.

### 4. Voice input mode (push-to-talk + voice tags) — `<voice>`/`<eq>` markup
- **Type**: Shared (both implemented), but heavily diverged.
- **Description**: Both forks ship a voice/realtime feature on `cfg(not(target_os = "linux"))`. Upstream gates voice unconditionally on non-Linux; local fork adds an explicit Cargo feature `voice-input = ["dep:cpal", "dep:hound"]` (default on) and adds richer wrapper tags (`<voice>`, `<eq latex="…">spoken</eq>`).
- **Implementation**:
  - Local: `tui/src/voice.rs` (1198 lines), `tui/src/vad.rs` (312 lines), `tui/src/audio_device.rs` (176 lines), `tui/src/chatwidget/voice_mode.rs`.
  - Upstream: simpler `#[cfg(not(target_os = "linux"))]` gate; no `vad.rs`.
- **Merge plan**: Switch to upstream's `cfg(not(linux))` skeleton, but reapply local additions on top.

### 5. ElevenLabs / realtime audio device picker integration
- **Type**: Shared. Both wire `RealtimeAudioDeviceKind`, `Op::RealtimeConversation*`, and `/realtime`+`/settings` slash commands.
- **Implementation**:
  - Local: `chatwidget/realtime.rs` (RealtimeConversationUiState, phases, capture stop flag).
  - Both: `audio_device.rs` provides `list_realtime_audio_device_names`; `chatwidget.rs:7020-7110` device-selection slash commands.
- **Merge plan**: Adopt upstream's realtime conversation UI structure. Port local's ElevenLabs TTS branch on top.

### 6. Mobile/Remote-control server (WebSocket + mDNS + QR)
- **Type**: Local-only.
- **Description**: `--remote-control` flag exposes the in-process app server over WebSocket so an ATA-Swift mobile app can drive the running TUI. Bonjour/`_codex-remote._tcp` advertises it on the LAN; the `/mobile` slash command pops a setup view that renders a QR code with the bind addr+token.
- **Implementation**:
  - `tui/src/remote_control.rs` (224 lines), `remote_discovery.rs` (94 lines), `mobile_daemon.rs` (238 lines), `qr_render.rs` (59 lines), `bottom_pane/mobile_setup_view.rs`.
  - `tui/src/cli.rs:115-125` flags `--remote-control`, `--remote-control-port`, `--remote-control-token`.
  - Deps: `gethostname`, `mdns-sd`, `qrcode = "0.14"`.
- **Merge plan**: Preserve.

### 7. ATA Account view (Supabase email-OTP login) + `/account` and `/logout`
- **Type**: Local-only.
- **Description**: `/account` opens an email-OTP sign-in walkthrough backed by Supabase. `/logout` is a separate slash command. Watches `~/.ata/session_expired` marker.
- **Implementation**: `tui/src/bottom_pane/account_view.rs` — `AtaLoginStep` state machine. `chatwidget.rs:4734-4736, 4971-4974`. `tui/tests/suite/logout.rs` — 172 lines.
- **Merge plan**: Preserve.

### 8. Research tools toggle popup + `/research` slash command
- **Type**: Local-only.
- **Description**: `/research` opens a toggle popup for paper search, Zotero, Hacker News, Patents, Repo Analysis, Reading View (tri-state Tui/Browser/Disabled), and Knowledge Base.
- **Implementation**: `tui/src/bottom_pane/research_tools_view.rs` — `RESEARCH_FEATURES` table, `ResearchToolItem`, `ResearchToolsView`. Wired in `bottom_pane/mod.rs:96, 167-168`; `chatwidget.rs:237, 7974`. Backed by `Feature::ResearchPaperSearch/Zotero/HackerNews/Patents/RepoAnalysis/ReadingView/ResearchKnowledgeBase`.
- **Merge plan**: Preserve.

### 9. Voice setup view + `/voice` and `/voice-setup` slash commands
- **Type**: Local-only.
- **Implementation**: `tui/src/bottom_pane/voice_setup_view.rs` — `VoiceSetupItemKind::{Toggle,ApiKey,Selection,Stepper}`. `slash_command.rs:62-64`: `Voice`, `VoiceSetup`. Event `AppEvent::ApplyVoiceSettings`.
- **Merge plan**: Preserve.

### 10. Multi-agent UI (`/agent`, `/subagents`, `/collab`, `/team`)
- **Type**: Shared (both implemented), local has extra surfaces.
- **Description**: Both have an Agent picker / collaboration mode. Local fork adds `/team` (lists coordination agents) and `/jobs` (scheduled jobs/daemon status).
- **Implementation**: Local `tui/src/multi_agents.rs`, `tui/src/collaboration_modes.rs`, `chatwidget/agent.rs` (60-line bootstrapper), `chatwidget.rs:5050,5065,5235`.
- **Merge plan**: Adopt upstream's `multi_agents.rs` + `side.rs`. Reapply `/team` + `/jobs`.

### 11. Chatwidget split files — local refactored differently from upstream
- **Type**: Shared (both refactored, different shapes).
- **Description**:
  - Local: `agent.rs`, `interrupts.rs`, `realtime.rs`, `session_header.rs`, `skills.rs`, `voice_mode.rs`, `tests.rs`.
  - Upstream: `goal_menu.rs`, `goal_status.rs`, `goal_validation.rs`, `hooks.rs`, `ide_context.rs`, `keymap_picker.rs`, `mcp_startup.rs`, `plan_implementation.rs`, `plugins.rs`, `reasoning_shortcuts.rs`, `side.rs`, `slash_dispatch.rs`, plus per-feature test files.
- **Merge plan**: Adopt upstream's split structure and re-port local's voice_mode.rs, realtime.rs ElevenLabs branch.

### 12. Chatwidget snapshots (test corpus) — major divergence
- **Type**: Shared but completely different sets.
- **Description**: Local has 85 snapshots; upstream has 174 covering features local removed.
- **Merge plan**: Adopt upstream's snapshot corpus wholesale, then add fork-specific snapshots only where local widgets exist.

### 13. Removed upstream modules (require migration)
- **Type**: Shared but local DELETED.
- **Description**: Upstream has these TUI modules local no longer imports: `app_command.rs`, `app_server_session.rs`, `approval_events.rs`, `auto_review_denials.rs`, `branch_summary.rs`, `clipboard_copy.rs` (renamed to `clipboard_text.rs`), `diff_model.rs`, `external_agent_config_migration*.rs`, `goal_display.rs`, `history_cell/`, `ide_context*.rs`, `keymap*.rs`, `local_chatgpt_auth.rs`, `model_catalog.rs`, `motion.rs`, `npm_registry.rs`, `permission_compat.rs`, `resize_reflow_cap.rs`, `resume_picker/`, `session_resume.rs`, `session_state.rs`, `terminal_probe.rs`, `terminal_title.rs`, `test_support.rs`, `token_usage.rs`, `transcript_reflow.rs`, `update_versions.rs`, `width.rs`, `workspace_command.rs`. Upstream `bottom_pane/` adds `action_required_title.rs`, `chat_composer/history_search.rs`, `hooks_browser_view.rs`, `memories_settings_view.rs`, `selection_tabs.rs`, `status_line_style.rs`, `status_surface_preview.rs`, `title_setup.rs`, plus `request_user_input/` subdir.
- **Merge plan**: For each, decide module-by-module.

### 14. Skill popup (`$`-prefix mention) and `SkillsToggleView`
- **Type**: Shared but local is fork-specific.
- **Implementation**: `bottom_pane/skill_popup.rs`, `bottom_pane/skills_toggle_view.rs`, `tui/src/skills_helpers.rs`, `chatwidget/skills.rs`. Local uses `$` prefix.
- **Merge plan**: Switch to upstream's skill popup files; reapply local's `$`-trigger.

### 15. Theme picker — shared, mostly identical
- **Type**: Shared.
- **Description**: Diff is essentially formatting + a removed `tx.send(AppEvent::SyntaxThemePreviewed)` event call.
- **Merge plan**: Adopt upstream's version.

### 16. Onboarding (auth + provider picker + trust)
- **Type**: Shared.
- **Implementation**: `tui/src/onboarding/`.
- **Merge plan**: Adopt upstream's. Verify any local additions to provider_picker.

### 17. Status, status indicator, streaming controller — moderate diffs
- **Type**: Shared, large diffs.
- **Description**: `status/card.rs` (+/- 433), `status/helpers.rs` (+/- 223), `status/tests.rs` (+/- 1057), `status_indicator_widget.rs` (+/- 117), `streaming/controller.rs` (+/- 682).
- **Merge plan**: Adopt upstream's structures; re-port local-specific fields.

### 18. `text_formatting.rs` huge expansion (+1102 lines)
- **Type**: Shared.
- **Description**: Local fork added the `<eq latex="…">spoken</eq>` parser, `latex_to_plain_text`, equation-marker handling for TTS karaoke, plus many tests.
- **Merge plan**: Take upstream's base + reapply all `<eq>`-related parsing helpers.

### 19. CLI flags additions — `--remote-control*`, `--web-search`/`--search`
- **Implementation**: `tui/src/cli.rs` adds 3 remote-control flags. `lib.rs:384-388` keeps the legacy `--search` → `web_search="live"` alias.
- **Merge plan**: Adopt upstream cli.rs; re-add the 3 remote-control flags and the `--search` mapping.

### 20. Removed local-only code surfaces
- **Type**: Shared but LOCAL deleted.
- **Description**: All upstream features local removed: `/title` setup, `/hooks`, `/plan`, `/plugins`, `/memories`, `/ide`, `/keymap`, `/goal`, `/raw`, `/auto-review` aka `/approve`, `/vim`. Each had a slash command, popup, and snapshots upstream.
- **Merge plan**: Decision per-feature.

### Items not fully classified
- **`chatwidget/agent.rs`** — small (60-line) bootstrapper; need confirmation upstream's equivalent.
- **`chatwidget/interrupts.rs`** — exists in both.
- **`bottom_pane/footer.rs` and `chat_composer.rs`** — both have large diffs.
- **Render-only files** (`render/highlight.rs`, `render/renderable.rs`, `markdown_render.rs`).
- **Removed `chat_composer/` subdir** — upstream split composer into a subdir; local kept flat.
