## Build/Infra Analysis (Agent 8)

### 1. Bazel toolchain stack (Local-only — major divergence)
- **Type**: Local-only / fork-specific (heavily slimmed from upstream)
- **Description**: The fork keeps a far smaller Bazel surface than upstream. Upstream rust-v0.129.0 ships ~531-line `MODULE.bazel`, ~544-line `defs.bzl`, ~197-line `.bazelrc`, with full V8/rusty-v8/Windows-MSVC support and many extra crate annotations. Our fork uses a stripped configuration: 185-line `MODULE.bazel`, 265-line `defs.bzl`, 62-line `.bazelrc`, no V8.
- **Implementation**: `MODULE.bazel`, `defs.bzl`, `.bazelrc`, `BUILD.bazel`, `rbe.bzl`, `MODULE.bazel.lock`.
- **Merge plan**: Treat upstream Bazel as authoritative if we want to keep Bazel viable; otherwise prefer to drop Bazel from the fork entirely (the fork already removes most upstream Bazel CI). Recommendation: keep current minimal Bazel (it builds the trimmed crate set) and selectively port any non-V8 annotations upstream added. Do NOT pull in upstream's V8/rusty-v8/coreaudio annotations — fork doesn't have those crates.

### 2. Patches directory (Shared but heavily reduced)
- **Type**: Shared — but only 3 patches in fork vs 25 upstream
- **Description**: Upstream ships 25 patches for windows-msvc, abseil, aws-lc, bzip2, llvm, ring, rules_rs/rules_rust on windows, rusty_v8, v8, webrtc-sys, xz, zstd-sys. Fork keeps just `aws-lc-sys_memcmp_check.patch` plus a fork-only `toolchains_llvm_bootstrapped_resource_dir.patch`, and removes all V8/Windows-MSVC patches. Fork drops upstream's `windows-link.patch`.
- **Implementation**: `patches/` (3 patches + empty `BUILD.bazel`).
- **Merge plan**: Keep fork's `toolchains_llvm_bootstrapped_resource_dir.patch` (referenced by the fork's MODULE.bazel via custom toolchain config). Skip all upstream Windows-MSVC and V8 patches — the fork uses windows-gnullvm and has no V8.

### 3. justfile recipes (Shared, fork-extended)
- **Type**: Shared with substantial fork additions
- **Description**: Fork's `justfile` is 8.7KB vs upstream's smaller version. Fork-only recipes: `test-reading-view`, `test-karaoke`, `test-tts-live`, `test-tts-sync`, `write-hooks-schema`, `argument-comment-lint`, `log`, `verify-openai-model-override`, `prompts`, `check-prompts`, `dump-context`, `sync-release` (private→public branch sync with sed-based privatization). Fork removes upstream's `tui-with-exec-server` recipe.
- **Implementation**: `justfile`.
- **Merge plan**: Keep all fork-only recipes. The `sync-release` recipe encodes the public/private split contract — must be preserved exactly. Adopt any new upstream recipes that don't conflict.

