set working-directory := "codex-rs"
set positional-arguments

# Display help
help:
    just -l

# `codex`
alias c := codex
codex *args:
    cargo run --bin codex -- "$@"

# `codex exec`
exec *args:
    cargo run --bin codex -- exec "$@"

# Run the CLI version of the file-search crate.
file-search *args:
    cargo run --bin codex-file-search -- "$@"

# Build the CLI and run the app-server test client
app-server-test-client *args:
    cargo build -p codex-cli
    cargo run -p codex-app-server-test-client -- --codex-bin ./target/debug/codex "$@"

# format code
fmt:
    cargo fmt -- --config imports_granularity=Item 2>/dev/null

fix *args:
    cargo clippy --fix --tests --allow-dirty "$@"

# Lint without --all-features
fix-fast *args:
    cargo clippy --fix --tests --allow-dirty "$@"

clippy:
    cargo clippy --tests "$@"

# Lint without --all-features
clippy-fast *args:
    cargo clippy --tests "$@"

install:
    rustup show active-toolchain
    cargo fetch

# Run `cargo nextest` since it's faster than `cargo test`, though including
# --no-fail-fast is important to ensure all tests are run.
#
# Run `cargo install cargo-nextest` if you don't have it installed.
# Prefer this for routine local runs; use explicit `cargo test --all-features`
# only when you specifically need full feature coverage.
test:
    cargo nextest run --no-fail-fast

# Test all workspace members
test-all:
    cargo nextest run --workspace --no-fail-fast

# Reading view unit + E2E tests (no API key needed)
test-reading-view:
    cargo test -p codex-tui --features voice-input --lib -- document_reader alignment_ find_word_ highlight_

# Karaoke pipeline integration tests (no API key needed)
test-karaoke:
    cargo test -p codex-tui --features voice-input --lib -- alignment_ find_word_ highlight_ voice_progress_
    cargo test -p codex-tui --test karaoke_integration

# Live TTS E2E tests (requires ELEVENLABS_API_KEY env var)
test-tts-live:
    cargo test -p codex-tui --test tts_e2e -- --ignored

# TTS/karaoke sync report (requires ELEVENLABS_API_KEY env var)
# Produces /tmp/tts-sync-report.md for agent-driven verification
test-tts-sync:
    cargo test -p codex-tui --features voice-input --test tts_sync_report -- --ignored --nocapture
    @echo "Report: /tmp/tts-sync-report.md"

# Build and run Codex from source using Bazel.
# Note we have to use the combination of `[no-cd]` and `--run_under="cd $PWD &&"`
# to ensure that Bazel runs the command in the current working directory.
[no-cd]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under="cd $PWD &&" -- "$@"

[no-cd]
bazel-lock-update:
    bazel mod deps --lockfile_mode=update

[no-cd]
bazel-lock-check:
    ./scripts/check-module-bazel-lock.sh

bazel-test:
    bazel test //... --keep_going

bazel-remote-test:
    bazel test //... --config=remote --platforms=//:rbe --keep_going

build-for-release:
    bazel build //codex-rs/cli:release_binaries --config=remote

# Run the MCP server
mcp-server-run *args:
    cargo run -p codex-mcp-server -- "$@"

# Regenerate the json schema for config.toml from the current config types.
write-config-schema:
    cargo run -p codex-core --bin codex-write-config-schema

# Regenerate vendored app-server protocol schema artifacts.
write-app-server-schema *args:
    cargo run -p codex-app-server-protocol --bin write_schema_fixtures -- "$@"

[no-cd]
write-hooks-schema:
    cargo run --manifest-path ./codex-rs/Cargo.toml -p codex-hooks --bin write_hooks_schema_fixtures

# Run the argument-comment Dylint checks across codex-rs.
[no-cd]
argument-comment-lint *args:
    ./tools/argument-comment-lint/run.sh "$@"

# Tail logs from the state SQLite database
log *args:
    if [ "${1:-}" = "--" ]; then shift; fi; cargo run -p codex-state --bin logs_client -- "$@"

