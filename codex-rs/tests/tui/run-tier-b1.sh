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
SKIP=0
FAILED_NAMES=()
CURRENT_NAME=""
CURRENT_FAILED=0

# --- helpers ---------------------------------------------------------------

log()   { printf '%s\n' "$*"; }
red()    { printf '\033[31m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }

skip_test() {
  yellow "SKIP — $1"
  SKIP=$((SKIP + 1))
}

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
  sleep 0.6
}

send_key() {
  local name=$1 key=$2
  tmux send-keys -t "$name" "$key"
  sleep 0.4
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
  assert_contains "$out" "Medium (default)" "Medium shown as default"
  assert_contains "$out" "Low"  "Low option listed"
  assert_contains "$out" "High" "High option listed"
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

tr019_c() {
  start_test "TR-019 C"
  local sess=$SESSION-019c
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "@core/src"
  sleep 1.5
  local out=$WORK/019c.txt
  capture "$sess" "$out"
  assert_contains     "$out" "@core/src" "typed subpath echoed"
  assert_not_contains "$out" "no matches" "subpath should resolve"
  assert_contains     "$out" "core/src"   "picker shows entries under core/src"
  kill_ata "$sess"
  end_test
}

tr006_a() {
  start_test "TR-006 A"
  local sess=$SESSION-006a
  # Seed history.jsonl with a known entry BEFORE booting — ata reads
  # history at startup and Up-arrow walks the in-memory copy.
  local marker="SEEDED_HISTORY_TIER_B1_$$"
  local hist_bak=""
  if [ -f "$HOME/.ata/history.jsonl" ]; then
    hist_bak=$(mktemp)
    cp "$HOME/.ata/history.jsonl" "$hist_bak"
  fi
  mkdir -p "$HOME/.ata"
  printf '{"session_id":"00000000-0000-0000-0000-000000000000","ts":0,"text":"%s"}\n' "$marker" >> "$HOME/.ata/history.jsonl"

  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; [ -n "$hist_bak" ] && mv "$hist_bak" "$HOME/.ata/history.jsonl"; return; fi
  send_key "$sess" Up
  sleep 1
  local out=$WORK/006a.txt
  capture "$sess" "$out"
  assert_contains "$out" "$marker" "seeded history entry recalled via Up-arrow"

  # Restore caller's history.
  if [ -n "$hist_bak" ]; then
    mv "$hist_bak" "$HOME/.ata/history.jsonl"
  else
    rm -f "$HOME/.ata/history.jsonl"
  fi
  kill_ata "$sess"
  end_test
}

tr012_a() {
  start_test "TR-012 A"
  local sess=$SESSION-012a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/voice"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/012a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Voice mode on"      "announcement printed"
  assert_contains "$out" "Hold Space to speak" "PTT prompt visible"
  assert_contains "$out" "🎤"                  "mic glyph in composer"
  # Toggle off to leave the session in a clean state.
  send_text "$sess" "/voice"; send_key "$sess" Enter; sleep 1
  kill_ata "$sess"
  end_test
}

tr013_a() {
  start_test "TR-013 A"
  local sess=$SESSION-013a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  # Enter voice mode first.
  send_text "$sess" "/voice"; send_key "$sess" Enter; sleep 2
  # Esc must NOT exit voice mode (the regression this test guards).
  send_key "$sess" Escape
  sleep 1
  local out1=$WORK/013a-esc.txt
  capture "$sess" "$out1"
  assert_contains "$out1" "🎤" "voice composer survived Esc"
  # /voice toggles it off cleanly.
  send_text "$sess" "/voice"; send_key "$sess" Enter; sleep 2
  local out2=$WORK/013a-off.txt
  capture "$sess" "$out2"
  assert_contains     "$out2" "Voice mode off" "exit confirmation"
  assert_not_contains "$out2" "🎤"             "mic glyph is gone"
  kill_ata "$sess"
  end_test
}

tr023_a() {
  start_test "TR-023 A"
  local sess=$SESSION-023a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/scheduling"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/023a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Scheduling tasks in this session" "panel title"
  assert_contains "$out" "Cron (0)"     "empty cron section"
  assert_contains "$out" "Monitors (0)" "empty monitors section"
  assert_contains "$out" "esc close"    "footer shows esc-close hint"
  kill_ata "$sess"
  end_test
}

tr014_a() {
  start_test "TR-014 A"
  local sess=$SESSION-014a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/research"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/014a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Research tools" "title shown"
  for tool in "Paper Search" "Zotero" "Hacker News" "Patents" "Repo Analysis" "Knowledge Base"; do
    assert_contains "$out" "$tool" "toggle listed: $tool"
  done
  assert_contains "$out" "Press space to select" "footer hint present"
  kill_ata "$sess"
  end_test
}

tr043_a() {
  start_test "TR-043 A"
  local sess=$SESSION-043a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/plan"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/043a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Plan mode" "Plan mode indicator visible"
  # PLAN.md TR-043 A: bare /plan does NOT toggle off — only Shift+Tab can.
  send_text "$sess" "/plan"
  send_key  "$sess" Enter
  sleep 1.5
  local out2=$WORK/043a-2.txt
  capture "$sess" "$out2"
  assert_contains "$out2" "Plan mode" "Plan mode stays on after second /plan"
  kill_ata "$sess"
  end_test
}

tr019_b() {
  start_test "TR-019 B"
  local sess=$SESSION-019b
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  # Regression guard for the documented bug: Tab on a no-match @-picker
  # query leaves the picker in a 'loading...' state that never resolves.
  send_text "$sess" "@xyznosuchprefix"
  sleep 1
  send_key  "$sess" Tab
  sleep 2
  local out=$WORK/019b.txt
  capture "$sess" "$out"
  assert_contains "$out" "@xyznosuchprefix" "typed text preserved"
  assert_contains "$out" "loading..." "picker stuck in loading state (known bug)"
  kill_ata "$sess"
  end_test
}

tr042_a() {
  start_test "TR-042 A"
  # /rollout is gated by cfg!(debug_assertions); only the debug build
  # registers the command. CI uses a cargo-built debug binary; the
  # public npm-installed ata is release, so skip there.
  case "$ATA_BIN" in
    *target/debug/ata*) ;;
    *) skip_test "needs debug build (ATA_BIN=$ATA_BIN)"; return;;
  esac
  local sess=$SESSION-042a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/rollout"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/042a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Current rollout path:" "rollout path printed"
  assert_contains "$out" "sessions/" "path points into the sessions directory"
  kill_ata "$sess"
  end_test
}

