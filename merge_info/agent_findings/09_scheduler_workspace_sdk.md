# Fork-vs-Upstream Divergence Analysis: Scheduler, Workspace, SDK, and Related Areas

**Analysis Date:** 2026-05-07  
**Upstream Reference:** `rust-v0.129.0`  
**Fork Base:** HEAD (Agents2AgentsAI/ata)

---

## Executive Summary

The fork contains **3 entirely new Rust crates** (scheduler, codex-workspace, package-manager) that do not exist in upstream, plus modifications to **CLI bridges, SDKs, and tooling**. These additions are strategic new features for ATA, not conflicts with upstream. The `shell-tool-mcp` and `sdk/` directories exist in both repos with divergent evolution (ATA-specific branding/features).

---

## Feature Analysis

### A. LOCAL-ONLY FEATURES (Not in Upstream)

#### 1. **codex-scheduler** Crate

**Name:** Background Job Scheduler

**Description:**  
A fully-featured scheduler for running jobs on cron-like triggers, file-watch triggers, HTTP polling, and webhooks. Provides CLI commands to manage job definitions, runs, and daemon lifecycle. Persists job state to SQLite.

**Implementation Summary:**
- **Location:** `/Users/huytho_ho/acli/ata/codex-rs/scheduler/`
- **Key Files:**
  - `src/cli.rs` (600+ lines): CLI interface for job/scheduler commands
  - `src/engine/scheduler.rs` (441 lines): Core scheduler logic
  - `src/storage/`: SQLite persistence (jobs_repo, runs_repo, state_repo)
  - `src/trigger/`: Trigger types (cron_trigger, file_watch, http_poll, webhook)
  - `src/daemon/`: Daemon lifecycle management
  - `migrations/001_init.sql`: Database schema
- **Dependencies:** sqlx (async SQL), tokio (async runtime), cron, notify (file watching), reqwest, serde_json

**Status vs Upstream:** **LOCAL-ONLY** — not in `rust-v0.129.0`

**Merge Plan:**  
Keep as-is; this is a new ATA capability. When merging upstream, verify no upstream scheduler/job-scheduling APIs conflict with this crate's public API. Monitor for duplicate functionality in future upstream versions.

---

#### 2. **codex-workspace** Crate

**Name:** Workspace Management and Git Repository Orchestration

**Description:**  
Comprehensive workspace/repository management system supporting:
- Multi-repo workspace resolution and manifest/lock files
- Audit logging for workspace operations
- Repository cloning, pinning, and state management
- Run artifact management and lifecycle
- Workspace-aware path resolution with @-syntax support
- Git integration for workspace reconciliation

**Implementation Summary:**
- **Location:** `/Users/huytho_ho/acli/ata/codex-rs/codex-workspace/`
- **Key Files:**
  - `src/commands/` (30 subcommand handlers): init, list, read, select, delete, audit, export_spec, materialize, etc.
  - `src/manifest.rs`: Workspace manifest structure and parsing
  - `src/spec.rs`: Repository specification and resolution
  - `src/workspace_resolution.rs`: Multi-repo workspace dependency resolution
  - `src/git.rs`: Git operations for repo management
  - `src/lock.rs`: Lock file handling for pinned states
  - `src/recipes.rs`: Execution recipes/templates
  - `src/audit.rs` (195 lines): Audit trail maintenance
- **Subcommands:** init, list, read, select, delete, resolve, check-host, audit, export-spec, diff-spec, materialize, validate, repo-clone, repo-remove, repo-pin, repo-unpin, repo-update-state, run-setup, run-locked, run-update-status, run-remove, and more

**Status vs Upstream:** **LOCAL-ONLY** — not in `rust-v0.129.0`

**Merge Plan:**  
Keep as-is; this is a cornerstone ATA feature for multi-repo research projects. Upstream codex has `collaboration-mode-templates`, `external-agent-migration`, `external-agent-sessions` which are orthogonal; no conflict expected.

---

#### 3. **codex-package-manager** Crate

**Name:** Package Download and Installation Manager

**Description:**  
Generic package management library for downloading, extracting, and managing binary/archive distributions. Supports multiple platforms, archive formats (tar.gz, zip), and SHA256 verification.

**Implementation Summary:**
- **Location:** `/Users/huytho_ho/acli/ata/codex-rs/package-manager/`
- **Key Files:**
  - `src/manager.rs`: Core PackageManager API
  - `src/archive.rs`: ArchiveFormat enum and extraction logic
  - `src/platform.rs`: Platform tuple (OS, arch) definitions
  - `src/package.rs`: ManagedPackage abstraction
  - `src/config.rs`: Configuration struct
