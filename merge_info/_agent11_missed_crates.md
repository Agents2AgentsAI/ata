## Wave-2 Gap-Fill: Missed Crates Analysis (Agent 11)

This pass examined crates wave-1 didn't (or only lightly) touch. Net: 6 substantial **local-only crates**, several **upstream-extracted crates** that wave-1 didn't flag, plus targeted change-sets in shared crates. The biggest signal is that upstream restructured `core/` into a swarm of new crates (`tools`, `rollout`, `sandboxing`, `memories`, `model-provider-info`, `realtime-webrtc`, `analytics`, etc.) that our fork has not adopted.

### 1. `codex-scheduler` (cron/event-driven job daemon)
- **Type**: Local-only.
- **Description**: Full job-scheduling subsystem with daemon lifecycle, sqlite-backed run/job repos, four trigger types (cron, interval, file-watch, http-poll, webhook listener via `tiny_http`), engine with concurrency limits, run history, pause/resume, and a `search-commands` clap-driven manual lookup. Jobs are TOML files in `~/.ata/jobs/` referencing skills or inline prompts. ~3.3 KLOC.
- **Implementation**: `codex-rs/scheduler/Cargo.toml`; `src/lib.rs`; `src/cli.rs` (JobsCli, SchedulerCli); `src/job/{definition,validation,loader,mod}.rs`; `src/engine/{scheduler,runner,concurrency,mod}.rs`; `src/storage/{db,jobs_repo,runs_repo,state_repo,mod}.rs`; `src/daemon/{mod,lifecycle}.rs`; `src/trigger/{cron_trigger,webhook,file_watch,http_poll,mod}.rs`; `migrations/001_init.sql`. Wired in `cli/src/main.rs:161,164,886,889`.
- **Merge plan**: Pure ATA addition; no upstream conflict.

### 2. `codex-artifacts` (artifact build/render runtime)
- **Type**: Local-only.
- **Description**: JS-based "artifact runtime" used by the artifacts tool handler. Handles release-locator URLs, package-manager-backed install into `~/.codex/packages/artifacts/`, runtime manifest validation, JS executable discovery, and command execution (`execute_build`, `execute_render`) with build/render targets (`PresentationRenderTarget`, `SpreadsheetRenderTarget`).
- **Implementation**: `codex-rs/artifacts/Cargo.toml`; `src/lib.rs`; `src/client.rs`; `src/runtime/{manager,installed,manifest,js_runtime,error,mod}.rs`. Tool handler: `core/src/tools/handlers/artifacts.rs`.
- **Merge plan**: No upstream collision. Watch tools refactor.

### 3. `codex-package-manager` (generic versioned-archive installer)
- **Type**: Local-only.
- **Description**: Reusable installer with `ManagedPackage` trait. Handles platform detection, manifest+archive fetch, SHA-256 + size validation, `.zip` and `.tar.gz` extraction, atomic staging→promotion with `fd_lock` cross-process locks, `resolve_cached()` vs `ensure_installed()` distinction.
- **Implementation**: `codex-rs/package-manager/Cargo.toml`; `src/{archive,config,error,manager,package,platform,tests}.rs`.
- **Merge plan**: Local-only.

### 4. `codex-workspace` (multi-repo research workspace manager)
- **Type**: Local-only.
- **Description**: Major ATA feature — workspace lifecycle CLI with ~28 subcommands. Uses fine-grained locking (`workspace`/`kb`/`run`/`index` levels), JSON manifest + audit log, repo host allow-list, worktree/copy/clone code materialization. ~6.4 KLOC.
- **Implementation**: `codex-rs/codex-workspace/Cargo.toml`; `src/lib.rs`; `src/commands/*.rs` (28 files); `src/{audit,error,git,lock,manifest,paths,recipes,resolve,selection,spec,types,url_validation,workspace_id,workspace_resolution}.rs`.
- **Merge plan**: Local-only.

### 5. `codex-lsp-client` (standalone LSP client lib)
- **Type**: Local-only.
- **Description**: Self-contained LSP client (zero codex deps). JSON-RPC transport over child stdio, server lifecycle, language→server registry with `phf` map, root-discovery, builtin server config for ~25 languages.
- **Implementation**: `codex-rs/lsp-client/Cargo.toml`; `src/{client,jsonrpc,server_config,server_registry,builtin_servers,language,root_discovery,config_merge,error}.rs`. ~4.7 KLOC.
- **Merge plan**: Pure-additive.

