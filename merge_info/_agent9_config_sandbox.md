## Config/Sandbox Analysis (Agent 9)

**Branch:** `merge_upstream_0.129.0` vs upstream tag `rust-v0.129.0`. Investigation focused on `codex-rs/config/`, `linux-sandbox/`, `windows-sandbox-rs/`, `process-hardening/`, `network-proxy/`, `execpolicy/`, `execpolicy-legacy/`, `shell-command/`, `shell-escalation/`, plus crates in `codex-rs/core/src/config/`. The dominant pattern is **massive upstream refactors** (config crate exploded from ~2.5k LOC to ~17k LOC; bwrap/landlock/seatbelt extracted into a brand-new `codex-rs/sandboxing/` crate; `codex-rs/bwrap/` standalone bin crate added). ATA has small targeted local-only additions on top of an older upstream baseline.

### 1. Upstream-new: `codex-rs/sandboxing/` crate

- **Type:** Upstream-new
- **Description:** A new crate `codex-sandboxing` at `codex-rs/sandboxing/` houses bubblewrap, landlock, seatbelt, and a unified `SandboxManager` plus policy transforms. Includes embedded sbpl files (`seatbelt_base_policy.sbpl`, `seatbelt_network_policy.sbpl`, `restricted_read_only_platform_defaults.sbpl`).
- **Implementation:** `codex-rs/sandboxing/Cargo.toml`, `src/{bwrap.rs, landlock.rs, manager.rs, policy_transforms.rs, seatbelt.rs}` (plus `_tests.rs` siblings).
- **Merge plan:** Adopt this new crate wholesale. Then move local linux-sandbox bwrap/landlock helpers (currently inline in `codex-rs/linux-sandbox/src/{bwrap.rs,landlock.rs}`) over to consume the shared crate. ATA's `vendored_bwrap.rs` integration must be preserved (see #6).

### 2. Upstream-new: `codex-rs/bwrap/` standalone bin crate

- **Type:** Upstream-new
- **Description:** Upstream split bubblewrap into a standalone bin crate with its own `build.rs`, `config.h`, `BUILD.bazel`, and a single `src/main.rs`. Replaces the multi-file bwrap implementation that previously lived inside `linux-sandbox/`.
- **Implementation:** `codex-rs/bwrap/{Cargo.toml, build.rs, config.h, src/main.rs}`.
- **Merge plan:** Adopt the new crate. Map ATA's vendored bwrap path onto the new entry point.

### 3. Upstream-new: large config-crate split

- **Type:** Upstream-new
- **Description:** Upstream's `codex-config` crate gained ~30 new modules: `config_toml.rs`, `loader/`, `mcp_edit.rs`, `mcp_types.rs`, `marketplace_edit.rs`, `permissions_toml.rs`, `plugin_edit.rs`, `profile_toml.rs`, `project_root_markers.rs`, `schema.rs`, `skills_config.rs`, `thread_config.rs` + proto, `tui_keymap.rs`, `types.rs`, `hook_config.rs`, `host_name.rs`, `key_aliases.rs`, plus their `_tests.rs`. Local lib.rs has ~58 lines of public re-exports; upstream has ~128 lines.
- **Implementation:** Files under `codex-rs/config/src/`. Local stub at `codex-rs/config/src/lib.rs`.
- **Merge plan:** Adopt all modules. Critical: ATA's `core/src/config/types.rs` (1194 lines added on our side) and `core/src/config/mod.rs` must be reconciled — many of those types now live in `codex-config` upstream.

### 4. Local-only: `realtime_audio` / `RealtimeAudioConfig` field on `Config`

- **Type:** Shared (added by both, but lives in different crates)
- **Description:** Both branches expose `realtime_audio: RealtimeAudioConfig` on `Config`, but locally it lives in `codex-rs/core/src/config/mod.rs` (lines 493 and 1202–1204) and `types.rs` (lines 1402–1415); upstream moved it into the new `codex-config::config_toml` module.
- **Implementation:** `RealtimeAudioConfig`, `RealtimeAudioToml`, `RealtimeConfig`, `RealtimeWsMode`, `RealtimeWsVersion` in `codex-rs/core/src/config/types.rs:1372-1415`.
- **Merge plan:** Port these structs over.