- **Core Exports:** `PackageManager`, `ManagedPackage`, `PackageManagerConfig`, `ArchiveFormat`, `PackagePlatform`, `PackageManagerError`
- **Dependencies:** reqwest (HTTP), tokio (async), serde, sha2 (hashing), zip, tar, flate2

**Status vs Upstream:** **LOCAL-ONLY** — not in `rust-v0.129.0`

**Merge Plan:**  
Keep as-is. This is a foundational utility for ATA binary distribution and MCP server management. No upstream equivalent exists; low merge conflict risk.

---

### B. FEATURES IN BOTH FORK AND UPSTREAM (Divergent Implementations)

#### 4. **sdk/** Directory (Python & TypeScript SDKs)

**Name:** Codex/ATA Application Server SDKs

**Description:**  
Client libraries for invoking Codex CLI as a library (Python) and embedding ATA agent in Node.js applications (TypeScript).

**Implementation Summary:**

**Local Fork (`/Users/huytho_ho/acli/ata/sdk/`):**
- **Python SDK** (`sdk/python/`):
  - Pydantic-based generated wire models from v2 app-server schema
  - Python async/sync client with snake_case field mapping
  - Examples, tests, notebooks
  - Distributed as `codex-app-server` package
  - Entry point: `from codex_app_server import Codex, TextInput`
  
- **Python Runtime** (`sdk/python-runtime/`):
  - Wrapper for vendoring `codex-cli-bin` runtime dependency
  - Hatch build hooks
  
- **TypeScript SDK** (`sdk/typescript/`):
  - Wraps `@a2a-ai/ata` CLI binary
  - Spawns CLI process, exchanges JSONL events over stdin/stdout
  - Supports streaming responses with typed event handlers
  - Published as `@a2a-ai/ata-sdk`
  - Entry point: `import { Ata } from "@a2a-ai/ata-sdk"`

**Upstream (`rust-v0.129.0`):**
- Identical structure: `sdk/python/`, `sdk/python-runtime/`
- Package names: `codex-app-server`, no TypeScript SDK in upstream
- Same Pydantic/generated-model approach

**Status vs Upstream:** **BOTH have implementations** — structure and naming conventions differ for ATA branding

**Key Differences:**
- Local: TypeScript SDK added (ATA-specific)
- Local: Package names updated for `@a2a-ai/` scope (branding)
- Upstream: References `@openai/codex`

**Merge Plan:**  
For Python SDKs: adopt upstream's structure first, then layer ATA TypeScript wrapper on top. Rename references from `@openai/codex` to `@a2a-ai/ata` in both Python and TypeScript. No functional conflicts; TypeScript SDK is ATA-only addition.

---

#### 5. **codex-cli/** Directory (Node.js CLI Wrapper)

**Name:** Node.js CLI Wrapper for ATA/Codex CLI

**Description:**  
NPM package that wraps native Codex binary, providing a Node.js entry point and vendored dependencies.

**Implementation Summary:**

**Local Fork (`/Users/huytho_ho/acli/ata/codex-cli/`):**
- `package.json`: Publishes as `@a2a-ai/ata` (not `@openai/codex`)
- `bin/ata.js`: Node wrapper script
- `scripts/build_npm_package.py`: Build automation
- Docker/container support with `Dockerfile`

**Upstream (`rust-v0.129.0`):**
- Identical structure and automation
- `package.json`: Publishes as `@openai/codex`
- `bin/codex.js`: Node wrapper

**Status vs Upstream:** **BOTH have implementations** — branding/naming differs

**Key Differences:**
- Local: Branded as `ata`, scoped to `@a2a-ai/`
- Upstream: Branded as `codex`, scoped to `@openai/`

**Merge Plan:**  
Maintain the fork's ATA branding. When syncing upstream, update references (`codex` → `ata`, `@openai/` → `@a2a-ai/`) in wrapper scripts and package metadata. No functional conflicts.

---

#### 6. **shell-tool-mcp/** Directory (Sandboxed Shell MCP)

**Name:** Model Context Protocol Server for Sandboxed Shell Tool

**Description:**  
MCP server providing a `shell` tool that runs bash/zsh in a sandbox with exec-hook support for command interception and approval. Includes patched shell binaries and Rust MCP server code.

**Implementation Summary:**

