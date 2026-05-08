## Core/Protocol Analysis (Agent 2)

This analysis covers `codex-rs/core/`, `codex-rs/protocol/`, `codex-rs/exec/`, `codex-rs/cli/`, `codex-rs/app-server/`, and `codex-rs/app-server-protocol/` between upstream `rust-v0.129.0` and local `merge_upstream_0.129.0` HEAD.

The diff is enormous (1,574 files, 210k insertions / 228k deletions across these crates). Upstream has aggressively split functionality into many small workspace crates (`codex-mcp`, `codex-rollout`, `codex-models-manager`, `codex-features`, `codex-login`, `codex-sandboxing`, `codex-context`, `codex-session`, `codex-tools`, `codex-thread-store`, `codex-utils-output-truncation`, `codex-utils-template`, `codex-utils-path`, `codex-utils-plugins`, `codex-feedback`) while the fork keeps a more monolithic `core` crate with these features as inline modules.

### Local-only features (preserve / re-apply)

#### 1. Research subsystem (paper / patent / Zotero / Hacker News tooling)
- **Type**: Local-only.
- **Description**: Multi-tool research agent for academic search, citation management, patent search, and Hacker News scraping. Backed by a separate `codex-research-tools` crate; exposes `paper_search`, `patent_search`, `zotero_*`, `hn_*` tool families and a researcher subagent prompt.
- **Implementation**:
  - `codex-rs/core/src/research/{mod.rs, prompt.rs, researcher_prompt.rs, tool_names.rs, output_schema.rs, types.rs}`.
  - Tool handler: `codex-rs/core/src/tools/handlers/research.rs` (uses `codex_research_tools::ResearchToolkit`).
  - Templates: `codex-rs/core/templates/research/researcher_system_prompt.md`, `zotero_developer_instructions.md`.
  - CLI: `codex-rs/cli/src/research.rs`, `cli/src/zotero_cmd.rs`, plus tests.
- **Merge plan**: Preserve verbatim — no upstream equivalent.

#### 2. Reading view / document_reader protocol + tool
- **Type**: Local-only.
- **Description**: `present_reading_view`, `update_document_section`, `append_to_section`, `add_document_section`, `patch_document_section` agent tools that cache markdown documents and stream sectioned reading-view updates to the TUI. Includes `ReadingViewDisplayMode` (TUI vs Browser).
- **Implementation**:
  - Protocol: `codex-rs/protocol/src/document_reader.rs`.
  - 5 new EventMsg variants in `codex-rs/protocol/src/protocol.rs`: `PresentDocument`, `UpdateDocumentSection`, `AppendDocumentSection`, `AddDocumentSection`, `PatchDocumentSection`.
  - Tool handler: `codex-rs/core/src/tools/handlers/document_reader.rs`.
  - TS schema files under `app-server-protocol/schema/typescript/PresentDocumentEvent.ts` etc.
- **Merge plan**: Preserve.

#### 3. PDF figure cropping (`crop_figure` tool)
- **Type**: Local-only.
- **Description**: Renders a region of a PDF page to a PNG via pdfium and attaches it as an image content item.
- **Implementation**: `codex-rs/core/src/tools/handlers/crop_figure.rs`, `core/src/tools/pdfium_downloader.rs`, `core/src/tools/url_downloader.rs`, `url_validation.rs`.
- **Merge plan**: Preserve.

#### 4. JS REPL tool (`js_repl`)
- **Type**: Local-only.
- **Description**: In-process JavaScript runtime for agent code execution.
- **Implementation**: `codex-rs/core/src/tools/handlers/js_repl.rs`, `core/src/tools/js_repl/{mod.rs, kernel.js, meriyah.umd.min.js}`.
- **Merge plan**: Preserve.

