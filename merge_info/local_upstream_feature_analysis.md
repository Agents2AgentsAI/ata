# Local vs Upstream (`rust-v0.129.0`) Feature Analysis

**Branch:** `merge_upstream_0.129.0`
**Upstream tag:** `rust-v0.129.0` (SHA `734b6c9cb0c77143b955da3b5ebc84e1959081b8`)
**Merge base:** `926b2f19e8c2a4c01b3a87bccd8ef8a1c23b22ab`
**Generated:** 2026-05-08 by 15 parallel analysis agents (10 wave-1 + 5 wave-2)

This document is the consolidated catalogue of every divergence between our fork at HEAD and upstream `rust-v0.129.0`. For each feature it records:
- **Type** — `Local-only`, `Shared` (both implemented; upstream and fork diverged), or `Upstream-new` (upstream added; fork should adopt)
- **Description** — what the feature does
- **Implementation** — key files, types, and call sites
- **Merge plan** — how this should be reconciled when we land the upstream merge

The per-agent raw reports are kept under `merge_info/_agent{1..15}_*.md` for reference. This document deduplicates and groups by area.

---

## How to read this document

1. **Type taxonomy:**
   - **Local-only**: feature exists only in our fork. Action is always "preserve and reapply on top of upstream after the merge."
   - **Shared**: both forks implement an overlapping feature, often with diverging structure. Action is usually "switch to upstream's structure, then layer our specifics on top" — to reduce future merge cost.
   - **Upstream-new**: upstream added a feature/refactor we don't yet have. Action is "adopt from upstream" unless the upstream feature is product-incompatible with ATA's positioning (e.g. `local_chatgpt_auth.rs`, V8/code-mode crates).

2. **Mass strategy:** the dominant pattern at upstream `rust-v0.129.0` is that `core/` was exploded into ~30 small workspace crates (`codex-mcp`, `codex-rollout`, `codex-features`, `codex-models-manager`, `codex-sandboxing`, etc.). Most "Shared (upstream rewrote)" entries below are this refactor. Long-term we should adopt as many of those as possible to keep the fork on the upstream architectural rail.

3. **Private vs public:** Some local features (Supabase auth, coordination/relay, mobile remote-control, `skills/src/assets/remote-exec/`) are private and must NOT reach the public release branch. The Justfile `_release_mixed_files` list and `just sync-release` enforce that — see `codex-rs/CLAUDE.md` for the contract.

---

## 1. New crates we own (Local-only, preserve verbatim)

These crates do not exist upstream. Each is a standalone workspace member; merge cost is limited to keeping their entries in `codex-rs/Cargo.toml`.

| Crate | Purpose | Key files | Notes |
|---|---|---|---|
| `codex-research-tools` | Multi-source academic search (Semantic Scholar, arXiv, OpenAlex, Hacker News, Patents, GitHub repo analysis) and Zotero integration with ~25 mutation/read tools. | `src/clients/{semantic_scholar,arxiv,openalex,hackernews,patents,github,zotero,epo_auth}.rs`; `src/tools/{paper_search,hackernews,patents,repo_analysis,zotero/}.rs`; `src/tool_specs.rs`; `src/lib.rs` (`ResearchToolkit`) | Wired into core via `core/src/tools/handlers/research.rs`. Gated by `Feature::ResearchPaperSearch` / `Zotero` / `HackerNews` / `Patents` / `RepoAnalysis`. |
| `codex-data-tools` | Dataset discovery and download (HuggingFace, Kaggle datasets + competitions). | `src/clients/{huggingface,kaggle}.rs`; `src/tools/dataset_ops.rs`; cargo features `huggingface`, `kaggle`. | Wired via `core/src/tools/handlers/data.rs`. Gated by `Feature::Data` and cargo feature `data`. |
| `codex-elevenlabs` | ElevenLabs streaming TTS (WebSocket, PCM 24 kHz, character-level alignment) + STT (`scribe_v1`). | `src/{tts,stt,types,error}.rs`; `tests/record_fixtures.rs` | Sole consumer is voice mode in TUI. |
| `codex-reading-view-server` | Local Axum HTTP+WebSocket server hosting `LivingReadingView.html` for the browser-mode reading view. | `src/lib.rs`; `src/assets/LivingReadingView.html` (3885 lines). | Used by `tui/src/chatwidget_document_reader.rs::ensure_reading_view_server`. |
| `codex-treesitter` | Tree-sitter project index, symbol tables, callers/tests/variables, multi-language grep, chunking, dual-storage annotations. 7 language packs. | `src/{annotations,chunking,content,ops,parser,project_index,symbol,symbol_table,walker}.rs`; `src/queries/{rust,python,typescript,javascript,go,java,scala}.rs` | Wired via `core/src/tools/handlers/code_intel.rs`. Gated by `Feature::TreeSitter`, cargo feature `treesitter`. |
| `codex-lsp-client` | Standalone LSP client (zero codex deps): JSON-RPC over child stdio, server registry, ~25 builtin language configs, root discovery. | `src/{client,jsonrpc,server_config,server_registry,builtin_servers,language,root_discovery,config_merge,error}.rs` (~4.7 KLOC) | Wired via `core/src/tools/handlers/lsp.rs`. Gated by `Feature::Lsp`. |
| `codex-scheduler` | Background job daemon: cron/interval/file-watch/http-poll/webhook triggers, sqlite-backed run/job repos, concurrency engine, pause/resume, run history, `search-commands`. | `src/{cli,job,engine,storage,daemon,trigger}/`; `migrations/001_init.sql` (~3.3 KLOC) | Wired via `cli/src/main.rs:161,164` (`Subcommand::Jobs`, `Subcommand::Scheduler`). |
| `codex-codex-workspace` | Multi-repo workspace manager with ~28 subcommands (init/list/select/repo-pin/clone/audit/recipe/run-locked/etc.), fine-grained locking, JSON manifest, repo allow-list. | `src/commands/*.rs` (28 files); `src/{audit,git,lock,manifest,paths,recipes,resolve,workspace_resolution}.rs` (~6.4 KLOC) | Wired via `cli/src/main.rs:167-168` (`Subcommand::Workspace` alias `ws`). |
| `codex-artifacts` | Artifact build/render runtime — installs JS-based artifact runtimes via package-manager, executes `build`/`render` (PowerPoint via `PresentationRenderTarget`, Spreadsheet, etc.). | `src/client.rs`; `src/runtime/{manager,installed,manifest,js_runtime,error}.rs` | Wired via `core/src/tools/handlers/artifacts.rs`. Gated by `Feature::Artifact`. |
| `codex-package-manager` | Generic versioned-archive installer: SHA-256 + size validation, zip/tar.gz extraction (rejects symlinks/devices), atomic staging→promotion with `fd_lock`. | `src/{archive,config,manager,package,platform}.rs` | Currently consumed only by `codex-artifacts`. |
| `codex-test-macros` | `#[large_stack_test]` proc-macro for tests needing a 16 MiB stack. | `src/lib.rs` | Trivial; keep workspace registration. |
| `codex-utils-git` (replaces upstream `git-utils`) | Ghost-commit snapshot/restore, apply_git_patch, merge-base helpers, platform-specific symlink. | `utils/git/src/{lib,apply,branch,errors,ghost_commits,operations,platform}.rs` | Used by Undo / GhostSnapshot tasks. |
| `codex-utils-file` | Small file-helpers crate. | `utils/file/src/{lib,error}.rs` | New ATA crate, no upstream equivalent. |
| `shell-tool-mcp` (root, TS) | Fork-only TypeScript MCP server packaged via tsup, with own bash/zsh patches and dedicated CI/release pipelines. | `shell-tool-mcp/{package.json,src/,tests/,patches/,tsup.config.ts}` | Renamed `@a2a-ai/ata-shell-tool-mcp` (was `@openai/codex-shell-tool-mcp` upstream). |
| `tools/argument-comment-lint` | Dylint plugin (cdylib using `clippy_utils`+`dylint_linting`) driven by `just argument-comment-lint`. | `tools/argument-comment-lint/{Cargo.toml,src/,run.sh}` | Replaces upstream's Bazel-driven version. |

**Merge plan (entire section):** preserve all entries in `codex-rs/Cargo.toml` workspace `members` and path-deps. After upstream merge, validate `cargo build --workspace` and `cargo nextest run -p <crate>` for each.

---

## 2. Local-only fork features layered into shared crates

