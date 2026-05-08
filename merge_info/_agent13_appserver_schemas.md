## Wave-2 App-Server & Schemas Analysis (Agent 13)

### 1. **`v2.rs` Monolithic Protocol Module**
- **Type**: Local-only structural divergence (content largely shared)
- **Description**: Local keeps `protocol/v2.rs` as a single 7,825-line file while upstream `rust-v0.129.0` splits it into 24 submodules under `protocol/v2/{account,apps,collaboration_mode,command_exec,config,device_key,experimental_feature,feedback,fs,hook,item,mcp,model,notification,permissions,plugin,process,realtime,review,shared,tests,thread,thread_data,turn,windows_sandbox}.rs`. Net diff: `-15,451 / +8,212` lines around v2 because every type effectively moved.
- **Merge plan**: Adopt the upstream module split during the merge. Single largest source of merge friction in the protocol crate.

### 2. **Missing `event_mapping` and `item_builders` Protocol Modules**
- **Type**: Upstream-new
- **Description**: Upstream introduced two new protocol-layer helper modules that translate core `EventMsg`/approval flows into v2 `ServerNotification`s and `ThreadItem`s.
- **Implementation**: Upstream `codex-rs/app-server-protocol/src/protocol/event_mapping.rs` (597 lines), `protocol/item_builders.rs` (312 lines, exec/patch approval `ThreadItem` synthesis using `codex_shell_command::parse_command`).
- **Merge plan**: Pull both files in verbatim. Add `codex-shell-command` back as a dependency (local removed it).

### 3. **Missing `serde_helpers` and `common_tests` Protocol Modules**
- **Type**: Upstream-new (partial)
- **Description**: Upstream has `protocol/serde_helpers.rs` and `protocol/common_tests.rs`. Local protocol/mod.rs declares `mod serde_helpers;` but lacks the corresponding `common_tests.rs` reference; the local `common.rs` is 1,550 lines smaller than upstream.
- **Merge plan**: Take upstream's `common.rs` and re-add the `common_tests.rs` `#[path]` mod attr.

### 4. **`codex_message_processor.rs` Monolith vs `request_processors/` Split**
- **Type**: Local-only structural divergence
- **Description**: Local has an 8,763-line single file with all RPC dispatch (`process_request`). Upstream split the same logic across `app-server/src/request_processors/{account,apps,catalog,command_exec,config,device_key,external_agent_config,feedback,fs,git,initialize,marketplace,mcp,plugins,process_exec,search,thread_goal,thread_lifecycle,thread,thread_summary,token_usage_replay,turn,windows_sandbox}_processor.rs` plus dedicated `*_tests.rs`.
- **Merge plan**: Highest-priority restructure. Adopt upstream's `request_processors/` split.

### 5. **`embedded.rs` ATA-only WebSocket Server**
- **Type**: Local-only (private)
- **Description**: 385-line module exposing `EmbeddedWebSocketConfig`/`run_embedded_websocket(...)` so the TUI can host an in-process WebSocket app-server endpoint that shares its `ThreadManager` with mobile clients. Includes a custom `MessageProcessor::new_with_thread_manager()` constructor.
- **Implementation**: `codex-rs/app-server/src/embedded.rs`. Used from `tui/src/remote_control.rs:157` and `app-server/tests/test_embedded_ws.rs`.
- **Merge plan**: Keep verbatim.

### 6. **`device_registration.rs` Supabase Heartbeat Loop**
- **Type**: Local-only (private)
- **Description**: 432-line module that registers this server as a "node" row in the Supabase `devices` table. PATCH `last_seen_at` every 30s; refresh JWT 5 min before expiry; DELETE on shutdown.
- **Implementation**: `codex-rs/app-server/src/device_registration.rs`. Depends on `codex_core::supabase::*` (private).
- **Merge plan**: Keep verbatim. Must stay listed in Justfile `_release_mixed_files`.

### 7. **`config_api.rs` ATA Config Read/Write API**
- **Type**: Shared protocol, ATA-only implementation
- **Description**: 451-line module implementing `config/read`, `config/value/write`, `config/batchWrite`, `configRequirements/read` RPCs. Protocol types exist upstream in `protocol/common.rs:952-980` so the wire interface is shared, but the local impl uses a standalone `ConfigApi` struct rather than the upstream `request_processors/config_processor.rs`.
- **Merge plan**: Switch to upstream's processor pattern.

### 8. **`external_agent_config_api.rs`**
- **Type**: Shared protocol, ATA-only impl wrapper
- **Description**: 106-line wrapper around `codex_core::external_agent_config::ExternalAgentConfigService` exposing `externalAgentConfig/detect` and `externalAgentConfig/import` RPCs. Upstream has equivalent processor.
- **Merge plan**: Switch to upstream's `external_agent_config_processor.rs`.

### 9. **`fs_api.rs` Filesystem RPC Implementation**
- **Type**: Shared protocol, ATA-only thin impl
- **Description**: 365-line module implementing `fs/readFile`, `fs/writeFile`, `fs/createDirectory`, `fs/getMetadata`, `fs/readDirectory`, `fs/remove`, `fs/copy`. Upstream has equivalent `request_processors/fs_processor.rs`.
- **Merge plan**: Replace with upstream `fs_processor.rs`.