#### 5. LSP and TreeSitter "code intelligence" tools
- **Type**: Local-only (cfg-gated under features `lsp` and `treesitter`).
- **Description**: Language-server-driven code intelligence and tree-sitter-driven `code_intel` tool.
- **Implementation**: Tool handlers `core/src/tools/handlers/{code_intel.rs, lsp.rs, lsp/code_actions.rs, lsp/formatting.rs, lsp/fuzz.rs, lsp/symbol_lookup.rs, lsp_workspace_edit.rs}`. State `core/src/state/multi_root.rs`. External crates: `codex-lsp-client`, `codex-treesitter`.
- **Merge plan**: Preserve.

#### 6. Memories subsystem (startup memory pipeline, phase1/phase2)
- **Type**: Local-only.
- **Description**: Two-phase startup memory pipeline.
- **Implementation**: `codex-rs/core/src/memories/{mod.rs,start.rs,phase1.rs,phase2.rs,storage.rs,citations.rs,control.rs,prompts.rs,usage.rs}`. Templates: `core/templates/memories/`. Protocol Ops: `Op::DropMemories`, `Op::UpdateMemories`. Test suite: `core/tests/suite/memories.rs`.
- **Merge plan**: Preserve.

#### 7. Plus / coordination scaffolding (cfg `ata-plus`, `relay`)
- **Type**: Local-only.
- **Description**: Trait-based coordination provider plumbing exposed through a no-op default in the public release.
- **Implementation**: `codex-rs/core/src/plus_provider.rs`, `plus_context.rs`. Tool handler: `core/src/tools/handlers/plus_tool.rs` (cfg `relay`).
- **Merge plan**: Preserve.

#### 8. Auth providers (Anthropic, Gemini OAuth, Google) — multi-provider auth
- **Type**: Local-only structure.
- **Description**: Pluggable provider registry for Anthropic, Google/Gemini API keys, and Gemini OAuth.
- **Implementation**: `codex-rs/core/src/auth.rs`, `auth/gemini_oauth.rs`, `auth/gemini_revoke.rs`, `auth/providers.rs`, `auth/providers/{env,status,storage_ops,types}.rs`, `auth/refresh.rs`, `auth/storage.rs`. Test suite: `core/tests/suite/auth_refresh.rs`.
- **Merge plan**: Preserve.

#### 9. Multi-provider model client (Anthropic, Gemini, Gemini Code Assist)
- **Type**: Local-only.
- **Implementation**: `codex-rs/core/src/client/{anthropic.rs,gemini.rs,gemini_code_assist.rs,provider_streaming.rs}` and `codex-rs/core/src/api_bridge.rs`.
- **Merge plan**: Preserve.

#### 10. Embedded WebSocket app-server (mobile/remote-control)
- **Type**: Local-only.
- **Description**: WebSocket transport for the existing app-server JSON-RPC protocol so the ATA Swift mobile app can drive an existing TUI session.
- **Implementation**: `codex-rs/app-server/src/embedded.rs` (`run_embedded_websocket`). `codex-rs/app-server/src/device_registration.rs`. `codex-rs/cli/src/mobile_cmd.rs`. Tests: `app-server/tests/test_embedded_ws.rs`.
- **Merge plan**: Preserve.

#### 11. Workspaces / multi-root knowledge base
- **Type**: Local-only.
- **Implementation**: `codex-rs/core/src/workspace_kb.rs`. `codex-rs/core/src/state/multi_root.rs`. CLI tests: `cli/tests/workspace_search_commands.rs`.
- **Merge plan**: Preserve.

#### 12. Undo / ghost-snapshot tasks
- **Type**: Local-only (`Op::Undo`, `EventMsg::UndoStarted`, `EventMsg::UndoCompleted`).
- **Description**: A dedicated `SessionTask` that takes a "ghost commit" snapshot at turn boundaries via `codex-git`.
- **Implementation**: `codex-rs/core/src/tasks/ghost_snapshot.rs`, `tasks/undo.rs`. Protocol: `Op::Undo` and `EventMsg::UndoStarted/UndoCompleted`. Test suite: `core/tests/suite/undo.rs`.
- **Merge plan**: Preserve.

