## CLI/Exec Analysis (Agent 7)

Comparison of `merge_upstream_0.129.0` vs `rust-v0.129.0` for `codex-rs/cli/`, `codex-rs/exec/`, `codex-rs/codex-workspace/`, `codex-rs/scheduler/`, `codex-rs/skills/`, `codex-rs/hooks/`, plus TUI slash commands.

### Top-level Subcommands

| # | Name | Type | Description / Implementation |
|---|------|------|------------------------------|
| 1 | `ata workspace` (alias `ws`) | **Local-only** | Manage workspaces (init/list/select/delete/resolve/audit/recipes). Backed by local-only crate `codex-rs/codex-workspace/`. Wired in `cli/src/main.rs:167-168` via `WorkspaceCli`. New crate has 16 source modules including `audit.rs`, `manifest.rs`, `recipes.rs`, `workspace_resolution.rs`. |
| 2 | `ata jobs` | **Local-only** | Manage scheduled jobs (list/show/create/delete/pause/resume/run/history/logs + `search-commands`). Backed by local-only crate `codex-rs/scheduler/` (`JobsCli` in `scheduler/src/cli.rs:25-53`). |
| 3 | `ata scheduler` | **Local-only** | Control the scheduler daemon (start/stop/+search-commands). Same crate, `SchedulerCli` in `scheduler/src/cli.rs:87-100`. |
| 4 | `ata zotero` | **Local-only** | Manage Zotero libraries/collections/items/attachments. ~2614 lines in `cli/src/zotero_cmd.rs` (new file). Wired at `cli/src/main.rs:170-171`. |
| 5 | `ata mobile` | **Local-only** (`#[cfg(not(windows))]`) | Manage mobile background server. New file `cli/src/mobile_cmd.rs` (367 lines). Wired at `cli/src/main.rs:156-158`. |
| 6 | `ata plus` | **Local-only** (gated by `feature = "ata-plus"`) | ATA-private subcommand surface (uses `ata_plus::SubcommandCli`). `cli/src/main.rs:173-174`. |
| 7 | `ata exec` (worker mode) | **Shared+extended** | Adds local-only flags: `--worker`, `--lead-session-id`, `--workspace`, `--progress-cursor` in `exec/src/cli.rs:90-124`. Note: `--worker`/`--lead-session-id` are coordination/relay related and per CLAUDE.md should be private — currently NOT under a `#[cfg(feature = "relay")]` guard, which is a sync-release concern. |
| 8 | `ata mcp-server` | **Shared** | Both have it. Local just renames branding strings. |
| 9 | `ata app-server` | **Shared, locally pruned** | Upstream has `proxy`, `generate-internal-json-schema`; local keeps only `generate-ts` and `generate-json-schema` (`cli/src/main.rs:380-388`). |
| 10 | `ata debug dump-initial-context` | **Local-only** | New debug subcommand to dump assembled initial context (`cli/src/main.rs:199-201`). Upstream's debug has `models`, `prompt-input`, `trace-reduce` instead — all dropped locally. |
| 11 | `ata update` | **Upstream-only (dropped locally)** | Upstream provides `Update`; local removes it (relies on package manager / has its own `run_update_action` flow tied to `AppExitInfo`). |
| 12 | `ata plugin` / `ata builtin-mcp` / `ata exec-server` / `ata marketplace` / `ata debug models` | **Upstream-only (dropped locally)** | Removed from local: `marketplace_cmd.rs` deleted, `Plugin`, `BuiltinMcp`, `ExecServer` not present. Tests `marketplace_*.rs`, `login.rs`, `update.rs`, `debug_models.rs` deleted. |
| 13 | `ata sandbox windows` | **Local-only** | Local `SandboxCommand::Windows(WindowsCommand)` (`cli/src/main.rs:275-276`); upstream restructured this differently. Note `cli/src/desktop_app/windows.rs` was deleted locally. |
| 14 | `ata research` (CLI module) | **Local-only / orphaned** | New 979-line file `cli/src/research.rs` defines `ResearchArgs` (task, --num-solutions, --max-agents, --framework, --generate-code, --output, --codebase, --prior-results, --feedback, --iteration). Currently not declared as a `mod` in `main.rs`/`lib.rs` — appears to be staged but unwired. Merge-plan: either wire it as `Subcommand::Research(...)` or delete. |

### Slash Commands (TUI)

Local enum `SlashCommand` in `codex-rs/tui/src/slash_command.rs` diverges substantially from upstream.

