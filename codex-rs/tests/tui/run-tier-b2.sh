#!/usr/bin/env bash
#
# Tier B2 runner — TUI driven via tmux, USES REAL LLM TOKENS.
#
# Unlike Tier A and B1, these scenarios actually send prompts to the
# model and wait for responses. Each run costs a small amount of money
# (~$0.05 for the starter batch of 3 scenarios as of 2026-05).
#
# Should NEVER auto-run on every PR. The accompanying workflow uses
# workflow_dispatch only.
#
# Exit code: 0 if all scenarios pass, 1 otherwise.
#
# Usage:
#   ./run-tier-b2.sh                 # use `ata` from PATH
#   ATA_BIN=./target/debug/ata ./run-tier-b2.sh

set -uo pipefail

ATA_BIN="${ATA_BIN:-ata}"
SESSION="tier-b2-$$"
WORK=$(mktemp -d -t ata-tier-b2.XXXXXX)
trap 'cleanup_all' EXIT

PASS=0
FAIL=0
SKIP=0
FAILED_NAMES=()
CURRENT_NAME=""
CURRENT_FAILED=0

log()    { printf '%s\n' "$*"; }
red()    { printf '\033[31m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }

start_test() {
  CURRENT_NAME="$1"
  CURRENT_FAILED=0
  printf '  %-16s ' "$CURRENT_NAME"
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

skip_test() {
  yellow "SKIP — $1"
  SKIP=$((SKIP + 1))
}

fail_assert() {
  CURRENT_FAILED=1
  red ""
  red "    [$CURRENT_NAME] $1"
  [ $# -ge 2 ] && red "      got: $2"
}

# Same boot helper pattern as B1 — wait until ata is idle.
boot_ata() {
  local name=$1
  tmux kill-session -t "$name" 2>/dev/null || true
  tmux new-session -d -s "$name" -x 132 -y 40 "$ATA_BIN --yolo"
  local deadline=$(( $(date +%s) + 60 ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    local pane
    pane=$(tmux capture-pane -t "$name" -p 2>/dev/null || true)
    if printf '%s' "$pane" | grep -qF "Agents2Agents ata" \
       && ! printf '%s' "$pane" | grep -qF "esc to interrupt"; then
      sleep 0.5
      return 0
    fi
    sleep 0.5
  done
  red "    [boot] ata not idle within 60s. Pane dump:"
  tmux capture-pane -t "$name" -p 2>/dev/null | sed 's/^/    | /' >&2 || true
  return 1
}

kill_ata() { tmux kill-session -t "$1" 2>/dev/null || true; }

send_text() { tmux send-keys -t "$1" "$2"; sleep 0.3; }
send_key()  { tmux send-keys -t "$1" "$2"; sleep 0.3; }
capture()   { tmux capture-pane -t "$1" -p > "$2"; }

# Wait for the turn to complete by watching the "esc to interrupt"
# indicator. Returns 0 on success, 1 on timeout.
wait_for_idle() {
  local name=$1 timeout=${2:-90}
  local deadline=$(( $(date +%s) + timeout ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if ! tmux capture-pane -t "$name" -p 2>/dev/null | grep -qF "esc to interrupt"; then
      sleep 2
      return 0
    fi
    sleep 1
  done
  return 1
}

assert_contains() {
  local file=$1 needle=$2 desc=${3:-}
  if ! grep -qF -- "$needle" "$file"; then
    fail_assert "${desc:-expected to contain: $needle}" "$(tail -c 800 "$file")"
  fi
}

cleanup_all() {
  local rc=$?
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$WORK" 2>/dev/null
  return $rc
}

# --- scenarios -------------------------------------------------------------

tr005_a() {
  start_test "TR-005 A"
  local sess=$SESSION-005a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "respond with only the word ping"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 60; then
    fail_assert "agent did not finish within 60s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/005a.txt
  capture "$sess" "$out"
  # Composer echoes the prompt with chevron, agent reply shows under '• ping'.
  assert_contains "$out" "respond with only the word ping" "prompt echoed"
  assert_contains "$out" "ping" "agent responded with 'ping'"
  kill_ata "$sess"
  end_test
}

tr022_a() {
  start_test "TR-022 A"
  local sess=$SESSION-022a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "write a 500 word essay on coffee"
  send_key  "$sess" Enter
  # Give the model a couple of seconds to start, then interrupt.
  sleep 3
  send_key "$sess" Escape
  # Wait for the interrupted marker to settle.
  sleep 4
  local out=$WORK/022a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Conversation interrupted" "interrupted marker shown"
  kill_ata "$sess"
  end_test
}

# Open a reading-view document for reader-based scenarios. Sends a
# fixed prompt and waits until "Sections (n/p" appears (the canonical
# reader-open marker). Each call costs one LLM round-trip.
boot_reader() {
  local name=$1
  if ! boot_ata "$name"; then return 1; fi
  send_text "$name" "give me 2 short slides on coffee in reading view, don't use any skills"
  send_key  "$name" Enter
  local deadline=$(( $(date +%s) + 180 ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    if tmux capture-pane -t "$name" -p 2>/dev/null | grep -qF "Sections (n/p"; then
      sleep 1
      return 0
    fi
    sleep 3
  done
  red "    [reader] document never opened in 180s. Pane dump:"
  tmux capture-pane -t "$name" -p 2>/dev/null | sed 's/^/    | /' >&2 || true
  return 1
}

tr001_a() {
  start_test "TR-001 A"
  local sess=$SESSION-001a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  local out=$WORK/001a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Sections (n/p" "reader marker present"
  assert_contains "$out" "Slide 1" "first section heading visible"
  assert_contains "$out" "q: close" "reader footer hint shown"
  kill_ata "$sess"
  end_test
}

tr008_a() {
  start_test "TR-008 A"
  local sess=$SESSION-008a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "q"
  sleep 2
  local out=$WORK/008a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Agent showed document:" "close marker shown in chat"
  # cleanup: the close triggers a silent follow-up turn; interrupt it
  send_key "$sess" Escape
  sleep 2
  kill_ata "$sess"
  end_test
}

tr036_a() {
  start_test "TR-036 A"
  local sess=$SESSION-036a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "n"
  sleep 1.5
  local out=$WORK/036a.txt
  capture "$sess" "$out"
  assert_contains "$out" "2/2" "advanced to section 2 of 2"
  assert_contains "$out" "Slide 2" "second section content visible"
  kill_ata "$sess"
  end_test
}

tr036_b() {
  start_test "TR-036 B"
  local sess=$SESSION-036b
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "t"
  sleep 1.5
  local out=$WORK/036b.txt
  capture "$sess" "$out"
  assert_contains "$out" "Table of Contents" "TOC title shown"
  assert_contains "$out" "j/k to navigate"   "TOC footer present"
  assert_contains "$out" "t/Esc to dismiss"  "dismiss hint shown"
  kill_ata "$sess"
  end_test
}

tr054_a() {
  start_test "TR-054 A"
  local sess=$SESSION-054a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "?"
  sleep 1.5
  local out=$WORK/054a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Reading View Help" "help title shown"
  assert_contains "$out" "Getting around"    "section heading present"
  assert_contains "$out" "Next section"      "n shortcut documented"
  assert_contains "$out" "Previous section"  "p shortcut documented"
  kill_ata "$sess"
  end_test
}

tr016_b() {
  start_test "TR-016 B"
  local sess=$SESSION-016b
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "write a 500 word essay on coffee"
  send_key  "$sess" Enter
  sleep 3
  # Now /clear should be hard-blocked.
  send_text "$sess" "/clear"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/016b.txt
  capture "$sess" "$out"
  assert_contains "$out" "'/clear' is disabled while a task is in progress" "hard-block message shown"
  # Cleanup: interrupt the running turn so kill_ata doesn't leave anything.
  send_key "$sess" Escape
  sleep 3
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

  log "Tier B2 runner — ata: $("$ATA_BIN" --version 2>&1 | head -1)"
  log "WARNING: this batch sends real prompts and costs real LLM tokens."
  log ""

  log "Numbered TRs (in order)"
  tr001_a
  tr005_a
  tr008_a
  tr016_b
  tr022_a
  tr036_a
  tr036_b
  tr054_a

  log ""
  log "----"
  log "PASS: $PASS  FAIL: $FAIL  SKIP: $SKIP"
  if [ "$FAIL" -gt 0 ]; then
    log "Failed: ${FAILED_NAMES[*]}"
    exit 1
  fi
}

main "$@"