### 5. Local-only: `voice_mode` and `reading_view` config sections

- **Type:** Local-only (ATA fork)
- **Description:** ATA adds two top-level config sections: `[voice_mode]` (enabled, output, auto_submit, vad_threshold, silence_duration_ms, tts_enabled, stt_enabled, verbosity, elevenlabs.{api_key,voice_id,model_id,language_code,speed}) and `[reading_view]` (mode: "tui"|"browser"|"disabled").
- **Implementation:** 
  - Types: `codex-rs/core/src/config/types.rs:957-1031` (`VoiceVerbosity`, `VoiceOutput`, `ElevenLabsToml`, `ReadingViewToml`, `VoiceModeToml`).
  - Wired into `ConfigToml` at `codex-rs/core/src/config/mod.rs:1338-1344`.
  - Edit helpers: `codex-rs/core/src/config/edit.rs:72-148`.
- **Merge plan:** Keep verbatim.

### 6. Local-only: `vendored_bwrap` ffi entrypoint in linux-sandbox

- **Type:** Local-only
- **Description:** ATA links bubblewrap C sources directly into the Rust binary and exposes `bwrap_main` via FFI. Upstream's equivalent path is the new `bazel_bwrap` + `bundled_bwrap` modules.
- **Implementation:** `codex-rs/linux-sandbox/src/vendored_bwrap.rs`; referenced from `linux-sandbox/src/lib.rs` and `linux_run_main.rs`.
- **Merge plan:** Either keep our `vendored_bwrap.rs` as a third backend OR migrate fully to the upstream `bundled_bwrap` model.

### 7. Local-only: `process-hardening` raises `RLIMIT_NOFILE`

- **Type:** Local-only
- **Description:** ATA's `pre_main_hardening` calls `raise_file_descriptor_limit()` on Linux/BSD/macOS to raise `RLIMIT_NOFILE` to its hard limit. Rationale: subagents each open ~15-25 FDs and macOS's default soft limit of 256 causes EMFILE panics when spawning many in parallel. Upstream does NOT do this.
- **Implementation:** `codex-rs/process-hardening/src/lib.rs`.
- **Merge plan:** Keep ATA's additions verbatim. Upstream additionally removed the macOS `MallocStackLogging`/`MallocLogFile` env-var sweep — decide whether to keep that scrub locally.

### 8. Local-only: `disable_process_dumping` retained

- **Type:** Local-only (deletion-conflict)
- **Description:** ATA still exports `pub fn disable_process_dumping()` from `process-hardening` (Linux: `prctl(PR_SET_DUMPABLE, 0)`). Upstream **deleted** this function. ATA's `linux-sandbox/src/proxy_routing.rs:617` calls it through a helper `harden_bridge_process()`.
- **Merge plan:** Decide whether the extra ptrace-attach hardening is worth keeping.

### 9. Local-only: `network-proxy/src/admin.rs` debug HTTP API

- **Type:** Local-only — and currently **orphaned**
- **Description:** ATA-only file (`+181 lines`, no upstream counterpart) exposing a debug HTTP admin API for the network proxy: `run_admin_api`, `run_admin_api_with_std_listener`, `run_admin_api_with_listener`, `handle_admin_request`.
- **Implementation:** `codex-rs/network-proxy/src/admin.rs`. **NOT declared in `network-proxy/src/lib.rs`** — currently dead code.
- **Merge plan:** Either (a) wire it back up or (b) delete the file.

### 10. Shared: `NetworkDomainPermission*` / `NetworkUnixSocketPermission*` types

- **Type:** Shared (both have them, but upstream relocated)
- **Description:** Upstream **removed them from `network-proxy::config`** and re-exposes them from `codex-config::config_requirements`.
- **Merge plan:** Accept the move. Update any ATA callers.

### 11. Local-only: ATA-prefixed Windows sandbox binaries