### 6. `codex-treesitter` (project index / symbols / chunking)
- **Type**: Local-only.
- **Description**: Full project-index engine on top of tree-sitter. Per-language queries (Rust, Python, TS, JS, Go, Java, Scala). `ProjectIndex` API exposes: `reindex_absolute_path`, `search_symbols`, `list_symbols`, `find_callers`, `find_tests`, `list_variables`, `implementation`, `define_symbol`, `define_file`, `mark_file`, `structure(depth)`, `peek`, `grep`, `chunk_indices`, `load_annotations`/`save_annotations`. ~3.6 KLOC.
- **Implementation**: `codex-rs/treesitter/Cargo.toml`; `src/lib.rs`; `src/{annotations,chunking,config,content,error,file_entry,file_tree,ops,parser,project_index,symbol,symbol_table,walker}.rs`; `src/queries/{rust,python,typescript,go,java,scala}.rs`.
- **Merge plan**: No conflict.

### 7. `codex-test-macros` (`#[large_stack_test]` proc-macro)
- **Type**: Local-only proc-macro crate.
- **Description**: `#[large_stack_test]` attribute that runs a test body on a 16 MiB-stack thread.
- **Implementation**: `codex-rs/test-macros/{Cargo.toml,src/lib.rs}`.
- **Merge plan**: Trivial.