| Slash command | Type | Notes |
|---|---|---|
| `/research` | **Local-only** | Toggle research tool integrations. Listed in `available_during_task=false` arms. |
| `/voice`, `/voice-setup` | **Local-only** | Voice mode toggle and TTS/STT defaults config. |
| `/realtime`, `/settings` | **Local-only** | Realtime experimental voice; mic/speaker config. |
| `/personality` | **Local-only** | Choose communication style. |
| `/team` | **Local-only** | Coordination agents/messages list (private/relay-adjacent — should likely be `#[cfg(feature = "...")]` for release-branch hygiene). |
| `/jobs` | **Local-only** | View scheduled jobs/daemon status. |
| `/mobile` | **Local-only** | Start mobile remote-control server. |
| `/apps`, `/account`, `/collab`, `/agent`/`/subagents` | **Local-only** | ATA app-store, account, collab modes, multi-agent thread switcher. |
| `/feedback`, `/test-approval`, `/rollout`, `/ps`, `/stop`/`/clean`, `/skills` | **Shared** | Both have. |
| `/ide`, `/keymap`, `/vim`, `/approve` (auto-review), `/memories`, `/hooks`, `/goal`, `/side`, `/raw`, `/title`, `/plugins` | **Upstream-only (dropped locally)** | Several useful upstream commands removed. |
| `/approvals`, `/permissions` | Both | Local has both as aliases for the same description. Upstream has `Permissions` but not `Approvals`. |
| `/debug-m-drop`, `/debug-m-update` | **Shared** | "DO NOT USE" debugging commands. |

### Crates: skills/ and hooks/

| Component | Type | Notes |
|---|---|---|
| `codex-rs/skills/` | **Shared, locally extended** | Upstream lib.rs is 169 lines / 3 public fns. Local lib.rs is 428 lines / 9 public fns adding `system_cache_root_dir`, `install_research_skills`, `install_workspace_skills`, `install_custom_skills`, `custom_skill_cache_root_dirs`, plus new asset trees: `assets/research/*` (paper-discoverer, paper-synthesizer, hn-synthesis, kb, zotero, etc.), `assets/workspace/*`, `assets/adapt-environment/`, plus `assets/remote-exec/` (private — must NOT reach release branch per CLAUDE.md). Local uses `PathBuf` whereas upstream switched to `AbsolutePathBuf`. |
| `codex-rs/hooks/` | **Shared, locally extended** | Local adds `engine/config.rs` (config rules engine). Upstream has `engine/mod_tests.rs` that local removed. JSON schema fixtures appear in both. |

### Tests added locally
- `cli/tests/jobs_scheduler_search_commands.rs` (75 lines) — new search-commands feature
- `cli/tests/workspace_search_commands.rs` (53 lines)
- `cli/tests/zotero_search_commands.rs` (101 lines)

### Merge Plan Summary
1. **Keep as-is** (local-only crates and surfaces): `codex-workspace`, `scheduler`, `zotero_cmd.rs`, `mobile_cmd.rs`. These are entirely additive and follow CLAUDE.md guidance to "put new private code in its own crate/directory".
2. **Audit private-leak risk**: `--worker`, `--lead-session-id` in `exec/src/cli.rs`, plus `/team` in `tui/slash_command.rs`. CLAUDE.md says fleet/relay code should be `#[cfg(feature = "relay")]`. These are currently unconditional on the merge branch — wrap them or add to Justfile `_release_mixed_files` before pushing release.
3. **Reconcile dropped upstream commands**: decide whether to re-introduce `update`, `plugin`, `builtin-mcp`, `exec-server`, `marketplace` (and their tests `marketplace_*.rs`, `login.rs`, `update.rs`, `debug_models.rs`), or document why ATA dropped them. Upstream's debug variants (`models`, `prompt-input`, `trace-reduce`) were also dropped — likely intentional but worth confirming.
4. **Decide `/ide`, `/keymap`, `/vim`, `/approve`, `/memories`, `/hooks`, `/goal`, `/side`, `/raw`, `/title`, `/plugins`** slash commands: upstream-new and currently absent locally. Likely worth adopting at minimum `/hooks` and `/memories` since the underlying crates exist locally.
5. **Wire or remove `cli/src/research.rs`**: the 979-line module exists but no `mod research` declaration — it is dead code unless intentionally staged for a follow-up.
6. **Update `exec/src/cli.rs` to upstream's `SharedCliOptions` pattern** (line 6 import removed, struct flattened): upstream introduced `SharedCliOptions` + `ExecSharedCliOptions` pattern; local re-expanded all flags inline. Adopting upstream pattern would shrink conflict surface for future merges (per CLAUDE.md "reduce conflict surface" mandate).

Key files (absolute paths):
- /Users/huytho_ho/acli/ata/codex-rs/cli/src/main.rs
- /Users/huytho_ho/acli/ata/codex-rs/cli/src/research.rs (orphaned)
- /Users/huytho_ho/acli/ata/codex-rs/cli/src/zotero_cmd.rs
- /Users/huytho_ho/acli/ata/codex-rs/cli/src/mobile_cmd.rs
- /Users/huytho_ho/acli/ata/codex-rs/exec/src/cli.rs
- /Users/huytho_ho/acli/ata/codex-rs/codex-workspace/src/lib.rs
- /Users/huytho_ho/acli/ata/codex-rs/scheduler/src/cli.rs
- /Users/huytho_ho/acli/ata/codex-rs/skills/src/lib.rs
- /Users/huytho_ho/acli/ata/codex-rs/hooks/src/engine/config.rs
- /Users/huytho_ho/acli/ata/codex-rs/tui/src/slash_command.rs
