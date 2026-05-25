#!/usr/bin/env bash
#
# Tier A runner — shell-only, no LLM, no TUI.
#
# Mirrors the scenarios documented in PLAN.md for TR-055, TR-056, TR-061
# A/A2/B/C. One bash function per scenario. PLAN.md is the spec; this
# script is the implementation.
#
# Exit code: 0 if all scenarios pass, 1 otherwise.
#
# Usage:
#   ./run-tier-a.sh                 # use `ata` from PATH
#   ATA_BIN=./target/debug/ata ./run-tier-a.sh

set -uo pipefail

ATA_BIN="${ATA_BIN:-ata}"
WORK=$(mktemp -d -t ata-tier-a.XXXXXX)
trap 'cleanup_all' EXIT

PASS=0
FAIL=0
FAILED_NAMES=()
CURRENT_NAME=""
CURRENT_FAILED=0

# --- helpers ---------------------------------------------------------------

log()   { printf '%s\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }

start_test() {
  CURRENT_NAME="$1"
  CURRENT_FAILED=0
  printf '  %-14s ' "$CURRENT_NAME"
}

end_test() {
  if [ "$CURRENT_FAILED" -eq 0 ]; then
    green PASS
    PASS=$((PASS + 1))
  else
    red FAIL
    FAIL=$((FAIL + 1))
    FAILED_NAMES+=("$CURRENT_NAME")
  fi
}

fail_assert() {
  CURRENT_FAILED=1
  red ""
  red "    [$CURRENT_NAME] $1"
  [ $# -ge 2 ] && red "      got: $2"
}

assert_contains() {
  local file=$1 needle=$2 desc=${3:-}
  if ! grep -qF -- "$needle" "$file"; then
    fail_assert "${desc:-expected to contain: $needle}" "$(head -c 400 "$file")"
  fi
}

assert_not_contains() {
  local file=$1 needle=$2 desc=${3:-}
  if grep -qF -- "$needle" "$file"; then
    fail_assert "${desc:-must NOT contain: $needle}" "$(head -c 400 "$file")"
  fi
}

assert_match() {
  local file=$1 regex=$2 desc=${3:-}
  if ! grep -qE -- "$regex" "$file"; then
    fail_assert "${desc:-expected to match regex: $regex}" "$(head -c 400 "$file")"
  fi
}

assert_eq() {
  local actual=$1 expected=$2 desc=${3:-equality}
  if [ "$actual" != "$expected" ]; then
    fail_assert "$desc: expected '$expected', got '$actual'"
  fi
}

assert_json() {
  local file=$1 desc=${2:-valid json}
  if ! jq -e . "$file" >/dev/null 2>&1; then
    fail_assert "$desc: jq failed to parse"
  fi
}

assert_jq() {
  local file=$1 query=$2 desc=$3
  if ! jq -e "$query" "$file" >/dev/null 2>&1; then
    fail_assert "$desc (failed query: $query)" "$(head -c 600 "$file")"
  fi
}

# Cleanup: nuke any non-global workspace created during the run.
cleanup_all() {
  local rc=$?
  rm -rf "$WORK" 2>/dev/null
  if [ -f "$HOME/.ata/config.toml.tier-a.bak" ]; then
    mv "$HOME/.ata/config.toml.tier-a.bak" "$HOME/.ata/config.toml"
  fi
  # Delete any tier-a-created workspaces left over.
  "$ATA_BIN" workspace list 2>/dev/null | jq -r '.[].id' 2>/dev/null \
    | grep -E '^(tr05[56789]|tr06[01]|bootstrap)-' \
    | while read -r wsid; do
        "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1 || true
      done
  return $rc
}

# --- TR-055: read-only inspection -----------------------------------------

tr055_a() {
  start_test "TR-055 A"
  local out=$WORK/055a.json
  "$ATA_BIN" workspace list > "$out"
  assert_json "$out" "list is valid JSON"
  assert_jq "$out" 'type == "array"' "list is an array"
  assert_jq "$out" 'length >= 1' "at least one workspace"
  assert_jq "$out" 'all(has("id") and has("name") and has("updatedAt") and has("repoCount"))' "each entry has required keys"
  end_test
}

tr055_b() {
  start_test "TR-055 B"
  local out=$WORK/055b.json
  "$ATA_BIN" workspace read > "$out"
  assert_json "$out"
  for k in schemaVersion id name createdAt updatedAt manifestVersion repos runs papers datasets artifacts links snapshots indexes policies knowledgeBase labels; do
    assert_jq "$out" "has(\"$k\")" "missing top-level key: $k"
  done
  assert_jq "$out" '.schemaVersion == 2' "schemaVersion is 2"
  assert_jq "$out" '.manifestVersion == 1' "manifestVersion is 1"
  assert_jq "$out" '.policies.defaultClone | has("depth") and has("singleBranch") and has("noTags") and has("filter") and has("submodules") and has("lfs")' "defaultClone has all policy fields"
  assert_jq "$out" '.knowledgeBase | has("path")' "knowledgeBase has path"
  end_test
}

tr055_c() {
  start_test "TR-055 C"
  local out=$WORK/055c.json
  "$ATA_BIN" workspace validate > "$out"
  assert_json "$out"
  for k in workspaceId ok missingRepos missingRuns orphanRepoDirs orphanRunDirs; do
    assert_jq "$out" "has(\"$k\")" "missing key: $k"
  done
  end_test
}

tr055_d() {
  start_test "TR-055 D"
  local out=$WORK/055d.txt
  "$ATA_BIN" workspace recipe list > "$out"
  assert_match "$out" '^Available recipes:' "first line is heading"
  for r in export export_spec import index_build link_add materialize repo_pin repo_remove repo_unpin repo_update resource_add run_delete run_exec run_gc snapshot_create snapshot_restore; do
    assert_contains "$out" "$r" "missing recipe: $r"
  done
  end_test
}

tr055_e() {
  start_test "TR-055 E"
  local out=$WORK/055e.txt
  "$ATA_BIN" workspace recipe repo_pin > "$out"
  assert_match "$out" '^#' "starts with a comment header"
  assert_contains "$out" 'ALIAS="' "contains ALIAS env-var assignment"
  assert_contains "$out" 'SHA="' "contains SHA env-var assignment"
  assert_contains "$out" 'ata workspace repo-pin --alias "$ALIAS" --sha "$SHA" --workspace "$WID"' "contains CLI invocation"
  assert_contains "$out" 'ata workspace audit --workspace "$WID"' "contains audit step"
  assert_contains "$out" '"op":"repo_pin"' "contains operation name"
  end_test
}

tr055_f() {
  start_test "TR-055 F"
  local out1=$WORK/055f1.txt out2=$WORK/055f2.txt out3=$WORK/055f3.txt
  "$ATA_BIN" workspace mirror-path https://github.com/openai/codex > "$out1"
  "$ATA_BIN" workspace mirror-path https://github.com/openai/codex > "$out2"
  "$ATA_BIN" workspace mirror-path https://github.com/openai/openai-cookbook > "$out3"
  assert_match "$out1" '/\.ata/caches/repo-mirrors/[0-9a-f]{16}' "matches hashed cache path"
  local h1 h2 h3
  h1=$(tr -d '\n' < "$out1"); h2=$(tr -d '\n' < "$out2"); h3=$(tr -d '\n' < "$out3")
  assert_eq "$h1" "$h2" "same URL → same hash"
  if [ "$h1" = "$h3" ]; then
    fail_assert "different URLs must produce different hashes"
  fi
  end_test
}

tr055_g() {
  start_test "TR-055 G"
  local out=$WORK/055g.txt
  "$ATA_BIN" workspace check-host https://github.com/openai/codex > "$out"
  local rc=$?
  assert_match "$out" '^https://github\.com/openai/codex$' "URL echoed back"
  assert_eq "$rc" "0" "exit code"
  end_test
}

tr055_g2() {
  start_test "TR-055 G2"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr055-g2-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace set-field --workspace "$wsid" --path policies.repoHostsAllowlist --value '["gitlab.com"]' >/dev/null
  local out=$WORK/055g2.txt rc=0
  "$ATA_BIN" workspace check-host --workspace "$wsid" https://github.com/foo/bar > "$out" 2>&1 || rc=$?
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1 || true
  assert_contains "$out" "error: host 'github.com' not in allowlist: [\"gitlab.com\"]" "rejection message"
  assert_eq "$rc" "1" "exit code is 1"
  end_test
}

tr055_h() {
  start_test "TR-055 H"
  local out=$WORK/055h.json
  "$ATA_BIN" workspace audit-query --workspace global > "$out"
  assert_json "$out"
  assert_jq "$out" 'type == "array"' "is array"
  end_test
}

tr055_i() {
  start_test "TR-055 I"
  local out=$WORK/055i.json
  "$ATA_BIN" workspace export-spec > "$out"
  assert_json "$out"
  for k in schemaVersion name repos labels; do
    assert_jq "$out" "has(\"$k\")" "missing key: $k"
  done
  assert_jq "$out" '.schemaVersion == 1' "spec schemaVersion is 1"
  end_test
}

tr055_j() {
  start_test "TR-055 J"
  local out=$WORK/055j.txt
  "$ATA_BIN" workspace search-commands repo > "$out"
  assert_match "$out" '^Matches:' "starts with Matches heading"
  assert_match "$out" '^1\.' "has numbered list"
  assert_contains "$out" "repo-clone" "lists repo-clone"
  assert_contains "$out" "repo-pin" "lists repo-pin"
  assert_contains "$out" "repo-remove" "lists repo-remove"
  assert_contains "$out" "Best match manual:" "has best match section"
  assert_contains "$out" "Usage:" "has Usage line"
  end_test
}

# --- TR-056: workspace lifecycle ------------------------------------------

tr056_a() {
  start_test "TR-056 A"
  local out=$WORK/056a.txt
  "$ATA_BIN" workspace init tr056-test > "$out"
  local rc=$?
  assert_match "$out" '^tr056-test-[0-9a-f]{8}$' "init prints id"
  assert_eq "$rc" "0"
  # cleanup happens in 056_c chain or final cleanup
  end_test
}

tr056_b() {
  start_test "TR-056 B"
  # Use the existing tr056-test workspace from 056_a.
  local wsid
  wsid=$("$ATA_BIN" workspace list | jq -r '.[].id | select(startswith("tr056-test-"))' | head -1)
  if [ -z "$wsid" ]; then
    fail_assert "no tr056-test workspace from prior step"
    end_test
    return
  fi
  local out=$WORK/056b.txt
  "$ATA_BIN" workspace select "$wsid" > "$out"
  assert_contains "$out" "selected: $wsid" "select confirmation"
  local manifest=$WORK/056b-read.json
  "$ATA_BIN" workspace read > "$manifest"
  assert_jq "$manifest" ".id == \"$wsid\"" "manifest id matches"
  end_test
}

tr056_c() {
  start_test "TR-056 C"
  local wsid
  wsid=$("$ATA_BIN" workspace list | jq -r '.[].id | select(startswith("tr056-test-"))' | head -1)
  if [ -z "$wsid" ]; then
    fail_assert "no tr056-test workspace to delete"
    end_test
    return
  fi
  "$ATA_BIN" workspace select global >/dev/null
  local out=$WORK/056c.txt
  "$ATA_BIN" workspace delete "$wsid" --force > "$out"
  assert_match "$out" "^deleted: $wsid$" "delete confirmation"
  local list=$WORK/056c-list.json
  "$ATA_BIN" workspace list > "$list"
  assert_jq "$list" "map(select(.id == \"$wsid\")) | length == 0" "workspace removed from list"
  end_test
}

tr056_d() {
  start_test "TR-056 D"
  local wsid out=$WORK/056d.txt
  wsid=$("$ATA_BIN" workspace init tr056-noforce | tr -d '\n')
  local rc=0
  "$ATA_BIN" workspace delete "$wsid" > "$out" 2>&1 || rc=$?
  assert_contains "$out" "error: workspace deletion requires --force" "refusal error"
  if [ "$rc" = "0" ]; then
    fail_assert "exit code must be non-zero; got 0"
  fi
  local list=$WORK/056d-list.json
  "$ATA_BIN" workspace list > "$list"
  assert_jq "$list" "map(select(.id == \"$wsid\")) | length == 1" "workspace not removed"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1 || true
  end_test
}

# --- TR-061: zotero CLI (no credentials) ----------------------------------

tr061_a() {
  start_test "TR-061 A"
  local out=$WORK/061a.txt
  "$ATA_BIN" zotero status > "$out"
  assert_contains "$out" "Effective mode: local"
  assert_contains "$out" "Base URL: http://localhost:23119/api"
  assert_contains "$out" "API key configured: no"
  assert_contains "$out" "Library scope: all accessible libraries"
  assert_contains "$out" "Default write scope: unconfigured"
  assert_contains "$out" "no Zotero API key is configured"
  end_test
}

tr061_a2() {
  start_test "TR-061 A2"
  # Back up config if present, install dummy key, restore at end.
  if [ -f "$HOME/.ata/config.toml" ]; then
    cp "$HOME/.ata/config.toml" "$HOME/.ata/config.toml.tier-a.bak"
  fi
  mkdir -p "$HOME/.ata"
  cat >> "$HOME/.ata/config.toml" <<'EOF'

[research]
zotero_api_key = "dummy-tier-a-key"
EOF
  local out=$WORK/061a2.txt
  "$ATA_BIN" zotero status > "$out"
  if [ -f "$HOME/.ata/config.toml.tier-a.bak" ]; then
    mv "$HOME/.ata/config.toml.tier-a.bak" "$HOME/.ata/config.toml"
  else
    rm -f "$HOME/.ata/config.toml"
  fi
  assert_contains "$out" "Effective mode: remote" "key flips to remote"
  assert_contains "$out" "API key configured: yes" "key detected"
  assert_contains "$out" "Fallback mode: local" "fallback line present"
  end_test
}

tr061_b() {
  start_test "TR-061 B"
  local out=$WORK/061b.txt
  "$ATA_BIN" zotero --help > "$out"
  assert_contains "$out" "Manage Zotero libraries, collections, items, and attachments"
  for cmd in search-commands status resolve-paper add-paper find-repos search tags recent advanced-search grep-text search-notes item collections collection groups items attachment help; do
    assert_contains "$out" "$cmd" "missing subcommand: $cmd"
  done
  end_test
}

tr061_c() {
  start_test "TR-061 C"
  local out=$WORK/061c.txt
  "$ATA_BIN" zotero search-commands paper > "$out"
  assert_match "$out" '^Matches:' "starts with Matches"
  assert_match "$out" '^1\.' "has numbered list"
  assert_contains "$out" "add-paper"
  assert_contains "$out" "resolve-paper"
  assert_contains "$out" "Best match manual:" "best match section"
  assert_contains "$out" "Usage: ata zotero" "Usage line"
  end_test
}

# --- driver ----------------------------------------------------------------

main() {
  if ! command -v "$ATA_BIN" >/dev/null 2>&1 && [ ! -x "$ATA_BIN" ]; then
    red "ata binary not found: $ATA_BIN"
    exit 2
  fi
  if ! command -v jq >/dev/null 2>&1; then
    red "jq is required"
    exit 2
  fi

  log "Tier A runner — ata: $("$ATA_BIN" --version 2>&1 | head -1)"
  log ""

  # Bootstrap: on a totally fresh ~/.ata the list is empty. Several
  # TR-055 scenarios assume at least one workspace exists.
  if [ "$("$ATA_BIN" workspace list 2>/dev/null | jq 'length' 2>/dev/null)" = "0" ]; then
    log "bootstrap: workspace list is empty, creating bootstrap workspace"
    "$ATA_BIN" workspace init bootstrap >/dev/null
    log ""
  fi

  log "TR-055: workspace read-only inspection"
  tr055_a; tr055_b; tr055_c; tr055_d; tr055_e; tr055_f
  tr055_g; tr055_g2; tr055_h; tr055_i; tr055_j

  log ""
  log "TR-056: workspace lifecycle"
  tr056_a; tr056_b; tr056_c; tr056_d

  log ""
  log "TR-061: zotero CLI (no credentials)"
  tr061_a; tr061_a2; tr061_b; tr061_c

  log ""
  log "----"
  log "PASS: $PASS  FAIL: $FAIL"
  if [ "$FAIL" -gt 0 ]; then
    log "Failed: ${FAILED_NAMES[*]}"
    exit 1
  fi
}

main "$@"