### 10. **Missing `fs/watch` / `fs/unwatch` / `fs/changed` (Upstream-new)**
- **Type**: Upstream-new
- **Description**: Upstream adds three new RPCs/notifications: `FsWatch`, `FsUnwatch`, `FsChanged`. Backed by `app-server/src/fs_watch.rs` (debounced 200ms `FileWatcherEvent` aggregator).
- **Merge plan**: Take upstream's types; add `fs_watch.rs` plus its `mod fs_watch;` in `lib.rs`.

### 11. **`in_process.rs` Significant Local Divergence**
- **Type**: Shared (both forks have the file), heavy local edits
- **Description**: 899 lines locally, 978 lines upstream, with `+381 / -230` net diff. Public API: `InProcessStartArgs`, `InProcessServerEvent`, `InProcessClientSender`, `InProcessClientHandle`, `start()`, `start_uninitialized()`. Special handling for `ServerNotification::TurnCompleted` to guarantee delivery.
- **Merge plan**: Use 3-way merge: take upstream's structural changes but preserve local invariants.

### 12. **`app-server-client` Lost the WebSocket `remote.rs` Transport**
- **Type**: Upstream-new
- **Description**: Upstream `codex-rs/app-server-client/src/remote.rs` (890 lines) implements a websocket-backed app-server client transport with a unified `AppServerEvent` surface. Local has no equivalent.
- **Merge plan**: Pull `remote.rs` from upstream.

### 13. **`app-server-test-client` Light Local Edits**
- **Type**: Shared, ATA-style cosmetic edits
- **Description**: Banner string changed `author = "Codex"` → `author = "Ata"`. Removed upstream's `device_code: bool` flag from `TestLogin`. Added `ReadOnlyAccess::FullAccess` to all `SandboxPolicy::ReadOnly` constructors.
- **Merge plan**: Trivial. Keep `author = "Ata"`.

### 14. **`app-server-protocol/Cargo.toml` Dependency Drift**
- **Type**: Local-only dep changes (likely accidental)
- **Description**: Local removed `codex-shell-command` and added `shlex` instead.
- **Merge plan**: When adopting `item_builders.rs` (#2), restore `codex-shell-command`.

### 15. **`export.rs` Schema Generation Regressed `generate_internal_json_schema`**
- **Type**: Upstream-new (regressed locally)
- **Description**: Upstream `export.rs` has `pub fn generate_internal_json_schema(out_dir)`. Local removed this function and several `ScanState` improvements.
- **Merge plan**: Adopt upstream `export.rs` wholesale.

### 16. **TypeScript SDK: `ata` Rebrand of `codex` Class**
- **Type**: Local-only (intentional product rename)
- **Description**: Renamed `sdk/typescript/src/codex.ts` → `ata.ts`, class `Codex` → `Ata`, `CodexOptions` → `AtaOptions`, `codexPathOverride` → `ataPathOverride`, `CodexExec` → `AtaExec`.
- **Merge plan**: Trivial mechanical rename. Consider committing a `scripts/rename-codex-to-ata.sh` post-merge codemod.

### 17. **Python SDK Generated Files Massively Smaller**
- **Type**: Likely codegen drift (needs regen)
- **Description**: `sdk/python/src/codex_app_server/generated/v2_all.py` shrank by `-3399/+574 lines (net -2825)`.
- **Merge plan**: Re-run `python sdk/python/scripts/update_sdk_artifacts.py` AFTER the protocol crate is merged.

### 18. **TS Schema: Many Local-only Files vs Upstream Pruned Set**
- **Type**: Mixed (local-added: 132 files; local-deleted: 132 files)
- **Description**: 132 TS schema files exist locally but not in upstream (e.g. `AddDocumentSectionEvent.ts`, `CollabAgentInteractionBeginEvent.ts`, `Personality.ts`, `RealtimeVoice.ts`, etc.). Conversely, ~132 v0.129.0 files exist upstream but not locally (e.g. `v2/DeviceKey*.ts`, `v2/Marketplace*.ts`, `v2/PluginShare*.ts`, `v2/ThreadGoal*.ts`, `v2/RemoteControl*.ts`, `v2/GuardianApproval*.ts`).
- **Merge plan**: After (#1) v2.rs split, **regen** the TS schema.

### 19. **Local-only `serde_json/JsonValue.ts` TS Schema Helper**
- **Type**: Local-only addition
- **Description**: Hand-written stub for `serde_json::Value`.
- **Merge plan**: Keep verbatim.

### Summary

The wave-2 picture is dominated by **structural divergence rather than feature divergence**: the bulk of "ATA-specific" endpoints (`fs/*`, `config/*`, `externalAgentConfig/*`, `configRequirements/*`) actually exist upstream as shared APIs — the local fork just hasn't adopted upstream's `request_processors/` and `protocol/v2/` module splits yet. The truly ATA-private surface is small and well-isolated.

Highest-leverage merge actions:
1. **Adopt upstream's `protocol/v2/*.rs` module split** (#1).
2. **Adopt upstream's `request_processors/` split** (#4) — break up the 8,763-line `codex_message_processor.rs`.
3. **Re-run codegen** for both Python (#17) and TS (#18) schemas.
4. **Pull missing protocol modules** (#2, #3, #15) and `fs_watch` (#10).
5. **Add WebSocket client `remote.rs`** (#12).
