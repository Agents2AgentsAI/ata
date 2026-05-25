#!/usr/bin/env bash
#
# Tier B1 runner — TUI driven via tmux, but no LLM calls.
#
# These scenarios verify the local TUI behavior: slash command parsing,
# overlay open/close, keymap surface. None of them submit a prompt to
# the model, so they don't burn LLM tokens.
#
# Exit code: 0 if all scenarios pass, 1 otherwise.
#
# Usage:
#   ./run-tier-b1.sh                 # use `ata` from PATH
#   ATA_BIN=./target/debug/ata ./run-tier-b1.sh

set -uo pipefail

ATA_BIN="${ATA_BIN:-ata}"
SESSION="tier-b1-$$"
WORK=$(mktemp -d -t ata-tier-b1.XXXXXX)
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

# Start an isolated tmux session running ata. Waits up to 30s for the
# welcome banner so we know the composer is ready before sending keys.
boot_ata() {
  local name=$1
  tmux kill-session -t "$name" 2>/dev/null || true
  tmux new-session -d -s "$name" -x 132 -y 40 "$ATA_BIN --yolo"
  local deadline=$(( $(date +%s) + 60 ))
  local banner_seen=0
  while [ "$(date +%s)" -lt "$deadline" ]; do
    local pane
    pane=$(tmux capture-pane -t "$name" -p 2>/dev/null || true)
    if printf '%s' "$pane" | grep -qF "Agents2Agents ata"; then
      banner_seen=1
      # ata is fully ready when no background task ("esc to interrupt")
      # is running. MCP server boot is the usual culprit during startup
      # and it hard-blocks slash commands while it runs.
      if ! printf '%s' "$pane" | grep -qF "esc to interrupt"; then
        sleep 0.5  # let the cursor settle
        return 0
      fi
    fi
    sleep 0.5
  done
  red "    [boot] ata not idle within 60s (banner_seen=$banner_seen). Pane dump:"
  tmux capture-pane -t "$name" -p 2>/dev/null | sed 's/^/    | /' >&2 || true
  return 1
}

kill_ata() {
  tmux kill-session -t "$1" 2>/dev/null || true
}

send_text() {
  local name=$1 text=$2
  tmux send-keys -t "$name" "$text"
  sleep 0.3
}

send_key() {
  local name=$1 key=$2
  tmux send-keys -t "$name" "$key"
  sleep 0.3
}

capture() {
  local name=$1 out=$2
  tmux capture-pane -t "$name" -p > "$out"
}

assert_contains() {
  local file=$1 needle=$2 desc=${3:-}
  if ! grep -qF -- "$needle" "$file"; then
    fail_assert "${desc:-expected to contain: $needle}" "$(tail -c 800 "$file")"
  fi
}

assert_not_contains() {
  local file=$1 needle=$2 desc=${3:-}
  if grep -qF -- "$needle" "$file"; then
    fail_assert "${desc:-must NOT contain: $needle}" "$(tail -c 800 "$file")"
  fi
}

cleanup_all() {
  local rc=$?
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$WORK" 2>/dev/null
  return $rc
}

# --- scenarios -------------------------------------------------------------

tr020_a() {
  start_test "TR-020 A"
  local sess=$SESSION-020a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/clera"
  send_key  "$sess" Enter
  sleep 1
  local out=$WORK/020a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Unrecognized command" "rejection message shown"
  assert_contains "$out" "/clera" "typed text is preserved or echoed"
  kill_ata "$sess"
  end_test
}

tr020_b() {
  start_test "TR-020 B"
  local sess=$SESSION-020b
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/CLEAR"
  send_key  "$sess" Enter
  sleep 1
  local out=$WORK/020b.txt
  capture "$sess" "$out"
  # If case-insensitive parsing works, /CLEAR is treated as /clear and
  # does NOT show the "Unrecognized command" error.
  assert_not_contains "$out" "Unrecognized command" "uppercase should be accepted"
  kill_ata "$sess"
  end_test
}

tr016_a() {
  start_test "TR-016 A"
  local sess=$SESSION-016a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/clear"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/016a.txt
  capture "$sess" "$out"
  # PLAN.md TR-016: /clear on an empty session is silent. The two
  # markers that appear after a real /clear (token usage line, resume
  # hint) must NOT be present.
  assert_not_contains "$out" "Token usage:" "no token line on empty /clear"
  assert_not_contains "$out" "ata resume"   "no resume hint on empty /clear"
  # Positive sanity check: capture wasn't empty.
  if [ ! -s "$out" ] || ! grep -q '[[:print:]]' "$out"; then
    fail_assert "pane capture was empty"
  fi
  kill_ata "$sess"
  end_test
}

