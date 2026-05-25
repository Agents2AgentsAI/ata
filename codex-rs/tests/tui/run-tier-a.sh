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
    | grep -E '^(tr05[56789]|tr06[01]|bootstrap)' \
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

# --- TR-057: workspace repo management ------------------------------------

# A tiny well-known public repo (master branch, single README, ~few KB).
HELLO_URL="https://github.com/octocat/Hello-World"
HELLO_SHA="7fd1a60b01f91b314f59955a4e4d4e80d8edf11d"

tr057_a() {
  start_test "TR-057 A"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr057-$(openssl rand -hex 4)" | tr -d '\n')
  local out=$WORK/057a.json
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" > "$out" 2>&1
  assert_json "$out"
  assert_jq "$out" '.alias == "hello-test"'                     "clone returns alias"
  assert_jq "$out" '.checkoutPath == "repos/hello-test"'        "checkoutPath set"
  assert_jq "$out" '.state.headSha | type == "string"'          "headSha returned"
  local mf=$WORK/057a-mf.json
  "$ATA_BIN" workspace read --workspace "$wsid" > "$mf"
  assert_jq "$mf" '.repos | map(select(.alias == "hello-test")) | length == 1' "repo entry in manifest"
  local audit=$WORK/057a-audit.json
  "$ATA_BIN" workspace audit-query --workspace "$wsid" > "$audit"
  assert_jq "$audit" 'map(select(.op == "repo_add")) | length >= 1' "audit has repo_add op"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr057_b() {
  start_test "TR-057 B"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr057b-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local out=$WORK/057b.json
  "$ATA_BIN" workspace repo-pin --alias hello-test --sha "$HELLO_SHA" --workspace "$wsid" > "$out"
  assert_jq "$out" '.repos[0].pin.mode == "pinned"'        "pin mode set"
  assert_jq "$out" ".repos[0].pin.pinnedSha == \"$HELLO_SHA\"" "pinnedSha matches"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr057_c() {
  start_test "TR-057 C"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr057c-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  "$ATA_BIN" workspace repo-pin --alias hello-test --sha "$HELLO_SHA" --workspace "$wsid" >/dev/null
  local out=$WORK/057c.json
  "$ATA_BIN" workspace repo-unpin --alias hello-test --workspace "$wsid" > "$out"
  assert_jq "$out" '.repos[0].pin.mode == "tracking"'        "pin reverts to tracking"
  assert_jq "$out" '.repos[0].pin | has("pinnedSha") | not'  "pinnedSha removed"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr057_d() {
  start_test "TR-057 D"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr057d-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local out=$WORK/057d.json
  "$ATA_BIN" workspace repo-update-state --alias hello-test --head-sha "$HELLO_SHA" --head-ref refs/heads/master --workspace "$wsid" > "$out"
  assert_jq "$out" ".repos[0].state.headSha == \"$HELLO_SHA\"" "headSha updated"
  assert_jq "$out" '.repos[0].state.headRef == "refs/heads/master"' "headRef updated"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr057_e() {
  start_test "TR-057 E"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr057e-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local out=$WORK/057e.txt
  "$ATA_BIN" workspace repo-remove --alias hello-test --workspace "$wsid" > "$out"
  assert_match "$out" '^removed: hello-test$' "removal confirmation"
  local mf=$WORK/057e-mf.json
  "$ATA_BIN" workspace read --workspace "$wsid" > "$mf"
  assert_jq "$mf" '.repos | length == 0' "repos list empty after remove"
  local audit=$WORK/057e-audit.json
  "$ATA_BIN" workspace audit-query --workspace "$wsid" > "$audit"
  assert_jq "$audit" 'map(select(.op == "repo_remove")) | length >= 1' "audit has repo_remove op"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

# --- TR-058: workspace runs lifecycle -------------------------------------

tr058_a() {
  start_test "TR-058 A"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr058a-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local out=$WORK/058a.json
  "$ATA_BIN" workspace run-setup tr058-run --source-alias hello-test --workspace "$wsid" > "$out"
  assert_jq "$out" '.name == "tr058-run"'      "run name set"
  assert_jq "$out" '.strategy == "worktree"'   "default strategy worktree"
  assert_jq "$out" '.source.repoAlias == "hello-test"' "source alias set"
  local mf=$WORK/058a-mf.json
  "$ATA_BIN" workspace read --workspace "$wsid" > "$mf"
  assert_jq "$mf" '.runs | length == 1'           "manifest has the run"
  assert_jq "$mf" '.runs[0].status == "created"'  "initial status is 'created'"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr058_b() {
  start_test "TR-058 B"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr058b-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  "$ATA_BIN" workspace run-setup tr058-run --source-alias hello-test --workspace "$wsid" >/dev/null
  local run_id
  run_id=$("$ATA_BIN" workspace read --workspace "$wsid" | jq -r '.runs[0].id')
  local out=$WORK/058b.json
  "$ATA_BIN" workspace run-update-status --id "$run_id" --status running --workspace "$wsid" > "$out"
  assert_jq "$out" '.runs[0].status == "running"' "status updated to running"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr058_c() {
  start_test "TR-058 C"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr058c-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  "$ATA_BIN" workspace run-setup tr058-run --source-alias hello-test --workspace "$wsid" >/dev/null
  local run_id
  run_id=$("$ATA_BIN" workspace read --workspace "$wsid" | jq -r '.runs[0].id')
  local out=$WORK/058c.txt
  "$ATA_BIN" workspace run-remove --id "$run_id" --workspace "$wsid" > "$out"
  assert_match "$out" "^removed: $run_id$" "removal confirmation"
  local mf=$WORK/058c-mf.json
  "$ATA_BIN" workspace read --workspace "$wsid" > "$mf"
  assert_jq "$mf" '.runs | length == 0' "runs list empty after remove"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

# --- TR-059: workspace manifest mutation ----------------------------------

tr059_a() {
  start_test "TR-059 A"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr059a-$(openssl rand -hex 4)" | tr -d '\n')
  local out=$WORK/059a.json
  "$ATA_BIN" workspace set-field --path policies.defaultClone.depth --value 5 --workspace "$wsid" > "$out"
  assert_jq "$out" '.policies.defaultClone.depth == 5' "field updated"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr059_b() {
  start_test "TR-059 B"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr059b-$(openssl rand -hex 4)" | tr -d '\n')
  local out=$WORK/059b.json
  "$ATA_BIN" workspace add-entry --collection links --json '{"id":"tr059-link","url":"https://example.com","title":"Test"}' --workspace "$wsid" > "$out"
  assert_jq "$out" '.links | map(select(.id == "tr059-link")) | length == 1' "entry added"
  assert_jq "$out" '.links[0].url == "https://example.com"' "url preserved"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr059_c() {
  start_test "TR-059 C"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr059c-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace add-entry --collection links --json '{"id":"tr059-link","url":"https://example.com","title":"Test"}' --workspace "$wsid" >/dev/null
  local out=$WORK/059c.json
  "$ATA_BIN" workspace remove-entry --collection links --id tr059-link --workspace "$wsid" > "$out"
  assert_jq "$out" '.links | length == 0' "entry removed"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr059_d() {
  start_test "TR-059 D"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr059d-$(openssl rand -hex 4)" | tr -d '\n')
  local paper=$WORK/tr059-paper.md
  printf "# Test paper\nbody\n" > "$paper"
  local out=$WORK/059d.json
  "$ATA_BIN" workspace add-paper "$paper" --alias tr059-paper --title "Test paper" --workspace "$wsid" > "$out"
  assert_jq "$out" '.papers | map(select(.alias == "tr059-paper")) | length == 1' "paper registered"
  assert_jq "$out" '.papers[0].title == "Test paper"' "title preserved"
  assert_jq "$out" '.papers[0].textMdPath == "papers/tr059-paper.md"' "textMdPath set"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

# --- TR-060: workspace spec round-trip ------------------------------------

tr060_a() {
  start_test "TR-060 A"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr060a-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local out=$WORK/060a.json
  "$ATA_BIN" workspace export-spec --workspace "$wsid" > "$out"
  assert_json "$out"
  assert_jq "$out" '.schemaVersion == 1'                "schemaVersion is 1"
  assert_jq "$out" 'has("name") and has("repos") and has("labels")' "spec keys present"
  assert_jq "$out" '.repos | length == 1'               "one repo exported"
  assert_jq "$out" '.repos[0].alias == "hello-test"'    "alias preserved"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr060_b() {
  start_test "TR-060 B"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr060b-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local spec=$WORK/060b-spec.json
  "$ATA_BIN" workspace export-spec --workspace "$wsid" > "$spec"
  # Add a second repo to the spec to force an Add line.
  local spec2=$WORK/060b-spec2.json
  jq '.repos += [{"url": "https://github.com/octocat/Hello-World", "alias": "hello2", "pinnedSha": null}]' "$spec" > "$spec2"
  local out=$WORK/060b.txt
  "$ATA_BIN" workspace diff-spec "$spec2" --workspace "$wsid" > "$out"
  assert_match    "$out" '^Add \(1\):' "Add section present"
  assert_contains "$out" "+ hello2"    "lists new alias"
  assert_match    "$out" 'Summary: 1 add' "summary line"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr060_c() {
  start_test "TR-060 C"
  local wsid
  wsid=$("$ATA_BIN" workspace init "tr060c-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$wsid" >/dev/null
  local spec=$WORK/060c-spec.json
  "$ATA_BIN" workspace export-spec --workspace "$wsid" > "$spec"
  local spec2=$WORK/060c-spec2.json
  jq '.repos += [{"url": "https://github.com/octocat/Hello-World", "alias": "hello2", "pinnedSha": null}]' "$spec" > "$spec2"
  local out=$WORK/060c.json
  "$ATA_BIN" workspace materialize "$spec2" --workspace "$wsid" --dry-run > "$out"
  assert_jq "$out" '.dryRun == true' "dryRun flag"
  assert_jq "$out" '.actions | map(select(.alias == "hello2" and .action == "add")) | length == 1' "hello2 add action"
  "$ATA_BIN" workspace delete "$wsid" --force >/dev/null 2>&1
  end_test
}

tr060_d() {
  start_test "TR-060 D"
  local source_wsid target_wsid
  source_wsid=$("$ATA_BIN" workspace init "tr060d-src-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace repo-clone "$HELLO_URL" hello-test --workspace "$source_wsid" >/dev/null
  local spec=$WORK/060d-spec.json
  "$ATA_BIN" workspace export-spec --workspace "$source_wsid" > "$spec"
  target_wsid=$("$ATA_BIN" workspace init "tr060d-tgt-$(openssl rand -hex 4)" | tr -d '\n')
  "$ATA_BIN" workspace materialize "$spec" --workspace "$target_wsid" >/dev/null 2>&1
  local mf=$WORK/060d-mf.json
  "$ATA_BIN" workspace read --workspace "$target_wsid" > "$mf"
  assert_jq "$mf" '.repos | map(select(.alias == "hello-test")) | length == 1' "repo materialized in target"
  "$ATA_BIN" workspace delete "$source_wsid" --force >/dev/null 2>&1
  "$ATA_BIN" workspace delete "$target_wsid" --force >/dev/null 2>&1
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
  log "TR-057: workspace repo management (network)"
  tr057_a; tr057_b; tr057_c; tr057_d; tr057_e

  log ""
  log "TR-058: workspace runs lifecycle"
  tr058_a; tr058_b; tr058_c

  log ""
  log "TR-059: workspace manifest mutation"
  tr059_a; tr059_b; tr059_c; tr059_d

  log ""
  log "TR-060: workspace spec round-trip"
  tr060_a; tr060_b; tr060_c; tr060_d

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