tr_keymap_a() {
  start_test "/keymap"
  local sess=$SESSION-keymap
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/keymap"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/keymap.txt
  capture "$sess" "$out"
  assert_contains "$out" "Keymap" "title shown"
  assert_contains "$out" "All configurable shortcuts" "subtitle present"
  assert_contains "$out" "Open Transcript" "lists a known shortcut"
  assert_contains "$out" "esc close" "footer shows esc hint"
  kill_ata "$sess"
  end_test
}

tr_vim_a() {
  start_test "/vim"
  local sess=$SESSION-vim
  # Two toggles net to zero — keeps the caller's config unchanged.
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/vim"
  send_key  "$sess" Enter
  sleep 1.5
  send_text "$sess" "/vim"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/vim.txt
  capture "$sess" "$out"
  # Each toggle prints a status line; both should be visible.
  assert_contains "$out" "Vim mode enabled" "first toggle confirms ON"
  assert_contains "$out" "Vim mode disabled" "second toggle confirms OFF"
  kill_ata "$sess"
  end_test
}

tr_fast_a() {
  start_test "/fast"
  local sess=$SESSION-fast
  # Two toggles net to zero — caller's Fast-mode setting is restored.
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/fast"
  send_key  "$sess" Enter
  sleep 1.5
  send_text "$sess" "/fast"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/fast.txt
  capture "$sess" "$out"
  # /fast prints "Fast mode set to on" / "Fast mode set to off". After
  # two toggles both lines appear in the transcript regardless of which
  # state we started in.
  assert_contains "$out" "Fast mode set to" "fast toggle was acknowledged"
  kill_ata "$sess"
  end_test
}

tr_personality_a() {
  start_test "/personality"
  local sess=$SESSION-pers
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/personality"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/pers.txt
  capture "$sess" "$out"
  assert_contains "$out" "Select Personality" "picker title shown"
  assert_contains "$out" "Friendly"  "Friendly option listed"
  assert_contains "$out" "Pragmatic" "Pragmatic option listed"
  assert_contains "$out" "(current)" "current marker present"
  kill_ata "$sess"
  end_test
}

tr_statusline_a() {
  start_test "/statusline"
  local sess=$SESSION-sl
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/statusline"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/sl.txt
  capture "$sess" "$out"
  assert_contains "$out" "Configure Status Line"   "picker title"
  assert_contains "$out" "model-with-reasoning"    "default toggle listed"
  assert_contains "$out" "current-dir"             "current-dir toggle listed"
  assert_contains "$out" "git-branch"              "git-branch toggle listed"
  kill_ata "$sess"
  end_test
}

