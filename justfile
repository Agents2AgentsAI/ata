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
    cargo clippy --fix --all-features --tests --allow-dirty "$@"

# Lint without --all-features
fix-fast *args:
    cargo clippy --fix --tests --allow-dirty "$@"

clippy:
    cargo clippy --all-features --tests "$@"

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
test:
    cargo nextest run --no-fail-fast

# Test all workspace members
test-all:
    cargo nextest run --workspace --no-fail-fast

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