**Local Fork (`/Users/huytho_ho/acli/ata/shell-tool-mcp/`):**
- `src/bashSelection.ts`: Selects appropriate Bash binary per host OS/arch
- `src/`: MCP server implementation (TypeScript)
- `bin/`: Prebuilt shell binaries (Bash, zsh) for multiple platforms
- `README.md`: Updated to reference `ata` CLI instead of `codex`

**Upstream (`rust-v0.129.0`):**
- NOT present as top-level directory
- Shell escalation support is in `codex-rs/shell-escalation/`

**Status vs Upstream:** **BOTH have implementations** — structure differs; upstream is Rust, local is TypeScript with prebuilt binaries

**Key Differences:**
- Local: TypeScript/Node.js MCP server with prebuilt binary bundles
- Upstream: Rust implementation in `codex-rs/shell-escalation/`
- Local: References `ata` and `~/.ata/config.toml`; upstream references `codex` and `~/.codex/config.toml`

**Merge Plan:**  
Keep local TypeScript MCP server; it's ATA-specific. During upstream merge, watch for updates to `codex-rs/shell-escalation/` (Rust implementation) and evaluate whether functionality should be ported or extended. Current separation (TypeScript MCP vs Rust core) is acceptable.

---

### C. REPOSITORY-LEVEL CONFIGURATION

#### 7. **Root-Level Build & Config Files**

**Name:** Bazel/Build Configuration, Justfile Recipes, Module Manifests

**Files Analyzed:**
- `justfile`: Build and test recipes
- `defs.bzl`: Bazel rule definitions (multiplatform_binaries, workspace_root_test)
- `BUILD.bazel`: Root Bazel targets
- `MODULE.bazel`: Bazel module dependencies
- `.bazelrc`: Bazel configuration
- `README.md`: Project overview (ATA-branded)
- `AGENTS.md`: Agent guidance (extended with ATA/codex-workspace/scheduler references)

**Local Fork Differences:**
- `justfile`: Added ATA-specific recipes (`test-reading-view`, `test-karaoke`, `test-tts-live`, `test-tts-sync`, `fix-fast`), removed `tui-with-exec-server`
- `AGENTS.md`: Extended with scheduler/workspace/SDK sections, release branch/private-code separation guidance
- `README.md`: ATA branding (install via `@a2a-ai/ata`, mentions research tools, LSP, voice support)
- Bazel targets: Include new codex-scheduler, codex-workspace, codex-package-manager crates

**Status vs Upstream:** **BOTH have implementations** — ATA-specific extensions/branding

**Key Differences:**
- Local: Includes 3 new crates in Cargo.toml workspace members and Bazel targets
- Local: Extended AGENTS.md with workspace/scheduler patterns
- Upstream: No scheduler/workspace; standard Codex references

**Merge Plan:**  
Adopt upstream's Bazel and justfile structure as baseline, then:
1. Re-add ATA-specific test targets (`test-reading-view`, `test-karaoke`, `test-tts-*`)
2. Register new crates in Cargo.toml members list and Bazel BUILD files
3. Update AGENTS.md with upstream guidance, then layer ATA-specific sections (workspace, scheduler)
4. Sync README.md branding (keep ATA flavor, incorporate upstream improvements)

---

#### 8. **scripts/** Directory (Top-Level Build & Install Scripts)

**Name:** Build, Installation, and Development Automation Scripts

**Implementation Summary:**
- `scripts/install.sh`: Interactive ATA CLI installer (cross-platform)
- `scripts/check_blob_size.py`: Enforces binary size limits (ATA-specific CI check)
- `scripts/stage_npm_packages.py`: NPM package staging for release
- `scripts/install/` subdirectory: OS-specific installers
- Standard utilities: `asciicheck.py`, `readme_toc.py`, `mock_responses_websocket_server.py`

**Upstream Equivalents:**
- Bazel-based build system (codex-rs/BUILD, MODULE.bazel)
- Standard CI/CD via GitHub Actions

**Status vs Upstream:** **BOTH have implementations** — different approaches

**Key Differences:**
- Local: Emphasizes npm/binary distribution, cross-platform installers
- Upstream: Bazel-first build, GitHub Actions for release

**Merge Plan:**  
Keep local scripts intact; they support ATA's distribution model. During upstream merges, check for new CI/release scripts and integrate selectively (e.g., new signing, attestation features).

---

#### 9. **tools/** Directory (Repository Maintenance Tools)

**Name:** Development and Analysis Tools