#### 13. URL file attachment & recovery (`attach_url_files`)
- **Type**: Local-only.
- **Implementation**: Tool: `core/src/tools/handlers/attach_url_files.rs`, supporting `tools/url_downloader.rs`, `tools/url_validation.rs`. File attachments: `core/src/codex/file_attachments.rs`. Recovery: `core/src/codex/url_file_recovery.rs`. Tests: `core/tests/suite/url_file_rejection.rs`.
- **Merge plan**: Preserve.

#### 14. Search-tool extras and tool suggester
- **Type**: Local-only.
- **Implementation**: `core/src/tools/handlers/{search_tool_bm25.rs,tool_suggest.rs,tool_suggest_tests.rs}` plus template `core/templates/search_tool/tool_suggest_description.md`.
- **Merge plan**: Preserve.

#### 15. Code-mode JS bridge / runtime
- **Type**: Local-only.
- **Implementation**: `core/src/tools/code_mode/{bridge.js,runner.cjs,protocol.rs,process.rs,service.rs,worker.rs,description.md,wait_description.md}` and `core/src/tools/code_mode_description.rs`.
- **Merge plan**: Preserve.

#### 16. Plugins / curated repo / marketplace + toggles
- **Type**: Local-only.
- **Implementation**: `core/src/plugins/{manager.rs,manifest.rs,marketplace.rs,curated_repo.rs,store.rs,toggles.rs}` plus `_tests`.
- **Merge plan**: Hybrid — upstream restructured plugins; rebase ATA additions.

#### 17. Local Anthropic / OpenAI / Gemini file attachments
- **Type**: Local-only.
- **Implementation**: `core/src/codex/file_attachments/*` consuming `codex_api::file_support::*`.
- **Merge plan**: Preserve.

#### 18. Voice / TTS / realtime additions
- **Type**: Local-only delta on a shared upstream realtime feature.
- **Implementation**: `codex-rs/protocol/src/prompts/realtime/realtime_start.md`, `realtime_end.md`.
- **Merge plan**: Preserve.

#### 19. Skills extensions (env-var dependencies, remote skills, render layer)
- **Type**: Local-only delta on upstream's `core/src/skills.rs`.
- **Description**: Where upstream has a single `skills.rs`, the fork has a full `core/src/skills/` directory: `env_var_dependencies.rs`, `injection.rs`, `invocation_utils.rs`, `loader.rs`, `manager.rs`, `model.rs`, `permissions.rs`, `remote.rs` (Hazelnut/ChatGPT-shared), `render.rs`, `system.rs`. Adds protocol Ops `Op::ListRemoteSkills`, `Op::DownloadRemoteSkill`, and EventMsgs `ListRemoteSkillsResponse`, `RemoteSkillDownloaded`.
- **Merge plan**: Preserve.

#### 20. Models manager
- **Type**: Local-only structure (upstream has `codex-models-manager` workspace crate).
- **Implementation**: `codex-rs/core/src/models_manager/{cache.rs,manager.rs,model_info.rs,model_presets.rs,collaboration_mode_presets.rs,mod.rs}` plus `core/models.json`, `third_party_models.json`.
- **Merge plan**: Hybrid.

#### 21. Supabase client (cloud sync, auth)
- **Type**: Local-only (private only; per CLAUDE.md must NOT go to release branch).
- **Implementation**: `codex-rs/core/src/supabase/{auth.rs,client.rs,error.rs,mod.rs,session.rs,types.rs}`.
- **Merge plan**: Keep on private branch only; `just sync-release` strips it.

#### 22. Analytics client
- **Type**: Local-only.
- **Implementation**: `codex-rs/core/src/analytics_client.rs` plus tests.
- **Merge plan**: Hybrid.