### 4. GitHub Actions workflows — heavy pruning + fork-only flows
- **Type**: Shared (modified) + Local-only
- **Description**: Fork removes upstream workflows: `rust-ci-full.yml` (770 lines), `rust-release-prepare.yml`, `rust-release-zsh.yml`, `rust-release-argument-comment-lint.yml`, `rusty-v8-release.yml`, `v8-canary.yml`, `issue-deduplicator.yml`, `issue-labeler.yml`. Fork-only workflows: `keyword-scan.yml` (scans for sentry.io/statsig/chatgpt.com/CODEX_OPENAI_API_KEY before publish), `shell-tool-mcp.yml` (563 lines, full release pipeline), `shell-tool-mcp-ci.yml`, `ci.bazelrc` (CI-specific bazel config). Modified: `rust-release.yml` (massive rewrite — fork uses `runs-on.yml` self-hosted runners, drops Apple signing as TODO, drops app-server bundle), `rust-ci.yml` (path-filtered triggers, concurrency cancel-in-progress, removes `argument_comment_lint` jobs), `bazel.yml` (renamed "Bazel (experimental)", strips down to a single Linux job vs upstream's multi-platform RBE matrix).
- **Implementation**: `.github/workflows/*`.
- **Merge plan**: Keep all fork-only workflows. Reject upstream additions that depend on V8/dotslash-zsh/issue-labeler infra. Carefully forward-port any upstream `rust-release.yml` security or signing improvements once the fork's signing story is sorted.

### 5. RunsOn self-hosted runner config (Local-only)
- **Type**: Local-only
- **Description**: Fork uses RunsOn-managed AWS runners (vs upstream's GitHub-hosted). Defines `ci-linux-x64`, `ci-linux-arm64`, `ci-windows-x64` (spot, cost-optimized) and `release-linux-*`, `release-windows-x64` (on-demand, larger). Custom AMI: `windows25-vstoolchain-x64`.
- **Implementation**: `.github/runs-on.yml`.
- **Merge plan**: Preserve. Upstream has no equivalent — keep entirely.

### 6. GitHub Actions infra cleanup (Local-only deletions)
- **Type**: Local-only deletions of upstream files
- **Description**: Fork deletes `.github/CODEOWNERS`, `.github/ISSUE_TEMPLATE/*` (all 6), `.github/dependabot.yaml`, `.github/dotslash-argument-comment-lint-config.json`, `.github/dotslash-zsh-config.json`, `.github/blob-size-allowlist.txt` (modified), `.github/codex-cli-splash.png` (replaced with `cli-splash.png`), `.github/actions/prepare-bazel-ci`, `.github/actions/setup-bazel-ci`, `.github/actions/setup-rusty-v8-musl`, `.github/actions/run-argument-comment-lint`, `.github/actions/macos-code-sign/codex.entitlements.plist`, `.github/scripts/{run-bazel-ci.sh,run-bazel-query-ci.sh,rusty_v8_*.py,verify_*.py,build-zsh-release-artifact.sh,run-argument-comment-lint-bazel.sh,compute-bazel-windows-path.ps1}`. Fork-only: `.github/prompts/issue-labeler.txt`.
- **Merge plan**: Confirm these stay deleted. The reusable Bazel/v8 actions are tied to upstream's RBE+V8 setup which fork doesn't run.

### 7. dotslash configuration rewrite (Shared, rebranded)
- **Type**: Shared (heavy rewrite)
- **Description**: `dotslash-config.json` rewritten to publish `ata`/`ata-responses-api-proxy` instead of `codex`/`codex-responses-api-proxy`. Fork drops upstream's `dotslash-zsh-config.json` (no zsh release in fork) and `dotslash-argument-comment-lint-config.json`.
- **Implementation**: `.github/dotslash-config.json`.
- **Merge plan**: Preserve fork's rebranding. Skip upstream zsh/lint dotslash artifacts.

### 8. codex-cli npm package (Shared, rebranded)
- **Type**: Shared (rebranded `@openai/codex` → `@a2a-ai/ata`)
- **Description**: `codex-cli/package.json` renames package to `@a2a-ai/ata`, binary entry from `codex` to `ata` (`bin/ata.js`), pnpm version pinned to `10.29.3` vs upstream `10.33.0`. `codex-cli/scripts/build_npm_package.py` is rewritten with `ATA_*` constants and drops upstream's `win32-arm64` platform tier and the `codex-app-server` / `codex-windows-sandbox-setup` / `codex-command-runner` binaries (fork ships fewer per-package binaries). Fork adds `codex-cli/Dockerfile`, `codex-cli/.dockerignore`, `codex-cli/scripts/build_container.sh` (none in upstream).
- **Implementation**: `codex-cli/{package.json,Dockerfile,.dockerignore,scripts/build_npm_package.py,scripts/build_container.sh}`, `scripts/stage_npm_packages.py`.
- **Merge plan**: Carefully forward-port any new upstream platform tiers if fork wants Windows ARM64. Keep the rebrand. Watch for upstream changes to `install_native_deps.py` — fork has divergence here too.

### 9. SDK packages (Shared, with fork-only branches)
- **Type**: Shared / Local-only mix
- **Description**: 
  - `sdk/typescript/`: rebranded to `@a2a-ai/ata-sdk` (upstream is `@openai/codex-sdk`). Fork-only file: `sdk/typescript/src/ataOptions.ts`.
  - `sdk/python/`: shared `codex-app-server-sdk` package; fork adds generated `v2_types.py`.
  - `sdk/python-runtime/`: hatch-built `codex-cli-bin` runtime wheel (custom hook for platform wheels); shared.
- **Implementation**: `sdk/`.
- **Merge plan**: Keep fork rebrand and `ataOptions.ts`. Re-run upstream's SDK code generators whenever app-server proto changes land.

### 10. shell-tool-mcp package (Local-only — fully fork-specific)
- **Type**: Local-only
- **Description**: A fork-only TypeScript MCP server packaged via tsup, including its own bash/zsh exec patches in `shell-tool-mcp/patches/`, jest tests, dedicated CI (`shell-tool-mcp-ci.yml`) and release (`shell-tool-mcp.yml`) pipelines. Added to `pnpm-workspace.yaml`.
- **Implementation**: `shell-tool-mcp/` (package.json, src/, tests/, patches/, tsconfig.json, tsup.config.ts, jest.config.cjs).
- **Merge plan**: Preserve in full — has zero upstream counterpart. Keep the `shell-tool-mcp` entry in `pnpm-workspace.yaml`.

### 11. tools/argument-comment-lint (Local-only Dylint plugin)
- **Type**: Local-only
- **Description**: Fork keeps a Dylint-based lint crate at `tools/argument-comment-lint` (cdylib, uses `clippy_utils`, `dylint_linting`). Driven by `tools/argument-comment-lint/run.sh` and the `argument-comment-lint` just recipe. Upstream has its own argument-comment-lint setup wired through Bazel/dotslash that fork dropped.
- **Implementation**: `tools/argument-comment-lint/{Cargo.toml,src/,run.sh,README.md,rust-toolchain}`.
- **Merge plan**: Keep fork's Cargo-driven version; reject upstream's Bazel-wrapped variant.

### 12. third_party trees (Local-only minimal vs upstream V8 vendor)
- **Type**: Both have third_party but contents differ
- **Description**: Upstream's `third_party/` has `v8` (full V8 vendor tree) and `wezterm`. Fork has `meriyah` (license only) and `wezterm` (license only). Fork dropped V8 entirely; added meriyah (used by `codex-rs/responses-api-proxy/npm`).
- **Implementation**: `third_party/`.
- **Merge plan**: Keep fork tree. Continue ignoring upstream V8 vendor — pulling it in would explode build size and reintroduce v8/rusty_v8 patches.

### 13. .devcontainer (Local-only simplification)
- **Type**: Shared (heavy reduction)
- **Description**: Fork keeps only `Dockerfile`, `devcontainer.json`, `README.md`. Drops upstream's `Dockerfile.secure`, `devcontainer.secure.json`, `init-firewall.sh`, `post-start.sh`, `post_install.py`, `codex-install/` subpackage.
- **Implementation**: `.devcontainer/`.
- **Merge plan**: Don't pull upstream's secure variant unless we need it; fork's simpler container is intentional.

### 14. Top-level scripts (Shared with fork additions)
- **Type**: Shared / Local-only
- **Description**: Fork-added: `scripts/install/install.sh`, `scripts/install/install.ps1` (curl-pipe installers for `agents2agents.ai/ata/install.sh`), `scripts/install.sh` (additional shim), `scripts/stage_npm_packages.py` (modified). Fork-deleted: `scripts/list-bazel-clippy-targets.sh`, `scripts/list-bazel-release-targets.sh`, `scripts/run_tui_with_exec_server.sh`, `scripts/start-codex-exec.sh`, `scripts/test-remote-env.sh`. Shared: `asciicheck.py`, `check-module-bazel-lock.sh`, `check_blob_size.py`, `debug-codex.sh`, `mock_responses_websocket_server.py`, `readme_toc.py`.
- **Implementation**: `scripts/`.
- **Merge plan**: Preserve installer scripts; let upstream's deleted scripts stay deleted (they support V8/Bazel CI features the fork doesn't have).

### 15. Workspace `Cargo.toml` divergence (Shared, very different members)
- **Type**: Shared (~295-line diff)
- **Description**: Upstream has many crates the fork removed: `aws-auth`, `analytics`, `agent-graph-store`, `agent-identity`, `bwrap`, `app-server-transport`, `builtin-mcps`, `cloud-tasks-mock-client`, `code-mode`, `collaboration-mode-templates`, `core-api`, `core-plugins`, `core-skills`, `device-key`, `exec-server`, `external-agent-migration/sessions`, `features`, `file-system`, `install-context`, `memories/{mcp,read,write}`, `model-provider-info`, `models-manager`, `realtime-webrtc`, `rollout`, `rollout-trace`, `response-debug-context`, `sandboxing`, `terminal-detection`, `test-binary-support`, `thread-manager-sample`, `thread-store`, `uds`, `tools`, `v8-poc`, `git-utils`, `plugin`, `model-provider`. Fork-only members: `lsp-client`, `treesitter`, `codex-research-tools`, `codex-data-tools`, `codex-elevenlabs`, `test-macros`, `scheduler`, `codex-workspace`, `package-manager`, `artifacts`, `reading-view-server`, `utils/file`. Renames: `git-utils` → `utils/git`. Workspace version pinned at `0.0.0` (upstream `0.129.0`). Fork removes `codex-utils-readiness/-template/-v8-poc` from `default-members`. Fork uses `[profile.dev] split-debuginfo="unpacked"` + `debug="line-tables-only"` (upstream uses `debug=1` plus a `dev-small` profile). Fork pins `crossterm`/`ratatui` to specific revs (upstream tracks branches). Fork-only deps: `fd-lock`, `mdns-sd`, `pdfium-render`, `qrcode`, `redis`, `tower-http`. Fork drops upstream-only deps: `crypto_box`, `deno_core_icudata`, `dns-lookup`, `ed25519-dalek`, `gix`, `glob`, `hmac`, `jsonwebtoken`, `p256`, `quick-xml`, `rcgen`, `tonic`, `tonic-prost`, `v8`, `whoami`, `winapi-util`. Fork removes the `await_holding_*` clippy denies.
- **Implementation**: `codex-rs/Cargo.toml`.
- **Merge plan**: This is the central conflict surface. Manually merge: keep all fork-only members and fork-only dep entries; for upstream-added deps, only adopt if needed by an upstream change you're pulling. Re-pin `crossterm`/`ratatui` once fork's voice/reading-view code is verified against new upstream pins. Restore the `await_holding_*` clippy denies if the codebase is ready (upstream already enforces them).

### 16. Root `package.json` resolutions (Shared, fork-trimmed)
- **Type**: Shared (rebrand+trim)
- **Description**: Fork drops upstream's heavy resolutions block (`@modelcontextprotocol/sdk`, `flatted`, `glob`, `handlebars`, `minimatch`, `path-to-regexp`, `picomatch`, `rollup`) keeping only `braces`, `micromatch`, `semver`. Fork pins pnpm to `10.29.3` (upstream `10.33.0`).
- **Implementation**: `package.json`.
- **Merge plan**: Decide whether to bump pnpm to upstream's `10.33.0` (would also need to update workspace lockfile). Restore upstream resolutions only if a fork-pulled dep regresses.

### 17. pnpm-workspace policy hardening (Local-only restraint)
- **Type**: Shared (fork removes some upstream policies)
- **Description**: Fork drops upstream's `strictDepBuilds: true`, `trustPolicy: no-downgrade`, `trustPolicyIgnoreAfter`, `trustPolicyExclude`, `allowBuilds`, `minimumReleaseAgeExclude` keys. Fork adds `shell-tool-mcp` to packages list.
- **Implementation**: `pnpm-workspace.yaml`.
- **Merge plan**: Consider re-adding upstream's trust-policy hardening — they're cheap supply-chain wins. Keep `shell-tool-mcp` in packages list.

### 18. UPSTREAM.md provenance ledger (Local-only)
- **Type**: Local-only
- **Description**: Tracks fork releases against upstream commit SHAs (v0.1.0 through v0.3.3 currently).
- **Implementation**: `UPSTREAM.md`.
- **Merge plan**: Add a new entry for the v0.129.0 sync as part of this merge.

### Key takeaways for merge planning
- Bazel and V8 are the two largest upstream divergence vectors; the fork has deliberately walked away from both. Resist any upstream change that drags V8 back in (including `code-mode`, `v8-poc`, `realtime-webrtc`, `tonic` deps, V8 patches, rusty_v8 actions/scripts).
- Workspace `Cargo.toml` is the messiest 3-way merge surface — the upstream renamed `git-utils` to `utils/git`, which the fork already did; check that fork's path matches upstream's exact value.
- The fork's release/CI pipeline is fully self-hosted (RunsOn) and rebrands every artifact (`codex` → `ata`, `@openai/codex` → `@a2a-ai/ata`); merging upstream release changes requires re-rebrand each time.
- Preserve at all costs: `runs-on.yml`, `keyword-scan.yml`, `shell-tool-mcp/`, `shell-tool-mcp.yml`, `shell-tool-mcp-ci.yml`, the `sync-release` just recipe, fork's installer scripts, `sdk/typescript/src/ataOptions.ts`, `tools/argument-comment-lint/`, `UPSTREAM.md`, `codex-cli/Dockerfile*`, fork's `dotslash-config.json`, `patches/toolchains_llvm_bootstrapped_resource_dir.patch`.