### 8. `codex-utils-git` and `codex-utils-file` (extracted utility crates)
- **Type**: Local-only **extractions** (replacing upstream's `git-utils`).
- **Description**:
  - `codex-utils-git` re-implements ghost-commit snapshot/restore (`create_ghost_commit`, `restore_ghost_commit`, `capture_ghost_snapshot_report`), `apply_git_patch`, `merge_base_with_head`, `stage_paths`, platform helpers.
  - `codex-utils-file` is small (`error.rs`, `lib.rs`).
- **Implementation**: `codex-rs/utils/git/{Cargo.toml,src/{lib,apply,branch,errors,ghost_commits,operations,platform}.rs}`; `codex-rs/utils/file/src/{lib,error}.rs`.
- **Merge plan**: Need to reconcile with upstream's `git-utils` crate.

### 9. `codex-api` multi-provider rewrite
- **Type**: Shared (heavily diverged: 60 files, +8860/-3708).
- **Description**: New `ProviderAdapter` trait + `ProviderFactory` (WireApi: Responses / AnthropicMessages / GeminiGenerate); concrete adapters in `providers/{openai,anthropic,gemini}.rs`; per-provider tool formatting; per-provider SSE state machines (`AnthropicStreamState`, `GeminiStreamState`); a complete `file_support/` subtree (cache, capabilities, data_url, errors, responses, routing, upload/{anthropic,gemini,openai}). Removed: `api_bridge`/`files.rs`/`realtime_call`/`methods_common/v1/v2`.
- **Implementation**: `codex-rs/codex-api/src/{provider_adapter,provider_factory}.rs`; `providers/`, `tools/`, `file_support/`, `sse/`.
- **Merge plan**: Highest-risk shared diff outside core/TUI. Strategy: take upstream `codex-api` skeleton, port multi-provider trait + adapters on top.

### 10. `codex-otel` ATA-flavored telemetry
- **Type**: Shared (22 files, +1404/-429).
- **Description**: Adds end-to-end OTLP provider integration: `OtelProvider` (`src/otel_provider.rs`, ~430 lines). Adds `TelemetryAuthMode::Ata` variant; removes upstream's `From<AuthMode>` for `TelemetryAuthMode`, removes `AutomatedReviewer` `ToolDecisionSource`, removes Statsig globals, drops `app-server-protocol` dep.
- **Implementation**: `codex-rs/otel/src/otel_provider.rs` (NEW); `traces/{mod,otel_manager}.rs` (NEW); modified `src/{lib,config,provider,trace_context}.rs`.
- **Merge plan**: Keep our `OtelProvider`/`otel_manager`; merge upstream's metric/event additions piecemeal.

### 11. Upstream-only crates extracted from `core/` (NOT in our fork)
- **Type**: Upstream-new (architectural refactor we haven't pulled).
- **Description**: Upstream `rust-v0.129.0` has these crates that we deleted/never adopted: `aws-auth`, `analytics`, `agent-graph-store`, `agent-identity`, `app-server-transport`, `builtin-mcps`, `bwrap`, `code-mode`, `cloud-tasks-mock-client`, `collaboration-mode-templates`, `core-api`, `core-plugins`, `core-skills`, `device-key`, `exec-server`, `external-agent-migration`, `external-agent-sessions`, `features`, `file-system`, `git-utils`, `install-context`, `codex-mcp`, `memories/{mcp,read,write}`, `model-provider-info`, `models-manager`, `realtime-webrtc`, `response-debug-context`, `rollout`, `rollout-trace`, `sandboxing`, `tools` (the upstream `codex-tools` crate — distinct from our local `tools/` developer-scripts dir), `utils/output-truncation`, `utils/path-utils`, `utils/plugins`, `utils/template`, `v8-poc`. Each is an extraction from upstream `core/`.
- **Merge plan**: Two options:
  - (a) **Keep monolithic `core/`** (current path) — cheap short-term, high merge cost long-term.
  - (b) **Adopt upstream's split** — higher upfront cost, lower steady-state friction.
  Recommend (b) at least for `tools`, `rollout`, `sandboxing`, `model-provider-info`, `memories`, and `utils/path-utils`+`utils/template`. Beware: the upstream `codex-tools` crate name **collides** with our local `codex-rs/tools/` developer-scripts directory.

### 12. `codex-state` upstream feature drift
- **Type**: Shared (36 files, +828/-5551 — net deletion in our direction means upstream is ahead).
- **Description**: Upstream has migrations 0020–0030 that don't exist in our fork: `0020_threads_model_reasoning_effort`, `0021_thread_spawn_edges`, `0022_threads_agent_path`, `0023_drop_logs`, `0024_remote_control_enrollments`, `0025_thread_timestamps_millis`, `0026_thread_dynamic_tools_namespace`, `0027_threads_cwd_sort_indexes`, `0028_device_key_bindings`, `0029_thread_goals`, `0030_threads_thread_source`. Module files: `state/src/runtime/{goals,remote_control,device_key}.rs`, `state/src/model/{thread_goal,graph}.rs`. Goals adds `ThreadGoalUpdate`, `ThreadGoalAccountingOutcome`.
- **Merge plan**: Decide per-feature. Re-number our future migrations starting at 0031+.

### 13. `codex-feedback` rebuild + `OPENAI_BASE_URL` diagnostic
- **Type**: Shared (3 files, +148/-348).
- **Description**: Upstream removed dep on `codex-login`, made `feedback_diagnostics` a public module, and added a connectivity-diagnostic for `OPENAI_BASE_URL`. We deleted `[lints]` from `Cargo.toml`.
- **Merge plan**: Restore `[lints] workspace = true`. Adopt the `OPENAI_BASE_URL` diagnostic.

### 14. `cloud-tasks-client` mock + cloud-tasks renamed types
- **Type**: Shared.
- **Description**: Upstream removed standalone `cloud-tasks-mock-client` crate and inlined `MockClient` into `cloud-tasks-client`.
- **Merge plan**: Adopt upstream's `mock.rs`.

### 15. `app-server-client` major slim-down
- **Type**: Shared (3 files, +228/-2381).
- **Description**: Upstream removed `remote.rs` (890 lines), `LogDbLayer`, `StateDbHandle`, `ServerNotification`/`ServerRequest` re-exports, `EnvironmentManager`/`ExecServerRuntimePaths`. Now imports from `codex_core` instead of `codex_config`.
- **Merge plan**: Risky if our fork still depends on `LogDbLayer`/`StateDbHandle`/`EnvironmentManager`. Search those imports in `cli/`, `exec/`, `core/`.

### Summary table

| # | Crate / Area | Type | Risk |
|---|---|---|---|
| 1 | scheduler | Local-only | Low |
| 2 | artifacts | Local-only | Low |
| 3 | package-manager | Local-only | Low |
| 4 | codex-workspace | Local-only | Low |
| 5 | lsp-client | Local-only | Low |
| 6 | treesitter | Local-only | Low |
| 7 | test-macros | Local-only | Trivial |
| 8 | utils/git, utils/file | Local-only | Med |
| 9 | codex-api multi-provider | Shared, heavy | **High** |
| 10 | otel ATA mode | Shared, heavy | Med |
| 11 | Upstream-extracted core crates (~30) | Upstream-new | **High** (architectural) |
| 12 | state migrations 0020–0030 | Upstream-new | Med |
| 13 | feedback diagnostics | Shared | Low |
| 14 | cloud-tasks mock inlined | Shared | Low |
| 15 | app-server-client slim | Shared | Med |

The biggest two items wave-1 missed: (#11) the upstream `core/` split into ~30 new crates, and (#9) the `codex-api` rewrite for multi-provider streaming.