#### 23. Custom prompts module
- **Type**: Local-only delta. Upstream's protocol does not have `custom_prompts.rs`.
- **Implementation**: `codex-rs/protocol/src/custom_prompts.rs`, `core/src/custom_prompts.rs`, plus protocol Op/EventMsg variants.
- **Merge plan**: Preserve.

#### 24. Message history protocol type
- **Type**: Local-only.
- **Description**: `HistoryEntry { conversation_id, ts, text }` used by `Op::AddToHistory` / `Op::GetHistoryEntryRequest` / `EventMsg::GetHistoryEntryResponse`.
- **Implementation**: `codex-rs/protocol/src/message_history.rs`, `core/src/message_history.rs`.
- **Merge plan**: Preserve.

#### 25. Permissions prompts (markdown templates)
- **Type**: Local-only.
- **Description**: Upstream moved permission prompt markdown out of `core/src/context/prompts/permissions/` and into `protocol/src/prompts/permissions/`. The fork adds `on_request_rule_request_permission.md` and `on_request_rule.md`.
- **Merge plan**: Preserve.

#### 26. Config loader (layered, with macOS specifics)
- **Type**: Local-only structure.
- **Implementation**: `codex-rs/core/src/config_loader/{layer_io.rs,macos.rs,mod.rs,README.md}`.
- **Merge plan**: Preserve.

#### 27. Tools handlers: `data` (Kaggle, datasets), `artifacts`, `multi_agents`
- **Type**: Mixed.
- **Description**: `data` and `artifacts` are local-only; `multi_agents` is shared but redesigned upstream as `multi_agents_v2`.
- **Merge plan**: Preserve `data` and `artifacts`. For `multi_agents`, take upstream's v2.

#### 28. Discoverable tools framework (`tools/discoverable.rs`)
- **Type**: Local-only.
- **Implementation**: `codex-rs/core/src/tools/discoverable.rs`, `tools/file_injection.rs`.
- **Merge plan**: Preserve.

### Shared / both-implemented (switch to upstream)

#### 29. Multi-agents / agent-jobs
- **Merge plan**: Switch to upstream's v2.

#### 30. Hooks / hook-runtime
- **Description**: Upstream's `HookEventName` is broader (`PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`, `SessionStart`, `Stop`); local has only `SessionStart` and `Stop`.
- **Merge plan**: Switch to upstream's broader enum.

#### 31. Compact / context compaction
- **Description**: Upstream additionally has `compact_remote_v2.rs` (fork removed it).
- **Merge plan**: Switch to upstream `compact.rs` and `compact_remote_v2.rs`.

#### 32. Realtime conversation streaming
- **Description**: Upstream adds `Op::RealtimeConversationListVoices` + `EventMsg::RealtimeConversationListVoicesResponse` + `RealtimeConversationSdpEvent`.
- **Merge plan**: Switch to upstream.

#### 33. Token data / rate limit / model verification
- **Description**: Upstream adds `EventMsg::ModelVerification(ModelVerificationEvent)` not present in fork.
- **Merge plan**: Switch to upstream.

#### 34. Patch apply update event
- **Description**: Upstream adds `EventMsg::PatchApplyUpdated(PatchApplyUpdatedEvent)`.
- **Merge plan**: Adopt from upstream.

#### 35. Thread goal / `ThreadGoalUpdatedEvent`
- **Description**: Upstream introduced thread-level goals (`core/src/goals.rs`, `EventMsg::ThreadGoalUpdated`, `core/src/tools/handlers/goal.rs`). Fork uses different `SetThreadName` Op + `ThreadNameUpdated` event.
- **Merge plan**: Adopt upstream's goal model where it does not conflict.

#### 36. Permissions framework
- **Description**: Upstream rewrote `protocol/src/permissions.rs` with a new `PermissionProfile` model.
- **Merge plan**: Adopt upstream's new permission model.

