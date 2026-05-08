# Local-vs-Upstream Feature Analysis

**Local repository:** `/Users/huytho_ho/acli/ata/` — branch `main` (HEAD `68dca0681`)
**Upstream baseline:** `openai/codex` tag `rust-v0.129.0`
**Merge base:** `926b2f19e8c2a4c01b3a87bccd8ef8a1c23b22ab`
**Goal:** Merge upstream into our fork with one unified codepath. Keep ATA-specific features, but adopt upstream's structure and absorb our parallel implementations into upstream's whenever possible.

This document is a synthesis of 10 parallel agent findings (see `merge_info/agent_findings/` for full per-area detail). It is the source of truth for the subsequent merge plan.

Each feature is captured with:
- **Description** — what it does, user-visible behavior.
- **Implementation** — key files / crates, brief code structure.
- **Status vs upstream** — `local-only`, `upstream-only`, or `both` (and which side is more advanced).
- **Merge plan** — concrete action: keep, adopt-upstream, unify, or skip.

---

## Table of Contents

- [0. Executive Summary](#0-executive-summary)
- [Part A — Local-only features (must preserve)](#part-a--local-only-features-must-preserve)
  - [A1. Research & Data tools](#a1-research--data-tools)
  - [A2. Reading view, document reader, figure extraction](#a2-reading-view-document-reader-figure-extraction)
  - [A3. ElevenLabs TTS/STT, voice mode, audio I/O](#a3-elevenlabs-ttsstt-voice-mode-audio-io)
  - [A4. Multi-provider model support (Anthropic/Gemini/OSS)](#a4-multi-provider-model-support-anthropicgeminioss)
  - [A5. Supabase auth & ATA account](#a5-supabase-auth--ata-account)
  - [A6. Mobile control, remote daemon, QR pairing](#a6-mobile-control-remote-daemon-qr-pairing)
  - [A7. Scheduler, workspace, package manager](#a7-scheduler-workspace-package-manager)
  - [A8. LSP client crate](#a8-lsp-client-crate)
  - [A9. Embedded WebSocket app-server, device registration](#a9-embedded-websocket-app-server-device-registration)
  - [A10. Network proxy admin API](#a10-network-proxy-admin-api)
  - [A11. Custom skill categories (research/workspace/adapt-environment)](#a11-custom-skill-categories-researchworkspaceadapt-environment)
  - [A12. Other ATA-only core modules](#a12-other-ata-only-core-modules)
  - [A13. Repo-level / branding / SDK](#a13-repo-level--branding--sdk)
- [Part B — In both (candidates to switch to upstream / unify)](#part-b--in-both-candidates-to-switch-to-upstream--unify)
  - [B1. Auth manager & login crate](#b1-auth-manager--login-crate)
  - [B2. Connectors crate (monolithic vs modular)](#b2-connectors-crate-monolithic-vs-modular)
  - [B3. Models manager & provider info](#b3-models-manager--provider-info)
  - [B4. Skills system architecture](#b4-skills-system-architecture)
  - [B5. Plugins / core-plugins](#b5-plugins--core-plugins)
  - [B6. Hooks event set](#b6-hooks-event-set)
  - [B7. App-server message processor](#b7-app-server-message-processor)
  - [B8. App-server protocol v2](#b8-app-server-protocol-v2)
  - [B9. MCP integration](#b9-mcp-integration)
  - [B10. Codex-protocol crate composition](#b10-codex-protocol-crate-composition)
  - [B11. TUI shared widgets and slash commands](#b11-tui-shared-widgets-and-slash-commands)
  - [B12. Core: agent control, rollout, compact, client, turn](#b12-core-agent-control-rollout-compact-client-turn)
  - [B13. Network proxy connect_policy](#b13-network-proxy-connect_policy)
  - [B14. ChatGPT crate / responses-api-proxy](#b14-chatgpt-crate--responses-api-proxy)
  - [B15. Realtime audio (ElevenLabs vs realtime-webrtc)](#b15-realtime-audio-elevenlabs-vs-realtime-webrtc)
  - [B16. Audio device selection import path](#b16-audio-device-selection-import-path)
- [Part C — Upstream-only features (to inherit by merging)](#part-c--upstream-only-features-to-inherit-by-merging)
- [Part D — Upstream features explicitly REVERTED](#part-d--upstream-features-explicitly-reverted)
- [Part E — Recommended merge sequencing](#part-e--recommended-merge-sequencing)

---

## 0. Executive Summary

| Bucket | Count (rough) | Highest-risk items |
|---|---|---|
| **Local-only features** (Part A) | ~70 modules / ~13 areas | reading-view-server, codex-elevenlabs, voice mode, codex-research-tools, codex-data-tools, codex-scheduler, codex-workspace, lsp-client, supabase auth, mobile daemon |
| **Both (unify candidates)** (Part B) | ~30 areas | app-server message processor, protocol v2 layout, TUI app.rs, core agent/control + rollout, models manager, plugins/skills crates, hooks event set |
| **Upstream-only crates** (Part C) | 39 crates | core-api, core-plugins, core-skills, plugin, model-provider*, models-manager, sandboxing, file-system, git-utils, memories, message-history, codex-mcp, builtin-mcps, exec-server, app-server-transport, device-key, agent-identity, terminal-detection, rollout/rollout-trace, thread-store, uds, analytics, features, install-context, external-agent-migration/sessions, aws-auth, realtime-webrtc, response-debug-context |
| **Reverts to honor** (Part D) | 2 | agent-graph-store hard injection (a8488fec5e), skills watcher motion to app-server |

**Strategic merge stance:**
- Adopt upstream's modular crate layout aggressively (split monolithic local files; replace embedded core code with upstream crates) so future syncs are mechanical.
- Keep all ATA-only product features — research, reading-view, voice, scheduler, workspace, LSP, mobile, Supabase — but layer them on top of upstream's plumbing.
- Where we have a parallel implementation that is functionally narrower than upstream's (auth manager, plugins, hooks event set, models-manager), switch to upstream's and re-apply ATA hooks.
- Ignore upstream features explicitly reverted at the tip (agent-graph-store hard dep).

---

## Part A — Local-only features (must preserve)

### A1. Research & Data tools

#### A1.1 — `codex-research-tools` crate
- **Description.** Unified Rust toolkit for paper search (arXiv, OpenAlex, Semantic Scholar), Zotero library mgmt (advanced search, citations, collections, mutations), HackerNews thread retrieval/search, USPTO/EPO patent search, GitHub repo analysis. Backs ~20 model-facing tools.
- **Implementation.** `codex-rs/codex-research-tools/` (~45 .rs files). `src/lib.rs` exposes `ResearchToolkit`; `src/clients/` (arxiv/openalex/semantic_scholar/zotero/hackernews/patents/epo_auth/github); `src/tools/{paper_search,zotero/*,hackernews,patents,repo_analysis}.rs`; `src/{config,types,http_client,cache,rate_limiter,tool_specs}.rs`.
- **Status.** local-only.
- **Merge plan.** Keep crate intact. Verify `codex-rs/Cargo.toml` workspace members and `core/Cargo.toml` non-optional dep on `codex-research-tools`. No upstream conflict.

#### A1.2 — `codex-data-tools` crate (Kaggle / Hugging Face datasets)
- **Description.** Search/list/download datasets and Kaggle competitions; fetch dataset metadata.
- **Implementation.** `codex-rs/codex-data-tools/` (~16 .rs). Clients in `src/clients/{kaggle,huggingface}.rs`; tools in `src/tools/dataset_ops.rs`; gated behind `data` cargo feature.
- **Status.** local-only.
- **Merge plan.** Keep. Build with `--features data`. Register in workspace.

#### A1.3 — Research bridge tool handler (`research.rs`)
- **Description.** Dispatches model tool calls (`paper_search`, `zotero_*`, `hackernews_*`, `patent_*`, `github_analyze_repo`, etc.) into `ResearchToolkit`.
- **Implementation.** `core/src/tools/handlers/research.rs` — `ResearchBridgeHandler` implements `ToolHandler`; panic-safe `execute_native_tool`; `dispatch_tool_call` router.
- **Status.** local-only.
- **Merge plan.** Keep. Ensure registered in `core/src/tools/handlers/mod.rs` post-merge.

#### A1.4 — Data bridge tool handler (`data.rs`)
- **Description.** Same pattern for data tools (`dataset_search`, `dataset_get`, `kaggle_*`, `hf_dataset_info`, etc.). Reads secrets via `codex-secrets`.
- **Implementation.** `core/src/tools/handlers/data.rs`. Gated by `data` feature.
- **Status.** local-only.
- **Merge plan.** Keep. Verify feature gates compile.

#### A1.5 — Research/KB skills assets
- **Description.** 13 SKILL.md bundles (`zotero`, `paper-discovery`, `paper-synthesis`, `paper-synthesizer`, `cross-paper-report`, `paper-discoverer`, `research-briefing`, `hn-synthesis`, `hn-synthesizer`, `hn-discoverer`, `conversation-report`, `kb`, plus workspace and adapt-environment categories). These are pure data (yaml/md).
- **Implementation.** `codex-rs/skills/src/assets/{research,workspace,adapt-environment}/`.
- **Status.** local-only.
- **Merge plan.** Keep. Coexists with upstream's `samples/` skills (no overlap).

#### A1.6 — Research module in core (`core/src/research/`)
- **Description.** Specialized prompts, output schemas, and tool-name constants for research-mode agent runs.
- **Implementation.** `core/src/research/{mod,prompt,output_schema,tool_names,types}.rs`.
- **Status.** local-only.
- **Merge plan.** Keep.

#### A1.7 — TreeSitter crate (`treesitter/`)
- **Description.** Tree-sitter language bindings + utilities for AST traversal/symbol extraction in code-intel features. Optional (`treesitter` feature).
- **Implementation.** `codex-rs/treesitter/`.
- **Status.** local-only.
- **Merge plan.** Keep. Include as workspace member; verify feature gate.

#### A1.8 — Tool-spec registries
- **Description.** JSON/Rust schema for ~20 research tools and ~9 data tools — what the model sees as callable.
- **Implementation.** `codex-research-tools/src/tool_specs.rs`, `codex-data-tools/src/tool_specs.rs`.
- **Status.** local-only.
- **Merge plan.** Keep; ensure loaded into the tool registry on init.

#### A1.9 — Document Reader protocol events
- **Description.** Protocol-level events (`PresentDocumentEvent`, `AddDocumentSectionEvent`, `AppendDocumentSectionEvent`, `UpdateDocumentSectionEvent`, `PatchDocumentSectionEvent`, plus `Args` siblings).
- **Implementation.** `codex-rs/protocol/src/document_reader.rs` (~280 lines).
- **Status.** local-only.
- **Merge plan.** Keep. Make sure protocol crate still compiles after upstream's protocol refactor (see B10).

---

### A2. Reading view, document reader, figure extraction

#### A2.1 — `reading-view-server` crate
- **Description.** Lightweight Axum + WebSocket server that hosts a single-page browser document reader (Living Reading View). Streams section/highlight/figure events; replays event buffer to late-joining clients; bidirectional messaging for follow-up questions and read-aloud requests.
- **Implementation.** `codex-rs/reading-view-server/Cargo.toml`, `src/lib.rs` (~238 lines, `ReadingViewServer::start`, broadcast channel, replay buffer), `src/assets/LivingReadingView.html` (~2861 lines) — embedded browser app with WebSocket client, dynamic DOM rendering, karaoke sync, MathML.
- **Status.** local-only.
- **Merge plan.** Keep crate intact; ensure workspace member entry.

#### A2.2 — Document reader tool handler (`document_reader.rs`)
- **Description.** Agent-facing commands `present_document`, `add_document_section`, `append_to_section`, `update_document_section`, `patch_document_section`. Manages document state cache, streaming indicators, citation-marker stripping, markdown section parsing, TUI vs browser mode formatting.
- **Implementation.** `core/src/tools/handlers/document_reader.rs` (~1459 lines). Talks to `reading-view-server` over its event channel.
- **Status.** local-only.
- **Merge plan.** Keep. Register in handlers/mod.rs.

#### A2.3 — Document reader TUI module
- **Description.** Renders documents in TUI: section navigation, syntax highlighting, embedded images, reading progress, karaoke highlight sync with TTS. Handles keyboard nav (arrows, page up/down, search).
- **Implementation.** `tui/src/bottom_pane/document_reader/` (mod.rs ~8634 lines, render.rs ~1573 lines), `tui/src/bottom_pane/document_reader_ext.rs` (~250 lines), `tui/src/chatwidget_document_reader.rs`.
- **Status.** local-only.
- **Merge plan.** Keep. Wire into bottom-pane routing of upstream's TUI when merging.

#### A2.4 — Crop figure handler + pdfium downloader
- **Description.** Extract figures from PDFs by rendering page at 150 DPI via `pdfium-render`, crop region (x,y,w,h), export PNG/WebP with caption/description metadata. Auto-downloads pdfium binary into `~/.ata/lib/` if missing.
- **Implementation.** `core/src/tools/handlers/crop_figure.rs` (~304 lines); `core/src/tools/pdfium_downloader.rs` (~148 lines, platform-detect, GitHub release URLs, cache check).
- **Status.** local-only.
- **Merge plan.** Keep. Register handler. Document pdfium runtime requirement.

#### A2.5 — Text formatting & rendering for reading view
- **Description.** HTML-to-styled-ratatui and markdown-to-styled-text conversion. Handles `<eq>` math tags (LaTeX + spoken), `<voice>` voice tags, syntect code highlighting, link/image references.
- **Implementation.** `tui/src/text_formatting.rs` (~1102 lines).
- **Status.** local-only (extension of an upstream module).
- **Merge plan.** Keep extensions; if upstream changed text_formatting, rebase our additions on top.

---

### A3. ElevenLabs TTS/STT, voice mode, audio I/O

#### A3.1 — `codex-elevenlabs` crate
- **Description.** ElevenLabs API client: streaming TTS via persistent WebSocket (24 kHz mono i16 PCM with sentence/word alignment), HTTP STT (WAV upload → transcription).
- **Implementation.** `codex-elevenlabs/{Cargo.toml, src/{lib.rs, tts.rs, stt.rs, types.rs}}` — `TtsClient::{connect,push_text,flush,recv_chunk}`, `SttClient::transcribe`.
- **Status.** local-only.
- **Merge plan.** Keep crate.

#### A3.2 — Voice-mode state machine in TUI
- **Description.** Spacebar push-to-talk recording, agent response listening, TTS playback with karaoke (word-level highlight sync), adjustable speed, two verbosity profiles (Verbose/Concise) that prefix agent instructions; phase enum `Off → Idle → Recording → Waiting → Listening → Speaking`.
- **Implementation.** `tui/src/chatwidget/voice_mode.rs` (~6500 lines). `VoiceModeState` plus event handlers for space press/release, agent deltas, TTS chunks, cancellation; sentence buffering, equation handling.
- **Status.** local-only.
- **Merge plan.** Keep. Layer on top of upstream's TUI event loop after we merge `app.rs`.

#### A3.3 — Voice setup TUI view
- **Description.** Configure voice mode (on/off, mic/output device, speed, verbosity). Backed by `audio_device.rs` device enumeration.
- **Implementation.** `tui/src/bottom_pane/voice_setup_view.rs` (~880 lines).
- **Status.** local-only.
- **Merge plan.** Keep.

#### A3.4 — Voice activity detection
- **Description.** Trim silence at edges of recordings; live VAD scoring while spacebar held.
- **Implementation.** `tui/src/vad.rs` (~200 lines).
- **Status.** local-only.
- **Merge plan.** Keep.

#### A3.5 — Extended `voice.rs` (recording + transcription auth)
- **Description.** Adds `RecordedAudio { data, sample_rate, channels }`, `TranscriptionAuthContext` (bearer + ChatGPT account ID + base URL), `VoiceCapture::start` for full-recording mode (vs upstream's pure realtime streaming), WAV writing via `hound`, channel/SR conversion. Coexists with upstream realtime-streaming version.
- **Implementation.** `tui/src/voice.rs` (~1300 lines local; upstream is shorter, uses `legacy_core::config::Config`).
- **Status.** both — local has STT pipeline; upstream has minimal realtime-stream-only.
- **Merge plan.** Adopt upstream as baseline (so import paths match `codex_core::config` → upstream's path), then re-apply ATA's RecordedAudio + WAV writer + transcription auth on top. See B15 for realtime audio strategy.

---

### A4. Multi-provider model support (Anthropic / Gemini / OSS)

#### A4.1 — Multi-protocol `WireApi` enum
- **Description.** First-class `WireApi::AnthropicMessages` and `WireApi::GeminiGenerate` variants beyond upstream's single `Responses` variant. Drives provider adapter dispatch.
- **Implementation.** `core/src/model_provider_info.rs` (444 lines). Built-in factories: `create_anthropic_provider()`, `create_gemini_provider()`, `create_oss_provider()` (Ollama / LMStudio).
- **Status.** local-only (protocol variants).
- **Merge plan.** Keep our enum variants. After adopting upstream's `model-provider-info` crate (B3), extend its `WireApi` with `AnthropicMessages` and `GeminiGenerate` rather than duplicating the type.

#### A4.2 — Multi-provider auth storage
- **Description.** Centralized credential lookup for OpenAI/Anthropic/Gemini in `~/.ata/auth.json`; mixes API keys and OAuth credentials per provider; falls back to env vars. `name_to_provider_id` maps display names → constants.
- **Implementation.** `core/src/auth/providers.rs` — `PROVIDER_OPENAI/PROVIDER_ANTHROPIC/PROVIDER_GEMINI` constants, `get_provider_api_key`, OAuth helpers. Used by `ModelProviderInfo::api_key_with_auth`.
- **Status.** local-only.
- **Merge plan.** Keep. Integrate behind upstream's auth manager (see B1) so it becomes "ATA's provider-credentials sidecar" inside upstream's auth structure.

#### A4.3 — Provider adapter trait + Anthropic / Gemini / OpenAI adapters
- **Description.** `ProviderAdapter` trait abstracts request building, SSE parsing, endpoint, headers, auth header name/format per provider. `ProviderFactory::create_adapter(wire_api)` dispatches.
- **Implementation.** `codex-api/src/provider_adapter.rs` (~87 lines), `codex-api/src/providers/{anthropic,gemini,openai}.rs`, `codex-api/src/sse/{anthropic,gemini,responses}.rs`. Plus `core/src/client/{anthropic,gemini,gemini_code_assist,provider_streaming}.rs`.
- **Status.** local-only.
- **Merge plan.** Keep. Treat as the "translation layer" between upstream's `model-provider-info` (just config) and concrete request/response handling. Move adapter trait into a stable `codex-api` crate that depends on upstream's `model-provider-info`.

#### A4.4 — Gemini OAuth + chat-history persistence-on-resume
- **Description.** OAuth credential flow for Gemini (Code Assist), plus `provider_completion_message_persistence.rs` to recover partial Gemini responses across session resumes.
- **Implementation.** `core/src/client/gemini_code_assist.rs` (~696 lines), provider-streaming helpers, persistence module.
- **Status.** local-only.
- **Merge plan.** Keep. Document as required for any provider that returns multi-turn deltas after resume.

#### A4.5 — `lmstudio/` and `ollama/` OSS provider crates
- **Description.** Local OSS model discovery + OpenAI-compatible endpoint probing. `ollama/src/url.rs::is_openai_compatible_base_url`, env-driven config (`CODEX_OSS_PORT`, `CODEX_OSS_BASE_URL`); default ports Ollama 11434, LMStudio 1234.
- **Implementation.** `codex-rs/lmstudio/`, `codex-rs/ollama/` (the crate names exist upstream too, but with much less content — see B3).
- **Status.** both, but local has substantial extensions.
- **Merge plan.** Keep our content; if upstream `lmstudio`/`ollama` are skeletons, just merge into ours.

#### A4.6 — `third_party_models.json` (model catalog)
- **Description.** Bundled metadata for Claude (Sonnet 4.6, Opus 4.6), Gemini (Flash 2.0, Pro), GPT (4o, 4o mini): slug, display_name, supported_reasoning_levels, default_verbosity, apply_patch_tool_type, truncation_policy, supports_parallel_tool_calls, context_window, prefer_websockets, etc.
- **Implementation.** `codex-rs/core/third_party_models.json` (~218 lines). Loaded via `core/src/models_manager/model_info.rs`. Tested in `core/tests/suite/list_models.rs`.
- **Status.** local-only.
- **Merge plan.** Keep file. After adopting upstream's `models-manager` (B3), ensure its bundled `models.json` is augmented (or replaced) with ATA's entries.

#### A4.7 — Model picker visibility filtering & migration
- **Description.** `show_in_picker` field gates model visibility; deprecated-model migration prompts (`tui/src/model_migration.rs`); reasoning-effort selection in picker.
- **Implementation.** `tui/src/chatwidget.rs` test `model_picker_hides_show_in_picker_false_models_from_cache`, `tui/src/bottom_pane/list_selection_view.rs`, `tui/src/app.rs`.
- **Status.** both (basic picker), local-extended.
- **Merge plan.** Keep ATA's filtering and migration UI; layer on upstream's picker logic.

---

### A5. Supabase auth & ATA account

#### A5.1 — Supabase auth flows (magic link + device code)
- **Description.** ATA-account login via email magic link (local server on port 1455) and device code; session token stored at `~/.codex/ata_session.json`.
- **Implementation.** `codex-rs/login/src/supabase_auth.rs` (~580 lines); `codex-rs/core/src/supabase/{auth,client,session,types,error,mod}.rs`; load/save in `core/src/auth.rs`.
- **Status.** local-only.
- **Merge plan.** Keep functionality. Re-shape to live as a sidecar inside upstream's `login/src/auth/` module structure once we adopt B1.

#### A5.2 — Account view TUI
- **Description.** TUI panel showing logged-in user, subscription tier, token usage; refresh + logout actions; OAuth popup.
- **Implementation.** `tui/src/bottom_pane/account_view.rs` (~517 lines). Slash command `/account`.
- **Status.** local-only.
- **Merge plan.** Keep. Map `/account` in the merged slash-command enum.

#### A5.3 — ATA-extended `cli/src/login.rs`
- **Description.** Adds `send_ata_otp`, `verify_ata_otp`, device-code flows on top of standard codex login.
- **Status.** local-extended.
- **Merge plan.** Take upstream's CLI as baseline; re-add ATA flows. Optional feature flag.

---

### A6. Mobile control, remote daemon, QR pairing

#### A6.1 — Mobile daemon & remote control
- **Description.** Detached WebSocket daemon for mobile clients to remote-control ATA; `~/.ata/mobile-server.pid` lifecycle; QR code pairing; mDNS discovery; bridges sessions through AppServer.
- **Implementation.** `tui/src/{mobile_daemon.rs, remote_control.rs, remote_discovery.rs, qr_render.rs}` (~700 lines combined); `tui/src/bottom_pane/mobile_setup_view.rs` (~709 lines). Slash command `/mobile`.
- **Status.** local-only.
- **Merge plan.** Keep. Consolidate over upstream's `app-server-transport` (Part C) once that lands.

---

### A7. Scheduler, workspace, package manager

#### A7.1 — `codex-scheduler` crate
- **Description.** Background-job scheduler with cron, file-watch, HTTP poll, and webhook triggers; SQLite persistence; daemon lifecycle commands.
- **Implementation.** `codex-rs/scheduler/` — `src/cli.rs` (~600 lines), `src/engine/scheduler.rs` (441 lines), `src/storage/{jobs_repo,runs_repo,state_repo}`, `src/trigger/{cron_trigger,file_watch,http_poll,webhook}`, `src/daemon/`, `migrations/001_init.sql`. Deps: sqlx, cron, notify, tokio, reqwest.
- **Status.** local-only.
- **Merge plan.** Keep crate. Verify Cargo + Bazel workspace registration after merge.

#### A7.2 — `codex-workspace` crate (multi-repo workspaces)
- **Description.** Multi-repo workspace mgmt: manifest + lock files, audit logging, repo clone/pin/unpin/state mgmt, `@`-syntax path resolution, run-artifact lifecycle, recipes/templates. ~30 subcommands (init, list, read, select, delete, resolve, audit, export-spec, materialize, validate, repo-*, run-*, etc.).
- **Implementation.** `codex-rs/codex-workspace/{src/commands/*, src/manifest.rs, src/spec.rs, src/workspace_resolution.rs, src/git.rs, src/lock.rs, src/recipes.rs, src/audit.rs}`.
- **Status.** local-only.
- **Merge plan.** Keep. Orthogonal to upstream `collaboration-mode-templates` / `external-agent-*`.

#### A7.3 — `codex-package-manager` crate
- **Description.** Generic package downloader/extractor: tar.gz, zip; SHA-256 verification; per-platform tuples. Used by SDK + shell-tool-mcp distribution.
- **Implementation.** `codex-rs/package-manager/{src/{manager,archive,platform,package,config}.rs}`. Public types `PackageManager`, `ManagedPackage`, `PackageManagerConfig`, `ArchiveFormat`, `PackagePlatform`.
- **Status.** local-only.
- **Merge plan.** Keep crate.

---

### A8. LSP client crate

#### A8.1 — `codex-lsp-client`
- **Description.** Standalone LSP client integration with language detection, server registry (phf hash map per language), workspace root discovery, config merging.
- **Implementation.** `codex-rs/lsp-client/src/{client,server_registry,language,root_discovery,config_merge,builtin_servers}.rs`. Uses `lsp-types 0.97`.
- **Status.** local-only.
- **Merge plan.** Keep. Ensure dev deps align with workspace constraint after merge.

---

### A9. Embedded WebSocket app-server, device registration

#### A9.1 — Embedded WebSocket mode
- **Description.** In-process WebSocket endpoint that shares the host's `ThreadManager` with embedding processes (TUI, mobile). Optional bearer/JWT auth.
- **Implementation.** `app-server/src/embedded.rs` (~385 lines), `app-server/src/transport.rs` (~1.6k lines). Adds `MessageProcessor::new_with_thread_manager` constructor.
- **Status.** local-only.
- **Merge plan.** Keep file. After upstream's `app-server-transport` extraction (Part C), re-implement embedded mode against the new transport trait.

#### A9.2 — Device registration API
- **Description.** Register and authenticate mobile/web clients; manages device keys, auth tokens, account/device endpoints.
- **Implementation.** `app-server/src/device_registration.rs` (~432 lines).
- **Status.** local-only.
- **Merge plan.** Keep. After upstream `device-key` lands (Part C), back this module by it.

---

### A10. Network proxy admin API

- **Description.** HTTP admin endpoints on the network proxy: `/health`, `/config`, `/patterns`, `/blocked`, plus `/mode` and `/reload`.
- **Implementation.** `codex-rs/network-proxy/src/admin.rs` (~72 lines).
- **Status.** local-only.
- **Merge plan.** Keep. Pair with upstream's `connect_policy.rs` (B13).

---

### A11. Custom skill categories (research / workspace / adapt-environment)

- **Description.** Extends upstream's single `.system` skill category with three additional fingerprinted categories. Each is compiled into the binary, written to `.system-<name>/` on startup if fingerprint changes. ATA-Plus adds a private `remote-exec` skill set behind a feature flag.
- **Implementation.** `codex-rs/skills/src/lib.rs` (lines 14–251) — `CUSTOM_SKILL_CATEGORIES_BASE` array, ATA-plus `#[cfg(feature = "ata-plus")]` block (lines 37–39, 72–77, 181–183), backwards-compat `install_research_skills`/`install_workspace_skills` wrappers. `codex-rs/skills/build.rs` for conditional artifacts.
- **Status.** local-only.
- **Merge plan.** Keep. Non-breaking extension over upstream's skills system. After adopting upstream's `core-skills` (B4), re-register custom categories via its extension hooks.

---

### A12. Other ATA-only core modules

These are local additions in `codex-rs/core/` with no upstream equivalent. Keep all unless noted; some will have upstream replacements after merge (flagged).

| Module | Purpose | Notes / merge plan |
|---|---|---|
| `codex.rs` (7324 lines) | Consolidated runtime loop (session init, turn pipeline, event streaming, response handling). | **Keep**. Upstream's `session/{session,turn,handlers}.rs` are decomposed; do not adopt. |
| `agent/guards.rs` + tests (226 + 243 lines) | Spawn-depth + count limits, nickname versioning. | **Keep**. Replaces upstream's `agent/registry.rs`. |
| `analytics_client.rs` + tests (~1055 lines) | Local telemetry pipeline (skill/app/plugin invocations) with dedup + queue. | **Keep**, but watch upstream's new `analytics` crate (Part C) — they may unify; for now keep ours. |
| `auth/{providers,refresh,storage,gemini_oauth,gemini_revoke}.rs` (~2000 lines) | Multi-provider creds (see A4.2). | Keep; later wrap in upstream's auth manager (B1). |
| `memories/{mod,phase1,phase2,control,start,storage,citations,prompts,usage}.rs` + README | Phase-1/Phase-2 memory extraction & consolidation. | **Keep**. Distinct from upstream's `memories` crate (Part C); evaluate later whether to swap. |
| `config_loader/{mod,layer_io,macos}.rs` | 8-layer config stacking (cloud > admin > system > user > cwd > tree > repo > runtime); macOS managed-device profile. | **Keep**. |
| `mcp_connection_manager.rs` (~330 lines) | MCP server lifecycle, connection pool, env-var resolution. | Keep until upstream `codex-mcp` adoption (B9). |
| `research/` (see A1.6) | — | — |
| `external_agent_config.rs` | Detect Claude.app config/skills/MCP for migration. | Keep; complementary to upstream's `external-agent-migration`. |
| `tools/code_mode/` (~1000 lines + bridge.js + runner.cjs) | Sandboxed JS REPL for inline computation; pragma parsing; tool composition. | **Keep**. No upstream equivalent. |
| `config/agent_roles.rs` (205 lines) | Role-based agent capabilities (TOML-driven). | Keep. |
| `state_db.rs`, `state_db_bridge.rs` | Session/turn persistence. | Keep (note upstream made state-DB optional, see Part D). |
| `session_prefix.rs`, `context/subagent_notification.rs` | Subagent context framing. | Keep. |
| `custom_prompts.rs` (149 lines) | User overrides for system/user/developer instructions. | Keep. |
| `data/{mod,tool_names}.rs` | Tool-name constants registry. | Keep. |
| `api_bridge.rs` (274 lines) | HTTP client for cloud backend. | Keep. |
| `tools/handlers/{agent_jobs*, goal*, mcp_resource*, multi_agents*, multi_agents_v2, plan*, request_plugin_install*, unavailable_tool, unified_exec, test_sync*}` | Expanded handler suite (CSV-spawn batches, goal mgmt, MCP resources, multi-agent v2, plan synth, plugin-install approval, unified exec). | Keep; `unified_exec` partially overlaps with upstream `exec-server` (Part C). |
| `unified_exec/{mod,errors,async_watcher,head_tail_buffer,process_manager,process,process_state}.rs` | Streamed exec with output buffering & process mgmt. | Keep; selectively adopt upstream `exec-server` improvements. |
| `hook_runtime.rs` | User-defined hook executor. | Keep; align event set with upstream (B6). |
| `seatbelt_*.{sbpl,rs}`, `sandboxing/macos_permissions.rs` | macOS Seatbelt policies + Landlock + Windows sandbox. | Keep; merge upstream's `sandboxing` crate improvements (Part C) on top. |
| `environment_selection.rs`, `environment_context.rs`, `contextual_user_message.rs` | Env / context injection. | Keep with upstream's reorg in mind (refactored module locations). |
| `review_prompts.rs`, `review_format.rs`, `prompt_snapshot.rs`, `prompt_debug.rs` | Specialized prompts (review, compaction, debug). | Keep. |
| `turn_metadata.rs`, `turn_diff_tracker.rs`, `thread_rollout_truncation.rs`, `state/turn.rs` | Turn-scoped metadata + truncation policies. | Keep. |

---

### A13. Repo-level / branding / SDK

- **TypeScript SDK** (`sdk/typescript/`) — wraps `@a2a-ai/ata` CLI, JSONL events over stdio. Local-only.
- **codex-cli wrapper** branding — `bin/ata.js` + `package.json` scoped `@a2a-ai/ata` (vs upstream `@openai/codex`).
- **shell-tool-mcp/** — TypeScript MCP server with prebuilt bash/zsh binaries for sandboxed shell exec. Upstream has Rust `shell-escalation/` instead. Both coexist non-overlappingly.
- **`tools/argument-comment-lint/`** — Lints `/*param_name*/` argument comments per AGENTS.md convention. Local-only.
- **Justfile recipes** — Adds `test-reading-view`, `test-karaoke`, `test-tts-live`, `test-tts-sync`, `fix-fast`; removes `tui-with-exec-server`.
- **AGENTS.md / README.md** — ATA branding, scheduler/workspace/SDK sections, release-branch-vs-private-code guidance.
- **scripts/** — `install.sh`, `check_blob_size.py`, `stage_npm_packages.py`, `install/` per-OS installers.
- **third_party/** — `meriyah` (JS parser) + `wezterm` (vs upstream `v8` + `wezterm`).

**Merge plan.** Adopt upstream's repo-config files as baselines; re-layer all ATA additions above (test targets, brand strings, new crate registrations). Keep TypeScript SDK + branding wrappers. Monitor upstream `shell-escalation` for features worth porting into our TS MCP.

---

## Part B — In both (candidates to switch to upstream / unify)

These are areas where both repos have implementations. The merge goal is one unified codepath — usually upstream's, with our customizations layered on. Each item lists which side to take as baseline.

### B1. Auth manager & login crate

- **Local.** Simplified auth in `core/src/auth.rs` (1490 lines) + `core/src/auth/{providers,refresh,storage,gemini_oauth,gemini_revoke}.rs`. Supabase session loading scattered.
- **Upstream.** Comprehensive `login/src/auth/{manager,storage,external_bearer,revoke,agent_identity}.rs` + `login/src/token_data.rs` (JWT) + `login/src/auth_env_telemetry.rs`. Includes lenient/strict storage, token refresh, agent identity.
- **Merge plan.** **Adopt upstream.** Move auth logic from core to login crate. Wrap our multi-provider storage (A4.2) and Supabase flows (A5.1) as plug-in providers within upstream's `AuthManager`. Adopt `device-key` + `agent-identity` (Part C) as prerequisites. **High risk** — Supabase flows must keep working.

### B2. Connectors crate (monolithic vs modular)

- **Local.** `connectors/src/lib.rs` ~535 lines with cache + listing + merging + filtering inline.
- **Upstream.** Same crate, modularized: `lib.rs` (~350) + `merge.rs` + `filter.rs` + `accessible.rs` + `metadata.rs`.
- **Merge plan.** **Adopt upstream's modular layout.** No behavior change. Easy refactor.

### B3. Models manager & provider info

- **Local.** Embedded in `core/src/models_manager/{manager,cache,model_info,model_presets,collaboration_mode_presets}.rs` and `core/src/model_provider_info.rs`. Multi-protocol `WireApi` (A4.1) is local.
- **Upstream.** Three dedicated crates: `model-provider-info/`, `model-provider/` (auth + Bedrock), `models-manager/` (with `ModelsEndpointClient` trait, `RefreshStrategy` enum {Online, Offline, OnlineIfUncached}, ETag cache invalidation, `ModelsManagerConfig`, async trait-based design).
- **Merge plan.** **Adopt upstream's three crates.** Extend upstream's `WireApi` with `AnthropicMessages` and `GeminiGenerate` (A4.1). Make our Anthropic/Gemini factories register with upstream's `built_in_model_providers`. Add ATA models to upstream `models.json` (or override at runtime). Keep our adapters in `codex-api` calling upstream's config.

### B4. Skills system architecture

- **Local.** `codex-rs/skills/` (custom-category installer) + `core/src/skills/{loader,manager,model,permissions,remote,render,system,env_var_dependencies,invocation_utils,injection,render}.rs`.
- **Upstream.** Dedicated `core-skills/` crate with same logical structure, cleaner crate boundary; uses `PluginSkillRoot` from `codex-utils-plugins`.
- **Merge plan.** **Adopt upstream's `core-skills` crate** for loader/manager/render/injection. Keep our `skills/` crate as the **system+custom category installer** (it's compatible). Extend `core-skills`'s system installer with our extra categories via its plugin/extension points. Preserves A11 with cleaner architecture.

### B5. Plugins / core-plugins

- **Local.** `core/src/plugins/{manager,marketplace,manifest,curated_repo,store,toggles,test_support,render,...}.rs` (~14 files). Has install/disable/enable but lacks marketplace removal/upgrade, remote bundle sync, workspace sharing, admin-disabled.
- **Upstream.** `core-plugins/` crate (~23 .rs) with `manager`, `marketplace`, `marketplace_add/remove/upgrade.rs`, `remote.rs`, `remote_bundle.rs`, `startup_remote_sync.rs`, `installed_marketplaces.rs`, etc. Plus `plugin/` crate for shared types (`PluginId`, `Namespace`, `LoadOutcome<T>`, `LoadedPlugin<T>`).
- **Merge plan.** **Adopt upstream's `core-plugins` and `plugin` crates.** Migrate ATA's curated-repo and other custom logic into the new structure. **Substantial refactor; medium-high risk** — guard with full plugin test suite.

### B6. Hooks event set

- **Local.** `codex-rs/hooks/src/{types,registry,engine/mod}.rs` + `events/{session_start,stop}.rs`. Only `AfterAgent` and `AfterToolUse` events.
- **Upstream.** Same crate, additionally has `events/{pre_tool_use,post_tool_use,compact (pre/post),user_prompt_submit,permission_request}.rs`, `output_spill.rs` (truncate >2500 token outputs, spill to /tmp), `config_rules.rs` (per-layer hook-state merging).
- **Merge plan.** **Add upstream's missing event modules** to our hooks crate; extend `HookEvent` enum; wire `pre/post_tool_use` into core tool execution; wire `pre/post_compact` into `core/src/compact.rs`; wire `user_prompt_submit` into turn submission. Add `output_spill.rs` and `config_rules.rs`. Add upstream's `/hooks` TUI browser. **Additive, low-risk.**

### B7. App-server message processor

- **Local.** Monolithic `app-server/src/codex_message_processor.rs` (~8.8k lines) handling all JSON-RPC dispatch (account/apps/marketplace/config/fs/git/hooks/thread/turn/command-exec/feedback/...).
- **Upstream.** Modular `app-server/src/request_processors/{account_processor,apps_processor,catalog_processor,...}.rs` per domain.
- **Merge plan.** **Adopt upstream's modular handler split.** Refactor our consolidated processor into per-domain modules; preserve account/device/embedded behavior. **Critical merge area; 3–5 days of careful work.**

### B8. App-server protocol v2

- **Local.** Monolithic `app-server-protocol/src/protocol/v2.rs` (~7.8k lines).
- **Upstream.** Directory `protocol/v2/` with 30+ files (`account.rs`, `apps.rs`, `collaboration_mode.rs`, `command_exec.rs`, `config.rs`, `device_key.rs`, `experimental_feature.rs`, `feedback.rs`, `fs.rs`, `hook.rs`, `item.rs`, `mcp.rs`, `model.rs`, `notification.rs`, `permissions.rs`, `plugin.rs`, `process.rs`, `realtime.rs`, `review.rs`, `shared.rs`, `tests.rs`, `thread_data.rs`, `thread.rs`, `turn.rs`, `windows_sandbox.rs`).
- **Merge plan.** **Adopt upstream's modular layout** (mechanical split). **2–3 days.** Necessary for clean future syncs.

### B9. MCP integration

- **Local.** Inline in `core/src/mcp.rs` + `mcp_tool_call.rs` + `mcp_tool_approval_templates.rs` + `mcp_tool_exposure.rs` + `tools/handlers/mcp.rs` + handling threaded through app-server message processor. `codex-rmcp-client` for client transport.
- **Upstream.** New crates: `codex-mcp/` (server/client coordination, Codex Apps auth elicitations through Guardian, OAuth login, scope discovery, status snapshots, tool provenance), `builtin-mcps/` (ships `memories` MCP), `core-api/` (public facade types).
- **Merge plan.** **Adopt upstream `codex-mcp` + `builtin-mcps`.** Migrate ATA's MCP code into them; expose MCP requests via upstream's app-server route; verify Guardian elicitation flow works for our auth setup. May enable removing MCP code from app-server.

### B10. Codex-protocol crate composition

- **Local.** Pruned: removed `account.rs`, `agent_path.rs`, `session_id.rs`, `tool_name.rs`, `auth.rs`, `error.rs`, `error_tests.rs` (8.2k lines), `exec_output.rs`, `exec_output_tests.rs`, `mcp_approval_meta.rs`, `memory_citation.rs`, `network_policy.rs`, `shell_environment.rs`. Added: `custom_prompts.rs`, `document_reader.rs` (~280), `message_history.rs` (~11).
- **Upstream.** Retains all pruned modules; further refactored some into `app-server-protocol`.
- **Merge plan.** **Audit each removed module before merging.** Restore those still imported by app-server / other crates. Keep our additions. **1–2 days careful review.**

### B11. TUI shared widgets and slash commands

#### B11.1 — Slash command set

- **Local-only commands.** `Research`, `Voice`, `VoiceSetup`, `Mobile`, `Account`, `Jobs`, `Team`, `MultiAgents`.
- **Upstream-only commands.** `Ide`, `Keymap`, `Vim`, `Goal`, `Hooks`, `Memories`, `Side`, `Raw`, `Title`, `Plugins`, `AutoReview`.
- **Merge plan.** Take upstream's slash-command enum as baseline; add the local-only commands; **decide per upstream-only** whether to re-expose or stay hidden. Low-hanging: bring back `Hooks` (paired with B6), `Memories` (we have own memories — separate command name?), `Plugins` (paired with B5), `Ide`, `Diff` workspace-aware.

#### B11.2 — Chat composer
- **Local.** Restructured (~6694 line diff), adds VoiceState (spacebar hold-to-talk), reverse search; removes upstream's `history_search/` (956 lines) and zellij detection.
- **Upstream.** Has history_search and Vim modal mode (PR #18595).
- **Merge plan.** Take upstream as baseline; layer voice state and reverse_search on top with feature gates.

#### B11.3 — Reverse search vs history search
- **Local.** `bottom_pane/reverse_search.rs` + `chat_composer_reverse_search.rs` (~436 lines) replacing upstream's `chat_composer/history_search.rs` (956 lines).
- **Merge plan.** Take upstream's history search; rebuild our reverse-search UX as a thin wrapper or accept upstream's UX.

#### B11.4 — Resume/fork picker
- **Local.** `tui/src/resume_picker.rs` (~600 lines).
- **Upstream.** Same file plus new `resume_picker/transcript.rs` submodule.
- **Merge plan.** Take upstream's picker (gets `transcript.rs` for free); reapply our UI tweaks if any.

#### B11.5 — Status line / footer
- **Local.** Streamlined; removed `status_line_style.rs`, `status_surface_preview.rs`, `title_setup.rs`, `action_required_title.rs`. No `/title` command.
- **Upstream.** Adds PR/branch info, raw scrollback toggle, `/keymap debug`, theme-aware colors.
- **Merge plan.** Take upstream's footer + status_line_setup as baseline; preserve title removal; integrate PR/branch rendering.

#### B11.6 — Approval / permission flows
- **Local.** Removed `auto_review_denials.rs` (131 lines); refactored `approval_overlay.rs`; renamed `/approve` → `/approvals`.
- **Upstream.** Has AutoReviewMode + auto_review_denials.
- **Merge plan.** Adopt upstream baseline. Decision needed on auto-review (re-add vs stay removed).

#### B11.7 — App.rs core state machine (massive refactor)
- **Local.** ~8665 line diff in `app.rs`; deleted ~25 modules (`app/{app_server_event_targets,app_server_events,app_server_requests,background_requests,config_persistence,event_dispatch,history_ui,input,loaded_threads,platform_actions,session_lifecycle,thread_routing}`, plus `app_server_session.rs`, `app_server_approval_conversions.rs`, `auto_review_denials.rs`, `app_command.rs`, `approval_events.rs`). Added `app_server_adapter.rs` (~72 lines).
- **Upstream.** Modular module layout for these concerns.
- **Merge plan.** **Critical area.** Decision: take upstream's modular structure as baseline (then re-port our improvements), OR take our consolidated approach and cherry-pick upstream's bug fixes. Recommend: upstream baseline + ATA event-handling improvements layered with feature gates.

#### B11.8 — Onboarding
- **Local.** Adds `onboarding/provider_picker.rs` (multi-provider OAuth setup).
- **Merge plan.** Keep our addition; layer over upstream's onboarding.

#### B11.9 — Other shared widgets
- `theme_picker.rs` — both, no change.
- `list_selection_view.rs` — both, local has UI tweaks; rebase on upstream.
- `textarea.rs` — large refactor in local (large paste detection, spinner controls); rebase on upstream.

### B12. Core: agent control, rollout, compact, client, turn

#### B12.1 — Agent control
- **Local.** `core/src/agent/control.rs` (1152 lines) using `Guards`-based spawn limits; simpler control flow.
- **Upstream.** 1074 lines using `AgentRegistry` + mailbox + `SpawnAgentForkMode {FullHistory, LastNTurns}`.
- **Merge plan.** **Keep our Guards approach** (A12). Don't import `AgentRegistry`/mailbox. Selectively adopt upstream's fork-mode capability if we need history-truncated forking.

#### B12.2 — Rollout management
- **Local.** Modularized: `rollout/{error,list,metadata,mod,policy,recorder,session_index,truncation}.rs`.
- **Upstream.** Less-modular `rollout.rs` (~800 lines).
- **Merge plan.** Keep our modular split; merge upstream's logic file-by-file. After full upstream `rollout` and `rollout-trace` crates land (Part C), evaluate moving into them.

#### B12.3 — Compaction
- **Local.** `compact.rs` (233) + `compact_remote.rs` (155); we **removed** `compact_remote_v2.rs`.
- **Upstream.** Adds `compact_remote_v2.rs`; service-tier propagation through compact (PR #21249).
- **Merge plan.** Investigate v2 contents; either reintegrate or document why removed.

#### B12.4 — Client / provider streaming
- **Local.** Modular per-provider in `core/src/client/{anthropic,gemini,gemini_code_assist,provider_streaming}.rs` (1221 lines).
- **Upstream.** Monolithic `client.rs` (~1440 lines).
- **Merge plan.** **Keep our modular structure**; merge upstream optimizations file-by-file.

#### B12.5 — Turn management
- Both modularized differently. Keep ours. Selectively adopt upstream improvements.

#### B12.6 — Sandboxing & permissions
- Both extensive. Keep our macOS Seatbelt/Landlock/Windows hardening; merge upstream's `sandboxing` crate (Part C) and standalone bwrap fallback.

#### B12.7 — Apply patch + shell tools
- Both compatible. Take upstream improvements; keep our `unified_exec`-based handler and zsh fork backend.

### B13. Network proxy connect_policy

- **Local.** No `connect_policy.rs`; we have `admin.rs` (A10) instead.
- **Upstream.** `network-proxy/src/connect_policy.rs` (~76 lines, `TargetCheckedTcpConnector` blocks non-public IPs unless `allow_local_binding`).
- **Merge plan.** **Add upstream's `connect_policy.rs`**; keep our `admin.rs`. They're complementary.

### B14. ChatGPT crate / responses-api-proxy

- **ChatGPT crate:** local missing upstream's `workspace_settings.rs` (~160 lines, 15-min cache for `enable_plugins` etc.). **Add it** post-merge.
- **responses-api-proxy:** local missing upstream's `dump.rs` (debug utility). **Add it.**

### B15. Realtime audio (ElevenLabs vs realtime-webrtc)

- **Local.** ElevenLabs WebSocket TTS + HTTP STT, cpal audio I/O, full karaoke / speed control, sentence boundary detection.
- **Upstream.** `realtime-webrtc` crate using OpenAI realtime API + libwebrtc (macOS).
- **Merge plan.** **Keep ElevenLabs as primary** (richer UX features). Add upstream `realtime-webrtc` only if we explicitly want OpenAI realtime as an alternative — gate behind feature flag if so. Avoid maintaining two providers as default.

### B16. Audio device selection import path

- Local uses `codex_core::config::Config`; upstream uses `legacy_core::config::Config`. **Trivial.** Update import path during merge.

---

## Part C — Upstream-only features (to inherit by merging)

### C1. Must-have crates (bring in fully)

| Crate | What it does | Notes |
|---|---|---|
| `core-api` | Public facade re-exporting ThreadManager, CodexThread, StateDbHandle, plus auth, config, plugins, analytics. Slims down direct `codex-core` imports. | Adoption cleans up app-server / TUI imports. |
| `model-provider`, `model-provider-info`, `models-manager` | Provider config registry + auth + aggregator (see B3). | Replaces our embedded versions; we extend with `WireApi` variants. |
| `git-utils` | Centralized git ops (info collection, patch apply, baselines, pagination flag handling, symlink creation). | Migrate from core/tools. |
| `file-system` | Trait-based read/write/delete + `FileMetadata`/`ReadDirectoryEntry`; integrates with Windows ConPTY/named pipes; Linux landlock. | Merges Windows-sandbox improvements (PRs #20270, #20685, #21409). |
| `sandboxing` | Cross-platform sandbox policy enforcement + command wrapping (bwrap on Linux, Seatbelt on macOS). Standalone bundled bwrap fallback (PR #21255). Updated bubblewrap 0.11.2. | Layer our existing Seatbelt customizations on top. |
| `rollout`, `rollout-trace` | Rollout persistence + trace recording (state DB optional). | Migrate from `core/src/rollout/`. |
| `thread-store`, `uds` | Thread persistence (storage-neutral) + cross-platform async UDS. | Foundational; bring in early. |
| `analytics` | Centralized async events client (tool lifecycle events PR #17090, goal lifecycle, plugin skill usage, thread sources). | Decision: replace or coexist with our `analytics_client.rs`. Recommend coexist for now; consolidate later. |
| `message-history` | Persistence layer for `~/.codex/history.jsonl` (atomic O_APPEND, async API, soft/hard caps). Moved out of core PR #21278. | Migrate from inline. |

### C2. Should-have crates

| Crate | What it does | Notes |
|---|---|---|
| `memories` (subcrates: `read`, `write`, `mcp`) | Long-term memory system (Phase 1 extraction async, Phase 2 consolidation agent, MCP surface, citations, telemetry, git baseline). | We have our own `core/src/memories/` (A12). Keep ours for now; evaluate full swap (~2–3 weeks). |
| `core-plugins`, `plugin` | See B5. | Adopt. |
| `core-skills` | See B4. | Adopt. |
| `codex-mcp`, `builtin-mcps` | See B9. | Adopt. |
| `external-agent-migration`, `external-agent-sessions` | Migrate Claude.app config + session histories. | Add — complementary to our `external_agent_config.rs`. |
| `exec-server` | Remote/local exec server with sandboxed FS. | Evaluate vs our `unified_exec`; likely partial adoption. |
| `app-server-transport` | Transport abstraction (WS/HTTP/stdio). | Adopt; rebase our embedded WS mode (A9.1) onto it. |
| `device-key` | Hardware/TPM/OS device key (`dk_hse_`/`dk_tpm_`/`dk_osn_`). | Required for upstream's `agent-identity`. |
| `agent-identity` | Agent identity types & resolution. | Add (B1). |
| `terminal-detection` | Detects Terminal.app, iTerm2, Ghostty, kitty, tmux/screen multiplexers. | Adopt for terminal-specific keybindings/colors. |
| `features` | Feature-flag registry + evaluator (`Feature` enum, aliases, config gating). | Adopt; consolidate ad-hoc checks. |
| `install-context` | Detect install method (npm/bun/brew/dev/standalone) → resource paths. | Adopt for npm release `bwrap` fallback. |

### C3. Optional / can-skip

| Crate | Notes |
|---|---|
| `agent-graph-store` | **Do NOT add** — upstream reverted it (Part D). |
| `realtime-webrtc` | Optional — only if we offer OpenAI realtime alongside ElevenLabs (B15). |
| `aws-auth` | Optional — only if ATA needs Bedrock backend. |
| `response-debug-context` | Nice-to-have (better error messages with x-request-id, cf-ray, etc.). |
| `test-binary-support`, `thread-manager-sample` | Dev/test infra. |

### C4. TUI features (release-notes summary)

- **`/vim` modal Vim mode** (PR #18595) + `default_mode = "vim"` config.
- **`/ide` IDE-context injection** (PR #20294).
- **Workspace-aware `/diff`** (PR #21001).
- **`/hooks` browser** (PR #19905).
- **`/keymap debug`** (PR #19631).
- **Redesigned resume/fork picker** (PR #20065) — `transcript.rs` submodule.
- **Raw scrollback mode** (PR #20819).
- **Status-line PR/branch info** (PR #19631, #20892, #20794) + theme-aware colors.
- **Ctrl-C draft + paste handling fixes** (PR #21091, #21190, #21351, #21397).
- **Bounded terminal probes** (startup latency, PR #20654, #21450) + `animations = false` for screen readers (PR #20564).

### C5. Core features (release-notes summary)

- Tool handler refactoring (PR #21395, #21416, #21427) — split handlers + tool specs into core handlers.
- Thread naming moved from core to app-server (PR #21260) — works without state DB.
- Pre/post-compaction + PreToolUse hooks + plugin-bundled hook discovery (PR #19705, #19905, #19882).
- Service-tier propagation through compact (PR #21249).
- Codex Apps auth elicitations through Guardian (PR #19431).
- MCP tool output truncation; auto-review bypasses always-allow MCP tools.
- Goals: paused-across-resume default; multi-day duration display (PR #20558); validation improvements (PR #20746).

### C6. Protocol & app-server refactoring

- App-server-transport extraction (PR #20545).
- Protocol module split.
- Item event mapping moved into app-server-protocol.
- Session ID in protocol + thread/fork return (PR #20437, #21336, #21329, #21332).
- Thread ID in MCP turn metadata.
- Installation ID resolution out of core startup (PR #21182).
- Apply-patch file changes as turn items (PR #20540).
- Turn items view in app-server (PR #21063).
- Model service tiers in protocol (PR #20971, #20969).

### C7. New top-level dirs

`.codex/`, `.devcontainer/`, `.github/`, `.vscode/` — adopt for consistency (some already exist locally).

---

## Part D — Upstream features explicitly REVERTED

We must NOT bring these in — upstream reverted them at the tip.

### D1. agent-graph-store hard injection (revert commit `a8488fec5e`, final commit before tag)
- **What was reverted.** Mandatory state-DB injection into core ThreadManager; agent-graph-store as hard descendant-lookup dep. Affected ~54 files, ~781 ins / ~834 del.
- **Why.** Made state DB mandatory when it must be optional; broke in-process client; broke many consumers.
- **What to do.** Make state DB optional everywhere; descendant lookups use optional state DB with fallback. Don't add agent-graph-store. Use thread-store-based thread naming.

### D2. Skills watcher motion to app-server (PR #21287 reverted by PR #21460)
- **What was reverted.** Moving skills watcher from core into app-server.
- **Why.** Caused concurrency issues with skills list loading.
- **What to do.** Keep skills watcher in core. Adopt parallelized cwd loading (PR #21441) as an optimization.

---

## Part E — Recommended merge sequencing

The merge is a multi-week effort. Suggested phasing:

### Phase 0 — Pre-merge prep (1–2 days)
- Build clean `merge_info` snapshots; freeze branches; capture green test baseline on local main.
- Identify CI gates (clippy, tests, snapshots) that must remain green.

### Phase 1 — Foundation crates (3–5 days)
- Add upstream **`thread-store`**, **`uds`**, **`device-key`**, **`agent-identity`**, **`features`**, **`install-context`**, **`terminal-detection`**, **`response-debug-context`**, **`git-utils`**, **`file-system`**, **`sandboxing`**, **`message-history`**, **`rollout`**, **`rollout-trace`**, **`app-server-transport`**, **`core-api`**, **`analytics`** (coexist), **`model-provider`**, **`model-provider-info`**, **`models-manager`** (B3).
- Make state DB optional (Part D).
- Move message history out of core into the new crate.

### Phase 2 — Modular refactors (5–7 days)
- **B8.** Split `app-server-protocol/v2.rs` into `v2/{...}.rs` modules.
- **B7.** Refactor `codex_message_processor.rs` into per-domain processors.
- **B2.** Modularize `connectors/`.
- **B10.** Audit `codex-protocol`; restore needed modules; keep our additions.

### Phase 3 — Plugins / skills / hooks unification (5–7 days)
- **B5.** Adopt `core-plugins` + `plugin` crates; migrate ATA plugin code.
- **B4.** Adopt `core-skills`; reroute custom-category installer (A11) through it.
- **B6.** Add missing hook events + `output_spill.rs` + `config_rules.rs` + `/hooks` TUI.

### Phase 4 — Auth & MCP (5–7 days)
- **B1.** Adopt upstream `login/src/auth/`; move multi-provider auth (A4.2) and Supabase (A5) into it.
- **B9.** Adopt `codex-mcp` + `builtin-mcps`; verify Guardian elicitations.
- **B14.** Add `chatgpt/workspace_settings.rs` and `responses-api-proxy/dump.rs`.

### Phase 5 — TUI (7–10 days)
- **B11.7.** Decide on app.rs baseline; rebase event handling.
- **B11.1–11.9.** Reconcile slash commands; layer voice / mobile / reading-view / account / research-tools / reverse-search on top of upstream baseline.
- Adopt new TUI features (Vim mode, `/ide`, workspace `/diff`, `/keymap debug`, status-line PR/branch, raw scrollback).

### Phase 6 — Core consolidation (5–7 days)
- **B12.** Reconcile agent/control, rollout, compact, client, turn, sandboxing.
- Adopt service-tier propagation, reasoning metadata.
- Verify our `unified_exec` vs upstream `exec-server`; choose strategy.

### Phase 7 — Restoration of ATA features (3–5 days)
- Verify A1–A13 still build and pass tests.
- Re-register `codex-research-tools`, `codex-data-tools`, `reading-view-server`, `treesitter`, `codex-elevenlabs`, `lsp-client`, `scheduler`, `codex-workspace`, `package-manager` workspace members.
- Add ATA `WireApi::AnthropicMessages`/`GeminiGenerate` variants to upstream's enum.
- Reapply branding (`@a2a-ai/ata`, ATA test recipes, AGENTS.md sections).

### Phase 8 — Verification (3–5 days)
- Full build with all features (`--features data,treesitter,research,ata-plus`).
- Smoke-test research tools (paper_search, zotero, hackernews, patents, github).
- Smoke-test reading view (TUI + browser); voice mode (record, karaoke, TTS, STT); figure extraction; mobile pairing.
- Verify Supabase + ChatGPT auth flows; test multi-provider creds.
- Run full test/snapshot suite; regenerate snapshots where appropriate.

**Total rough estimate:** 30–50 engineer-days, with most risk concentrated in Phase 2 (B7), Phase 3 (B5), Phase 4 (B1), and Phase 5 (B11.7).

---

## Appendix — Per-area finding files

For full detail, see `merge_info/agent_findings/`:

1. `01_research_data_tools.md` — Research/data tools, KB skills, document-reader handler, figure extraction, treesitter (485 lines).
2. `02_reading_view_voice.md` — Reading-view server, voice mode, TTS/STT, figure extraction, Supabase auth (600 lines).
3. `03_model_providers.md` — Anthropic/Gemini/OpenAI/OSS, models manager, model picker, providers (495 lines).
4. `04_tui_divergence.md` — TUI: voice, reading view, mobile, slash commands, app.rs refactor (640 lines).
5. `05_core_crate.md` — codex.rs, agent guards, memories, config_loader, MCP mgr, code_mode, etc. (640 lines).
6. `06_auth_login_secrets.md` — Auth manager, Supabase, device-key, network-proxy (138 lines).
7. `07_skills_hooks.md` — Skills/hooks crates, custom categories, plugins (483 lines).
8. `08_mcp_appserver_protocol.md` — MCP, app-server, protocol v2, message processor, lsp-client (425 lines).
9. `09_scheduler_workspace_sdk.md` — Scheduler, codex-workspace, package-manager, SDKs, branding (367 lines).
10. `10_upstream_new_features.md` — Comprehensive catalog of 39 upstream-new crates, reverts, TUI/core changes (792 lines).
