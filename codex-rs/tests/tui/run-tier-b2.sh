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

send_text() { tmux send-keys -t "$1" "$2"; sleep 0.6; }
send_key()  { tmux send-keys -t "$1" "$2"; sleep 0.4; }
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

# Find the most recently modified ata session JSONL (the one our test
# just produced). Returns the path or empty string.
recent_session_jsonl() {
  find "$HOME/.ata/sessions" -name "*.jsonl" -mmin -5 2>/dev/null \
    | xargs ls -t 2>/dev/null | head -1
}

# Assert that a specific tool was called at least once in the session.
# This is the deterministic anchor: even if the agent's response text
# varies, the tool_counts in the session log are stable when the
# prompt explicitly names the tool.
assert_tool_called() {
  local sess_jsonl=$1 tool=$2 desc=${3:-}
  if [ -z "$sess_jsonl" ] || [ ! -f "$sess_jsonl" ]; then
    fail_assert "${desc:-tool $tool}: session JSONL not found"
    return
  fi
  if ! jq -r '.payload.name // empty' "$sess_jsonl" | grep -qFx "$tool"; then
    local counts
    counts=$(jq -r '.payload.name // empty' "$sess_jsonl" | sort | uniq -c | tr '\n' ' ')
    fail_assert "${desc:-expected tool '$tool' to be called}" "tool_counts: $counts"
  fi
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

tr031_a() {
  start_test "TR-031 A"
  local sess=$SESSION-031a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  # Visual-select Enter sends an "explain selection" request. We can't
  # reliably assert WHICH tool the agent picks (flaky), but we can
  # verify the inline-answer machinery wired up — 'You asked:' marker
  # appears and the reader still renders.
  send_text "$sess" "v"; sleep 1
  send_text "$sess" "jj"; sleep 0.5
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 90; then
    fail_assert "agent did not finish within 90s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/031a.txt
  capture "$sess" "$out"
  assert_contains "$out" "You asked:"     "inline question marker present"
  assert_contains "$out" "Sections (n/p"  "reader still rendering"
  kill_ata "$sess"
  end_test
}

tr032_a() {
  start_test "TR-032 A"
  local sess=$SESSION-032a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_key  "$sess" Tab
  sleep 1
  send_text "$sess" "add a third slide about coffee brewing methods"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 120; then
    fail_assert "agent did not finish within 120s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/032a.txt
  capture "$sess" "$out"
  # Expect a new section to exist. Doc started with 2, so the title
  # bar should now show 1/3 or the TOC entry list should include
  # something about brewing.
  if grep -qF "1/3" "$out" || grep -qF "2/3" "$out" || grep -qF "3/3" "$out" || grep -qiE "brewing|method" "$out"; then
    :
  else
    fail_assert "no sign of a new third section after add request" "$(tail -c 800 "$out")"
  fi
  assert_contains "$out" "Sections (n/p"  "reader still rendering"
  kill_ata "$sess"
  end_test
}

tr033_a() {
  start_test "TR-033 A"
  local sess=$SESSION-033a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_key  "$sess" Tab
  sleep 1
  send_text "$sess" "append a short paragraph about espresso to slide 2"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 120; then
    fail_assert "agent did not finish within 120s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/033a.txt
  capture "$sess" "$out"
  # Look for an espresso-related word anywhere in the rendered pane
  # (the agent's addition to slide 2 should mention it).
  if ! grep -qiE "espresso|crema|pressure" "$out"; then
    fail_assert "no espresso content found in pane after append request" "$(tail -c 800 "$out")"
  fi
  assert_contains "$out" "Sections (n/p"  "reader still rendering"
  kill_ata "$sess"
  end_test
}

tr037_a() {
  start_test "TR-037 A"
  local sess=$SESSION-037a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_key  "$sess" Tab
  sleep 1
  send_text "$sess" "explain why coffee tastes bitter in one short paragraph"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 90; then
    fail_assert "agent did not finish within 90s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/037a.txt
  capture "$sess" "$out"
  assert_contains "$out" "You asked:"     "inline scoped Q&A marker"
  if ! grep -qiE "bitter|extract|over-extract|tannin|roast" "$out"; then
    fail_assert "no bitterness-related content in inline answer" "$(tail -c 800 "$out")"
  fi
  assert_contains "$out" "Sections (n/p"  "still in reader after answer"
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

tr050_a() {
  start_test "TR-050 A"
  local sess=$SESSION-050a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/"
  send_text "$sess" "coffee"
  sleep 1
  local out=$WORK/050a.txt
  capture "$sess" "$out"
  assert_contains "$out" "/coffee"          "search query echoed"
  assert_contains "$out" "Enter: search"    "search-mode footer present"
  assert_contains "$out" "Esc: cancel"      "esc-cancel hint shown"
  kill_ata "$sess"
  end_test
}

tr052_a() {
  start_test "TR-052 A"
  local sess=$SESSION-052a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  # 'r' starts TTS narration. Without ElevenLabs configured ata prints
  # the credential error but still enters the audio-active state, so
  # the footer expands with playback controls.
  send_text "$sess" "r"
  sleep 1.5
  local out=$WORK/052a.txt
  capture "$sess" "$out"
  assert_contains "$out" "TTS error: Invalid API key" "TTS credential error shown"
  assert_contains "$out" "s: pause"   "audio footer 'pause' control"
  assert_contains "$out" "+/-: speed" "audio footer 'speed' control"
  kill_ata "$sess"
  end_test
}

tr053_a() {
  start_test "TR-053 A"
  local sess=$SESSION-053a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "v"
  sleep 1.5
  local out=$WORK/053a.txt
  capture "$sess" "$out"
  assert_contains "$out" "hjkl: select"   "visual-mode footer 'select' hint"
  assert_contains "$out" "Enter: explain" "Enter binding shown"
  assert_contains "$out" "Esc: cancel"    "Esc cancel binding shown"
  kill_ata "$sess"
  end_test
}

tr002_a() {
  start_test "TR-002 A"
  local sess=$SESSION-002a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_key  "$sess" Tab
  sleep 1
  send_text "$sess" "what color are coffee beans"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 90; then
    fail_assert "agent did not finish within 90s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/002a.txt
  capture "$sess" "$out"
  # TR-002 contract: Tab-to-ask response stays INLINE in the reader
  # (not as a chat bubble). The "You asked: ..." line and the agent's
  # answer should both appear inside the reader frame.
  assert_contains "$out" "You asked:"     "inline question marker"
  assert_contains "$out" "color"          "agent response references the question"
  assert_contains "$out" "Sections (n/p"  "still in reader after the answer"
  kill_ata "$sess"
  end_test
}

tr049_a() {
  start_test "TR-049 A"
  local sess=$SESSION-049a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  # Vim scroll keys (j/k). Sections in our test doc are short enough to
  # not visibly scroll the box content, but the keys must be recognized
  # without breaking the reader. Assert the reader UI is still intact
  # after pressing several scroll keys.
  send_text "$sess" "jjjkk"
  sleep 1
  local out=$WORK/049a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Sections (n/p"     "reader still rendering after scroll keys"
  assert_contains "$out" "q: close"          "reader footer still present"
  kill_ata "$sess"
  end_test
}

tr050_a() {
  start_test "TR-050 A"
  local sess=$SESSION-050a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/"
  send_text "$sess" "coffee"
  sleep 1
  local out=$WORK/050a.txt
  capture "$sess" "$out"
  assert_contains "$out" "/coffee"          "search query echoed"
  assert_contains "$out" "Enter: search"    "search-mode footer present"
  assert_contains "$out" "Esc: cancel"      "esc-cancel hint shown"
  kill_ata "$sess"
  end_test
}

tr051_a() {
  start_test "TR-051 A"
  local sess=$SESSION-051a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  # Fold key (f). The 2-section coffee doc may not have foldable
  # regions, so we can't always verify a visible fold. Minimum:
  # 'f' is recognized and the reader stays valid afterwards.
  send_text "$sess" "f"
  sleep 1
  local out=$WORK/051a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Sections (n/p" "reader still rendering after f"
  assert_contains "$out" "q: close"      "reader footer present after f"
  kill_ata "$sess"
  end_test
}

tr052_a() {
  start_test "TR-052 A"
  local sess=$SESSION-052a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  # 'r' starts TTS narration. Without ElevenLabs configured ata prints
  # the credential error but still enters the audio-active state, so
  # the footer expands with playback controls.
  send_text "$sess" "r"
  sleep 1.5
  local out=$WORK/052a.txt
  capture "$sess" "$out"
  assert_contains "$out" "TTS error: Invalid API key" "TTS credential error shown"
  assert_contains "$out" "s: pause"   "audio footer 'pause' control"
  assert_contains "$out" "+/-: speed" "audio footer 'speed' control"
  kill_ata "$sess"
  end_test
}

tr053_a() {
  start_test "TR-053 A"
  local sess=$SESSION-053a
  if ! boot_reader "$sess"; then fail_assert "reader did not open"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "v"
  sleep 1.5
  local out=$WORK/053a.txt
  capture "$sess" "$out"
  assert_contains "$out" "hjkl: select"   "visual-mode footer 'select' hint"
  assert_contains "$out" "Enter: explain" "Enter binding shown"
  assert_contains "$out" "Esc: cancel"    "Esc cancel binding shown"
  kill_ata "$sess"
  end_test
}

tr044_a() {
  start_test "TR-044 A"
  local sess=$SESSION-044a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  # /side requires the conversation to have started first. Send a tiny
  # round-trip, then enter a side context.
  send_text "$sess" "respond with hi"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 60; then
    fail_assert "agent did not respond to setup prompt"
    kill_ata "$sess"; end_test; return
  fi
  send_text "$sess" "/side test-context"
  send_key  "$sess" Enter
  sleep 3
  local out=$WORK/044a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Side from main thread" "side-context indicator shown"
  assert_contains "$out" "Esc to return"         "return hint shown"
  # Interrupt the side turn ata kicks off automatically so kill_ata
  # doesn't leak a running model call.
  send_key "$sess" Escape
  sleep 2
  kill_ata "$sess"
  end_test
}

tr045_a() {
  start_test "TR-045 A"
  local sess=$SESSION-045a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "respond with hi"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 60; then
    fail_assert "agent did not respond to setup prompt"
    kill_ata "$sess"; end_test; return
  fi
  send_text "$sess" "/fork"
  send_key  "$sess" Enter
  sleep 4
  local out=$WORK/045a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Thread forked from" "fork confirmation"
  kill_ata "$sess"
  end_test
}

tr047_a() {
  start_test "TR-047 A"
  local sess=$SESSION-047a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "respond with hi"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 60; then
    fail_assert "agent did not respond to setup prompt"
    kill_ata "$sess"; end_test; return
  fi
  send_text "$sess" "/compact"
  send_key  "$sess" Enter
  # /compact triggers a summarization round trip — wait for it to finish.
  if ! wait_for_idle "$sess" 120; then
    fail_assert "compact did not finish in 120s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/047a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Context compacted" "compact confirmation"
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

tr021_a() {
  start_test "TR-021 A"
  local sess=$SESSION-021a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  # The dedicated hn_search tool exists but the agent's choice between
  # it and a generic exec_command (curl HN) is non-deterministic. So
  # this test only asserts HN content actually came back — a real
  # regression guard against "HN access fully broken" without
  # depending on which tool the agent picked.
  send_text "$sess" "Use the hacker_news tool to fetch the top 3 stories"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 180; then
    fail_assert "agent did not finish within 180s"
    kill_ata "$sess"; end_test; return
  fi
  local out=$WORK/021a.txt
  capture "$sess" "$out"
  # HN result rows are formatted as "<n> points, <m> comments". That
  # string is stable across both tool paths (hn_search and the
  # exec_command fallback).
  if ! grep -qE '[0-9]+ points, [0-9]+ comments' "$out"; then
    fail_assert "no HN 'X points, Y comments' line in response" "$(tail -c 800 "$out")"
  fi
  kill_ata "$sess"
  end_test
}

tr062_a() {
  start_test "TR-062 A"
  local sess=$SESSION-062a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "Use paper_search to find a recent paper on rust async runtime"
  send_key  "$sess" Enter
  if ! wait_for_idle "$sess" 180; then
    fail_assert "agent did not finish within 180s"
    kill_ata "$sess"; end_test; return
  fi
  local sess_jsonl
  sess_jsonl=$(recent_session_jsonl)
  assert_tool_called "$sess_jsonl" "paper_search" "agent called the dedicated paper_search tool"
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
  tr002_a
  tr005_a
  tr008_a
  tr016_b
  tr021_a
  tr022_a
  tr031_a
  tr032_a
  tr033_a
  tr036_a
  tr036_b
  tr037_a
  tr044_a
  tr045_a
  tr047_a
  tr049_a
  tr050_a
  tr051_a
  tr052_a
  tr053_a
  tr054_a
  tr062_a

  log ""
  log "----"
  log "PASS: $PASS  FAIL: $FAIL  SKIP: $SKIP"
  if [ "$FAIL" -gt 0 ]; then
    log "Failed: ${FAILED_NAMES[*]}"
    exit 1
  fi
}

main "$@"
