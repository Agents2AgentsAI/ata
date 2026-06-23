# ATA TUI test plan

Manual + agent-driven functional tests for the ATA TUI. The agent uses the
`ata-tmux-test` skill to drive a live `./target/debug/ata` in a tmux pane and
verifies state via `tmux capture-pane`. Results land in `reports/<ISO>.md`.

Each test below has structured fields the agent reads literally:

- **Setup**: bash commands to put the TUI in a known state. Run in order; wait
  for the indicated state before proceeding.
- **Action**: numbered steps the agent performs. Side-effects are noted
  inline as `→ capture X` etc.
- **Expect**: bullet predicates. Each is a single assertion against a named
  capture. Predicates use this micro-DSL:
  - `<capture> contains "<substring>"` — pane text matches
  - `<capture> not contains "<substring>"` — pane text does not match
  - `<capture> row <n> starts with "<prefix>"`
  - `<capture> row <n> ends with "<suffix>"`

If every Expect predicate holds, the test PASSes. Otherwise it FAILs and the
report captures all `<capture>` snapshots verbatim.

The skill resolves `$ATA_REPO` at runtime (see `SKILL.md`). All paths below
use that variable — there is no hardcoded home directory.

## OS cron safety (mandatory)

Tests that create OS cron entries (TR-024, TR-027) leave entries in the
user's crontab that fire every minute. On macOS, every fire triggers a
TCC popup (`"ata" would like to access data from other apps`) because
neither the local debug binary (unsigned) nor the npm release binary
(adhoc-signed) has a Developer ID signature stable enough for TCC to
remember the Allow click. The popup also has a known macOS bug where
clicking Allow or Deny does not dismiss it, so a stuck cron will spam
the screen until the entry is removed.

**Mandatory teardown for any OS-cron test, run regardless of pass/fail**:

```bash
# 1. Remove every ata cron entry from the user's crontab.
crontab -l 2>/dev/null | grep -vE 'ata-cron|/ata exec' | crontab - 2>/dev/null \
  || crontab -r 2>/dev/null

# 2. Kill any in-flight ata exec children spawned by cron before deletion.
pkill -9 -f 'ata exec' 2>/dev/null

# 3. Verify: both must report empty.
crontab -l 2>/dev/null | grep -i ata    # must print nothing
pgrep -fl 'ata exec'                    # must print nothing
```

If the test runner aborts mid-run (Ctrl-C, crash, timeout), run the
three commands above manually before walking away from the machine.

---


# Group 1: TUI infrastructure (startup, smoke, escape)

## TR-003: TUI startup smoke

**Setup**: Build (`cd "$ATA_REPO/codex-rs" && cargo build -p codex-cli`),
then split a tmux pane running `./target/debug/ata --yolo` and wait until
the pane shows the welcome banner.

**Action**: `tmux capture-pane -t <new> -p > <capture>`.

**Expect**:
- `capture` contains `Agents2Agents ata (v`
- `capture` contains `YOLO mode` (if launched with `--yolo`)
- `capture` contains `directory:`

---

## TR-005: Chat round-trip

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "respond with just hi"`; sleep 1; `Enter`.
2. Poll until `tmux capture-pane -t <new> -p | grep -E "^• [Hh]i\b"`.
3. → capture `post`.

**Expect**:
- `post` contains `respond with just hi` (the user message, with `›` marker)
- `post` matches `^• [Hh]i\b` (response line in chat)

---

## TR-022: Escape interrupts an in-flight turn cleanly

The Action below polls at a tight 0.2s cadence and uses a heavy prompt because fast reasoning models can finish a response before a slower poll fires, racing past the interrupt window.

The thinking indicator reads `esc to interrupt`, so Escape is the
documented interrupt key. Pressing it while the agent is working stops
the turn and shows `Conversation interrupted - tell the model what to do
differently.` plus a `/feedback` hint. (Escape has context-dependent
behavior: it edits the previous message when idle — the voice-mode case is
covered by `specs/voice-tts.md` — and interrupts when a turn is
mid-flight.)

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "write me a 5000-word essay analyzing the history of espresso, with detailed sections on origin, technique, and modern variations"`; sleep 0.5; `Enter`.
2. Tight-poll up to 15s, sleeping 0.2s per iteration, until `tmux capture-pane -t <new> -p` contains `esc to interrupt`. As soon as the substring is seen, immediately `tmux send-keys -t <new> Escape` in the same iteration (do NOT wait for the next poll).
3. Sleep 1.
4. → capture `out`.

**Expect**:
- `out` contains `Conversation interrupted`
- `out` contains `tell the model what to do differently`
- `out` contains `/feedback`
- `out` not contains `esc to interrupt` — thinking indicator cleared

---


# Group 2: Composer and history

## TR-004: Slash menu opens

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/"`; sleep 1.
2. `tmux capture-pane -t <new> -p > <capture>`.

**Expect**:
- `capture` contains `/model`
- `capture` contains `/experimental`
- `capture` contains `/permissions`

---

## TR-006: Up-arrow history

**Setup**: TR-005 (submit at least one message first).

### Scenario A: baseline — Up recalls last submission
1. `tmux send-keys -t <new> C-u`; sleep 0.3.
2. `tmux send-keys -t <new> Up`; sleep 0.5.
3. → capture `up`.

**Expect**:
- `up` contains `respond with just hi` in the composer (`›` line)

### Scenario B: history persists across /clear
1. From TR-005 state with the prior submission visible.
2. Send `/clear`; Enter; sleep 2.
3. `tmux send-keys -t <new> Up`; sleep 0.5. → capture `up_after_clear`.

**Expect**:
- `up_after_clear` contains `respond with just hi` in the composer
- (Proves `/clear` does NOT wipe `~/.ata/history.jsonl` — Up still recalls)

### Scenario C: Up/Down navigates bidirectionally
1. From C-u (empty composer), press Up four times: → composer cycles through entries 1..4 oldest-first as we move further back.
2. Then press Down once: → composer moves forward to entry 3.

**Expect**:
- Each Up changes the composer to an older entry
- Down moves forward (toward more recent)
- No "end of history" indicator at oldest entry — silently stops moving
- History file path: `~/.ata/history.jsonl`

### Scenario D: in-session Up-arrow buffer is broader than persistent history.jsonl
1. In one ata session: run `/model`; Esc out. Run `/permissions`; Esc out. Send chat `hi`.
2. Now `tmux send-keys C-u; Up; Up; Up` and observe what the composer recalls.
3. Inspect `tail -3 ~/.ata/history.jsonl | jq -r .text` and compare.

**Expect**:
- Up-arrow recalls `/model`, `/permissions`, `hi` (slash commands included)
- `history.jsonl` only contains `hi` (recognized slash commands EXCLUDED from persistent history)
- This divergence means slash commands are recallable in the same session but not across restarts.

---

## TR-009: Up-arrow history excludes system-injected prompts

When voice-mode prefixes or reading-view question wrappers are sent to the
agent, they should NOT appear in the up-arrow history (the user typed only
the visible part).

**Setup**: a reading view open (see `specs/reading-view.md` for the config
gating and how to open one) + at least one Tab-to-ask submission from the reader.

### Scenario A: baseline — reader/voice wrappers excluded from Up
1. Close reader, return to chat.
2. `tmux send-keys -t <new> C-u`; sleep 0.3.
3. For i in 1..8: `tmux send-keys -t <new> Up`; sleep 0.2.
4. → capture `history`.

**Expect** (all must hold):
- `history` not contains `[The user is reading`
- `history` not contains `<voice>`
- `history` not contains `<!-- READER_TOOL_INSTRUCTIONS`
- `history` not contains `[The user closed the document reader`

### Scenario B: in-session Up-arrow INCLUDES slash commands
1. In a fresh session, run `/model`; Esc out (no model change).
2. Run `/permissions`; Esc out.
3. Send chat `hi`; wait for reply.
4. `tmux send-keys -t <new> C-u; Up; Up; Up`; capture composer after each.

**Expect**:
- After Up #1: composer shows `hi`
- After Up #2: composer shows `/permissions`
- After Up #3: composer shows `/model`
- Slash commands ARE recallable in the same session via Up-arrow.

### Scenario C: persistent ~/.ata/history.jsonl EXCLUDES recognized slash commands
1. After Scenario B's session: `tail -3 ~/.ata/history.jsonl | jq -r .text`.