tr018_a() {
  start_test "TR-018 A"
  local sess=$SESSION-018a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/model"
  send_key  "$sess" Enter
  sleep 1
  local out=$WORK/018a.txt
  capture "$sess" "$out"
  assert_contains "$out" "gpt-5.5" "current model listed"
  assert_contains "$out" "(current)" "current marker present"
  kill_ata "$sess"
  end_test
}

tr018_d() {
  start_test "TR-018 D"
  local sess=$SESSION-018d
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/model"
  send_key  "$sess" Enter
  sleep 1
  send_key  "$sess" Escape
  sleep 0.7
  local out=$WORK/018d.txt
  capture "$sess" "$out"
  # Picker closed → composer placeholder is visible again, model picker text gone.
  assert_contains     "$out" "Agents2Agents ata" "banner visible again after Esc"
  assert_not_contains "$out" "(current)" "picker is dismissed"
  kill_ata "$sess"
  end_test
}

tr018_b() {
  start_test "TR-018 B"
  local sess=$SESSION-018b
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/model"
  send_key  "$sess" Enter
  sleep 1.5
  send_key  "$sess" Enter  # select gpt-5.5 (highlighted)
  sleep 1.5
  local out=$WORK/018b.txt
  capture "$sess" "$out"
  assert_contains "$out" "Select Reasoning Level" "step 2 reasoning picker open"
  assert_contains "$out" "Medium (default) (current)" "current reasoning level marked"
  send_key "$sess" Escape
  sleep 1
  local out2=$WORK/018b-back.txt
  capture "$sess" "$out2"
  assert_contains "$out2" "Select Model and Effort" "Esc returns to step 1"
  assert_not_contains "$out2" "Select Reasoning Level" "step 2 closed"
  kill_ata "$sess"
  end_test
}

tr017_a() {
  start_test "TR-017 A"
  local sess=$SESSION-017a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/permissions"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/017a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Update Model Permissions" "permissions picker open"
  assert_contains "$out" "Default"     "Default option listed"
  assert_contains "$out" "Auto-review" "Auto-review option listed"
  assert_contains "$out" "Full Access" "Full Access option listed"
  assert_contains "$out" "(current)"   "current option marker present"
  kill_ata "$sess"
  end_test
}

tr010_a() {
  start_test "TR-010 A"
  local sess=$SESSION-010a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/experimental"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/010a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Experimental features"     "title shown"
  assert_contains "$out" "Terminal resize reflow"    "first toggle listed"
  assert_contains "$out" "Press space to select"     "footer hint present"
  kill_ata "$sess"
  end_test
}

tr020_d() {
  start_test "TR-020 D"
  local sess=$SESSION-020d
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/"
  sleep 1.5
  local out=$WORK/020d.txt
  capture "$sess" "$out"
  # Bare / opens the slash command picker (no Enter pressed).
  assert_contains "$out" "/model"        "picker lists /model"
  assert_contains "$out" "/permissions"  "picker lists /permissions"
  assert_contains "$out" "/experimental" "picker lists /experimental"
  kill_ata "$sess"
  end_test
}

tr019_a() {
  start_test "TR-019 A"
  local sess=$SESSION-019a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "@xyznosuchprefix"
  sleep 1.5
  local out=$WORK/019a.txt
  capture "$sess" "$out"
  assert_contains "$out" "@xyznosuchprefix" "typed text echoed in composer"
  assert_contains "$out" "no matches"       "picker shows no matches"
  kill_ata "$sess"
  end_test
}

# --- driver ----------------------------------------------------------------

main() {
  if ! command -v "$ATA_BIN" >/dev/null 2>&1 && [ ! -x "$ATA_BIN" ]; then
    red "ata binary not found: $ATA_BIN"
    exit 2
  fi
  if ! command -v tmux >/dev/null 2>&1; then
    red "tmux is required"
    exit 2
  fi

  log "Tier B1 runner — ata: $("$ATA_BIN" --version 2>&1 | head -1)"
  log ""

  log "Slash command parsing & overlays"
  tr020_a; tr020_b; tr016_a; tr018_a; tr018_d
  tr018_b; tr017_a; tr010_a; tr020_d; tr019_a

  log ""
  log "----"
  log "PASS: $PASS  FAIL: $FAIL"
  if [ "$FAIL" -gt 0 ]; then
    log "Failed: ${FAILED_NAMES[*]}"
    exit 1
  fi
}

main "$@"