- **Type:** Local-only
- **Description:** ATA renamed the Windows sandbox binaries from `codex-windows-sandbox-setup` / `codex-command-runner` → `ata-windows-sandbox-setup` / `ata-command-runner`. References to `.ata` (workspace marker dir) appear throughout `windows-sandbox-rs/src/`.
- **Implementation:** `codex-rs/windows-sandbox-rs/Cargo.toml` (`[[bin]] name = "ata-..."`); `.ata` paths in modules.
- **Merge plan:** Keep the rename. Where upstream's Windows sandbox refactor replaces our files, re-apply the `.ata`/`ata-*` substitutions on top.

### 12. Local-only: stripped Windows sandbox Cargo deps

- **Type:** Local-only delta
- **Description:** ATA's `windows-sandbox-rs/Cargo.toml` removed several workspace deps (`codex-utils-pty`, `codex-otel`, `glob`, `tokio`) and `windows-sys` features. Also forced `edition = "2021"` instead of `edition.workspace = true` and removed `[lints] workspace = true`.
- **Merge plan:** Looks like accidental drift — re-add `edition.workspace`, `[lints] workspace = true`.

### 13. Shared: `ApprovalPolicy` / `SandboxMode` plumbing

- **Type:** Shared.
- **Description:** Both branches use `AskForApproval` and `SandboxMode { ReadOnly, WorkspaceWrite, DangerFullAccess }`. Local additionally consults a `WindowsSandboxLevel` to override on Windows.
- **Implementation:** `codex-rs/core/src/config/mod.rs:197, 1018, 1039, 1356-1357, 1576-1660`. ATA additions: `WindowsSandboxModeToml` at `core/src/config/types.rs:34-48`.
- **Merge plan:** No conflict on policy semantics.

### 14. Shared: `execpolicy` + `execpolicy-legacy` (upstream-driven changes)

- **Type:** Shared
- **Description:** Upstream made all execpolicy submodules `pub` and removed `PatternToken`, `PrefixPattern`, `PrefixRule` from the `lib.rs` re-exports. **No ATA-specific tokens.**
- **Merge plan:** Take upstream verbatim.

### 15. Shared: `shell-command` safety lists (upstream-driven changes)

- **Type:** Shared
- **Description:** Upstream rewrote `is_safe_command.rs` (-150 net), removed `powershell_parser.rs`, and updated `windows_safe_commands.rs`/`windows_dangerous_commands.rs`.
- **Merge plan:** Take upstream verbatim. Verify `core/src/exec_policy*.rs` still type-checks.

### 16. Shared: `shell-escalation` API rename

- **Type:** Shared (upstream-driven rename)
- **Description:** Upstream renamed the re-exported type `ResolvedPermissionProfile` → `Permissions`, made internal modules `pub`, and moved `ESCALATE_SOCKET_ENV_VAR` re-export.
- **Merge plan:** Adopt upstream rename. Search the workspace for `ResolvedPermissionProfile` usages.

### 17. Local-only: ATA-only crates depending on config

- **Type:** Local-only crates
- **Description:** Four ATA-only crates that consume config: `codex-rs/codex-elevenlabs/`, `codex-rs/codex-research-tools/`, `codex-rs/codex-data-tools/`, `codex-rs/reading-view-server/`.
- **Merge plan:** Out of scope — pure ATA crates.

### Recommended merge order

1. Adopt upstream's `codex-rs/sandboxing/` and `codex-rs/bwrap/` crates (#1, #2).
2. Adopt the config-crate split (#3); shim `core/src/config/` re-exports through `codex-config`.
3. Re-apply ATA's `voice_mode`/`reading_view`/`realtime_audio` deltas onto the new `config_toml` location (#4, #5).
4. Keep ATA's `process-hardening` `raise_file_descriptor_limit` (#7). Decide on `disable_process_dumping` (#8).
5. Take upstream `execpolicy`, `shell-command`, `shell-escalation` changes verbatim (#14, #15, #16).
6. Network proxy: accept upstream's removal of permission types (#10), then either delete `admin.rs` or wire it up properly (#9).
7. Windows sandbox: take upstream's restructure, then re-apply `ata-`/`.ata` renames (#11, #12).