#### 37. Sandboxing / seatbelt / Windows sandbox
- **Description**: Upstream extracted `codex-sandboxing` workspace crate.
- **Merge plan**: Switch to upstream's `codex-sandboxing` crate.

#### 38. Rollout / session index / truncation
- **Description**: Upstream extracted to `codex-rollout` and `codex-rollout-trace` crates.
- **Merge plan**: Switch to upstream `codex-rollout` crate.

#### 39. Guardian (assessment / approval review)
- **Description**: Upstream removed several `GuardianAssessment*` re-exports.
- **Merge plan**: Switch to upstream's slimmer guardian re-exports.

#### 40. Op `UserInputWithTurnContext` (upstream-only)
- **Description**: Upstream added `Op::UserInputWithTurnContext` (a fused user-input + override-turn-context op). Fork still uses two-step.
- **Merge plan**: Adopt upstream Op variant.

#### 41. Op `UserInput.environments` and `responsesapi_client_metadata`
- **Description**: Upstream's `Op::UserInput` carries optional `environments: Vec<TurnEnvironmentSelection>` and `responsesapi_client_metadata: HashMap<String,String>`.
- **Merge plan**: Adopt upstream's extended Op shape.

#### 42. Service tier / `ServiceTier` enum
- **Merge plan**: Adopt upstream typed enum.

#### 43. App-server: `codex_message_processor` directory
- **Merge plan**: Take upstream restructure, re-apply the helper files.

#### 44. App-server: `config_api`, `external_agent_config_api`, `fs_api` (local-only additions)
- **Type**: Local-only.
- **Implementation**: `codex-rs/app-server/src/{config_api.rs,external_agent_config_api.rs,fs_api.rs}`.
- **Merge plan**: Preserve.

#### 45. Exec processor rewrite (event_processor_with_human_output / jsonl)
- **Description**: Both have but upstream restructured. Fork has +1797 lines diff in `event_processor_with_human_output.rs`.
- **Merge plan**: Switch to upstream's structure; re-apply ATA-only event handling.

#### 46. Exec CLI rewrite
- **Description**: Upstream rewrote `exec/src/cli.rs` and `exec/src/lib.rs`.
- **Merge plan**: Adopt upstream's exec CLI.

### Summary of merge strategy

- **Adopt from upstream**: split crates, the new `Op::UserInputWithTurnContext`, broader `HookEventName`, `RealtimeConversationListVoices`, `ModelVerification`/`PatchApplyUpdated`/`ThreadGoalUpdated`/`RealtimeConversationSdp` events, the `multi_agents_v2` redesign, the new permissions/profile model, and goal/`ThreadGoal` plumbing.
- **Preserve fork-only**: research subsystem, document_reader/reading view, crop_figure/pdfium, JS REPL, LSP/code-intel, memories, plus_provider, multi-provider auth, Anthropic/Gemini clients, embedded WS app-server + mobile CLI, workspace_kb / multi-root, undo / ghost-snapshot, URL-file attachments + recovery, search-tool BM25 + tool_suggest, code-mode JS bridge, plugins curated/marketplace/toggles, local file_attachments, realtime voice prompts, skills directory/remote skills, models_manager directory, supabase (private only), analytics_client, custom_prompts protocol module, HistoryEntry, extra permission prompt markdown, config_loader directory, data/artifacts tool handlers, tools/discoverable.rs, app-server config_api/external_agent_config_api/fs_api.
- **Re-apply on top of upstream rewrites**: exec event processors and exec CLI, `codex_message_processor` helpers, guardian re-exports, compact remote v2.
- **Protocol additions to re-add to `protocol/src/lib.rs`**: `pub mod custom_prompts;`, `pub mod document_reader;`, `pub mod message_history;` plus the Op variants (`DropMemories`, `UpdateMemories`, `Undo`, `ListSkills`, `ListRemoteSkills`, `DownloadRemoteSkill`, `SetThreadName`, `RunUserShellCommand`, `ListModels`) and EventMsg variants.