**Implementation Summary:**

**Local Fork (`/Users/huytho_ho/acli/ata/tools/`):**
- `tools/argument-comment-lint/`: Lint enforcer for `/*param_name*/` argument comments (ATA convention, mentioned in AGENTS.md)

**Upstream Equivalents:**
- `codex-rs/tools/`: Contains prompt-inspector, rollout-analyzer (both shared with local)

**Status vs Upstream:** **BOTH have implementations** — local adds argument-comment-lint

**Key Differences:**
- Local: Adds custom lint for ATA code style (argument comments)
- Upstream: Broader tool suite (prompt-inspector, rollout-analyzer)

**Merge Plan:**  
Keep argument-comment-lint; it's a localized coding convention. When syncing upstream tools, incorporate new utilities (e.g., if upstream adds prompt validation tools, integrate them).

---

#### 10. **third_party/** Directory (External Libraries)

**Name:** Third-Party Library Vendoring

**Implementation Summary:**

**Local Fork (`/Users/huytho_ho/acli/ata/third_party/`):**
- `third_party/meriyah/`: JavaScript parser library (vendored)
- `third_party/wezterm/`: WezTerm terminal emulator resources (vendored)

**Upstream (`rust-v0.129.0`):**
- `third_party/v8/`: V8 JavaScript engine (vendored)
- `third_party/wezterm/`: Same WezTerm

**Status vs Upstream:** **BOTH have implementations** — different libraries

**Key Differences:**
- Local: Uses meriyah (JavaScript parser) + wezterm
- Upstream: Uses v8 (JavaScript engine) + wezterm

**Merge Plan:**  
These are independent selections. Keep local choices. If upstream significantly improves v8 integration, evaluate whether to add v8 alongside meriyah. No conflict expected (different use cases).

---

## Merge Conflict Risk Assessment

| Component | Risk Level | Notes |
|-----------|-----------|-------|
| **codex-scheduler** | Very Low | New crate, no upstream equivalent; zero conflict surface |
| **codex-workspace** | Very Low | New crate, no upstream equivalent; orthogonal to collaboration-mode-templates |
| **codex-package-manager** | Very Low | New crate, no upstream equivalent; used by SDKs/shell-tool-mcp |
| **SDK (Python/TypeScript)** | Low | Shared structure, only branding/naming differs; easy rebase |
| **codex-cli wrapper** | Low | Branding only; source code identical to upstream |
| **shell-tool-mcp** | Low | Different implementation (TS vs Rust), non-overlapping with upstream shell-escalation |
| **Root config files** | Medium | Need careful merge to preserve ATA features (test targets, crate registrations, guidance); adopt upstream as base, re-layer ATA additions |
| **scripts/** | Low | Orthogonal distribution model; minimal upstream churn expected |
| **tools/** | Very Low | Isolated to ATA conventions; upstream tools coexist |
| **third_party/** | Very Low | Different selections; no direct overlap |

---

## Integration Summary

### When Merging Upstream → Local:

1. **Preserve new crates**: Ensure scheduler, codex-workspace, package-manager remain in Cargo.toml members and Bazel workspace
2. **Branding**: Update any new SDK/CLI references from `@openai/codex` → `@a2a-ai/ata`
3. **Configuration files**:
   - Start with upstream's `justfile`, `defs.bzl`, `AGENTS.md`, `README.md`
   - Re-add ATA-specific test targets and guidance sections
   - Re-register new crates in build files
4. **shell-tool-mcp**: Monitor upstream's shell-escalation crate for improvements; decide whether to port features to local TypeScript MCP
5. **SDKs**: Update generated models from upstream schema; keep TypeScript SDK wrapper

### Recommended Merge Strategy:

```
1. Merge upstream changes into temporary branch
2. Restore codex-scheduler, codex-workspace, package-manager crate entries to Cargo.toml/Bazel
3. Reapply ATA branding (README, AGENTS.md, package names)
4. Re-add ATA-specific test/build recipes
5. Verify compilation and test suite
6. Fast-forward local main
```

---

## Conclusion

The fork successfully maintains **3 new strategic capabilities** (scheduler, workspace management, package manager) that are local-only and pose minimal merge risk. Existing shared areas (SDKs, CLI wrappers, shell-tool-mcp) diverge only in branding and naming, making rebases straightforward. The recommended merge strategy is to adopt upstream as the baseline structure, then carefully re-layer ATA-specific additions, ensuring the 3 new crates remain integrated.