These are fork additions that live inside crates upstream also has, so each one is a "preserve & reapply" task on top of any upstream rewrite of the surrounding file.

### 2.1 Reading view subsystem

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.1.1 | `present_reading_view` / `update_document_section` / `append_to_section` / `add_document_section` / `patch_document_section` agent tools (5 tools) | Local-only | `core/src/tools/handlers/document_reader.rs` (~1450 LOC); `protocol/src/document_reader.rs` types; 5 new `EventMsg` variants in `protocol/src/protocol.rs` (`PresentDocument`, `UpdateDocumentSection`, `AppendDocumentSection`, `AddDocumentSection`, `PatchDocumentSection`); registered in `core/src/tools/spec.rs:2207-2222` when `Feature::ReadingView` is on. | Reapply protocol additions and the spec.rs `if config.features.enabled(Feature::ReadingView)` block. |
| 2.1.2 | TUI `DocumentReaderView` (sectioned reader) | Local-only | `tui/src/bottom_pane/document_reader/{mod.rs,render.rs}`; `bottom_pane/document_reader_ext.rs` (`include!`'d into `bottom_pane/mod.rs`); `tui/src/chatwidget_document_reader.rs` (1399 lines, `include!`'d into `chatwidget.rs`); `chatwidget.rs:1604,5931` dispatches `PresentDocument`. Closed-document dedup at `bottom_pane/mod.rs:207, 1027, 1045`. | Preserve `include!` pattern — it's the explicit isolation barrier. |
| 2.1.3 | Browser-mode reading view (server + HTML) | Local-only | `codex-reading-view-server` crate (see §1); `tui/src/app_event.rs:391-399` events `ReadingViewServerStarted`, `ReadingViewBrowserMessage`, `ReadingViewModeChanged`; `ReadingViewMode` enum (`Tui`/`Browser`/`Disabled`). | Reapply event-enum arms and dispatch handlers. |
| 2.1.4 | Crop-and-store-figure tool (PDF → PNG) | Local-only | `core/src/tools/handlers/crop_figure.rs`; `core/src/tools/{pdfium_downloader,url_downloader,url_validation}.rs`; spec at `core/src/tools/spec/workspace.rs::create_crop_and_store_figure_tool`. Auto-downloads pdfium dylib into `~/.ata/lib/`. | Reapply alongside reading-view registration. |
| 2.1.5 | Attach URL files (`attach_url_files` tool + recovery) | Local-only | `core/src/tools/handlers/attach_url_files.rs`; `core/src/codex/url_file_recovery.rs`; `core/src/codex/file_attachments/`. Per-turn limit; cache in `cache_entry_dir`. Tests at `core/tests/suite/url_file_rejection.rs`. | Reapply via `register_attach_url_files` call (`spec.rs:2430`). |

### 2.2 Voice mode + TTS karaoke

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.2.1 | Full voice-mode state machine (`mic → STT → agent → TTS → speaker`, `<voice>`/`<eq>` tags, karaoke, prefetch) | Local-only | `tui/src/chatwidget/voice_mode.rs` (6572 LOC); `VoiceModePhase`, `VoiceModeState`, `VoiceTagParser`, `SentenceBuffer`, alignment timeline (`AlignmentEntry`, `build_alignment_entries`, `find_active_word`, `repair_timeline_monotonicity`), VOICE_MODE_INSTRUCTION agent prompts. | Preserve verbatim. |
| 2.2.2 | Voice config TOML (`[voice_mode]`, `[voice_mode.elevenlabs]`) + `ConfigEdit` helpers | Local-only | `core/src/config/types.rs:957-1031` (`VoiceModeToml`, `ElevenLabsToml`, `VoiceVerbosity`, `VoiceOutput`); `core/src/config/edit.rs:72-141` (8 setter helpers). | Reapply onto whichever struct upstream renames `ConfigToml` to (likely `codex-config::config_toml`). |
| 2.2.3 | `Feature::VoiceMode` and `Feature::VoiceTranscription` | Local-only | `core/src/features.rs` (see §6). | Keep. |
| 2.2.4 | Voice-input cargo feature gate (`voice-input = ["dep:cpal", "dep:hound"]`) + Linux exclusion + no-op fallback | Local-only | `tui/Cargo.toml:16,21,118-120,142`; `tui/src/lib.rs:73-102, 159-277`. | Reapply in `tui/Cargo.toml`. |
| 2.2.5 | RMS-based VAD with TTS-suppression multiplier | Local-only | `tui/src/vad.rs` (312 LOC). | Preserve as-is. |
| 2.2.6 | OpenAI `gpt-4o-mini-transcribe` STT path + WAV encoders + push-to-talk capture | Shared (skeleton) + local extensions | `tui/src/voice.rs` (1198 LOC vs 486 upstream): `VoiceCapture::start()`, `encode_wav_for_voice_mode`, `transcribe_async`, plus extra methods on `RealtimeAudioPlayer` (`enqueue_pcm`, `seek_to_sample`, `seek_to_ms`, `set_playback_speed`, `pause`, `resume`, `is_paused`, `playback_position_ms`, `reset_playback_position`). | Mixed file: re-apply local methods on top of upstream's `start_realtime` skeleton. |
| 2.2.7 | `/voice` and `/voice-setup` slash commands | Local-only | `tui/src/slash_command.rs:62-64,108-109,196-197`; `tui/src/bottom_pane/voice_setup_view.rs` (882 LOC); `app.rs:3890-3940` `UpdateVoiceSettings` handler. | Reapply slash-command enum arms and bottom-pane wiring. |
| 2.2.8 | ~25 voice `AppEvent` variants (TtsAudioChunk + alignment, MeterTick, HighlightTick, NarrateSection, PrefetchSection, InterruptTts, PauseTts, ResumeTts, PlaybackSpeedChange, TranscriptionComplete/Failed, etc.) | Local-only | `tui/src/app_event.rs:220-598`; dispatched in `app.rs:4025-4048`. | Re-apply on `AppEvent` enum. |
| 2.2.9 | `<voice>`/`<eq>` tag stripping in text formatting (with HTML-attr-aware parser, `latex_to_plain_text`) | Local-only | `tui/src/text_formatting.rs:967-1457` (+1102 LOC). Comprehensive tests at lines 1286-1457. | Preserve. |
| 2.2.10 | Reading-view + voice integration: auto-narration, karaoke, prefetch adjacent sections, pause/resume/interrupt | Local-only | Wired through `chatwidget_document_reader.rs`, `bottom_pane/document_reader/render.rs` (strips `<voice>` tags before rendering, displays voice-status footer), and the browser HTML's `tts-playing`/`ttsStateChanged`. | Preserve. |
| 2.2.11 | TTS e2e + sync test harness | Local-only | `tui/tests/{tts_e2e,tts_sync_report,karaoke_integration}.rs`; `tui/tests/support/{recorded_tts,sync_oracle}.rs`; fixtures `tts_short.json`, `tts_medium.json`. | Preserve. Rename script: `run-reading-view-tests.sh`. |