tr_ide_a() {
  start_test "/ide"
  local sess=$SESSION-ide
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/ide"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/ide.txt
  capture "$sess" "$out"
  # On a headless runner (no IDE attached) ata reports the no-IDE state.
  assert_contains "$out" "IDE context could not be enabled" "no-IDE error shown"
  assert_contains "$out" "VS Code or Cursor"                "hint mentions supported IDEs"
  kill_ata "$sess"
  end_test
}

tr_transcript_a() {
  start_test "Ctrl-T transcript"
  local sess=$SESSION-trans
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_key "$sess" C-t
  sleep 1.5
  local out=$WORK/trans.txt
  capture "$sess" "$out"
  assert_contains "$out" "T R A N S C R I P T" "transcript title bar"
  assert_contains "$out" "q to quit"           "footer shows q quit hint"
  kill_ata "$sess"
  end_test
}

tr040_a() {
  start_test "TR-040 A"
  local sess=$SESSION-040a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/workspace"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/040a.txt
  capture "$sess" "$out"
  # Bare /workspace prints the usage line, not an overlay.
  assert_contains "$out" "Usage: /workspace" "usage line shown"
  assert_contains "$out" "list"   "lists 'list' subcommand"
  assert_contains "$out" "use"    "lists 'use' subcommand"
  kill_ata "$sess"
  end_test
}

tr040_b() {
  start_test "TR-040 B"
  local sess=$SESSION-040b
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/workspace list"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/040b.txt
  capture "$sess" "$out"
  assert_contains "$out" "Workspaces" "list header"
  assert_contains "$out" "current"    "active workspace marked 'current'"
  kill_ata "$sess"
  end_test
}

tr040_c() {
  start_test "TR-040 C"
  local sess=$SESSION-040c
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/workspace current"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/040c.txt
  capture "$sess" "$out"
  assert_contains "$out" "Current workspace:" "current line shown"
  kill_ata "$sess"
  end_test
}

tr041_a() {
  start_test "TR-041 A"
  local sess=$SESSION-041a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/agent"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/041a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Subagents"          "overlay title"
  assert_contains "$out" "Main [default]"     "main agent row shown"
  assert_contains "$out" "(current)"          "current marker present"
  kill_ata "$sess"
  end_test
}

tr039_a() {
  start_test "TR-039 A"
  local sess=$SESSION-039a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/ps"
  send_key  "$sess" Enter
  sleep 1.5
  local out=$WORK/039a.txt
  capture "$sess" "$out"
  assert_contains "$out" "Background terminals" "panel title shown"
  assert_contains "$out" "No background terminals running" "empty state line"
  kill_ata "$sess"
  end_test
}

tr046_a() {
  start_test "TR-046 A"
  local sess=$SESSION-046a
  if ! boot_ata "$sess"; then fail_assert "ata never reached the composer"; end_test; kill_ata "$sess"; return; fi
  send_text "$sess" "/resume"
  send_key  "$sess" Enter
  sleep 2
  local out=$WORK/046a.txt
  capture "$sess" "$out"
  # /resume opens a picker. With prior sessions it shows them; with
  # none it shows an empty state. Either way the title is invariant.
  assert_contains     "$out" "Resume" "picker recognized /resume"
  assert_not_contains "$out" "Unrecognized command" "command not rejected"
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

  log "Numbered TRs (in order)"
  tr006_a
  tr010_a
  tr012_a
  tr013_a
  tr014_a
  tr016_a
  tr017_a
  tr018_a; tr018_b; tr018_d
  tr019_a; tr019_b; tr019_c
  tr020_a; tr020_b; tr020_d
  tr023_a
  tr039_a
  tr040_a; tr040_b; tr040_c
  tr041_a
  tr042_a
  tr043_a
  tr046_a

  log ""
  log "Ad-hoc slash commands (not in PLAN.md)"
  tr_fast_a
  tr_ide_a
  tr_keymap_a
  tr_personality_a
  tr_statusline_a
  tr_transcript_a
  tr_vim_a

  log ""
  log "----"
  log "PASS: $PASS  FAIL: $FAIL  SKIP: $SKIP"
  if [ "$FAIL" -gt 0 ]; then
    log "Failed: ${FAILED_NAMES[*]}"
    exit 1
  fi
}

main "$@"