**Expect**:
- Only `hi` appears
- `/model` and `/permissions` are NOT in the persistent file
- An unrecognized slash like `/clear extra junk args` (which falls back to chat per TR-020 D) DOES appear in persistent history (it's a regular user message)
- This divergence is intentional: session-level buffer is broader; persistent buffer is restricted to chat input.

---

## TR-019: @ file-mention autocomplete + Tab accepts top match

Typing `@<prefix>` in the composer pops an autocomplete with matching repo
files; Tab selects the top entry and inserts it into the composer. An
empty `@` should render "no matches" (i.e. the picker is alive but has
nothing to suggest yet).

**Setup**: TR-003 setup. Run from a repo with multiple `Cargo.toml` files
(`$ATA_REPO/codex-rs` is the canonical case).

### Scenario A: baseline — Cargo prefix + Tab
1. `tmux send-keys -t <new> "@"`; sleep 0.5. → capture `empty`.
2. `tmux send-keys -t <new> "Cargo"`; sleep 0.5. → capture `prefix`.
3. `tmux send-keys -t <new> Tab`; sleep 0.5. → capture `accepted`.
4. Cleanup: `tmux send-keys -t <new> C-u`.

**Expect**:
- `empty` contains `no matches`
- `prefix` contains `Cargo.toml` AND `Cargo.lock` (multi-result rendered)
- `accepted` contains `› Cargo.toml` (top match inserted)
- `accepted` not contains `no matches` AND not contains `@Cargo`

### Scenario B: no-match prefix shows "no matches" (picker stays open)
1. `tmux send-keys -t <new> "@xyznosuchprefix"`; sleep 1.
2. → capture `out`.
3. Cleanup: backspace until composer empty.

**Expect**:
- `out` contains `no matches`
- Picker remains open (still under composer)

### Scenario B2: Tab on a no-match query returns to `no matches`
1. After Scenario B's `@xyznosuchprefix` is typed (no matches visible):
2. `tmux send-keys -t <new> Tab`; sleep 3.
3. → capture `out`.

**Expect**:
- `out` contains `no matches`
- `out` not contains `loading...`

### Scenario C: subdirectory path traversal in @-picker
1. `tmux send-keys -t <new> "@core/src"`; sleep 1.
2. → capture `out`.
3. Cleanup: backspace clear.

**Expect**:
- `out` contains `core/src` (top entry)
- `out` contains multiple `core/src/<subdir>` entries (proves subdir traversal)
- Picker accepts both files AND directories

### Scenario D: Tab accepts top match; second Tab does NOT cycle
1. `tmux send-keys -t <new> "@Cargo"`; sleep 0.5.
2. `tmux send-keys -t <new> Tab`; sleep 0.3. → capture `tab1`.
3. `tmux send-keys -t <new> Tab`; sleep 0.3. → capture `tab2`.

**Expect**:
- `tab1` contains `› Cargo.toml` (top match accepted into composer, picker dismissed)
- `tab2` does NOT show the second-match entry (`tui/Cargo.toml`) in composer — Tab does not cycle through matches; it either submits or triggers focus action depending on context
- After tab2: composer either submits `Cargo.toml` or the agent indicator appears (`• Working`)

### Scenario E: Escape does NOT dismiss the @-picker
1. `tmux send-keys -t <new> "@xyz"`; sleep 0.5.
2. `tmux send-keys -t <new> Escape`; sleep 0.5.
3. → capture `out`.

**Expect**:
- `out` still shows the @-picker (no matches view or matches view)
- Escape is consumed by the composer, NOT by the picker
- Only backspacing through the `@` (or accepting via Tab) closes the picker

---

## TR-020: Unknown slash command shows a helpful hint

ata does not have a `/help` command; when an unrecognized slash is sent,
the TUI must print `Unrecognized command '<name>'. Type "/" for a list…`
rather than silently forwarding the text to the agent or crashing.

**Setup**: TR-003 setup.

### Scenario A: baseline — unknown slash
1. `tmux send-keys -t <new> "/help"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `out`.

**Expect**:
- `out` contains `Unrecognized command '/help'`
- `out` contains `Type "/" for a list of supported commands`

### Scenario B: typo of a real command (no fuzzy hint, composer retains text)
1. `tmux send-keys -t <new> "/clera"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `out`.

**Expect**:
- `out` contains `Unrecognized command '/clera'`
- `out` not contains `did you mean` (ata does NOT suggest near-matches)
- Composer keeps the typed text (`/clera`) so the user can edit and resend without re-typing

### Scenario C: slash commands are case-insensitive
1. Plant a marker: `tmux send-keys "marker-xyz"; Enter; sleep 8` (let agent reply).
2. `tmux send-keys -t <new> "/CLEAR"`; sleep 0.3; `Enter`; sleep 2.
3. → capture `out`.

**Expect**:
- `out` not contains `marker-xyz` (uppercase `/CLEAR` actually cleared the chat — proves case-insensitive parser)
- `out` contains `To continue this session, run ata resume`
- `out` not contains `Unrecognized command`

### Scenario D: recognized command with extra args falls through to chat
1. `tmux send-keys -t <new> "/clear extra junk args"`; sleep 0.3; `Enter`; sleep 8.
2. Inspect session JSONL: `jq -r 'select(.payload.type=="user_message") | .payload.content[0].text' $SESS | tail -1`.

**Expect**:
- JSONL contains `/clear extra junk args` as a user message (sent to agent, not parsed as slash command)
- TUI output shows the agent's reply, NOT the system `Cleared.` ack
- Slash parser is strict for arg-less commands: trailing text disables slash routing

### Scenario E: bare `/` opens slash picker
1. `tmux send-keys -t <new> "/"`; sleep 1.
2. → capture `out`.

**Expect**:
- `out` contains `/model` and `/permissions` and `/scheduling` (picker list visible)
- `out` not contains `Unrecognized command`

---

## TR-034: `@` file mention is path-injection, not content-injection

A common user misconception: typing `@Cargo.toml` and sending must
auto-attach the file's content to the agent's prompt. It does NOT.
Tab-accepting the `@` completion inserts the literal filename as
plain text into the composer; the user message that reaches the agent
contains only the path string, no file content. The agent has to
explicitly read the file via a tool call (`exec_command sed`, `read`,
etc.) to actually inspect it. This test documents that invariant and
catches two regression classes:

1. False auto-attach: a future change "improves" @ mention by silently
   attaching content. That might be a feature, but it must be intentional
   and detectable — this test would catch the unannounced change.
2. Lost path injection: a future change breaks the Tab-accept path so
   the filename never reaches the user message at all (agent has nothing
   to read).

**Setup**: TR-003 setup.

**Action**:
1. In ata, type `@Cargo` (no Enter yet); sleep 0.5. Picker should show matches.
2. Press `Tab` to accept the top match (`Cargo.toml`); sleep 0.3.
3. Type ` explain this file` (space + question); sleep 0.5.
4. Press `Enter`. Sleep 1.
5. Poll up to 3 min until pane contains `[workspace]` OR `workspace members` OR a clear sign the agent actually read the file (ata-specific terms like `reading-view-server`, `scheduling`, `codex-workspace`).
6. → capture `response`.
7. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`
   - `grep -o '"text":"[^"]*Cargo[^"]*"' "$SESS" | head -1 > <user_msg>` → capture `user_msg`.
   - `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>` → capture `tool_counts`.
   - `jq -r 'select(.payload.name=="exec_command") | .payload.arguments' "$SESS" > <exec_args>` → capture `exec_args`.

**Expect** (all must hold):

User message — path string is what reaches the agent:
- `user_msg` contains `Cargo.toml explain this file` — literal filename + question
- `user_msg` not contains `[workspace]` — the FILE CONTENT was NOT injected into the user message (this is the headline invariant)
- `user_msg` not contains `[patch.crates-io]` — same: no content auto-attached
- `user_msg` not contains `serde =` — same: no inline manifest dependencies

Agent — must read the file via a tool to answer:
- `tool_counts` contains `exec_command` OR a dedicated read-file tool — the agent went and read the file itself
- `exec_args` contains `Cargo.toml` — the read call targeted the right file

Pane — response cites ata-specific Cargo.toml content (proves the read actually happened):
- `response` contains `reading-view-server` OR `scheduling` OR `codex-workspace` — names that only exist in ata's Cargo.toml, not a hallucinated generic manifest
- `response` contains `Cargo.toml` — the file is named in the response

---


# Group 3: Slash commands

## TR-015: superseded by TR-042

The original `/rollout` smoke test is replaced by TR-042, which covers both the public-release path (where `/rollout` is unrecognized) and the debug-build path (where it prints the live session JSONL with a cross-check against the actual file on disk). See TR-042 below.

---

## TR-017: /permissions menu opens, marks current, dismisses without changing

`/permissions` shows the three permission tiers (Default / Auto-review /
Full Access) with the active one marked `(current)`. Escape must dismiss
the menu *without* mutating the active permission — a regression where
Escape silently saves would degrade or escalate access.

**Setup**: TR-003 setup (launched with `--yolo`, so Full Access is active).

### Scenario A: baseline — open, current marked, Esc dismisses without change
1. `tmux send-keys -t <new> "/permissions"`; sleep 0.5; `Enter`; sleep 1. → `menu`.
2. `tmux send-keys -t <new> Escape`; sleep 1. → `dismissed`.

**Expect**:
- `menu` contains `Update Model Permissions`
- `menu` contains `Default`, `Auto-review`, `Full Access`
- `menu` contains `Full Access (current)` (--yolo maps to Full Access)
- `dismissed` not contains `Update Model Permissions` (menu gone)
- `dismissed` contains `permissions: YOLO mode` (banner unchanged)

### Scenario B: elevating to Full Access triggers a confirmation dialog
1. Pre-state: launch ata WITHOUT `--yolo` so current is Default. Open `/permissions`.
2. Navigate to `Full Access` (Down twice) and press Enter. → `confirm`.

**Expect**:
- `confirm` contains `Enable full access?` header
- `confirm` contains three options:
  - `Yes, continue anyway` (this session)
  - `Yes, and don't ask again` (persist to config)
  - `Cancel` (go back)
- Cancel returns to the picker without changing permissions
- Even re-selecting the current Full Access (from a `--yolo` start) re-shows the confirmation — the elevation gate is unconditional.

### Scenario C: downgrading to Default or Auto-review is immediate (no confirmation)
1. From a Full Access state, open `/permissions`, navigate to `Default` (option 1) and press Enter.

**Expect**:
- TUI prints `Permissions updated to Default` immediately
- NO `Enable ... access?` confirmation step
- The picker closes; chat resumes

### Scenario D: welcome banner shows LAUNCH-TIME permissions, not current
1. Launch with `--yolo` (banner shows `permissions: YOLO mode`).
2. Use `/permissions` to switch to Default. Confirm via `Permissions updated to Default`.
3. Re-open `/permissions`. The picker correctly shows `Default (current)`.

**Expect**:
- Despite Default now being active, the WELCOME BANNER at the top of the chat still shows `permissions: YOLO mode`
- Banner reflects launch-time flag, not runtime mutations
- The picker is the source of truth for current state; banner is stale by design (or by oversight).

### Scenario E: /permissions hard-blocked during in-flight
1. Submit a slow prompt (`respond with just hi`); Enter; sleep 1.
2. `/permissions`; Enter; sleep 2.

**Expect**:
- TUI prints `'/permissions' is disabled while a task is in progress.`
- Menu does NOT open

---

## TR-038: /copy — full behavior matrix

`/copy` is a TUI-only command. It grabs the last `•` agent line,
formats it as markdown, and writes it to the system clipboard. Six
scenarios in a 2x3 matrix: simple/multi-line/special content × normal
chat / side conversation / post-clear state. Plus a "no message to
copy" negative case and an in-flight-turn check.

**Setup**: TR-003 setup. Save existing clipboard so the test doesn't
trash user data: `ORIG=$(pbpaste)`; restore in cleanup.

### Scenario A: simple single-line agent message (baseline)

1. `tmux send-keys -t <new> "respond with just hi"`; sleep 1; `Enter`.
2. Poll until pane matches `^• [Hh]i\b`.
3. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
4. → capture `out_simple`; `pbpaste > <clipboard_simple>`.

**Expect**:
- `out_simple` contains `Copied last message to clipboard`
- `out_simple` matches `^• [Hh]i\b` — prior message still rendered
- `clipboard_simple` matches `^[Hh]i\s*$` — clipboard is just `hi` with no extra framing

### Scenario B: multi-line agent message with markdown structure

5. `tmux send-keys -t <new> "respond with a 5-item numbered list of fruits, no preamble, no postscript"`; sleep 1; `Enter`.
6. Poll up to 60s until pane contains `5.` on a numbered-list line.
7. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
8. → capture `out_list`; `pbpaste > <clipboard_list>`.

**Expect**:
- `out_list` contains `Copied last message to clipboard`
- `clipboard_list` matches `^\s*1\.\s` — list starts with `1.`
- `clipboard_list` contains `\n2.` — markdown line breaks preserved
- `clipboard_list` contains `5.` — full content (not truncated)
- `clipboard_list` not contains `›` — user-prompt marker is NOT in clipboard
- `clipboard_list` not contains `• ` — TUI bullet prefix stripped (clipboard is raw markdown, not the rendered TUI form)

### Scenario C: agent response with a fenced code block

9. `tmux send-keys -t <new> "show me the rust hello world program in a fenced code block, no preamble"`; sleep 1; `Enter`.
10. Poll up to 60s until pane contains `fn main` AND ```` ``` ```` (fenced).
11. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
12. → capture `out_code`; `pbpaste > <clipboard_code>`.

**Expect**:
- `clipboard_code` contains ```` ```rust ```` OR ```` ``` ```` — code fence preserved in markdown
- `clipboard_code` contains `fn main` — code body preserved
- `clipboard_code` contains `println!` — verbatim code character preserved
- `clipboard_code` not contains `  └` — TUI's left-margin glyph for tool results is stripped

### Scenario D: negative — no agent message to copy (fresh session)

13. `tmux send-keys -t <new> "/clear"`; sleep 0.5; `Enter`; sleep 2.
14. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
15. → capture `out_empty`; `pbpaste > <clipboard_empty>`.

**Expect**:
- `out_empty` contains `No agent response to copy` — exact error string
- `out_empty` not contains `Copied last message to clipboard`
- `clipboard_empty` not contains `No agent response to copy` — the error string itself was NOT pushed to the clipboard (would be a regression where ata writes its own UI error into the user's clipboard)

(Note: predicate `clipboard equals Scenario C value` is not used because external clipboard activity between scenarios can confound it. The invariant we actually care about is that ata's empty-case error doesn't leak into the clipboard.)

### Scenario E: copy from inside a /side conversation

**Precondition (critical)**: `/side` requires the current conversation
to have at least one completed user→agent turn since the most recent
`/clear` or session start. Hitting `/side` on a fresh or freshly-cleared
session prints `'/side' is unavailable until the current conversation
has started. Send a message first, then try /side again.` So Scenario D's `/clear` leaves us in the "conversation not
started" state — we must re-prime with a turn before /side.

16. Re-prime the conversation: `tmux send-keys -t <new> "respond with just hello from side parent"`; sleep 1; `Enter`. Poll until pane matches `^• hello from side parent\b` (proves the turn completed).
17. `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`; sleep 2. Wait until side-conversation context label appears AND an agent response is rendered.
18. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
19. → capture `out_side`; `pbpaste > <clipboard_side>`.

**Expect**:
- `out_side` contains `Copied last message to clipboard` — /copy works inside /side
- `out_side` contains `Side from main thread · Esc to return` — side context label confirms we ARE in a side conversation (not main thread)
- `clipboard_side` contains `4` — the side-conversation answer, not the parent thread's most recent agent message
- `clipboard_side` not contains `primed` (or whatever the parent thread's last agent message was) — side scope is respected for copy

### Scenario E2 (negative): /side blocked on a not-started conversation

This is a separate-test variant of the precondition above — explicit
coverage that the negative case prints the expected error, and that
the error is recoverable by sending a message.

20. `/clear` to reset to not-started state.
21. `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`; sleep 1.
22. → capture `side_blocked`.
23. `tmux send-keys -t <new> "respond with just primed"`; sleep 1; `Enter`. Poll until `^• primed\b`.
24. `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`; sleep 2.
25. → capture `side_recovered`.

**Expect**:
- `side_blocked` contains `'/side' is unavailable until the current conversation has started.`
- `side_blocked` contains `Send a message first, then try /side again.`
- `side_blocked` not contains `2+2` answered (no agent response was generated)
- `side_recovered` contains a side-conversation context label OR the side answer (`4`) — proves that priming the conversation un-blocks /side

### Scenario F: /copy during an in-flight turn

20. Exit side: `Escape` until back in main chat.
21. `tmux send-keys -t <new> "write me a 1000-word essay on espresso"`; sleep 0.3; `Enter`.
22. Tight-poll up to 10s for `esc to interrupt`. Within that window: `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 0.5.
23. → capture `out_inflight`.
24. `tmux send-keys -t <new> Escape`; sleep 1 (cancel the long essay).

**Expect**:
- `out_inflight` contains `Copied last message to clipboard` — /copy is allowed during an in-flight turn (no blocking)
- `clipboard_inflight` contains the previous COMPLETED agent message body (e.g. `primed` if that was the last main-thread reply) — NOT the partial essay still streaming
- `clipboard_inflight` not contains a substring from the in-flight essay (e.g. `espresso` if the essay had started writing but hadn't completed) — proves /copy grabs the last *completed* turn, not the in-flight one
- After /copy: the essay turn keeps running. The pane still shows `esc to interrupt`. /copy is non-blocking AND non-cancelling.
- An explicit `Escape` (separate keystroke from /copy) is what cancels the turn, producing `Conversation interrupted - tell the model what to do differently.` (the TR-022 invariant).

**Cleanup**: `printf "%s" "$ORIG" | pbcopy` (restore original clipboard). Also `/clear` to reset chat for the next test.

---

## TR-039: /ps — full behavior matrix

`/ps` lists processes the agent has spawned via background tool calls.
Six scenarios: empty state, single live process, multiple processes,
just-completed process (lifecycle of the row), PID cross-check with
system `ps`, and overlap-vs-divergence with `/scheduling`.

**Setup**: TR-003 setup.

### Scenario A: empty state

1. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `empty`.

**Expect**:
- `empty` contains `Background terminals` — heading
- `empty` contains `No background terminals running.` — empty-state copy

### Scenario B: persistent interactive shell populates /ps (the actual happy path)

**What /ps tracks**:
- `/ps` enumerates `unified_exec_processes` — long-lived interactive shells started by `exec_command` with `ExecCommandSource::UnifiedExecStartup`. These are persistent processes the agent can `write_stdin` to (Python REPL, interactive bash, node REPL, etc.).
- `/ps` does NOT track: `monitor_start` rows (those go to `/scheduling` Monitors), `exec_command(background:true)` async commands (also Monitors), `cron_create*` (Cron section), or one-shot `exec_command` calls (no persistent process).
- Naming caveat: the heading reads "Background terminals" but in practice it's "Persistent interactive shells".

3. Send: `open a persistent python REPL and don't close it`. Poll up to 60s until pane contains both `Persistent Python REPL is open` AND `/ps to view · /stop to close` (the footer hint that appears when at least one shell is alive).
4. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
5. → capture `populated`.

**Expect**:
- `populated` contains `Background terminals` — heading
- `populated` not contains `No background terminals running.` — empty-state copy is gone
- `populated` matches `^\s*•\s+python` — at least one bullet row for the python process
- `populated` contains `Python 3.` — recent stdout/stderr preview is rendered under the row (last N stream chunks)
- `populated` contains `>>>` — REPL prompt visible in the preview (proves recent_chunks is being captured live)
- `populated` contains `1 background terminal running` — footer counter
- `populated` contains `/ps to view · /stop to close` — footer hint

### Scenario B2: monitor-spawned process DOES NOT show in /ps (negative)

starting a monitor with `start a monitor named tr039-bg that runs: sleep 90` puts the row in `/scheduling` under `Monitors (1) [Running]`, but `/ps` continues to report `No background terminals running.` Same for `exec_command(background:true)` — they appear in `/scheduling` as `Monitors` rows, `pgrep -fl` returns a live PID, and yet `/ps` shows empty. This is by design: `/ps` is scoped to `unified_exec` startup processes only.

6. Send: `start a monitor named tr039-bg that runs: sleep 90`. Wait until pane contains `Started monitor tr039-bg`.
7. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
8. → capture `monitor_check`.

**Expect**:
- `monitor_check` contains `python` — the prior REPL row still listed
- `monitor_check` not contains `tr039-bg` — monitor is NOT enumerated by /ps
- `monitor_check` matches `^\s*•\s+python\s*$|^\s*•\s+python\b` — still exactly 1 bullet row (the python REPL), monitors don't add rows
- `monitor_check` matches `1 background terminal running` — counter unchanged from Scenario B (still 1, not 2)
- Cross-check (open `/scheduling`): contains `Monitors (1)` with `tr039-bg` `[Running]` — proves the monitor IS being tracked, just by a different surface

### Scenario C: /ps and /scheduling have non-overlapping coverage (verified)

`/scheduling` shows: in-session crons (`cron_create_session`), OS crons (`cron_create`), monitors (`monitor_start`), and async background shell commands (`exec_command(background:true)`). `/ps` shows: persistent interactive shells (`unified_exec` startup). The two surfaces never duplicate a row.

9. With the python REPL alive (Scenario B) AND a monitor alive (Scenario B2): `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
10. → capture `sched_view`.
11. `tmux send-keys -t <new> Escape`; sleep 1.
12. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
13. → capture `ps_view`.

**Expect**:
- `sched_view` contains `Monitors (1)` — the monitor row from B2 is tracked
- `sched_view` contains `tr039-bg` — by name
- `sched_view` not contains `python` — REPL is NOT in scheduling (it's a unified_exec shell, different surface)
- `ps_view` contains `python` — REPL IS in /ps
- `ps_view` not contains `tr039-bg` — monitor is NOT in /ps
- `ps_view` contains `1 background terminal running` — counter only includes the REPL

### Scenario D: /stop closes all persistent shells

When /ps shows ≥1 row, the footer hint promises `/stop to close`. This scenario verifies the cleanup tool actually works and /ps is empty afterward.

14. `tmux send-keys -t <new> "/stop"`; sleep 0.5; `Enter`; sleep 1.
15. → capture `stop_out`.
16. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
17. → capture `ps_after_stop`.
18. From outside ata: `pgrep -fl "python3" | grep -v Adobe | grep -v JetBrains > <pgrep_after>` (filter common false positives).

**Expect**:
- `stop_out` contains `Stopping all background terminals.` — exact confirmation copy
- `ps_after_stop` contains `No background terminals running.` — empty state restored
- `ps_after_stop` not contains `python` — the REPL row is gone
- `<pgrep_after>` does NOT contain the python REPL PID from before — the OS process was actually killed (not just dropped from UI; to fully verify, snapshot `pgrep` before /stop and diff)

### Scenario E: single-shell `/ps` output shape

when one `exec_command` (unified persistent) session is open, `/ps` renders exactly:

```
Background terminals

  • bash -i

  1 background terminal running · /ps to view · /stop to close
```

**Expect**:
- Header line: `Background terminals`
- Per-row format: `  • <command>` (two-space indent, bullet, command line as the agent invoked it)
- Footer: `<N> background terminal[s] running · /ps to view · /stop to close` (singular/plural ".terminal/.terminals" varies with N)

Note: `/ps` output does NOT expose PIDs in 0.7.0 — only the command line. PID cross-check via system `pgrep` is the only way to verify the underlying OS process, which is covered in Scenario D's `pgrep_after`.

### Scenario F: multi-shell `/ps` (predicted, needs reliable repro)

Spawning a SECOND persistent shell via the agent proved unreliable in manual testing — the agent often reuses or refuses to open a second session even when explicitly prompted. When reliably reproducible:

**Expect** (predicted from Scenario E pattern):
- Two `  • <cmd>` rows, one per shell
- Footer count: `2 background terminals running` (note plural)
- Row order: stable (likely creation order)

Method that may work: directly invoke the tool API rather than going through the agent (bypassing the LLM's session-reuse preference). If/when verified, replace this predicted block with locked predicates.

**Cleanup**:
- `/stop` (if not already done in Scenario D) — clears all unified_exec shells.
- `/scheduling` → `d` per row to remove monitor entries.
- Verify with `pgrep -fl "sleep 90"` and `pgrep -fl "python3.*REPL"` both returning nothing — if anything is alive, `pkill -f` the survivor.

---

> Converted to `specs/workspace.md` (agentic behavioral spec): the former
> TR-040 (/workspace TUI matrix) and Group 7 (TR-055..060, workspace CLI).

> Converted to `specs/subagents.md` (agentic behavioral spec): the former
> TR-041 (/agent and /subagents matrix) and TR-044 (/side).

## TR-042: /rollout is debug-build only (gated by cfg!(debug_assertions))

`/rollout` prints the current session's JSONL rollout file path. The
handler is wrapped in `if cfg!(debug_assertions)` so the variant is
stripped from release builds. This is by design — rollout paths are a
debugging aid, not a user-facing feature on the public npm release.
This TR documents both states so a release build that suddenly EXPOSES
`/rollout`, or a debug build that loses it, both register as
regressions.

**Setup**: TR-003 setup. Run in two passes — once against
`./target/debug/ata --yolo` (debug build) and once against the npm
release binary (which is built with `--release`). Record build type
per pass.

### Scenario A: public release / `--release` build → unrecognized command

1. Confirm binary type: `file $(which ata)` should show node script (npm) OR `ata --version` should NOT contain debug build markers. Record as `BUILD=release`.
2. `tmux send-keys -t <new> "/rollout"`; sleep 0.5; `Enter`; sleep 1.
3. → capture `release_out`.

**Expect**:
- `release_out` contains `Unrecognized command '/rollout'`
- `release_out` contains `Type "/" for a list of supported commands`
- `release_out` not contains `Current rollout path:` — handler did not execute
- `release_out` not contains `.jsonl` from `/rollout` output (any prior `.jsonl` reference in scrollback is fine)

### Scenario B: debug build → prints session rollout path

4. Switch to debug binary: launch `./target/debug/ata --yolo` (rebuild if needed with `cargo build -p codex-cli`). Record `BUILD=debug`.
5. `tmux send-keys -t <new> "/rollout"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `debug_out`.

**Expect**:
- `debug_out` contains `Current rollout path:` — handler executed
- `debug_out` contains `.ata/sessions/` — path is the standard sessions dir
- `debug_out` contains `rollout-` — filename prefix
- `debug_out` contains `.jsonl` — file extension
- `debug_out` not contains `Unrecognized command` — command IS recognized in debug
- The printed path corresponds to an existing readable file: `ls -la <extracted_path>` succeeds and returns a non-zero size.

### Scenario C: rollout path matches the actual current session

7. Extract the printed path from `debug_out` as `RPATH`.
8. Cross-check: `ACTUAL=$(find ~/.ata/sessions -name "*.jsonl" -mmin -2 | xargs ls -t | head -1)`.

**Expect**:
- `RPATH` equals `$ACTUAL` (printed path is the live session, not stale)
- `[ -f "$RPATH" ]` (file exists on disk)

**Notes**:
- If a future change moves the handler out of `cfg!(debug_assertions)` (intentional public exposure), Scenario A predicates flip and we update the test to match.

---

## TR-043: /plan — Plan-mode toggle, inline-args submit, Shift+Tab binary toggle

`/plan` enters Plan mode (a UI/reasoning-budget variant of normal chat).
Three things to verify: (a) bare `/plan` toggles the mode on/off,
(b) `/plan <text>` enters mode AND submits the inline text in one go,
(c) `Shift+Tab` is a binary toggle of the same mode (NOT a multi-mode
cycle despite the hint saying "to cycle").

**Setup**: TR-003 setup. Plan-mode must be feature-enabled — verified
on by default on ata 0.7.0 public release.

### Scenario A: bare /plan turns ON (it does NOT toggle off — Shift+Tab does that)

bare `/plan` only activates Plan mode. A second `/plan` does NOT turn it off — the footer still shows `Plan mode (shift+tab to cycle)`. The only way to turn off Plan mode is `Shift+Tab` (the binary toggle covered in Scenario C). Earlier-pass observations that suggested `/plan` was a toggle were measuring activation, not deactivation.

1. `tmux send-keys -t <new> "/plan"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `on1`.
3. `tmux send-keys -t <new> "/plan"`; sleep 0.5; `Enter`; sleep 1.
4. → capture `on2_still_on`.
5. `tmux send-keys -t <new> BTab`; sleep 0.5.  (Shift+Tab)
6. → capture `off`.

**Expect**:
- `on1` footer contains `Plan mode (shift+tab to cycle)` — mode indicator present after first activation
- `on2_still_on` footer ALSO contains `Plan mode (shift+tab to cycle)` — second `/plan` did NOT toggle off (re-confirm behavior)
- `off` footer does NOT contain `Plan mode (shift+tab to cycle)` — Shift+Tab is what actually turns it off

### Scenario B: /plan with inline args enters mode AND submits

5. `tmux send-keys -t <new> "/plan respond with just hi"`; sleep 0.5; `Enter`. Poll up to 60s until pane matches `^• [Hh]i\b`.
6. → capture `inline`.

**Expect**:
- `inline` contains `› respond with just hi` — inline text was submitted as a user message
- `inline` matches `^• [Hh]i\b` — agent responded
- `inline` contains `Plan mode (shift+tab to cycle)` — Plan mode active during/after
- `inline` not contains `/plan respond with just hi` on the user-message line — the `/plan ` prefix was stripped before submission (the agent sees only the prompt text, not the slash)

### Scenario C: Shift+Tab is a binary toggle (NOT a multi-mode cycle)

7. With Plan mode ON: `tmux send-keys -t <new> BTab`; sleep 0.5. → capture `bt1`.
8. `tmux send-keys -t <new> BTab`; sleep 0.5. → capture `bt2`.
9. `tmux send-keys -t <new> BTab`; sleep 0.5. → capture `bt3`.

**Expect**:
- `bt1` not contains `Plan mode (shift+tab to cycle)` — first Shift+Tab toggles OFF
- `bt2` contains `Plan mode (shift+tab to cycle)` — second toggles ON
- `bt3` not contains `Plan mode (shift+tab to cycle)` — third toggles OFF again
- No third or fourth mode ever appears — the "cycle" hint is misleading; it's a binary toggle

### Scenario D: Plan mode persists across /side trip

10. With Plan mode ON: `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`. Wait for `• 4` in the side context. → capture `in_side`.
11. `tmux send-keys -t <new> Escape`; sleep 1. → capture `back_main`.

**Expect**:
- `back_main` contains `Main [default]` AND `Plan mode (shift+tab to cycle)` — Plan mode survived the /side detour

**Cleanup**: toggle Plan mode off with Shift+Tab or another `/plan`.

---

## TR-048: /goal behaviour (blocked on Feature::Goals build)

`/goal` is feature-gated on `Feature::Goals`. The public release does not enable it, and we don't currently have a build that does. This TR is a placeholder: when someone has a build with the Goals feature on, fill in the four states below with verified predicates.

**Setup**: TR-003 setup AND a build where `Feature::Goals` is enabled (debug build with the flag forced, or a future release that exposes it).

**Action** (to be specified):
- `/goal <text>` — set a goal
- `/goal clear` — clear it
- `/goal pause` — pause goal status
- `/goal resume` — resume it

**Expect**: predicates to be captured during the first run on a Goals-enabled build.

Note: do not test `/goal` on builds without the feature flag. "Unrecognized command" is a distribution-channel fact, not feature behaviour, and the predicate would flip the day Goals ships.

---


# Group 5: Scheduling

## TR-023: /scheduling empty panel opens, dismisses, chat survives

Opens the `/scheduling` panel on a session with no cron/monitor tasks
and verifies the empty state renders correctly, Escape dismisses the
panel cleanly, and the chat composer still accepts input afterward. The
"dismiss + chat round-trip" half is the deep regression guard — if the
panel leaves frame cells or focus state behind on close, the next
message gets garbled (same failure mode the reading-view spec's
close-cleanly contract guards against).

**Setup**: TR-003 setup on a session with no scheduling tasks (a fresh
ata launch is always empty).

**Action**:
1. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `panel`.
3. `tmux send-keys -t <new> Escape`; sleep 1.
4. `tmux send-keys -t <new> "say ok"`; sleep 1; `Enter`.
5. Poll up to 60s until `tmux capture-pane -t <new> -p` matches `^• [oO][kK]\b`.
6. → capture `after`.

**Expect**:
- `panel` contains `Scheduling tasks in this session`
- `panel` contains `Active cron jobs and monitors for this thread.`
- `panel` contains `Cron (0)`
- `panel` contains `Monitors (0)`
- `panel` contains `(none)` — empty-state copy
- `panel` contains `↑/↓ select · enter details · d delete · esc close` — footer help
- `panel` contains `Updated:` — timestamp present
- `after` matches `^• [oO][kK]\b` — agent responded after the panel dismissed
- `after` contains `› say ok` — user message landed in chat
- `after` not contains `Scheduling tasks in this session` — panel content fully gone
- `after` not contains `Cron (0)` — panel content fully gone
- `after` not contains `↑/↓ select` — panel footer fully gone

---

## TR-024: /scheduling `d` deletes OS cron + kills in-flight subprocesses (PR #20 regression guard)

The headline bug PR #20 fixed: pressing `d` in `/scheduling` on an OS-cron
row removed the crontab entry but left already-spawned `ata exec`
children running until their natural end. This test guards against
re-introducing that orphan-process leak. It's cross-layer — pane (row
gone), filesystem (crontab entry gone), AND kernel (no matching
processes) must all agree.

**Setup**: TR-003 setup + precondition below.

**Precondition (macOS)**: Terminal needs Full Disk Access in System
Settings → Privacy & Security → Full Disk Access. Without it,
`crontab -l` / `crontab` writes fail with `Operation not permitted`
and the `cron_create` tool returns an error. Verify by running
`crontab -l` from the same shell before the test and confirming it
does not error.

**Action**:
1. In ata, send: `create a cron job named tr024-test that runs every minute and does: sleep 90 && echo done`
2. Poll up to 30s until `tmux capture-pane -t <new> -p` contains `Created persistent cron job tr024-test` (proves the cron tool succeeded).
3. Extract the `Task ID:` from the response (UUID) — call this `TASK_ID`.
4. Wait up to 65s for the first fire: poll the cron log path `~/.ata/cron/<TASK_ID>.log` for non-zero size.
5. Snapshot pre-delete state:
   - `crontab -l | grep <TASK_ID> > <crontab_before>` → capture `crontab_before`.
   - Record the in-flight process tree as a fixed set of PIDs:
     `pgrep -f "<TASK_ID>" > /tmp/pre_pids.txt` — these are the specific PIDs whose death we will assert. (Recording PIDs rather than re-pgrep'ing later avoids a documented OS cron race; see "Known caveat" below.)
   - `pgrep -fl "<TASK_ID>" > <procs_before>` → capture `procs_before` (human-readable form for predicates).
6. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
7. → capture `panel_before`.
8. `tmux send-keys -t <new> d`; sleep 10.  (10s settle window — kill is async per PR #20 fix and macOS process reaping takes a beat.)
9. → capture `panel_after`.
10. Snapshot post-delete state:
    - `crontab -l | grep <TASK_ID> > <crontab_after>` → capture `crontab_after`.
    - Check whether each PID from `/tmp/pre_pids.txt` is still alive:
      `for p in $(cat /tmp/pre_pids.txt); do ps -p $p -o pid= 2>/dev/null; done > <pre_pids_alive>` → capture `pre_pids_alive`.

**Expect** (all must hold):
- `panel_before` contains `tr024-test` — row visible in panel
- `panel_before` contains `Cron (1)` — count reflects one job
- `crontab_before` contains the task id — crontab entry present
- `procs_before` contains `ata exec` — at least one `ata exec` subprocess in flight
- `panel_after` contains `Cron (0)` — row removed from panel
- `panel_after` not contains `tr024-test` — row gone
- `crontab_after` is empty — crontab entry removed
- `pre_pids_alive` is empty — every process that was in flight at delete time is gone (this is the precise PR #20 invariant: in-flight subprocesses get killed; the test does NOT assert "no new processes ever spawn matching the task id", because a known OS-cron race can leak one extra fire — see caveat)

**Known caveat — macOS cron-daemon race**:
macOS's cron daemon caches the next-fire schedule from the crontab. If
the `d` press happens within the same minute as a scheduled fire, the
daemon may have already enqueued that fire from its in-memory state
even after the crontab entry is removed — the result is a new process
tree (with a NEW set of PIDs, distinct from the pre-delete ones) that
runs to natural completion. The PR #20 fix correctly kills the
*pre-delete* process tree, but cannot preempt a fire that the cron
daemon already scheduled. This is why the post-delete predicate is
"every PID we recorded pre-delete is dead", not "no processes match
the task id" — the latter is too strict and flags the OS race as a
test failure.

**Teardown** (mandatory, pass or fail): run the three commands in the "OS cron safety" section at the top of this file. Verify the crontab is empty and no `ata exec` children remain. Skipping this leaves a popup-spamming cron firing every minute.

---

## TR-025: Monitor lifecycle — start, stream output, complete, retain row, delete

Covers the full monitor task lifecycle: a `monitor_start` call streams
stdout lines into chat live, fires a completion event when the command
exits, and the `/scheduling` panel retains the row as `[Completed]`
with an accurate line count until the user presses `d` to clear it.
This exercises three layers (chat stream, scheduling panel, session
JSONL) for one feature — the kind of coverage a single-pane smoke test
would miss.

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: ``start a monitor named tr025-watch that runs: for i in 1 2 3 4 5; do echo "tick $i"; sleep 1; done``
2. Sleep 1; `Enter`.
3. Poll up to 30s until `tmux capture-pane -t <new> -p` contains both `Started monitor tr025-watch.` AND `completed successfully` (monitor announced + terminated).
4. → capture `chat`.
5. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `panel_retained`.
7. `tmux send-keys -t <new> d`; sleep 1.
8. → capture `panel_deleted`.
9. `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1); jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.
10. `jq -r 'select(.payload.name=="monitor_start") | .payload.arguments' "$SESS" > <monitor_args>`. → capture `monitor_args`.

**Expect** (all must hold):

Chat stream:
- `chat` contains `Started monitor tr025-watch.` — start announced
- `chat` contains `tick 1` — first stream line delivered
- `chat` contains `tick 5` — last stream line delivered (proves all 5 made it through, not just the first)
- `chat` contains `completed` — completion event present in chat
- `chat` contains `Monitor tr025-watch completed successfully.` — final agent summary
- `chat` matches `\[m [a-f0-9]+ out\] tick` — stream prefix format is the `[m <prefix> out]` shape

Scheduling panel — retention:
- `panel_retained` contains `Monitors (1)` — count reflects the completed monitor
- `panel_retained` contains `tr025-watch` — name visible
- `panel_retained` contains `[Completed]` — status reflects termination
- `panel_retained` contains `lines 5` — line count reflects actual stream output

Scheduling panel — delete:
- `panel_deleted` contains `Monitors (0)` — count zeroed
- `panel_deleted` not contains `tr025-watch` — row gone

Session JSONL:
- `tool_counts` contains `monitor_start` — the dedicated monitor tool was used
- `tool_counts` not contains `shell` — not a shell-tool fallback
- `monitor_args` contains `"name"` and `tr025-watch` — name argument passed correctly
- `monitor_args` contains `tick` — the for-loop command argument made it through

---

## TR-026: In-session cron lifecycle — create, fire, panel update, delete

In-session cron (`cron_create_session`) lives entirely in the chat
session — no crontab, no Full Disk Access precondition. It fires the
agent in chat on a sub-minute schedule. This test verifies the
full lifecycle: tool routes to `cron_create_session` (not the OS `cron_create`),
the panel shows status `[Pending]`, the fire counter increments after
each fire, the cron's prompt actually runs the agent inline, and `d`
removes the row before it fires again.

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: ``create an in-session cron named tr026-ping that runs every 30 seconds and says: respond with just "ping"``
2. Sleep 1; `Enter`.
3. Poll up to 30s until `tmux capture-pane -t <new> -p` contains `Created in-session cron tr026-ping.` — proves the cron was registered.
4. → capture `created`.
5. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `panel_pending`.
7. `tmux send-keys -t <new> Escape`; sleep 1.
8. Poll up to 60s until pane contains both `Respond with just "ping"` on a `›` line AND `• ping` on a `•` line (first fire completed).
9. → capture `after_fire`.
10. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
11. → capture `panel_after_fire`.
12. `tmux send-keys -t <new> d`; sleep 1.
13. → capture `panel_deleted`.
14. `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1); jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.

**Expect** (all must hold):

Creation:
- `created` contains `Created in-session cron tr026-ping.`
- `created` contains `every 30 seconds while this session is active.` — in-session signature copy

Panel — pending state (before first fire):
- `panel_pending` contains `Cron (1)`
- `panel_pending` contains `tr026-ping`
- `panel_pending` contains `[Pending` — in-session status (differs from OS cron's `[Scheduled]`)
- `panel_pending` contains `fired 0` — no fires yet
- `panel_pending` contains `next in ` — countdown rendering

Fire — chat:
- `after_fire` contains `› Respond with just "ping"` — cron prompt rendered as a user message
- `after_fire` matches `^• ping` — agent responded with the expected literal

Panel — after first fire:
- `panel_after_fire` contains `fired 1` — fire counter incremented
- `panel_after_fire` contains `[Pending` — still scheduled for next fire (in-session crons stay Pending between fires)

Delete:
- `panel_deleted` contains `Cron (0)` — row removed
- `panel_deleted` not contains `tr026-ping`

Session JSONL — correct tool routing:
- `tool_counts` contains `cron_create_session` — in-session creator, not OS `cron_create`
- `tool_counts` contains `cron_delete_session` — in-session deleter (from the `d` press, which routes through the panel's delete handler)

---

## TR-027: OS cron survives ata restart and keeps firing while ata is off

OS cron lives in the system crontab — not in the ata session — so it
must (a) persist across ata exits, (b) keep firing from the system
crontab while ata isn't running, and (c) reappear in `/scheduling` on
the next launch with its accumulated fire count intact. This is the
core "OS cron vs in-session cron" distinction (compare TR-026, which
verifies in-session crons are explicitly session-scoped).

**Setup**: TR-003 setup + TR-024's macOS Full Disk Access precondition.

**Action**:
1. In ata, send: `create a cron job named tr027-persist that runs every minute and does: echo done`
2. Sleep 1; `Enter`.
3. Poll up to 30s until pane contains `Created persistent cron job tr027-persist`. Extract `Task ID:` → `TASK_ID`.
4. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
5. → capture `panel_session1`.
6. `tmux send-keys -t <new> Escape`; sleep 1.
7. `crontab -l | grep "$TASK_ID" > <crontab_active>`. → capture `crontab_active`.
8. Quit ata: `tmux send-keys -t <new> C-d`; sleep 2.
9. Verify ata exited: pane should now show a shell prompt (e.g. contains `$ ` or `% ` near the bottom). → capture `exited`.
10. Wait 70 seconds (lets the system cron fire at least once while ata is off).
11. Relaunch ata in the same pane: `tmux send-keys -t <new> "./target/debug/ata --yolo"`; sleep 1; `Enter`; sleep 6.
12. Wait for the welcome banner (`OpenAI Codex (v` substring).
13. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
14. → capture `panel_session2`.
15. `tmux send-keys -t <new> d`; sleep 1.
16. → capture `panel_deleted`.
17. `crontab -l | grep "$TASK_ID" > <crontab_clean>` (expected empty). → capture `crontab_clean`.

**Expect** (all must hold):

Session 1 — cron registered:
- `panel_session1` contains `Cron (1)`
- `panel_session1` contains `tr027-persist`
- `panel_session1` contains `[Scheduled]` — OS cron status
- `panel_session1` contains `fired 0` — no fires yet at creation
- `crontab_active` contains the task id — system crontab entry was written

Exit:
- `exited` not contains `/scheduling` — TUI dismissed
- `exited` matches `(\$|%) $` OR contains `To continue this session, run ata resume` — shell prompt or ata's exit message visible

Session 2 — cron persisted across restart:
- `panel_session2` contains `Cron (1)` — cron reappears in fresh session
- `panel_session2` contains `tr027-persist` — same task name
- `panel_session2` contains `[Scheduled]` — still scheduled
- `panel_session2` not contains `fired 0` — fire counter is non-zero (at least one fire happened while ata was off; proves the cron is running from the system crontab, not from ata)

Cleanup:
- `panel_deleted` contains `Cron (0)` — row removed in session 2
- `panel_deleted` not contains `tr027-persist`
- `crontab_clean` is empty — system crontab entry removed too

**Teardown** (mandatory, pass or fail): run the three commands in the "OS cron safety" section at the top of this file. Even if the test passed and `/scheduling d` already removed the row, verify the crontab is empty and no `ata exec` children remain. Skipping this leaves a popup-spamming cron firing every minute.

---

## TR-028: Agent picks monitor_watch_for (not monitor_wait) when prompt asks to react to a pattern

The monitor agent has two blocking primitives: `monitor_wait` (block
until the subprocess terminates) and `monitor_watch_for` (block until
a specific line appears on stdout/stderr). The right tool depends on
intent: "tell me when it finishes" → `monitor_wait`; "tell me when
'X' appears" → `monitor_watch_for`. This test verifies the model
disambiguates correctly. The monitor-stream rendering already gets
coverage in TR-025; here the depth is in tool-routing correctness,
verified via JSONL cross-check (the pane can render a plausible
"matched X" response even when the wrong primitive was called).

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: ``start a monitor that runs: for i in 1 2 3 4 5; do echo "tick $i"; sleep 1; done. Watch for the line "tick 3" and tell me when it appears.``
2. Sleep 1; `Enter`.
3. Poll up to 30s until pane contains `tick 3 appeared` OR `matched` OR a similar agent-narrated success line referencing `tick 3` (proves the watch returned).
4. → capture `chat`.
5. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `panel`.
7. `tmux send-keys -t <new> d`; sleep 1.  (cleanup — remove the completed monitor row.)
8. `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1); jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.
9. `jq -r 'select(.payload.name=="monitor_watch_for") | .payload.arguments' "$SESS" > <watch_args>`. → capture `watch_args`.

**Expect** (all must hold):

Chat:
- `chat` contains `tick 3` — the matched line appears in the agent's response
- `chat` contains `appeared` OR `matched` — agent narrated the match outcome
- `chat` not contains `did not match` — no fallback to terminated-without-match

Tool routing:
- `tool_counts` contains `monitor_start` — monitor was spawned
- `tool_counts` contains `monitor_watch_for` — pattern-matching variant was used
- `tool_counts` not contains `monitor_wait` — did NOT fall back to wait-for-completion
- `tool_counts` not contains `shell` — did not shell out to grep / tail / etc.

Argument fidelity:
- `watch_args` contains `"pattern"` — arguments include a pattern field
- `watch_args` contains `tick 3` — the pattern value reflects the user's literal request (not a paraphrase like "tick three" or "third tick")
- `watch_args` contains `"task_id"` — the args target the just-spawned monitor

Panel state — completed monitor retained:
- `panel` contains `Monitors (1)` — completed monitor still visible until user dismisses it
- `panel` contains `[Completed]` — status reflects natural termination
- `panel` contains `lines 5` — line count reflects all 5 ticks streamed (the watch returning early on tick 3 did NOT abort the underlying monitor process)

---

## TR-029: /scheduling panel renders both sections populated (mixed task kinds)

Earlier tests verified single-section states (empty in TR-023, one cron in
TR-024/026/027, one monitor in TR-025/028). This test exercises the layout
when *both* sections are populated — catches layout regressions that only
surface with mixed task kinds (e.g. wrong section ordering, missing
section header when adjacent section is non-empty, status-string spacing
that only breaks under simultaneous rendering).

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: `create an in-session cron named tr029-cron that runs every 60 seconds and says: hi`. Sleep 1; `Enter`.
2. Poll up to 30s until pane contains `Created in-session cron tr029-cron`.
3. In ata, send: `start a monitor named tr029-mon that runs: sleep 30`. Sleep 1; `Enter`.
4. Poll up to 30s until pane contains `Started monitor tr029-mon`.
5. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `panel`.
7. Cleanup: press `d` to remove the focused row, sleep 1, capture `after_first_d`, press `d` again to remove the second, sleep 1, capture `after_second_d`.

**Expect** (all must hold):

Layout:
- `panel` contains `Cron (1)` — cron section non-empty
- `panel` contains `Monitors (1)` — monitor section non-empty (same panel render)
- `panel` contains `tr029-cron` — cron row name
- `panel` contains `tr029-mon` — monitor row name

Status disambiguation — both kinds visible side-by-side:
- `panel` contains `[Pending` — in-session cron status
- `panel` contains `[Running` — live monitor status
- `panel` not contains `[Scheduled` — no OS cron present
- `panel` not contains `[Completed` — monitor hasn't terminated yet

Cleanup — sequential `d` clears both rows independently:
- `after_first_d` contains either `Cron (0)` (cron was focused first) OR `Monitors (0)` (monitor was focused first)
- `after_second_d` contains `Cron (0)` — both sections cleared
- `after_second_d` contains `Monitors (0)`
- `after_second_d` not contains `tr029-cron`
- `after_second_d` not contains `tr029-mon`

---

## TR-030: /scheduling row detail view shows id, command, and output tail

Pressing Enter on a row in `/scheduling` opens a detail view with full
task metadata: id, status, command, line count, and a recent-output
tail. The list view truncates command + status into a single line for
density; the detail view is where users actually inspect task internals.
Worth its own regression guard because (a) detail rendering is a
separate code path from list rendering and can rot independently, and
(b) the tail formatting (e.g. `[stdout]` prefix per line) is a small
contract that downstream tools rely on.

Also documents a UI behavior: Escape from the detail view exits the
whole panel — it does NOT go back to the list view. (To return to the
list, the user has to reopen `/scheduling`.)

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: `start a monitor named tr030-detail that runs: for i in 1 2 3 4 5; do echo "tick $i"; sleep 2; done`. Sleep 1; `Enter`.
2. Poll up to 30s until pane contains `Started monitor tr030-detail`.
3. `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
4. `tmux send-keys -t <new> Enter`; sleep 1.  (Enter on the focused monitor row → detail view.)
5. → capture `detail`.
6. `tmux send-keys -t <new> Escape`; sleep 1.  (Escape from detail.)
7. → capture `after_escape`.
8. Cleanup: `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1; `tmux send-keys -t <new> d`; sleep 1.

**Expect** (all must hold):

Detail view rendering:
- `detail` contains `Monitor · tr030-detail` — heading uses `Kind · Name` format
- `detail` contains `id ` — task id label rendered
- `detail` matches `id [a-f0-9-]{36}` — id field shows a UUID-shaped value
- `detail` contains `status ` — status field label
- `detail` contains `Completed` OR `Running` — status reflects monitor state at the moment of capture
- `detail` contains `lines 5` — line count present (the 5-iteration loop takes ~10s; by the time you press Enter on the row it should be complete)
- `detail` contains `command:` — command label
- `detail` contains `for i in 1 2 3 4 5;` — full command shown (the list view truncates at row width; detail view shows the full string)
- `detail` contains `Recent output tail:` — output section header
- `detail` contains `[stdout] tick 1` — first output line with stream-tag prefix
- `detail` contains `[stdout] tick 5` — last output line with stream-tag prefix (proves the tail isn't truncated for a 5-line run)

Escape behavior — exits the entire panel:
- `after_escape` not contains `Monitor · tr030-detail` — detail view gone
- `after_escape` not contains `Recent output tail:` — detail content gone
- `after_escape` not contains `Scheduling tasks in this session` — list view ALSO gone (Escape from detail does NOT bounce back to the list; it exits the panel entirely)

---


# Group 6: Tool routing

## TR-021: Agent picks the Hacker News tool on its own

When the user asks for HN content, ata must call the dedicated `hn_search`
tool — NOT `web_search` and NOT shell out via `exec_command` / `curl`.
This test guards against silent tool-routing regressions that the rendered
text alone won't catch (the model can produce a plausible inline answer
even while calling the wrong tool, or no tool at all).

The prompt deliberately does NOT name `hn_search`. Naming the tool tests
nothing; the point is to verify the model picks it.

**Setup**: TR-003 setup + precondition below.

**Precondition**: the Hacker News research skill must be enabled. It
ships `default_enabled: true`, so a fresh install passes — but if a user
turned it off via `/research`, the test will fail because the agent has
no `hn_search` tool to route to. Verify `~/.ata/config.toml` does NOT
have:

```toml
[features]
research_hacker_news = false
```

(Absence of the key or `= true` both work.)

**Action**:
1. `tmux send-keys -t <new> "find me a top story on Hacker News about Rust"`; sleep 1; `Enter`.
2. Poll up to 3 min until `tmux capture-pane -t <new> -p` contains
   `news.ycombinator.com` (proves a real HN response landed).
3. → capture `resp`.
4. `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`.
5. `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts_capture>`.
   → capture `tool_counts`.
6. `jq -r 'select(.payload.name=="hn_search") | .payload.arguments' "$SESS" > <hn_args_capture>`.
   → capture `hn_args`.

**Expect** (all must hold):
- `resp` contains `Hacker News` — feature answered
- `resp` contains `news.ycombinator.com` — real HN URL cited
- `tool_counts` contains `hn_search` — dedicated tool was invoked
- `tool_counts` not contains `web_search` — did not fall back to generic search
- `tool_counts` not contains `shell` — did not shell out
- `tool_counts` not contains `exec_command` — did not shell out
- `hn_args` contains `"query"` — arguments object well-formed
- `hn_args` contains `Rust` — query reflected the user's topic

---

## TR-035: Multi-source synthesis — agent uses BOTH papers and Hacker News (direct calls or sub-agents)

TR-021 verified that a single-source HN prompt routes to `hn_search`.
This test extends to a *two-source* prompt that legitimately requires
BOTH academic and practitioner sources. The failure mode it catches:
the agent picks one source, silently drops the other, and writes a
plausible-looking answer with vague references to the missed source.

A key empirical finding from authoring this test: ata's multi-source
strategy is often NOT "call two skills directly". For complex queries
the main agent spawns sub-agents (one per source / per thread) via
`spawn_agent` + `wait_agent`, and only the main session shows the
orchestration — the underlying `hn_search` / `web_search` calls live
inside each sub-agent's session. Predicates therefore accept either
pattern: direct skill calls in the main session, OR sub-agent
delegation with evidence of both sources reached.

**Setup**: TR-003 setup.

**Action**:
1. In ata, send: `find recent papers on Rust async performance and check Hacker News for related discussion`. Sleep 1; `Enter`.
2. Poll up to 10 minutes (this can be slow — multi-skill orchestration, sub-agent boots, MCP servers) until pane contains a clear synthesis signal: either a reading-view banner like `Papers and HN Signal` / `Papers and HN` / a `1/6` section counter on the final synthesis document, OR an inline response that mentions both `Hacker News` AND `paper`.
3. → capture `response`.
4. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -15 | xargs ls -t | head -1)`
   - `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>` → capture `tool_counts`.
   - `ls ~/.ata/workspaces/global/knowledge-base/staging/ 2>/dev/null > <staging>` → capture `staging` (sub-agent output dir).

**Expect** (all must hold — the "OR" predicates accept either pattern):

Response content — both sources actually contributed:
- `response` contains `Hacker News` OR `HN Signal` — practitioner source represented in a section title or body
- `response` contains `Paper` OR `literature` OR `academic` — academic source represented
- `response` contains `Bottom Line` — synthesis includes a unified bottom-line section (the typical multi-source synthesis pattern)
- `response` not contains `I couldn't find any` — the agent didn't bail on either source
- `response` not contains `no results` — same
- `response` contains a numbered TOC entry that's HN-specific (e.g. `Hacker News Signal`) AND a numbered TOC entry that's paper-specific (e.g. `Paper Landscape` / `Recent Paper`) — proves the synthesis dedicated section structure to each source, not just lip service

Tool routing — either direct calls OR sub-agent orchestration:
- `tool_counts` contains `paper_search` OR `spawn_agent` — academic side was either searched directly or delegated
- `tool_counts` contains `hn_search` OR `spawn_agent` — HN side same
- `tool_counts` not contains `shell` — did NOT shell out via `curl https://news.ycombinator.com` (a fallback regression)

Sub-agent evidence (if delegation route was taken):
- Either `tool_counts` contains `hn_search` (direct route, no delegation needed) OR `staging` contains `hn-` (sub-agent delegation route — `hn-<thread-id>.md` staging notes were produced and saved to `~/.ata/workspaces/global/knowledge-base/staging/`)

(Note: this is intentionally a permissive predicate set. The point is to assert "both sources contributed", not to mandate one orchestration strategy. A future refactor that changes from direct calls to sub-agents — or vice versa — should still pass.)

---

## TR-062: paper_search tool — multi-source academic search with paraphrasing orchestration

`paper_search` is the agent-callable tool for searching academic
papers across three indexes (Semantic Scholar, arXiv, OpenAlex). When
prompted to find papers, the agent typically orchestrates MULTIPLE
calls — paraphrasing the user's query and routing different variations
to different sources, then synthesizing the best match.

This routing is implicit (the user does NOT name the tool). The deep
regression guard is: under a paper-finding prompt, paper_search is the
tool that gets called, NOT web_search and NOT shell-out via curl.

**Note on `paper_discovery`**: this is a SKILL (markdown file in
`.system-research/paper-discovery/SKILL.md`), not a tool. The skill
instructs the agent to: (a) check the local knowledge base via `rg`,
(b) make paper_search calls, (c) write a research-journal entry. The
agent loads the skill when the user's prompt has a paper-discovery
intent.

**Setup**: TR-003 setup. Network access. No state preconditions.

### Scenario A: natural-language prompt routes to paper_search (multi-source orchestration)

1. In ata: `find me a recent paper on rust async performance`. Sleep 1; Enter.
2. Poll up to 3 minutes until the agent prints sources / paper titles / DOI / arxiv ids.
3. → capture `resp`.
4. `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1); jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`.
5. `jq -r 'select(.payload.name=="paper_search") | .payload.arguments' "$SESS" > <search_args>`.

**Expect**:
- `resp` contains a paper title and/or `arxiv.org`, `dblp.org`, or `dagstuhl.de` link — proves an actual paper landed
- `resp` not contains `I couldn't find` OR `no results` — agent didn't bail
- `tool_counts` matches `[0-9]+ paper_search` with the leading count ≥ 1 — paper_search WAS called
- `tool_counts` typically shows paper_search called 3-6 times (orchestrated paraphrasing across sources — observed 6 calls on first verification run)
- `tool_counts` does NOT contain `web_search` — agent did NOT fall back to generic web search
- `tool_counts` does NOT contain `shell` — no shell-out fallback
- `tool_counts` may contain `exec_command` (count 1-4) for KB grep / journal writes — these are skill-orchestrated side effects, not search fallbacks

### Scenario B: argument schema verified across calls

6. Inspect `<search_args>` (one JSON object per line, one per call).

**Expect**:
- Each call's args contain `query` (string) — the search term
- Each call's args contain `source` with one of: `semantic_scholar`, `arxiv`, `openalex`
- The 6+ calls collectively cover ALL THREE sources (the orchestration is breadth-first across sources, not depth-first into one)
- Each call has `year_from` and `year_to` (numeric) — date range filter
- Each call has `limit` (numeric, typically 8-10)
- Each call has `fields[]` (array of strings) including at least `title`, `year`, `abstract`, `authors`, `url` — field-selection schema
- Each call has `include_abstract: true`
- Each call has `sort_by: "year"` — recent-first
- Each call has `max_chars_per_item` (numeric, 1200-1500) — abstract truncation control
- Query strings VARY across calls (agent paraphrases — e.g. `Rust async performance Tokio async runtime benchmark` vs `Rust asynchronous programming performance futures async await runtime` vs `performance evaluation Rust async await runtime`)

### Scenario C: paper_discovery skill is loaded and orchestrates KB search

7. Inspect `resp` for the skill-loading log line.

**Expect**:
- `resp` contains `Read SKILL.md (paper-discovery skill)` — the skill was loaded
- `resp` contains a `Ran KB_DIR=` shell line (the skill's local-KB pre-check before searching papers)
- `resp` contains a `Ran KB_DIR=... mkdir -p ... research-journal.md` line — the skill writes a journal entry as final step

### Scenario D: bad query / no results path

`find me a paper about quantum entangled toaster pancakes from 2071`.

**Action**:
1. In ata: `find me a paper about quantum entangled toaster pancakes from 2071`; sleep 1; Enter.
2. Poll up to 3 min for completion.
3. Capture response; inspect session JSONL.

**Expect** (verified):
- `tool_counts` contains `paper_search` (count ≥ 1, often 2–3 — the agent retries with paraphrased queries before giving up)
- `resp` contains an explicit negative statement (e.g. `didn't find any real paper from 2071`, `no papers found matching`, or similar literal disclaimer). Pin the actual phrase on first run.
- The agent does NOT fabricate a fake paper with an invented DOI — instead it falls back to a real adjacent paper with a verifiable DOI or arxiv id, AND explicitly notes the substitution.
- `tool_counts` does NOT contain a write tool like `add-paper` (the agent doesn't add the fake to your library).

Anti-regression: if a future build silently makes up a `quantum entangled toaster pancake 2071` paper with a plausible-sounding DOI, this test fails — that's a fabrication-prevention guard.

---

## TR-063: paper_get — fetch a specific paper by ID

`paper_get` retrieves a single paper by its arxiv id, DOI, or Semantic
Scholar id. The tool DOES exist and works when explicitly named in the
prompt. Important finding: natural-language prompts like "look up arxiv
2505.21323" route to `exec_command` (curl scrape of arxiv.org) rather
than `paper_get`. The dedicated tool is only invoked when the user
explicitly names it.

**Setup**: TR-003.

### Scenario A: natural prompt falls back to exec_command (weak routing)

1. In ata: `look up arxiv 2505.21323`. Sleep 1; Enter. Poll for the response.
2. Inspect JSONL.

**Expect**:
- Response contains paper title (`Asynchronous Rust`), authors, DOI — correct content
- `tool_counts` contains `exec_command` (≥1 — used to curl arxiv.org/abs/2505.21323)
- `tool_counts` does NOT contain `paper_get` — natural prompt didn't route to the dedicated tool

### Scenario B: explicit tool naming triggers paper_get

3. In ata: `use the paper_get tool to fetch the paper with arxiv id 2505.21323 and tell me the abstract`. Sleep 1; Enter. Poll.
4. Inspect JSONL.

**Expect**:
- `tool_counts` contains `paper_get` (count = 1)
- `paper_get` args match `{"paper_id":"arXiv:2505.21323"}` — id format is `arXiv:<number>` (capital X, colon-prefixed). The arg field name is `paper_id`.
- Response contains the paper's abstract

---

## TR-064: paper_citations — papers citing a given paper

For literature reviews: "find papers that cite X". When prompted with
explicit tool naming, the agent calls `paper_citations` with the source
paper's id.

### Scenario A

1. In ata: `use paper_citations to find recent papers that cite arxiv 2505.21323`. Sleep 1; Enter. Poll.
2. Inspect JSONL.

**Expect**:
- `tool_counts` contains `paper_citations` (count = 1)
- Args:
  - `paper_id: "arXiv:2505.21323"` — same id-format as paper_get
  - `limit: 20`
  - `fields[]`: `title`, `authors`, `year`, `venue`, `abstract`, `doi`, `arxiv_id`, `url`, `citation_count`
  - `max_chars_per_item: 1000`
- Response is a numbered list of citing papers

---

## TR-065: paper_references — papers referenced by a given paper

The reverse of paper_citations — what does paper X cite.

### Scenario A

1. In ata: `use paper_references to list the references cited inside arxiv 2505.21323`. Sleep 1; Enter. Poll.
2. Inspect JSONL.

**Expect**:
- `tool_counts` contains `paper_references` (count = 1)
- Args:
  - `paper_id: "arXiv:2505.21323"`
  - `limit: 50` (larger than citations default — references list tends to be longer)
  - `fields[]`: title, authors, year, venue, doi, arxiv_id, url, citation_count (NO `abstract` field by default for references)
  - `max_chars_per_item: 1000`

---

## TR-066: paper_recommendations — recommend papers similar to given examples

For exploratory research. Takes an array of seed paper ids and
recommends similar work.

### Scenario A

1. In ata: `use paper_recommendations to recommend 5 papers similar to arxiv 2505.21323 about real-time Rust executors`. Sleep 1; Enter. Poll.
2. Inspect JSONL.

**Expect**:
- `tool_counts` contains `paper_recommendations` (count = 1)
- Args:
  - `positive_paper_ids: ["arXiv:2505.21323"]` — ARRAY of ids (note plural and array structure, distinct from `paper_id` in paper_get/citations/references)
  - `limit: 10` (default — actually returns more than the user asked for; the agent filters down in its response)
  - `fields[]`: title, authors, year, venue, abstract, doi, arxiv_id, url, citation_count
  - `max_chars_per_item: 1200`

---



# Group 8: Zotero CLI

## TR-061: `ata zotero` CLI — status, search-commands, and subcommand inventory

The CLI's `status` reports the effective Zotero mode (`local` vs
`cloud`), endpoint, auth, and scope. `search-commands <query>` ranks
matches and prints a clap-style manual for the top hit (same pattern
as `ata workspace search-commands`).

**Setup**: ata 0.7.0 installed. `status` and `search-commands` work without Zotero. Subcommands D–R (collections, search, recent, etc.) need a running Zotero with the local API enabled — see the prerequisite below.

**Zotero workspace prerequisite** (one-time setup, required for Scenarios D–R):

1. Install Zotero desktop from https://www.zotero.org/download/ (or `brew install --cask zotero`).
2. Launch Zotero. Confirm at least one item exists in your library (use any sample paper — DOI search in Zotero will add one in seconds).
3. Enable the local API: **Zotero → Settings (Cmd+,) → Advanced → General → "Allow other applications on this computer to communicate with Zotero"**. Effect is immediate, no restart needed.
4. Verify: `curl -s 'http://localhost:23119/api/users/0/items?limit=1'` should return JSON (an empty `[]` if root library is empty is fine).
5. Verify ata can talk to it: `ata zotero collections` should return JSON with your collections.

Once those four steps pass, Scenarios D–R below can run against real data.

### Scenario A: `status` reports effective mode and config

1. Shell command: `ata zotero status > <out>`.

**Expect**:
- `<out>` contains `Effective mode: local` — local mode is the default fallback
- `<out>` contains `Base URL: http://localhost:23119/api` — Zotero desktop's local API endpoint
- `<out>` contains `API key configured: no` — no key in this shell
- `<out>` contains `Library scope: all accessible libraries`
- `<out>` contains `Default write scope: unconfigured`
- `<out>` contains `Note: The effective Zotero mode is local because no Zotero API key is configured for this shell.` — explanation line

### Scenario A2: `status` reports remote mode when API key is configured (credential-free)

`ata zotero status` reports configured state without making API calls — so a dummy key in config.toml is enough to verify the reporting logic. We don't need a real, working key.

**Setup**: Back up your real config first if any: `cp ~/.ata/config.toml ~/.ata/config.toml.bak`.

**Action**:
1. Append a dummy key under the `[research]` section (this is where ata's config struct reads it from — `zotero_api_key`, not `[zotero] api_key`):
   ```bash
   cat >> ~/.ata/config.toml <<'EOF'
   [research]
   zotero_api_key = "dummy-test-key-not-real"
   EOF
   ```
2. Run: `ata zotero status > <out>`.
3. Cleanup: `mv ~/.ata/config.toml.bak ~/.ata/config.toml` (or delete the appended block).

**Expect**:
- `<out>` contains `Effective mode: remote` (key flips the mode from local to remote — the label is "remote", not "cloud")
- `<out>` contains `API key configured: yes` (config parsing sees the key)
- `<out>` contains `Fallback mode: local` — when a key is set, ata still falls back to local for queries the remote can't answer
- `<out>` `Base URL` stays at `http://localhost:23119/api` even in remote mode (the base URL is the local-fallback endpoint; remote uses api.zotero.org internally but the status line doesn't show that)

This verifies the config plumbing + status reporting. Actual remote API behavior (auth, fetches) needs a real key and lives outside this test.

### Scenario B: `--help` lists all 17 first-level subcommands

2. Shell command: `ata zotero --help > <out>`.

**Expect**:
- `<out>` lists each of: `search-commands`, `status`, `resolve-paper`, `add-paper`, `find-repos`, `search`, `tags`, `recent`, `advanced-search`, `grep-text`, `search-notes`, `item`, `collections`, `collection`, `groups`, `items`, `attachment`, `help`.
- `<out>` starts with `Manage Zotero libraries, collections, items, and attachments`.

### Scenario C: `search-commands <query>` includes nested commands too

3. Shell command: `ata zotero search-commands paper > <out>`.

**Expect**:
- `<out>` first line is `Matches:`.
- `<out>` numbered list includes top-level subcommands AND nested ones (verified: `item citation` appears as a nested-subcommand match alongside top-level `add-paper` and `resolve-paper`).
- `<out>` includes `Best match manual:` block with clap-style help for the top hit.
- The top hit's help shows `Usage: ata zotero <command> [OPTIONS]` and its `Options:` table.

### Scenarios D–L: subcommands requiring a live Zotero

**Prerequisite**: complete the Zotero workspace setup above. Replace concrete keys (`A46QKYJI`, `DNAJYNGP`) with ones from your library.

#### Scenario D: `collections` lists all accessible collections
- Command: `ata zotero collections`
- **Expect**: JSON object with top-level `collections: [...]` array. Each entry has `key` (8-char alphanum) and `name` (string). Also has `total_available: <int>` and `has_more: <bool>`.

#### Scenario E: `groups list` lists accessible groups
- Command: `ata zotero groups list`
- **Expect**: JSON object with `groups: [...]`, each entry `id` (string of digits) + `name` (string). Plus `total_available` + `has_more`.
- Note: bare `ata zotero groups` prints subcommand help, not data — must use `groups list`.

#### Scenario F: `recent --limit N` returns recent items in scope
- Command: `ata zotero recent --limit 3`
- **Expect**: JSON object with `items: [...]`, `total_available`, `has_more`. Empty list (`items: []`) is valid when the scope's root library has no recent items — items inside collections don't count unless library scope is configured.

#### Scenario G: `search --query <q> --limit N` returns keyword matches
- Command: `ata zotero search --query neural --limit 2` (use a query you know matches at least one item)
- **Expect**: JSON `items: [...]` with per-item `key`, `title`, `authors` (comma-string), `year`, `item_type`, `doi`, `abstract_snippet`, `tags: [...]`, `linked_items: [...]`. `--query` is required; bare positional arg errors with `error: unexpected argument`.

#### Scenario H: `tags` returns the tag inventory
- Command: `ata zotero tags`
- **Expect**: JSON `tags: [...]`, plus `total_available` + `has_more`. Empty list valid when the scope has no tags or scope doesn't include collection tags.

#### Scenario I: `item get --item-key <K>` returns full item metadata
- Command: `ata zotero item get --item-key A46QKYJI`
- **Expect**: JSON with `key`, `title`, `authors: [...]` (ARRAY of strings here, not comma-string like in search), `abstract_text`, `date`, `doi`, `url`, `item_type`, `tags: [...]`, `linked_items: [...]`. Note `--item-key` is the required flag (not `--key`, not a positional arg).

#### Scenario J: `item citation --item-key <K>` returns a citation
- Command: `ata zotero item citation --item-key A46QKYJI`
- **Expect**: JSON with `item_key`, `format` (e.g. `"bibtex"`), `citation` (the full citation string), `citation_key`, `generator` (e.g. `"fallback_formatter"`).

#### Scenario K: `collection items --collection-key <K> --limit N`
- Command: `ata zotero collection items --collection-key DNAJYNGP --limit 2`
- **Expect**: JSON `items: [...]` similar to `search` output. Includes attachment items (`item_type: "attachment"`) interleaved with full bibliographic items.

#### Scenario L: `advanced-search --json '<schema>'` requires `operation` field
- Command (missing field): `ata zotero advanced-search --json '{"conditions":[],"limit":2}'`
- **Expect (negative)**: stderr contains `Error: parse JSON payload for \`ata zotero advanced-search\`` and `Caused by:` `missing field \`operation\``.
- Correct schema requires `operation: "and"|"or"`. Full positive predicates depend on a valid schema doc — pin on first successful run with `{"operation":"and","conditions":[{"field":"title","operator":"contains","value":"<q>"}]}`.

#### Scenarios M-Q: write ops + niche reads (deferred)

The following subcommands either MUTATE the user's library (`add-paper`, `items create/update/delete`, `attachment create/link/delete`, `collection create/find-or-create/add-items`) or need specific test fixtures (`grep-text`, `search-notes`, `resolve-paper`, `find-repos`). Deferred — capture predicates only when running against a throwaway/scratch Zotero library where mutations are safe to verify. Document in a follow-up TR rather than asserting predicates against the user's real library here.

### Scenario R: error when Zotero desktop is not reachable (negative test)

4. With Zotero desktop NOT running: `ata zotero collections > <out> 2>&1`.

**Expect** (to verify exact error on first run):
- `<out>` contains a connection error referencing `http://localhost:23119/api` (the local endpoint).
- Exit code is non-zero.
- Error string is stable across versions (pin it).

---

# Adding tests

Append `## TR-<NNN>: <name>` sections following the same shape. Pick **Expect** predicates that fail when the bug regresses and pass otherwise — keep them narrow. A predicate like "contains 'Section'" is too loose; one like "row 1 starts with '╭'" is right because that's a specific property of the rendering.

# Inspecting session logs

Many TUI tests verify that the agent actually called the right tool, not just that the rendering looks right. Use the JSONL session logs at `~/.ata/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl` for that:

```bash
# Find latest session (within 5 minutes)
SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)

# Tool names called
jq -r '.payload.name // empty' $SESS | sort | uniq -c

# Specific tool's arguments
jq -r 'select(.payload.name=="present_reading_view") | .payload.arguments' $SESS
jq -r 'select(.payload.name=="patch_document_section") | .payload.arguments' $SESS
jq -r 'select(.payload.name=="code_intel") | .payload.arguments' $SESS
```

Cross-checking the session log against the rendered pane catches "agent rendered the right text but didn't call the right tool" regressions — for example a Tab-to-ask response that's rendered inline by the model but didn't go through `patch_document_section` is broken even if the chat looks correct.
