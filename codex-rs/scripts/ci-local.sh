#!/usr/bin/env bash
#
# Run the local equivalent of CI checks against the codex-rs workspace.
#
# Steps (in order):
#   1. just fmt                                    (rustfmt check)
#   2. just clippy                                 (workspace clippy, default features)
#   3. cargo clippy --all-features                 (mirrors ata-ci-platform clippy; catches cross-workspace path-dep issues)
#   4. codespell                                   (mirrors .github/workflows/codespell.yml)
#   5. verify_cargo_workspace_manifests.py         (Cargo.toml policy)
#   6. cargo shear                                 (unused deps; pinned to 1.5.1 in CI)
#   7. cargo deny check                            (advisories / licenses / bans / sources)
#   8. cargo nextest run --workspace --all-features (full test suite, matches CI nextest invocation)
#
# CI also runs `cargo chef cook` on release-profile matrix rows; we don't
# reproduce that locally because cargo-chef rewrites every workspace crate's
# src/lib.rs into an empty stub in place (requires `git checkout` to restore).
# The workflow fix to restore source post-cook lives in
# .github/workflows/ata-ci-platform.yml; a local regression would show up as
# the `cargo clippy --all-features` step (3) failing anyway.
#
# By default the script is fail-fast. Pass --no-fail-fast to run every step and
# report a final summary. Pass --skip-tests to stop before step 8 when you just
# want the fast lints. Pass --tests-only to jump straight to the nextest step.

set -uo pipefail

fail_fast=1
skip_tests=0
tests_only=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-fail-fast)
      fail_fast=0
      shift
      ;;
    --skip-tests)
      skip_tests=1
      shift
      ;;
    --tests-only)
      tests_only=1
      shift
      ;;
    -h|--help)
      sed -n '2,24p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# Resolve the repo layout.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cargo_rs_dir="$(cd "${script_dir}/.." && pwd)"
repo_root="$(cd "${cargo_rs_dir}/.." && pwd)"
verify_script="${repo_root}/.github/scripts/verify_cargo_workspace_manifests.py"

declare -a step_names
declare -a step_status

cyan=$'\033[0;36m'
green=$'\033[0;32m'
red=$'\033[0;31m'
bold=$'\033[1m'
reset=$'\033[0m'

run_step() {
  local name="$1"
  shift

  step_names+=("$name")

  printf '\n%s==> %s%s\n' "${cyan}${bold}" "$name" "${reset}"
  printf '    %s\n' "$*"

  local start_ts
  start_ts=$(date +%s)

  if "$@"; then
    local elapsed=$(( $(date +%s) - start_ts ))
    printf '%s    ✓ %s (%ds)%s\n' "${green}" "$name" "$elapsed" "${reset}"
    step_status+=("pass")
    return 0
  fi

  local rc=$?
  local elapsed=$(( $(date +%s) - start_ts ))
  printf '%s    ✗ %s failed (rc=%d, %ds)%s\n' "${red}" "$name" "$rc" "$elapsed" "${reset}"
  step_status+=("fail")
  if [[ $fail_fast -eq 1 ]]; then
    summarize
    exit "$rc"
  fi
  return "$rc"
}

summarize() {
  printf '\n%s==> Summary%s\n' "${cyan}${bold}" "${reset}"
  local any_fail=0
  local i
  for i in "${!step_names[@]}"; do
    if [[ "${step_status[$i]}" == "pass" ]]; then
      printf '  %s✓%s %s\n' "${green}" "${reset}" "${step_names[$i]}"
    else
      printf '  %s✗%s %s\n' "${red}" "${reset}" "${step_names[$i]}"
      any_fail=1
    fi
  done
  if [[ $any_fail -ne 0 ]]; then
    printf '%sone or more steps failed%s\n' "${red}" "${reset}"
  fi
}

require() {
  local name="$1" hint="$2"
  if ! command -v "$name" >/dev/null 2>&1; then
    printf '%s==> Missing dependency: %s%s\n' "${red}" "$name" "${reset}"
    printf '    install hint: %s\n' "$hint"
    exit 127
  fi
}