# Verify OpenAI model-version behavior before publishing a release.
# Runs core regression tests, validates launcher JS syntax, and stages an npm
# tarball for release smoke checks.
verify-openai-model-override release_version="0.1.0-rc.1":
    #!/usr/bin/env bash
    set -euo pipefail

    cargo test -p codex-core config_schema_matches_fixture -- --nocapture
    cargo test -p codex-core test_precedence_fixture_with_gpt5_profile -- --nocapture
    cargo test -p codex-core refresh_available_models_uses_default_client_version -- --nocapture
    cargo test -p codex-core refresh_available_models_refetches_when_version_mismatch -- --nocapture

    node --check ../codex-cli/bin/ata.js

    stage_dir="$(mktemp -d)"
    out_tgz="$(mktemp /tmp/ata-npm-XXXXXX.tgz)"
    trap 'rm -rf "${stage_dir}" "${out_tgz}"' EXIT

    NPM_CONFIG_CACHE=/tmp/npm-cache ../codex-cli/scripts/build_npm_package.py \
      --package ata \
      --release-version "{{release_version}}" \
      --staging-dir "${stage_dir}" \
      --pack-output "${out_tgz}"

    echo "PASS: OpenAI model override verification and npm package staging complete"

# Launch prompt inspector in neovim
prompts:
    nvim --clean --cmd "set rtp+=tools/prompt-inspector/plugin" -c "lua require('prompt-inspector').setup({codex_root=vim.fn.getcwd()})" -c "PromptInspector"

# Validate prompt registry and @agent-facing annotations
check-prompts:
    python3 tools/prompt-inspector/validate.py --codex-root .

# Dump full assembled agent context to terminal
dump-context:
    cargo run -p codex-cli -- debug dump-initial-context

# ---------------------------------------------------------------------------
# Release branch sync
# ---------------------------------------------------------------------------

# Private directories to strip when syncing main -> release.
_release_private_paths := "codex-rs/ata-plus codex-rs/supabase codex-rs/coordination codex-rs/coordination-relay codex-rs/core/templates/coordination codex-rs/skills/src/assets/remote-exec"

# Sync main -> release (safe: copies files, no history link).
# 1. Copies all files from main (no merge, no history connection)
# 2. Removes private directories
# 3. Patches Cargo.toml files to remove references to private crates
# 4. Verifies workspace compiles
# 5. Commits
#
# Shared .rs files keep their #[cfg(feature = "ata-plus")] blocks — those
# are dead code on release since the feature and crates don't exist.
sync-release:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"

    # Must be on release branch
    current=$(git branch --show-current)
    if [ "$current" != "release" ]; then
        echo "ERROR: must be on release branch (currently on $current)"
        echo "Run: git checkout release"
        exit 1
    fi

    echo "==> Copying all files from main..."
    git checkout main -- .

    echo "==> Removing private directories..."
    for p in {{_release_private_paths}}; do
        git rm -rf "$p" 2>/dev/null || true
    done

    echo "==> Patching Cargo.toml files (removing private crate references)..."

    # --- Workspace Cargo.toml: remove private members and deps ---
    sed -i '' \
        -e '/"ata-plus",/d' \
        -e '/"coordination",/d' \
        -e '/"coordination-relay",/d' \
        codex-rs/Cargo.toml
    sed -i '' \
        -e '/^ata-plus = { path = "ata-plus" }/d' \
        -e '/^codex-coordination-relay = { path = "coordination-relay" }/d' \
        -e '/^codex-coordination = { path = "coordination" }/d' \
        codex-rs/Cargo.toml

    # --- cli/Cargo.toml ---
    sed -i '' \
        -e '/^ata-plus = { workspace = true, optional = true }/d' \
        -e '/^ata-plus = \[/d' \
        -e '/^relay = \["codex-core\/relay"/d' \
        codex-rs/cli/Cargo.toml

    # --- tui/Cargo.toml ---
    sed -i '' \
        -e '/^ata-plus = { workspace = true, optional = true }/d' \
        -e '/^ata-plus = \[/d' \
        -e '/^relay = \[\]/d' \
        codex-rs/tui/Cargo.toml

    # --- exec/Cargo.toml ---
    sed -i '' \
        -e '/^ata-plus = { workspace = true, optional = true }/d' \
        -e '/^codex-coordination = { workspace = true, optional = true }/d' \
        -e '/^ata-plus = \[/d' \
        -e '/^relay = \[\]/d' \
        codex-rs/exec/Cargo.toml

    # --- core/Cargo.toml ---
    sed -i '' \
        -e '/^codex-coordination = { workspace = true, optional = true }/d' \
        -e '/^ata-plus = \[/d' \
        -e '/^relay = \["codex-coordination/d' \
        codex-rs/core/Cargo.toml

    echo "==> Verifying workspace compiles..."
    cd codex-rs && cargo check --workspace 2>&1 | tail -10
    cd ..

    echo "==> Staging and committing..."
    git add -A
    if git diff --cached --quiet; then
        echo "Nothing to sync — release is up to date with main."
    else
        git commit -m "sync: merge public-safe changes from main"
        echo "==> Done. Review with: git log --oneline -1"
        echo "    Push with: git push public release:main"
    fi