### 2.3 Auth + multi-provider

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.3.1 | `AuthMode::Ata` (Supabase email-OTP) + `AtaAuth` variant on `CodexAuth` + separate `~/.ata/ata_session.json` | Local-only (private) | `core/src/supabase/{auth,client,session,types,error,mod}.rs` (~825 LOC); `login/src/supabase_auth.rs` (600 LOC, `send_ata_otp`, `verify_ata_otp`, `supabase_device_code_login`, port 1455 `/auth/callback`); `app-server-protocol/src/protocol/common.rs:44`; `otel/src/lib.rs:42`; `core/src/auth.rs:101-114`. `AtaAccountConfig` at `core/src/config/types.rs:1151`. | Preserve & reapply. **Private only — must NOT reach release branch** per `codex-rs/CLAUDE.md`. |
| 2.3.2 | `ata login --a2a` and `ata login --ata-only` CLI flags | Local-only | `cli/src/main.rs:311 (--a2a)`, `337 (--ata-only)`; `cli/src/login.rs:423 (run_login_with_a2a)`, `480`, `534`. | Reapply. |
| 2.3.3 | TUI ATA OTP onboarding (multi-step state machine) + `bottom_pane/account_view.rs` (`/account`) | Local-only (private bits) | `tui/src/onboarding/auth.rs` (~+1200 LOC additions: `AtaOtpInputState`, `spawn_ata_send_otp`, `spawn_ata_verify_otp`, render fns); `tui/src/bottom_pane/account_view.rs` (517 LOC). | Preserve & reapply. Heavy conflicts on `onboarding/auth.rs` expected. |
| 2.3.4 | Multi-provider credential map (OpenAI / Anthropic / Gemini), `ProviderCredential` enum | Local-only | `core/src/auth/providers/types.rs` (243 LOC: `ProviderCredential`, `ProviderAuthSource`, `ProviderAuthMethod`, `ProviderAuthStatus`, `GeminiAuthSource`, constants `PROVIDER_OPENAI/ANTHROPIC/GEMINI`, `ANTHROPIC_API_KEY_ENV_VAR`, `GOOGLE_API_KEY_ENV_VAR`); `auth/providers/storage_ops.rs` (147 LOC); `auth/providers/{status,env}.rs`; `auth/providers.rs` (807 LOC, `AuthDotJson` v2 migration). CLI `--with-api-key` and `--provider {openai\|anthropic\|gemini}`. TUI `provider_picker.rs` (369 LOC). | Preserve. |
| 2.3.5 | Gemini OAuth (Code Assist) + Anthropic API-key auth | Local-only | `login/src/gemini_server.rs` (518 LOC); `core/src/auth/gemini_oauth.rs` (987 LOC); `core/src/auth/gemini_revoke.rs` (88 LOC). Default client id and token URL configurable via env vars. Refresh skew 300s. | Preserve. |
| 2.3.6 | Multi-provider model client (Anthropic Messages API, Gemini GenerateContent, Gemini Code Assist) | Local-only | `core/src/client/{anthropic,gemini,gemini_code_assist,provider_streaming}.rs`; `core/src/api_bridge.rs`. | Preserve. |
| 2.3.7 | `chatgpt_token` global cache (replaces upstream's per-call `AuthManager`) | Local-only refactor | `chatgpt/src/chatgpt_token.rs` (36 LOC: global `RwLock<Option<TokenData>>`, `init_chatgpt_token_from_auth`); `chatgpt/src/connectors.rs` rewritten. | Likely **adopt upstream's design** for `connectors.rs` after verifying `AuthMode::Ata` still works. |
| 2.3.8 | ATA-branded keyring service (`KEYRING_SERVICE = "ata"`) | Local-only | `secrets/src/lib.rs:21`; `rmcp-client/src/oauth.rs:56` (`"Ata MCP Credentials"`). | Preserve (single-line override per file). |
| 2.3.9 | Inlined `get_git_repo_root` in `secrets` (drops `codex-git-utils` dep) | Local-only | `secrets/Cargo.toml`; `secrets/src/lib.rs:165-178`. | Reapply. |
| 2.3.10 | ATA-branded device-code login messages | Local-only | `login/src/device_code_auth.rs:85,151`. | Reapply branding. |

### 2.4 Research / data subagent prompts and infrastructure

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.4.1 | Research subagent prompt construction (multi-phase, availability-driven) | Local-only | `core/src/research/{prompt.rs (build_research_prompt), researcher_prompt.rs (RESEARCHER_SYSTEM_PROMPT), tool_names.rs (find_mcp_tool_matches, configured_native_tool_context, native_tool_availability), output_schema.rs, types.rs, mod.rs}`. Templates: `core/templates/research/{researcher_system_prompt,zotero_developer_instructions}.md`. | Preserve. |
| 2.4.2 | `ResearchToolkit` / `DataToolkit` per-thread lifecycle | Local-only | `core/src/thread_manager.rs:164-166, :711, :733` (`OnceCell<Arc<SharedResearchToolkit>>`); `core/src/codex.rs:393-394` (`research_toolkit`/`data_toolkit` on `TurnContext`); `core/src/tools/router.rs:103,163`; `core/src/tools/spec.rs:2555,2746`. | Heavy conflicts on every upstream merge — reapply field additions. |
| 2.4.3 | Tool-name aliasing (MCP ↔ native: `search_papers` ↔ `paper_search`) | Local-only | `core/src/research/tool_names.rs` (`ResearchToolNames::from_native`, `from_resolved`, `resolve_tool_alias`); `core/src/data/tool_names.rs`. | Self-contained. |
| 2.4.4 | Skill bundles for research (`paper-discoverer`, `paper-synthesis`, `cross-paper-report`, `conversation-report`, `hn-discoverer`, `hn-synthesis`, `kb`, `research-briefing`, `zotero`) | Local-only | `skills/src/assets/research/` (11 skill dirs) + `samples/job-manager/` + `workspace/` + `adapt-environment/`. Embedded via `skills/build.rs` + `include_dir!`. | Per-skill triage; merge upstream sample updates (e.g. `samples/openai-docs/`) but keep all `research/*`, `adapt-environment/`, `workspace/`. |

### 2.5 Coordination, scheduler, mobile

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.5.1 | `Plus` provider scaffolding (cfg `ata-plus`, `relay`) — trait + no-op default + `team_post`-style messaging tool | Local-only (private) | `core/src/plus_provider.rs`, `plus_context.rs` (cfg `ata-plus`); `core/src/tools/handlers/plus_tool.rs` (cfg `relay`). | Preserve trait; private coordination crate stays out of release branch. |
| 2.5.2 | `--worker`, `--lead-session-id`, `--workspace`, `--progress-cursor` exec flags | Local-only (worker + lead-session-id are private/relay) | `exec/src/cli.rs:90-124`. | **Audit**: `--worker`/`--lead-session-id` should be `#[cfg(feature = "relay")]` per CLAUDE.md but currently unconditional; either gate or add to Justfile `_release_mixed_files`. |
| 2.5.3 | `/team` slash command (private) | Local-only (private) | `tui/src/slash_command.rs`; dispatch via `chatwidget.rs:10405` to `ata_plus::team_ui`. | Audit private-leak risk; same plan as 2.5.2. |
| 2.5.4 | `/jobs` slash command + `job-manager` skill | Local-only | `tui/src/chatwidget.rs:5050,5065,5235`; `skills/src/assets/samples/job-manager/SKILL.md` (+189 lines). | Preserve. Sandbox-writable-roots widening for scheduler dirs (commit `2cb7d19e9b`) is non-obvious. |
| 2.5.5 | `/mobile` slash command + `bottom_pane/mobile_setup_view.rs` (QR pairing) | Local-only | `tui/src/{remote_control.rs (224), remote_discovery.rs (94), mobile_daemon.rs (238), qr_render.rs (59)}`; `cli/src/mobile_cmd.rs` (367 lines, `#[cfg(not(windows))]`); `cli/src/main.rs:156-158`; deps `gethostname`, `mdns-sd`, `qrcode`. | Preserve. |
| 2.5.6 | Embedded WebSocket app-server + Supabase device registration (heartbeat) | Local-only (private) | `app-server/src/embedded.rs` (385 LOC, `run_embedded_websocket`, `EmbeddedWebSocketConfig`); `app-server/src/device_registration.rs` (432 LOC, PATCH `last_seen_at` every 30s, JWT refresh 5min before expiry); `tui/src/remote_control.rs:157,204`; tests at `app-server/tests/test_embedded_ws.rs`. | Preserve. Must stay listed in Justfile `_release_mixed_files`. |
| 2.5.7 | `ata jobs` / `ata scheduler` / `ata workspace` (alias `ws`) / `ata zotero` / `ata mobile` / `ata plus` CLI subcommands | Local-only | `cli/src/main.rs:156-174`; `cli/src/{zotero_cmd.rs (2614 lines), mobile_cmd.rs (367)}`; new tests `cli/tests/{jobs_scheduler_search_commands.rs, workspace_search_commands.rs, zotero_search_commands.rs}`. | Preserve. |
| 2.5.8 | `ata debug dump-initial-context` subcommand | Local-only | `cli/src/main.rs:199-201`. | Preserve. Upstream's `debug models`/`prompt-input`/`trace-reduce` were dropped locally — verify intent. |

### 2.6 Memories, undo, custom prompts, history

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.6.1 | Two-phase startup memories pipeline (extractor LLM per-rollout, then global consolidation) | Local-only | `core/src/memories/{mod,start,phase1,phase2,storage,citations,control,prompts,usage}.rs`; templates `core/templates/memories/{stage_one_input,stage_one_system,consolidation,read_path}.md`; protocol `Op::DropMemories`, `Op::UpdateMemories`. Tests `core/tests/suite/memories.rs`. | Preserve. |
| 2.6.2 | Undo / ghost-snapshot tasks (`Op::Undo`, `EventMsg::UndoStarted/UndoCompleted`) | Local-only | `core/src/tasks/{ghost_snapshot,undo}.rs`; `protocol/src/protocol.rs` Op + Event variants; tests `core/tests/suite/undo.rs`. | Preserve. |
| 2.6.3 | `protocol::custom_prompts` module + `Op::ListCustomPrompts` + `EventMsg::ListCustomPromptsResponse` | Local-only | `protocol/src/custom_prompts.rs`; `core/src/custom_prompts.rs`. | Re-add `pub mod custom_prompts;` to `protocol/src/lib.rs`. |
| 2.6.4 | `protocol::message_history::HistoryEntry { conversation_id, ts, text }` + `Op::AddToHistory`/`GetHistoryEntryRequest` + `EventMsg::GetHistoryEntryResponse` | Local-only | `protocol/src/message_history.rs`; `core/src/message_history.rs`. | Reapply. |
| 2.6.5 | Skills extensions: env-var dependencies, remote skills, render layer | Local-only delta on upstream's `core/src/skills.rs` | Local has full `core/src/skills/` directory: `env_var_dependencies.rs`, `injection.rs`, `invocation_utils.rs`, `loader.rs`, `manager.rs`, `model.rs`, `permissions.rs`, `remote.rs`, `render.rs`, `system.rs`. Adds `Op::ListRemoteSkills`, `Op::DownloadRemoteSkill`, `EventMsg::ListRemoteSkillsResponse`, `RemoteSkillDownloaded`. `skills/src/lib.rs` is 428 LOC vs upstream 169 LOC, exposing 9 public fns including `system_cache_root_dir`, `install_research_skills`, `install_workspace_skills`, `install_custom_skills`. | Preserve directory + protocol additions. |

### 2.7 Sandbox & process hardening

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.7.1 | `vendored_bwrap` FFI entrypoint (links bubblewrap C sources directly into the Rust binary, dispatches via `vendored_bwrap_available`) | Local-only | `linux-sandbox/src/vendored_bwrap.rs`; `linux-sandbox/src/lib.rs`; `linux-sandbox/src/linux_run_main.rs` (`exec_vendored_bwrap`). | Decide: keep as a third bwrap backend OR migrate to upstream's new `bundled_bwrap` model. |
| 2.7.2 | `RLIMIT_NOFILE` raise in `pre_main_hardening` | Local-only | `process-hardening/src/lib.rs::raise_file_descriptor_limit()`. Raise to hard limit on Linux/BSD/macOS — necessary because subagents each open ~15-25 FDs and macOS default soft limit (256) causes EMFILE panics. | Preserve verbatim. |
| 2.7.3 | `disable_process_dumping()` retained (Linux: `prctl(PR_SET_DUMPABLE, 0)`) | Local-only (deletion-conflict) | `process-hardening/src/lib.rs`; `linux-sandbox/src/proxy_routing.rs:617` (`harden_bridge_process`). Upstream **deleted** this fn. | Audit whether the extra ptrace-attach hardening is worth keeping. |
| 2.7.4 | ATA-prefixed Windows sandbox binaries (`ata-windows-sandbox-setup`, `ata-command-runner`) + `.ata` workspace marker dir | Local-only | `windows-sandbox-rs/Cargo.toml` (`[[bin]] name = "ata-..."`); `.ata` paths throughout `windows-sandbox-rs/src/`. | Re-apply rename on top of upstream's restructure. |
| 2.7.5 | Stripped Windows sandbox Cargo deps (likely accidental drift) | Local-only delta | `windows-sandbox-rs/Cargo.toml` removed `codex-utils-pty`, `codex-otel`, `glob`, `tokio`; forced `edition = "2021"`; removed `[lints] workspace = true`. | Re-add `edition.workspace = true`, `[lints] workspace = true`. Re-add deps as needed by adopted modules. |
| 2.7.6 | `WindowsSandboxModeToml` + `windows_sandbox_mode`/`windows_sandbox_private_desktop` config fields | Local-only | `core/src/config/types.rs:34-48`; `core/src/config/mod.rs:221-223`. | Preserve. |
| 2.7.7 | `network-proxy/src/admin.rs` debug HTTP API (currently **orphaned**) | Local-only — dead code | `network-proxy/src/admin.rs` (+181 LOC). NOT declared in `network-proxy/src/lib.rs`. | Either wire up (`mod admin;` + `run_admin_api*` exports) or delete. |

### 2.8 TUI surfaces wave-1 cataloged

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.8.1 | `/research` slash command + `bottom_pane/research_tools_view.rs` (toggle popup with tri-state ReadingView) | Local-only | `tui/src/bottom_pane/research_tools_view.rs` — `RESEARCH_FEATURES` table. Wired via `bottom_pane/mod.rs:96, 167-168`; `chatwidget.rs:237, 7974`. | Preserve. |
| 2.8.2 | `/personality` slash command (communication-style picker) | Local-only | `tui/src/slash_command.rs`. | Preserve. |
| 2.8.3 | `/apps`, `/account`, `/collab`, `/agent`/`/subagents` slash commands | Local-only | `tui/src/slash_command.rs`. | Preserve. |
| 2.8.4 | `--remote-control`, `--remote-control-port`, `--remote-control-token` CLI flags | Local-only | `tui/src/cli.rs:115-125`. | Reapply. |
| 2.8.5 | `--web-search`/`--search` legacy alias (`web_search="live"`) | Local-only | `tui/src/lib.rs:384-388`. | Reapply. |
| 2.8.6 | `chatwidget/{agent,realtime,session_header,skills,voice_mode,interrupts}.rs` split files (different from upstream's split) | Local-only structure | `tui/src/chatwidget/` directory. | Defer; coupled to AppServer migration (see §5). |

### 2.9 App-server & schemas (private endpoints)

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.9.1 | `app-server/src/{config_api,external_agent_config_api,fs_api}.rs` thin RPC implementations | Shared protocol, ATA-only impl | Wire types are upstream-defined in `protocol/common.rs:952-980`; impl is fork-only. | Switch to upstream's `request_processors/{config,external_agent_config,fs}_processor.rs` after adopting the request-processor split. |
| 2.9.2 | TS schema additions for ATA features (132 files) | Local-only | `app-server-protocol/schema/typescript/{AddDocumentSectionEvent,CollabAgentInteractionBeginEvent,Personality,RealtimeVoice,...}.ts`. Plus hand-written `serde_json/JsonValue.ts` (no upstream equivalent). | Regenerate after protocol merge; preserve `JsonValue.ts`. |
| 2.9.3 | Python SDK `v2_types.py` extensions (+25 lines) | Local-only | `sdk/python/src/codex_app_server/generated/v2_types.py`. | Re-run `update_sdk_artifacts.py` after merge. |
| 2.9.4 | TypeScript SDK `Codex` → `Ata` rebrand (`codex.ts` → `ata.ts`, `CodexOptions` → `AtaOptions`, `CodexExec` → `AtaExec`) | Local-only | `sdk/typescript/src/{ata.ts, ataOptions.ts, exec.ts, thread.ts, index.ts, events.ts}`; tests `tests/ataExecSpy.ts`. | Trivial mechanical rename. |

### 2.10 Build & dev infra

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.10.1 | RunsOn self-hosted runner config | Local-only | `.github/runs-on.yml` (custom AMI `windows25-vstoolchain-x64`, spot/on-demand split). | Preserve. |
| 2.10.2 | `keyword-scan.yml`, `shell-tool-mcp.yml`, `shell-tool-mcp-ci.yml`, `ci.bazelrc` | Local-only | `.github/workflows/`. | Preserve. |
| 2.10.3 | `rust-release.yml` rewrite (RunsOn, drops Apple signing as TODO, drops app-server bundle) | Shared (rebrand) | `.github/workflows/rust-release.yml`. | Re-apply rebrand after each upstream sync. |
| 2.10.4 | Bazel toolchain (slimmed, no V8) | Local-only | `MODULE.bazel` (185 lines vs 531), `defs.bzl` (265 vs 544), `.bazelrc` (62 vs 197); `patches/` (3 patches: `aws-lc-sys_memcmp_check.patch`, `toolchains_llvm_bootstrapped_resource_dir.patch`, `windows-link.patch` removed). | Keep current minimal Bazel; do NOT pull in V8/rusty-v8/coreaudio annotations. |
| 2.10.5 | Justfile recipes: `test-reading-view`, `test-karaoke`, `test-tts-live`, `test-tts-sync`, `write-hooks-schema`, `argument-comment-lint`, `verify-openai-model-override`, `prompts`, `check-prompts`, `dump-context`, `sync-release` | Local-only | `justfile`. | Preserve. `sync-release` encodes the public/private split. |
| 2.10.6 | `codex-cli` package rebrand (`@a2a-ai/ata`, binary `ata`/`bin/ata.js`), Dockerfile, `build_container.sh`, install scripts (`scripts/install/install.sh`/`.ps1`) | Local-only | `codex-cli/`, `scripts/install/`, `scripts/install.sh`. | Preserve. |
| 2.10.7 | dotslash configuration rebrand | Shared | `.github/dotslash-config.json`. | Preserve fork's rebranding. |
| 2.10.8 | `pnpm-workspace.yaml` policy hardening removed (drops `strictDepBuilds`, `trustPolicy`) | Local-only | `pnpm-workspace.yaml`. | Consider re-adding upstream's trust-policy hardening. |
| 2.10.9 | `UPSTREAM.md` provenance ledger | Local-only | Top-level `UPSTREAM.md`. | Add row for `rust-v0.129.0` sync. |
| 2.10.10 | `tools/argument-comment-lint` Dylint plugin | Local-only | `tools/argument-comment-lint/{Cargo.toml,src/,run.sh,README.md,rust-toolchain}`. | Preserve. |
| 2.10.11 | `third_party/` minimal: `meriyah` (license only), `wezterm` (license only); upstream's `v8` vendor tree dropped | Local-only | `third_party/`. | Continue ignoring upstream V8 vendor. |

### 2.11 Docs/prompts

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.11.1 | `codex-rs/AGENTS.md` (build/test iteration strategy) | Local-only | 111 lines. | Keep verbatim. |
| 2.11.2 | `codex-rs/CLAUDE.md` (public/private separation playbook) | Local-only | 46 lines. | Keep. |
| 2.11.3 | `codex-rs/ata-research-explainer.md` (orphaned persona blurb) | Local-only | 5-line marketing blurb. | **Investigate**: not wired via `include_str!`. Either wire in, move to `docs/`, or delete. |
| 2.11.4 | `codex-rs/core/templates/collaboration_mode/{default,execute,pair_programming,plan}.md` (replaces upstream `templates/personalities/`, `collab/`, `compact/`) | Local-only | 4-mode collaboration system using `<collaboration_mode>` tagging. | Keep. Verify upstream `personalities/`, `collab/`, `compact/` directories stay deleted post-merge. |
| 2.11.5 | `codex-rs/core/templates/research/{researcher_system_prompt,zotero_developer_instructions}.md` | Local-only | Loaded via `include_str!` in `core/src/research/researcher_prompt.rs`. | Keep. |
| 2.11.6 | `codex-rs/core/templates/tools/presentation_artifact.md` (PowerPoint tool description) | Local-only | 200 lines. | Keep. |
| 2.11.7 | `codex-rs/core/templates/search_tool/tool_suggest_description.md` (replaces upstream `request_plugin_install_description.md`) | Local-only | Companion to upstream-shared `tool_description.md`. | Keep. |
| 2.11.8 | `codex-rs/core/src/tools/code_mode/{description,wait_description}.md` | Local-only | `include_str!`'d in code-mode tool plumbing. | Keep. |
| 2.11.9 | Codex → Ata rebrand of all 6+ system prompts | Shared (rebrand) | `codex-rs/core/{prompt,gpt_5_codex_prompt,gpt_5_1_prompt,gpt_5_2_prompt,gpt-5.1-codex-max_prompt,gpt-5.2-codex_prompt,prompt_with_apply_patch_instructions}.md`; `protocol/src/prompts/base_instructions/default.md`. | Re-apply via sed-style script. |
| 2.11.10 | `core/templates/agents/orchestrator.md` (+64 lines fork-only persona content: tone/style, sub-agent flow, AGENTS.md handling, planning, file-path linking) | Shared (large fork divergence) | `core/templates/agents/orchestrator.md`. | Merge — preserve all fork-only blocks. |
| 2.11.11 | Local-only fork docs (~17 docs, ~2500 lines) — `docs/{paper-search-setup,patent-search-setup,zotero-setup,lsp-treesitter-setup,COORDINATION_SETUP,js_repl,exit-confirmation-prompt-design,browser-automation-findings,tui-alternate-screen,tui-chat-composer,tui-request-user-input,tui-stream-chunking-{review,tuning,validation},superpowers/plans/2026-03-20-{alignment-driven-karaoke,tts-karaoke-sync-test},prompts}.md`; plus `codex-rs/docs/superpowers/{plans,specs}/2026-03-18-prompt-inspector*.md`; READMEs at `codex-rs/{artifacts,package-manager,tools/prompt-inspector,tools/rollout-analyzer,utils/git}/README.md` | Local-only | `docs/`, `codex-rs/docs/`. | Keep all. |
| 2.11.12 | `docs/` Codex→Ata rebrand (`developers.openai.com/codex/...` → `github.com/Agents2AgentsAI/ata/blob/main/docs/...`, `~/.codex/` → `~/.ata/`) | Shared (rebrand) | All shared docs. | Re-apply rebrand sweep after each upstream sync. |
| 2.11.13 | `announcement_tip.toml` ATA-specific tip (regex `0.0.x..0.119.x`, expires 2026-05-08 — already expired today) | Shared (replaced) | `announcement_tip.toml`. | Update or remove the expired tip; do NOT merge upstream's now-stale Codex announcements back in. |

### 2.12 ATA-only RPC & telemetry

| # | Feature | Type | Implementation | Merge plan |
|---|---|---|---|---|
| 2.12.1 | OTLP `OtelProvider` (`SdkLoggerProvider`, `SdkTracerProvider`, `MetricsClient`, `OpenTelemetryTracingBridge`, distributed-tracing `TRACEPARENT`/`TRACESTATE` env parsing) + `traces/otel_manager.rs` + `TelemetryAuthMode::Ata` | Shared (heavy) | `otel/src/otel_provider.rs` (~430 lines, NEW); `otel/src/traces/{mod,otel_manager}.rs` (NEW). Removed: upstream `From<AuthMode>` for `TelemetryAuthMode`, `AutomatedReviewer` `ToolDecisionSource`, Statsig globals. | Keep our `OtelProvider`/`otel_manager`; merge upstream metric/event additions piecemeal. |
| 2.12.2 | Telemetry disabled by default + OpenAI endpoint removed | Local-only | Multiple commits in cluster 15 (`a4070902c3`, `b14e9619d0`). | Audit upstream merge for any new telemetry hooks. |

---

## 3. Shared features both forks have (typically: switch-to-upstream)

These features exist in both, often with structural divergence. The recommendation in nearly every case is to adopt upstream's structure to reduce future merge cost, then layer ATA-specific extensions on top.

### 3.1 Crate-split refactors (largest single class of "Upstream-new")

Upstream extracted ~30 crates from `core/`. Each is "Shared semantically, but live in different crates":

| Upstream crate | Local equivalent | Recommendation |
|---|---|---|
| `codex-mcp` (`codex-rs/codex-mcp/`) | `core/src/mcp/` directory + `core/src/mcp_connection_manager.rs` + `mcp_tool_call.rs` + `mcp_tool_approval_templates.rs` | Adopt upstream crate; relocate our `mcp/{auth,skill_dependencies}.rs` into it. **Highest-impact divergence-reduction.** |
| `codex-builtin-mcps` + `codex-memories-mcp` | Missing locally | Adopt verbatim. |
| `codex-rollout` + `codex-rollout-trace` | Local has rollout logic inside `core/src/codex/rollout_reconstruction.rs` and `state/` | Adopt upstream crate. |
| `codex-models-manager` | `core/src/models_manager/` directory | Adopt upstream crate API; bridge `third_party_models`/`collaboration_mode_presets` on top. |
| `codex-features` | `core/src/features.rs` | Adopt upstream crate (see §6). |
| `codex-login` (heavily restructured) | `core/src/auth.rs` + `core/src/auth/` | **Long-term**: pull auth back into `login/src/auth/`. Single biggest source of conflict against upstream auth churn. |
| `codex-sandboxing` + `codex-bwrap` | `linux-sandbox/` + `windows-sandbox-rs/` | Adopt upstream crate; map vendored_bwrap into the new entry point. |
| `codex-config` (new modules: `config_toml`, `loader/`, `mcp_edit`, `mcp_types`, `marketplace_edit`, `permissions_toml`, `plugin_edit`, `profile_toml`, `project_root_markers`, `schema`, `skills_config`, `thread_config`, `tui_keymap`, `types`, `hook_config`, `host_name`, `key_aliases`) | `config/src/lib.rs` (58 lines stub) + `core/src/config/` (1194 lines locally) | Adopt all modules; shim `core/src/config/` re-exports through `codex-config`. |
| `codex-tools` (`codex_tools::ConfiguredToolSpec`, `DiscoverableTool`, `ToolName`, `ToolSpec`, `ToolsConfig`, `ResponsesApiNamespaceTool`, etc.) | `core/src/tools/` directory | **Name collision** with our local `codex-rs/tools/` developer-scripts dir — rename one. |
| `codex-state` migrations 0020–0030 + `state/runtime/{goals,remote_control,device_key}.rs` + `state/model/{thread_goal,graph}.rs` | Missing locally | Decide per-feature; re-number our future migrations starting at 0031+. |
| `codex-analytics` | `core/src/analytics_client.rs` | Switch where API matches; keep ATA endpoints layered on top. |
| `codex-feedback` (rebuilt with `OPENAI_BASE_URL` diagnostic) | `feedback/` (light edits) | Restore `[lints] workspace = true`. Adopt the `OPENAI_BASE_URL` diagnostic. |
| `codex-app-server-client` (slimmed; lost `LogDbLayer`/`StateDbHandle`/`EnvironmentManager`) | `app-server-client/src/lib.rs` (228 lines vs 1712 upstream) | Risky if our fork still depends on those exports. Search `cli/`, `exec/`, `core/`. |
| `codex-cloud-tasks-client::mock` (inlined `MockClient`) | Local has standalone `cloud-tasks-mock-client/` crate | Adopt upstream's `mock.rs`; drop our standalone mock crate. |

### 3.2 Protocol additions to re-add to `protocol/src/lib.rs`

After the merge, ensure these are present:

- `pub mod custom_prompts;`
- `pub mod document_reader;`
- `pub mod message_history;`

And these `Op` variants: `DropMemories`, `UpdateMemories`, `Undo`, `ListSkills`, `ListRemoteSkills`, `DownloadRemoteSkill`, `SetThreadName`, `RunUserShellCommand`, `ListModels`.

And these `EventMsg` variants: `PresentDocument`, `UpdateDocumentSection`, `AppendDocumentSection`, `AddDocumentSection`, `PatchDocumentSection`, `ListSkillsResponse`, `ListRemoteSkillsResponse`, `RemoteSkillDownloaded`, `SkillsUpdateAvailable`, `ThreadNameUpdated`, `UndoStarted`, `UndoCompleted`, `ListCustomPromptsResponse`, `AgentMessageDelta`, `AgentReasoningDelta`, `AgentReasoningRawContentDelta`.

### 3.3 Shared TUI surfaces requiring three-way merge

| # | Feature | Local | Upstream | Plan |
|---|---|---|---|---|
| 3.3.1 | `chatwidget.rs` | 10 518 lines | 11 210 lines, with new `CodexOpTarget` enum and app-server-protocol bridge replacing the `Arc<ThreadManager>` flow | Defer; coupled to AppServer migration. |
| 3.3.2 | `chatwidget/interrupts.rs` | 105 lines (per-event variants) | 245 lines (collapsed to `ItemStarted`/`ItemCompleted`) | Defer. |
| 3.3.3 | `bottom_pane/footer.rs` | 849-line diff | New `FooterKeyHints`, `GoalStatusIndicator`, `FooterMode::HistorySearch` | **Rebase**: take upstream's new shape; re-add ATA fields (`voice_mode_available`, `scheduler_enabled`, `mobile_available`, `research_enabled`, `context_window_*`). |
| 3.3.4 | `bottom_pane/chat_composer.rs` | 9878 lines, `include!`s `chat_composer_reverse_search.rs` | 10 468 lines + sibling `chat_composer/history_search.rs` | Pull upstream's `history_search.rs` as a sibling; keep local `reverse_search` and `SkillPopup`/voice integration. |
| 3.3.5 | `render/highlight.rs`, `render/renderable.rs`, `markdown_render.rs` | Minor local cosmetic drift | Pure upstream improvements (`foreground_style_for_scopes`, `cursor_style` trait method, list/code-block blank-line fix, URL-decode in markdown links) | **Take wholesale**; drop local dead-code helpers `push_ref`. |
| 3.3.6 | `chatwidget/{snapshots/}` | 85 snapshots | 174 snapshots | Adopt upstream's snapshot corpus; add fork-specific snapshots only where local widgets exist. |
| 3.3.7 | Modules upstream has that local should pull: `goal_menu/status/validation`, `hooks`, `ide_context`, `keymap_picker`, `mcp_startup`, `plan_implementation`, `plugins`, `reasoning_shortcuts`, `side`, `slash_dispatch`, `status_surfaces`, `user_messages`, `warnings` | Missing locally | Present | Pull additive modules (lower-coupling first: `slash_dispatch`, `user_messages`, `warnings`). Defer `keymap_picker`, `goal_*` until app-server flows arrive. |
| 3.3.8 | Modules local removed but upstream still has: `app_command.rs`, `goal_display.rs`, `history_cell/`, `keymap*.rs`, `local_chatgpt_auth.rs`, `terminal_title.rs`, `motion.rs`, `npm_registry.rs`, `permission_compat.rs`, `resize_reflow_cap.rs`, `resume_picker/`, `session_state.rs`, `terminal_probe.rs`, `transcript_reflow.rs`, `update_versions.rs`, `width.rs`, `workspace_command.rs` | Deleted | Present | Decide module-by-module after merge. Most look intentional; `keymap.rs`/`goal_display.rs`/`history_cell/` may want to come back. |

### 3.4 Other notable shared rewrites

| # | Feature | Type | Plan |
|---|---|---|---|
| 3.4.1 | `core/src/codex.rs` (7324 lines) → upstream's `core/src/session/{session,mod,turn_context,handlers,multi_agents,review,config_lock}.rs` rename | Shared (upstream rewrote) | **Wave-3 dedicated rename**. Until then, manually translate import paths. |
| 3.4.2 | `core/src/thread_manager.rs` (833 → 1449 lines) | Shared | Cherry-pick `TurnAbortReason`, `TurnAbortedEvent`, `SubAgentSource`, `ThreadSource`. |
| 3.4.3 | `core/src/tools/spec.rs` (2777 lines) → upstream `tools/spec_plan.rs` + `spec_plan_types.rs` + `hosted_spec.rs` | Shared | Keep local `tools/spec/{agent_jobs,integrations,javascript,workspace}.rs` subdir. Defer migration. |
| 3.4.4 | `core/src/tools/router.rs` (434 → 334 lines) | Shared | Keep research/data toolkit injection. Port upstream's `parallel_mcp_server_names`, `unavailable_called_tools`, `ToolName` (replacing `String`/`tool_namespace`), `deferred_mcp_tools`. |
| 3.4.5 | `protocol/v2.rs` (7825 lines monolithic) → upstream `protocol/v2/{account,apps,collaboration_mode,command_exec,config,device_key,experimental_feature,feedback,fs,hook,item,mcp,model,notification,permissions,plugin,process,realtime,review,shared,thread,thread_data,turn,windows_sandbox}.rs` (24 files) | Shared (upstream rewrote) | **Adopt upstream split.** Single largest source of merge friction in protocol crate. |
| 3.4.6 | `app-server/src/codex_message_processor.rs` (8763 lines monolithic) → upstream `app-server/src/request_processors/{account,apps,catalog,command_exec,config,device_key,external_agent_config,feedback,fs,git,initialize,marketplace,mcp,plugins,process_exec,search,thread_goal,thread_lifecycle,thread,thread_summary,token_usage_replay,turn,windows_sandbox}_processor.rs` | Shared (upstream rewrote) | **Adopt upstream split.** Highest-priority restructure. |
| 3.4.7 | `app-server-protocol/Cargo.toml` (drops `codex-shell-command`, adds `shlex`) | Local-only dep change (likely accidental) | Restore `codex-shell-command` when adopting `item_builders.rs`. |
| 3.4.8 | `export.rs` regressed `generate_internal_json_schema()` and `ScanState` improvements | Upstream-new (regressed locally) | Adopt upstream `export.rs` wholesale. |
| 3.4.9 | `app-server-test-client/src/lib.rs` (`author = "Ata"` rebrand; removed `device_code: bool`; added `ReadOnlyAccess::FullAccess` to all `SandboxPolicy::ReadOnly`) | Shared, ATA-style cosmetic | Trivial. Keep `author = "Ata"`; restore `device_code` flag if upstream still requires. |
| 3.4.10 | `codex-api` multi-provider rewrite (60 files, +8860/-3708): `ProviderAdapter` trait, `ProviderFactory`, providers/{openai,anthropic,gemini}, tools/{...}, sse/{anthropic,gemini}, file_support/* | Shared (heavy) | **Highest-risk shared diff outside core/TUI.** Take upstream skeleton; port multi-provider trait + adapters on top. |

### 3.5 Shared protocol features upstream extended that local should adopt

| # | Feature | Plan |
|---|---|---|
| 3.5.1 | Broader `HookEventName` (`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`, `SessionStart`, `Stop`) — local has only `SessionStart`/`Stop` | Adopt upstream's enum. |
| 3.5.2 | New realtime ops/events: `Op::RealtimeConversationListVoices`, `EventMsg::RealtimeConversationListVoicesResponse`, `RealtimeConversationSdpEvent` | Adopt. |
| 3.5.3 | `EventMsg::ModelVerification(ModelVerificationEvent)`, `EventMsg::PatchApplyUpdated(PatchApplyUpdatedEvent)`, `EventMsg::ThreadGoalUpdated` | Adopt. |
| 3.5.4 | `Op::UserInputWithTurnContext` (fused user-input + override-turn-context) | Adopt. |
| 3.5.5 | `Op::UserInput.environments: Vec<TurnEnvironmentSelection>` and `responsesapi_client_metadata: HashMap<String,String>` | Adopt. |
| 3.5.6 | Typed `ServiceTier` enum | Adopt. |
| 3.5.7 | New `McpServerConfig` fields: `experimental_environment`, `supports_parallel_tool_calls` (default true), `default_tools_approval_mode`, `tools: HashMap<String, McpServerToolConfig>` | Adopt all four. |
| 3.5.8 | `mcp_openai_file.rs` (Apps SDK file-upload bridge for `_meta["openai/fileParams"]`, 471 lines) | Adopt as-is. |
| 3.5.9 | `mcp_tool_exposure.rs` (`DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD = 100`, `Feature::ToolSearchAlwaysDeferMcpTools`) | Adopt. Critical with 100+ Apps SDK connectors. |
| 3.5.10 | `connectors/` split (`accessible.rs`, `filter.rs`, `merge.rs`, `metadata.rs`) | Take split verbatim; keep local `tier=categorized` query param. |
| 3.5.11 | Four new `rmcp-client` modules: `elicitation_client_service.rs`, `executor_process_transport.rs`, `http_client_adapter.rs`, `stdio_server_launcher.rs` | **Biggest single conflict-reduction win.** Adopt; refactor our `RmcpClient::new_stdio_client` to consume them. |
| 3.5.12 | `perform_oauth_login_silent` | Port the function. |
| 3.5.13 | `event_mapping.rs` (597 lines) + `item_builders.rs` (312 lines) protocol helpers | Pull both; restore `codex-shell-command` dep. |
| 3.5.14 | `serde_helpers.rs` + `common_tests.rs` protocol helpers | Take upstream's `common.rs` and re-add `common_tests.rs` `#[path]`. |
| 3.5.15 | `fs/watch` / `fs/unwatch` / `fs/changed` RPCs + `fs_watch.rs` (200ms debounced `FileWatcherEvent`) | Pull. Required for parity with v0.129.0 mobile/web SDKs. |
| 3.5.16 | `app-server-client/src/remote.rs` WebSocket transport (890 lines, `AppServerEvent`) | Pull. ATA already has the embedded server (§2.5.6); this gives us the matching client transport. |

### 3.6 Auth (additional shared/upstream-new items)

| # | Feature | Type | Plan |
|---|---|---|---|
| 3.6.1 | Upstream `external_bearer.rs` (subprocess-based bearer-token refresher) | Upstream-new | Adopt — relatively self-contained. |
| 3.6.2 | Upstream `auth_env_telemetry.rs` (telemetry of which auth env vars are set) | Upstream-new | Adopt; rename env-var list to include ATA equivalents (`ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`). |
| 3.6.3 | Upstream `agent_identity.rs` (short-lived JWT auth for agent processes) | Upstream-new | Optional — adopt if we want feature parity. |

---

## 4. Feature flag registry (`features.rs`)

Local's `core/src/features.rs` has 67 variants; upstream's standalone `codex-features` crate has 75. See agent-14 report for line-by-line.

### 4.1 Local-only flags to keep
`Research`, `ResearchPaperSearch`, `ResearchZotero`, `ResearchHackerNews`, `ResearchPatents`, `ResearchRepoAnalysis`, `ResearchKnowledgeBase`, `ReadingView`, `VoiceMode`, `VoiceTranscription`, `Lsp`, `TreeSitter`, `Coordination` (private), `Data`, `Scheduler`, `AppsMcpGateway`, `PowershellUtf8`. The carve-out `is_research_feature(...)` skips warnings for the research family.

### 4.2 Local-flag drift to reconcile
- `JsRepl` / `JsReplToolsOnly` / `ImageDetailOriginal`: upstream marks `Stage::Removed`; local still ships them — **keep local live**, do NOT adopt upstream's removal.
- `CodexHooks` (local key `codex_hooks`, `UnderDevelopment`, default off) → upstream `Hooks` (key `hooks`, `Stable`, default on) — **adopt upstream key+stage**, add legacy alias `codex_hooks → hooks`.
- `ShellSnapshot`: upstream `Stable` default-on — **adopt**.
- `Apps`: upstream `Stable` default-on but ATA's pipeline depends on ChatGPT auth — **likely keep ATA Experimental + default-off**.
- `GuardianApproval`: upstream `Stable` default-on — **adopt** but **keep ATA's menu copy**.
- `UseLegacyLandlock`: upstream `Deprecated` — **adopt**.

### 4.3 Upstream-new flags to adopt
Most need adopting if their gate logic is meaningful: `ToolSearch`, `UnavailableDummyTools`, `ToolSearchAlwaysDeferMcpTools`, `BuiltInMcp`, `MultiAgentV2` (with structured config `MultiAgentV2ConfigToml`), `EnableMcpApps`, `AppsMcpPathOverride` (structured config), `TerminalResizeReflow`, `ApplyPatchStreamingEvents`, `Goals`, `RemoteCompactionV2`, `WorkspaceDependencies`, `WorkspaceOwnerUsageNudge`, `ResponsesWebsocketResponseProcessed`, `ExternalMigration`, `AuthElicitation`, `PluginHooks`. Skip unless desired: `Chronicle`, `RemoteControl` (collides with ours), `RemotePlugin`, `InAppBrowser`, `BrowserUse(External)`, `ComputerUse`.

### 4.4 Reconciliation strategy
1. Introduce `codex-rs/features/` as a workspace member containing upstream's lib.
2. Re-export `pub use codex_features::*` from `codex_core::features`.
3. Move `apps_enabled`/`apps_enabled_cached` into `core::auth` (don't bring `AuthManager` import into the new `features` crate).
4. Append ATA-only variants in a small `core/src/features_ata.rs` extension.
5. Reject upstream `Removed` markers for `JsRepl`, `JsReplToolsOnly`, `ImageDetailOriginal`.
6. Adopt upstream stage promotions for `ShellSnapshot`, `GuardianApproval`, `Hooks` (rename).
7. Keep ATA's `is_research_feature` carve-out inside the local warning emitter.

This collapses the registry diff to ~15 ATA-only rows plus a handful of stage tweaks.

---

## 5. Things wave-1 might have miscategorized

The commit-archeology pass (Agent 12) flagged these features that look like ATA additions in the diff but are actually **upstream blob commits we imported**, not authored locally:

- Presentation/PowerPoint (`pptx`) tool (`presentation_artifact.md`)
- Supabase auth (Cluster 1 in our local commits)
- Ghost-commit / undo
- Package-manager skill
- Memories-clear command

If wave-1 catalogued these as "ATA features," they should be re-classified as "upstream-imported, possibly customized." Spot-check by `git log --follow <file>` whether the original commit was authored by Codex upstream or ATA before assuming we own them.

---

## 6. Recommended merge order (synthesizes all 15 agents)

**Phase 0 — preparation:**
1. Bump `UPSTREAM.md` with the new merge target.
2. Snapshot CI status pre-merge (we're at v0.3.3 → v0.129.0 — ~9 month upstream catch-up).

**Phase 1 — take wholesale (low risk, no fork features touched):**
3. `render/{highlight,renderable,markdown_render}.rs` (drop local dead-code `push_ref` helpers).
4. `execpolicy`/`execpolicy-legacy`, `shell-command` (drop our `powershell_parser.rs`), `shell-escalation` (rename `ResolvedPermissionProfile` → `Permissions`, restore `[lints] workspace = true` in shell-escalation `Cargo.toml`).
5. `connectors/` four-module split (keep local `tier=categorized` query param).
6. `feedback`'s `OPENAI_BASE_URL` diagnostic + restore `[lints] workspace = true`.
7. `cloud-tasks-client::mock` inline; drop our standalone `cloud-tasks-mock-client` crate.
8. `app-server-protocol/src/export.rs` wholesale.
9. `agent_identity.rs`, `external_bearer.rs`, `auth_env_telemetry.rs` (auth-side adopts).

**Phase 2 — adopt upstream crate splits (medium risk, big future-proofing):**
10. `codex-features` crate: introduce + re-export from `core::features`. Append our 17 ATA-only variants. Reject upstream's `Removed` for `JsRepl`/`JsReplToolsOnly`/`ImageDetailOriginal`.
11. `codex-config`: adopt all new modules. Shim `core/src/config/` re-exports through it. Re-add `voice_mode`, `reading_view`, `realtime_audio` onto the new `config_toml` location.
12. `codex-mcp` + `codex-builtin-mcps` + `codex-memories-mcp`: adopt; relocate our `core/src/mcp/{auth,skill_dependencies}.rs` into the new crate.
13. `codex-rollout` + `codex-rollout-trace`: adopt.
14. `codex-models-manager`: adopt; keep our `third_party_models`/`collaboration_mode_presets` extensions on top.
15. `codex-sandboxing` + `codex-bwrap`: adopt; reroute `vendored_bwrap.rs` as a third backend or migrate fully to `bundled_bwrap`.
16. `codex-tools` (the upstream crate, not our local `tools/` dir): adopt — **rename our `codex-rs/tools/` developer-scripts dir** to avoid collision. Adopt `tools/runtimes/` (apply_patch, shell with unix_escalation, unified_exec).
17. Restore `codex-shell-command` dep in `app-server-protocol/Cargo.toml`. Pull `event_mapping.rs` + `item_builders.rs` + `serde_helpers.rs` + `common_tests.rs`.
18. `state/` migrations 0020–0030 (adopt selectively; re-number our future migrations 0031+). Adopt `MailboxDeliveryPhase`, `RemovedTask`, `AnySessionTask`, `PendingRequestPermissions`, `AdditionalPermissionProfile`.

**Phase 3 — pull missing upstream modules:**
19. `mcp_openai_file.rs`, `mcp_tool_exposure.rs` in core.
20. New `rmcp-client` modules (`elicitation_client_service`, `executor_process_transport`, `http_client_adapter`, `stdio_server_launcher`); refactor our `RmcpClient::new_stdio_client`.
21. `perform_oauth_login_silent`.
22. `fs/watch` family + `app-server/src/fs_watch.rs`.
23. `app-server-client/src/remote.rs` (WebSocket transport).

**Phase 4 — adopt structural splits (large, mostly mechanical):**
24. `protocol/v2.rs` → upstream's 24-file split (single largest protocol-side win).
25. `app-server/src/codex_message_processor.rs` → upstream's `request_processors/` split. Re-host our `apps_list_helpers.rs` + `plugin_app_helpers.rs` into the matching processor files. Rewrite `config_api.rs` / `external_agent_config_api.rs` / `fs_api.rs` to use upstream's processors.
26. Re-run codegen for both Python (`update_sdk_artifacts.py`) and TS (`cargo run --bin export -- typescript ./schema/typescript`) schemas. Preserve `serde_json/JsonValue.ts`.

**Phase 5 — TUI rebases:**
27. `bottom_pane/footer.rs`: adopt upstream `FooterKeyHints`/`GoalStatusIndicator`/`HistorySearch`; re-add ATA fields.
28. Pull `chat_composer/history_search.rs` as new sibling.
29. Pull additive `chatwidget/` modules: `slash_dispatch.rs`, `user_messages.rs`, `warnings.rs` (lower coupling first).
30. Adopt upstream's broader `HookEventName` + new `Op`/`EventMsg` variants (`UserInputWithTurnContext`, `RealtimeConversationListVoices`, `ModelVerification`, `PatchApplyUpdated`, `ThreadGoalUpdated`, `RealtimeConversationSdpEvent`, `Op::UserInput.environments`, `responsesapi_client_metadata`, typed `ServiceTier`).
31. Adopt new `McpServerConfig` fields (`experimental_environment`, `supports_parallel_tool_calls`, `default_tools_approval_mode`, `tools` map).

**Phase 6 — large local-feature reapplications (high effort):**
32. Re-apply Codex → Ata system prompt rebrand (sed-style script).
33. Re-apply docs Codex → Ata rebrand (sed-style script).
34. Reapply all local-only research/data/voice/reading-view/scheduler/workspace/mobile feature additions on top.
35. Re-apply `core/src/features.rs` (or replacement layer per Phase 2 step 10) and the `is_research_feature` carve-out.
36. Re-apply `process-hardening` `raise_file_descriptor_limit`. Decide on `disable_process_dumping`.
37. Re-apply Windows sandbox `ata-`/`.ata` renames; restore `edition.workspace`/`[lints] workspace = true` in `Cargo.toml`.

**Phase 7 — high-risk shared rewrites (defer or schedule):**
38. `codex-api` multi-provider rebase (Phase 1 of dedicated wave): `cargo build -p codex-api`, port multi-provider trait + adapters onto upstream's removed `realtime_websocket/methods_common/v1/v2`. Keep `file_support` regardless.
39. `codex/` → `session/` rename (Wave-3 dedicated).
40. AppServer migration of TUI (Wave-3 dedicated): replace direct `Arc<ThreadManager>` + `next_event()` flow with `codex_app_server_protocol` / `CodexOpTarget` enum. Remove `chatwidget/agent.rs`. Rewrite `chatwidget/interrupts.rs` with `ItemStarted`/`ItemCompleted` shape.

**Phase 8 — final hygiene:**
41. Decide whether to wire up or delete `network-proxy/src/admin.rs`.
42. Decide whether to wire up or delete `cli/src/research.rs` (979-line orphan).
43. Audit `--worker`/`--lead-session-id` exec flags + `/team` slash command for `#[cfg(feature = "relay")]` gating.
44. Update `announcement_tip.toml` (current tip already expired 2026-05-08).
45. Move `codex-rs/ata-research-explainer.md` to `docs/` or wire it into a prompt.
46. Verify no upstream `Codex` branding leaked into agent-facing strings (`just check-prompts`).
47. Run `just test-research`, `just test-reading-view`, `just test-karaoke`, `just argument-comment-lint`, full nextest sweep.

---

## 7. Inventory: per-agent reports referenced

| Agent | Focus | Report |
|---|---|---|
| 1 | TUI | `merge_info/_agent1_tui.md` |
| 2 | Core / protocol | `merge_info/_agent2_core_protocol.md` |
| 3 | Research / data tools | `merge_info/_agent3_research_data.md` |
| 4 | Auth / login | `merge_info/_agent4_auth_login.md` |
| 5 | MCP | `merge_info/_agent5_mcp.md` |
| 6 | TTS / voice | `merge_info/_agent6_tts_voice.md` |
| 7 | CLI / exec | `merge_info/_agent7_cli_exec.md` |
| 8 | Build / infra | `merge_info/_agent8_build_infra.md` |
| 9 | Config / sandbox | `merge_info/_agent9_config_sandbox.md` |
| 10 | Docs / prompts | `merge_info/_agent10_docs_prompts.md` |
| 11 | Missed crates (gap-fill) | `merge_info/_agent11_missed_crates.md` |
| 12 | Commit-archeology | `merge_info/_agent12_commit_archeology.md` |
| 13 | App-server / schemas | `merge_info/_agent13_appserver_schemas.md` |
| 14 | Feature-flag registry | `merge_info/_agent14_features_registry.md` |
| 15 | Unclassified resolution + core cross-cuts | `merge_info/_agent15_unclassified_resolution.md` |

Always cross-reference these for line-by-line evidence before acting on the consolidated plan above.