prepare_cargo_deny_env() {
  local default_cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
  local advisory_dir="${default_cargo_home}/advisory-dbs"

  if mkdir -p "${advisory_dir}" 2>/dev/null && [[ -w "${advisory_dir}" ]]; then
    return 0
  fi

  export CARGO_HOME="${TMPDIR:-/tmp}/codex-cargo-deny-home"
  mkdir -p "${CARGO_HOME}/advisory-dbs"
}

# Mirror the rusty_v8 archive handling in .github/workflows/ata-ci-platform.yml:
# only the musl targets need an override (denoland/rusty_v8 ships no musl
# prebuilt; the openai/codex release republishes a plain `release` musl
# variant). Every other target — including this dev machine — falls back to the
# v8 build script's default denoland `release` download. Neither archive
# enables `v8_enable_sandbox`, which is what `codex-v8-poc`'s
# `linked_v8_has_no_sandbox` test asserts.
prepare_rusty_v8_env() {
  local target
  target="$(rustc -vV | awk -F': ' '/^host:/ {print $2}')"
  case "${target}" in
    *-linux-musl) ;;
    *) return 0 ;;
  esac
  if [[ -n "${RUSTY_V8_ARCHIVE-}" && -n "${RUSTY_V8_SRC_BINDING_PATH-}" ]]; then
    return 0
  fi
  local version
  version="$(python3 "${repo_root}/.github/scripts/rusty_v8_bazel.py" resolved-v8-crate-version)"
  local base_url="https://github.com/openai/codex/releases/download/rusty-v8-v${version}"
  local binding_dir="${TMPDIR:-/tmp}/codex-rusty-v8-${version}"
  local binding_path="${binding_dir}/src_binding_release_${target}.rs"
  mkdir -p "${binding_dir}"
  if [[ ! -f "${binding_path}" ]]; then
    curl -fsSL "${base_url}/src_binding_release_${target}.rs" -o "${binding_path}"
  fi
  export RUSTY_V8_ARCHIVE="${base_url}/librusty_v8_release_${target}.a.gz"
  export RUSTY_V8_SRC_BINDING_PATH="${binding_path}"
}

cd "${cargo_rs_dir}"

# Mirror the `tests` job env in .github/workflows/ata-ci-platform.yml: a larger
# thread stack is needed both for rustc on `--all-features` builds and for the
# test runtime itself (several agent/session-spawn futures overflow the default
# 2 MiB thread stack otherwise).
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"

if [[ $tests_only -eq 0 ]]; then
  require just "brew install just"
  require python3 "install Python 3.x"
  require cargo "install the Rust toolchain"
  require codespell "brew install codespell (or pipx install codespell)"
  require cargo-shear "cargo install cargo-shear --version 1.5.1 (version pinned by CI)"
  require cargo-deny "cargo install --locked cargo-deny"

  run_step "just fmt" just fmt
  run_step "just clippy" just clippy
  prepare_rusty_v8_env
  run_step "cargo clippy --all-features" \
    cargo clippy --workspace --all-features --tests -- -D warnings
  # codespell runs from the repo root so it sees .codespellrc and .codespellignore.
  # Skip node_modules/target/dist/build locally since CI runs on a fresh
  # checkout that doesn't have them populated.
  run_step "codespell" \
    env -C "${repo_root}" codespell --ignore-words .codespellignore \
      --skip '*/node_modules/*,*/target/*,*/dist/*,*/build/*'
  run_step "verify cargo workspace manifests" python3 "${verify_script}"
  run_step "cargo shear" cargo shear
  prepare_cargo_deny_env
  run_step "cargo deny check" cargo-deny --log-level warn --manifest-path ./Cargo.toml --all-features check
fi

if [[ $skip_tests -eq 0 ]]; then
  require cargo-nextest "cargo install cargo-nextest --locked"
  # nextest with --all-features needs the same rusty_v8 override as the
  # clippy step above; safe to call again (no-op if already exported).
  prepare_rusty_v8_env
  # Matches ata-ci-platform.yml tests job: --all-features --no-fail-fast.
  run_step "cargo nextest" cargo nextest run --workspace --all-features --no-fail-fast -j 4
fi

summarize

for status in "${step_status[@]}"; do
  if [[ "$status" == "fail" ]]; then
    exit 1
  fi
done
exit 0
