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

## TR-001: Reading view survives a resize cycle

This is the canonical regression test for the v0.129.0 merge resize-corruption
bug. It opens a reading view, shrinks the pane, grows it back, and verifies
that no welcome-banner / chat-composer cells leak into the reader.

**Setup**:
0. Precondition: reading view is gated by two config flags in
   `~/.ata/config.toml` that must BOTH be set. Verify and flip if needed:
   ```toml
   [features]
   reading_view = true

   [reading_view]
   mode = "enabled"
   ```
   Without both, ata replies in chat instead of opening a reader and step 6 polls forever. (This precondition applies to TR-001, TR-002, TR-008, and TR-009 — all reading-view tests share this setup.)
1. `cd "$ATA_REPO/codex-rs" && cargo build -p codex-cli` and confirm the binary at
   `$ATA_REPO/codex-rs/target/debug/ata` is newer than `tui/src/tui.rs`.
2. Find the user's tmux session/window via `tmux list-clients` +
   `tmux display-message`.
3. Record the original pane width as `BASE_W` (typically 200+).
4. `tmux split-window -h -t <session>:<window>.<pane> -c "$ATA_REPO/codex-rs" './target/debug/ata --yolo'`.
   Wait until `tmux capture-pane -t <new> -p` contains `OpenAI Codex (v`.
5. `tmux send-keys -t <new> "give me 2 short slides on coffee in reading view, don't use any skills"` then sleep 1, then `tmux send-keys -t <new> Enter`.
6. Poll `tmux capture-pane -t <new> -p` every 6s until it contains
   `Sections (n/p`. This can take up to 3 minutes.

**Action**:
1. `tmux capture-pane -t <new> -p > <baseline_capture>`.
   → capture `baseline`.
2. `tmux resize-pane -t <new> -x 70`, then sleep 4.
   (Multi-frame repaint needs ~200ms; 4s leaves a wide margin.)
   **Detached-session note**: `resize-pane -x` is a no-op on a session
   with no attached client (its size is governed by the window). For
   detached runs use `tmux resize-window -t <session> -x 70 -y 50`
   instead — the inner pane will reflow.
3. `tmux capture-pane -t <new> -p > <narrow_capture>`.
   → capture `narrow`.
4. `tmux resize-pane -t <new> -x $BASE_W`, then sleep 4.
   (Same detached-session caveat: use `resize-window -x $BASE_W -y 50`.)
5. `tmux capture-pane -t <new> -p > <restored_capture>`.
   → capture `restored`.
6. `tmux send-keys -t <new> "q"` to close the reader (cleanup).
7. `tmux kill-pane -t <new>`.

**Expect** (every predicate must hold):
- `baseline` contains `Sections (n/p`
- `baseline` row 1 starts with `╭`
- `narrow` contains `Sections (n/p`
- `narrow` not contains `OpenAI Codex (v`
- `narrow` not contains `/model to change`
- `narrow` not contains `directory:   ~/`
- `narrow` not contains `Tip: New For a limited time`
- `narrow` row 1 starts with `╭`
- `narrow` row 1 ends with `╮`
- `restored` contains `Sections (n/p`
- `restored` not contains `OpenAI Codex (v`
- `restored` not contains `/model to change`
- `restored` not contains `directory:   ~/`
- `restored` not contains `Tip: New For a limited time`
- `restored` row 1 starts with `╭`
- `restored` row 1 ends with `╮`

---

## TR-002: Tab-to-ask response stays inline in the reader

When the user is in the reading view and presses Tab to ask a follow-up,
the agent is supposed to answer by patching the section (via
`patch_document_section` / `append_to_section`) — the answer should appear
**inside the reader**, not as a chat bubble above it. This test verifies
the inline-response path is wired and the system-prompt wrappers don't
leak into chat.

**Setup**: identical to TR-001 Setup steps 1–6 (build, split a pane, send the
prompt, wait for `Sections (n/p`).

**Action**:
1. `tmux capture-pane -t <new> -p > <pre_capture>`.
   → capture `pre`.
2. Pick a question whose answer should clearly extend the current section.
   For the coffee-slides prompt, use:
   `what is the caffeine content of a typical cup?`
3. `tmux send-keys -t <new> Tab`, sleep 1, then `tmux send-keys -t <new> "<question>"`, sleep 1, then `tmux send-keys -t <new> Enter`.
4. Poll `tmux capture-pane -t <new> -p` every 6s, up to 3 minutes. Stop when
   the capture contains the word `caffeine` AND no longer contains
   `Tab: ask` on the same line as `q: close` is **absent or unchanged**.
   (Simpler proxy: stop when the section content visibly differs from
   `pre`.)
5. `tmux capture-pane -t <new> -p > <post_capture>`.
   → capture `post`.
6. Cleanup: `tmux send-keys -t <new> "q"`, then `tmux kill-pane -t <new>`.

**Expect** (every predicate must hold):
- `post` contains `caffeine` — the agent's answer mentioned the topic
- `post` not contains `[The user is reading` — system prompt didn't leak
- `post` not contains `<!-- READER_TOOL_INSTRUCTIONS` — instructions block didn't leak
- `post` not contains `The user selected specific text from the section` — selection variant prompt didn't leak
- `post` not contains `› what is the caffeine content` — the user's question didn't render as a chat bubble (would be a regression of the Tab-question-hide fix)
- `post` row 1 starts with `╭` — reader frame still intact
- `post` row 1 ends with `╮`
- `post` contains `Sections (n/p` — sections list still rendered

---

## TR-003: TUI startup smoke

**Setup**: Build + launch as in TR-001 setup steps 1–4.

**Action**: `tmux capture-pane -t <new> -p > <capture>`.

**Expect**:
- `capture` contains `Agents2Agents ata (v`
- `capture` contains `YOLO mode` (if launched with `--yolo`)
- `capture` contains `directory:`

---

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

## TR-006: Up-arrow history

**Setup**: TR-005 (submit at least one message first).

**Action**:
1. `tmux send-keys -t <new> C-u`; sleep 0.3.
2. `tmux send-keys -t <new> Up`; sleep 0.5.
3. → capture `up`.

**Expect**:
- `up` contains `respond with just hi` in the composer (`›` line)

---

## TR-008: Reader close + chat doesn't garble

This regression-tests the v0.129.0 bug where after pressing `q` to close
the reader, chat would render jumbled escape sequences.

**Setup**: TR-007 + reader open with `Sections (n/p` visible.

**Action**:
1. `tmux send-keys -t <new> Escape`; sleep 0.5; `q`; sleep 2.
2. → capture `post_close`.
3. `tmux send-keys -t <new> "reply with just OK"`; sleep 1; `Enter`.
4. Poll until `tmux capture-pane -t <new> -p | grep -E "^• OK\b"`.
5. → capture `post_chat`.

**Expect**:
- `post_close` contains `Agent showed document:`
- `post_close` not contains `Sections (n/p` (reader closed)
- `post_chat` contains `• OK`
- `post_chat` not contains any of `╭`, `╮`, `╯`, `╰` from a leftover reader frame

---

## TR-009: Up-arrow history excludes system-injected prompts

When voice-mode prefixes or reading-view question wrappers are sent to the
agent, they should NOT appear in the up-arrow history (the user typed only
the visible part).

**Setup**: TR-007 + at least one Tab-to-ask submission from the reader.

**Action**:
1. Close reader, return to chat.
2. `tmux send-keys -t <new> C-u`; sleep 0.3.
3. For i in 1..8: `tmux send-keys -t <new> Up`; sleep 0.2.
4. → capture `history`.

**Expect** (all must hold):
- `history` not contains `[The user is reading`
- `history` not contains `<voice>`
- `history` not contains `<!-- READER_TOOL_INSTRUCTIONS`
- `history` not contains `[The user closed the document reader`

---

## TR-010: /experimental menu

> Updated 2026-05-21: ata's `/experimental` menu no longer contains
> `Repository Understanding` (removed when ata diverged from upstream
> codex) and the voice row is labeled `Voice mode` (lowercase `m`).
> Predicates now assert two stable ata-specific rows: `Voice mode` and
> `Scheduling`.

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/experimental"`; sleep 0.5; `Enter`; sleep 2.
2. → capture `menu`.
3. `tmux send-keys -t <new> Escape`; sleep 1.
4. → capture `post`.

**Expect**:
- `menu` contains `Experimental features`
- `menu` contains `Voice mode` — flagship feature row present
- `menu` contains `Scheduling` — flagship feature row present
- `post` not contains `Experimental features` (menu dismissed)

---

## TR-011: Code-understanding via code_intel tool

The repo_context / code_intel tool uses LSP + treesitter to answer symbol
queries. This test verifies the tool is registered and successfully
returns a definition location.

**Setup**: TR-003 setup. Run inside a Rust workspace (`$ATA_REPO/codex-rs`
works) so the LSP has something to index.

**Action**:
1. `tmux send-keys -t <new> "use code_intel to find where parse_sections is defined"`; sleep 1; `Enter`.
2. Poll up to 3 min until response contains `parse_sections` AND a file
   path like `.rs:`.
3. → capture `resp`.
4. Inspect the active session JSONL (`~/.ata/sessions/<latest>.jsonl`):
   `jq -r 'select(.payload.name=="code_intel") | .payload.arguments' <file>`.
   → save as `tool_calls`.

**Expect**:
- `resp` matches `parse_sections.*\.rs:\d+`
- `tool_calls` contains `symbolSearch` (the operation)
- `tool_calls` contains `parse_sections` (the query)

If `code_intel` falls back to `exec_command` grep (i.e. `tool_calls` is
empty for `code_intel`), the test still passes if `resp` cites correct
locations — but the failure mode is "tool not registered", which is a
real regression worth noting in the report.

---

## TR-012: /voice enters voice mode

> Updated 2026-05-21: the original test also asserted on `Recording` /
> `Transcribing` states triggered by Hold-Space. That path is
> unautomatable from tmux: `tmux send-keys Space` is press-and-release
> (no real hold), and even a real hold needs microphone audio that tmux
> can't supply. The Hold-Space-to-record flow stays a manual test;
> automated coverage stops at "voice mode entered".

**Setup**: TR-003 setup + precondition below.

**Precondition** (applies to TR-012 and TR-013): voice mode is an
experimental feature with `default_enabled: false`. Verify
`~/.ata/config.toml` has:

```toml
[features]
voice_mode = true
```

Without it, `/voice` will print "Voice mode is disabled" instead of
entering voice mode, and the predicates below will not match.

**Action**:
1. `tmux send-keys -t <new> "/voice"`; sleep 0.5; `Enter`; sleep 2.
2. → capture `entered`.

**Expect**:
- `entered` contains `Voice mode on. Hold Space to speak.` — announcement printed
- `entered` contains `🎤  Hold Space to speak` — composer switched to PTT prompt

---

## TR-013: Escape does NOT exit voice mode; /voice does

> Updated 2026-05-21 (v3): two earlier "fixes" were themselves wrong.
> (a) `contains "previous message"` fails on a fresh session because
> Escape produces no visible hint at all when there's no prior chat
> history — only when something was typed before. (b) `contains "Find
> and fix a bug"` fails because ata's composer placeholder rotates
> randomly per launch (`Summarize recent commits`, `Run /review on my
> current changes`, etc.). Predicates now use the only stable
> composer-state signal: presence or absence of the `🎤` emoji.

Escape is bound to "edit previous message", not to leaving voice mode.
Only the `/voice` slash toggles voice mode off. This test guards against
a regression that silently rebinds Escape.

**Setup**: TR-012 through step 2 (voice mode entered, composer shows `🎤  Hold Space to speak`). Inherits TR-012's voice-mode precondition.

**Action**:
1. `tmux send-keys -t <new> Escape`; sleep 1.
2. → capture `after_escape`.
3. `tmux send-keys -t <new> "/voice"`; sleep 0.5; `Enter`; sleep 1.
4. → capture `after_toggle_off`.

**Expect**:
- `after_escape` contains `🎤  Hold Space to speak` — voice mode survived Escape (the core assertion)
- `after_toggle_off` contains `Voice mode off.` — exit confirmation announcement
- `after_toggle_off` not contains `🎤` — voice composer is gone (any 🎤 elsewhere on the pane would also indicate a stuck voice composer)

---

## TR-014: /research menu opens, toggles, and saves without crashing ata

`/research` was previously observed exiting the ata process when saving
(Space toggle → Enter). This test guards against that regression: menu
opens, Space toggles a row from `[x]` to `[ ]`, Enter returns to chat,
and the pane must still exist afterwards.

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/research"`; sleep 0.5; `Enter`; sleep 2.
2. → capture `menu`.
3. `tmux send-keys -t <new> Down Down`; sleep 0.3; `Space`; sleep 0.5.
   (Arrows down to "Hacker News" — the 3rd row — then Space to toggle.)
4. → capture `toggled`.
5. `tmux send-keys -t <new> Enter`; sleep 2.
6. → capture `saved`.
7. `tmux list-panes -t <session>:<window> > <panes_capture>`.
   → capture `panes`.

**Expect** (all must hold):
- `menu` contains `Research tools`
- `menu` contains `Paper Search`
- `menu` contains `Hacker News`
- `menu` contains `Press space to select or enter to save`
- `toggled` contains `[ ] Hacker News` — toggle flipped the box
- `saved` not contains `Research tools` — menu dismissed
- `saved` contains `gpt-` — back in chat (model footer line is the stable "in chat" signal; the composer placeholder rotates per launch and can't be asserted)
- `panes` contains the ata pane index — pane survived (regression guard)

---

## TR-015: /rollout prints the current session JSONL path

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/rollout"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `out`.

**Expect**:
- `out` contains `Current rollout path:`
- `out` contains `.ata/sessions/`
- `out` contains `rollout-`
- `out` contains `.jsonl`

---

## TR-016: /clear wipes visible chat but keeps the session resumable

`/clear` resets the rendered chat (welcome banner back, prior messages gone)
but the underlying session continues — ata prints a `resume` hint with the
session id so the user can pick the conversation back up.

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "clear-test-marker-xyz"`; sleep 1; `Enter`.
2. Poll up to 60s until `tmux capture-pane -t <new> -p` contains `clear-test-marker-xyz`
   on a `›` line (proves submission landed).
3. `tmux send-keys -t <new> "/clear"`; sleep 0.5; `Enter`; sleep 2.
4. → capture `cleared`.

**Expect**:
- `cleared` contains `Agents2Agents ata (v` — welcome banner re-rendered
- `cleared` contains `To continue this session, run ata resume` — resume hint shown
- `cleared` not contains `clear-test-marker-xyz` — prior user message wiped from view

---

## TR-017: /permissions menu opens, marks current, dismisses without changing

`/permissions` shows the three permission tiers (Default / Auto-review /
Full Access) with the active one marked `(current)`. Escape must dismiss
the menu *without* mutating the active permission — a regression where
Escape silently saves would degrade or escalate access.

**Setup**: TR-003 setup (launched with `--yolo`, so Full Access is active).

**Action**:
1. `tmux send-keys -t <new> "/permissions"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `menu`.
3. `tmux send-keys -t <new> Escape`; sleep 1.
4. → capture `dismissed`.

**Expect**:
- `menu` contains `Update Model Permissions`
- `menu` contains `Default`
- `menu` contains `Auto-review`
- `menu` contains `Full Access`
- `menu` contains `Full Access (current)` — `--yolo` maps to Full Access
- `dismissed` not contains `Update Model Permissions` — menu gone
- `dismissed` contains `permissions: YOLO mode` — banner unchanged

---

## TR-018: /model is a two-step flow (model → reasoning level)

`/model` opens a model picker; Enter on a model opens a second menu for
reasoning effort. Enter on the active reasoning level returns to chat
with a `Model changed to <model> <effort>` confirmation and the banner
must reflect the same value.

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/model"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `model_menu`.
3. `tmux send-keys -t <new> Enter`; sleep 1.  (confirm current model)
4. → capture `effort_menu`.
5. `tmux send-keys -t <new> Enter`; sleep 1.  (confirm current effort)
6. → capture `applied`.

**Expect** (model-name-agnostic — guards the flow, not the user's chosen model):
- `model_menu` contains `Select Model and Effort`
- `model_menu` contains `(current)` — some model is marked active
- `model_menu` contains `gpt-` — at least one gpt-* row rendered
- `effort_menu` contains `Select Reasoning Level for gpt-`
- `effort_menu` contains `Low`
- `effort_menu` contains `Medium`
- `effort_menu` contains `High`
- `effort_menu` contains `(current)` — some effort is marked active
- `applied` contains `Model changed to gpt-` — confirmation printed
- `applied` contains `model:       gpt-` — banner shows a model
- `applied` not contains `Select Model and Effort` — menus dismissed
- `applied` not contains `Select Reasoning Level` — menus dismissed

---

## TR-019: @ file-mention autocomplete + Tab accepts top match

Typing `@<prefix>` in the composer pops an autocomplete with matching repo
files; Tab selects the top entry and inserts it into the composer. An
empty `@` should render "no matches" (i.e. the picker is alive but has
nothing to suggest yet).

**Setup**: TR-003 setup. Run from a repo with multiple `Cargo.toml` files
(`$ATA_REPO/codex-rs` is the canonical case).

**Action**:
1. `tmux send-keys -t <new> "@"`; sleep 0.5.
2. → capture `empty`.
3. `tmux send-keys -t <new> "Cargo"`; sleep 0.5.
4. → capture `prefix`.
5. `tmux send-keys -t <new> Tab`; sleep 0.5.
6. → capture `accepted`.
7. Cleanup: `tmux send-keys -t <new> C-u`.

**Expect**:
- `empty` contains `no matches` — picker is open but empty
- `prefix` contains `Cargo.toml` — top match rendered
- `prefix` contains `Cargo.lock` — second match rendered (proves multi-result)
- `accepted` contains `› Cargo.toml` — top match inserted into composer
- `accepted` not contains `no matches` — picker dismissed
- `accepted` not contains `@Cargo` — raw `@`-syntax replaced

---

## TR-020: Unknown slash command shows a helpful hint

ata does not have a `/help` command; when an unrecognized slash is sent,
the TUI must print `Unrecognized command '<name>'. Type "/" for a list…`
rather than silently forwarding the text to the agent or crashing.

**Setup**: TR-003 setup.

**Action**:
1. `tmux send-keys -t <new> "/help"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `out`.

**Expect**:
- `out` contains `Unrecognized command '/help'`
- `out` contains `Type "/" for a list of supported commands`

---

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

## TR-022: Escape interrupts an in-flight turn cleanly

> Updated 2026-05-21: the original setup polled for `esc to interrupt`
> at the skill-default 6s cadence. gpt-5.5 often finishes the response
> before the next poll fires, so the test sent Escape after the turn
> had already completed and the interrupt banner never appeared
> (manual repro confirms the interrupt path itself works). Action now
> polls at a tight 0.2s cadence and uses a heavier prompt to widen the
> interrupt window.

The thinking indicator reads `esc to interrupt`, so Escape is the
documented interrupt key. Pressing it while the agent is working stops
the turn and shows `Conversation interrupted - tell the model what to do
differently.` plus a `/feedback` hint. (Escape has context-dependent
behavior: it edits the previous message when idle — see TR-013 for the
voice-mode case — and interrupts when a turn is mid-flight.)

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

## TR-023: /scheduling empty panel opens, dismisses, chat survives

Opens the `/scheduling` panel on a session with no cron/monitor tasks
and verifies the empty state renders correctly, Escape dismisses the
panel cleanly, and the chat composer still accepts input afterward. The
"dismiss + chat round-trip" half is the deep regression guard — if the
panel leaves frame cells or focus state behind on close, the next
message gets garbled (same failure mode TR-008 guards against for the
reading view).

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
*pre-delete* process tree, but cannot pre-empt a fire that the cron
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

## TR-031: Reading view visual selection → scoped patch hits the right section only

The hardest reading-view regression class: selection state in the
viewer must be passed to the agent as the patch scope, the agent must
call `patch_document_section` / `update_document_section` with the
correct `section_index`, and the patch must NOT touch adjacent
sections. Single-pane smoke tests cannot catch wrong-section patching
because the rendering may look fine while the JSONL shows scope drift.
This test is the strongest cross-layer guard for reading view because
it triangulates pane content, JSONL tool args, and pane-after-patch
state.

**Setup**: TR-003 setup + TR-001's reading-view precondition (both
`[features] reading_view = true` AND `[reading_view] mode = "enabled"`
in `~/.ata/config.toml`).

**Action**:
1. In ata, send: `give me 3 short slides on coffee in reading view, don't use any skills`. Sleep 1; `Enter`.
2. Poll up to 3 min until pane contains `Sections (n/p` (reader open). → capture `pre`.
3. Press `v` to enter visual selection mode. Sleep 0.3.
4. Press `l` 25 times (extends the selection ~25 characters across the section title). Sleep 0.3 between batches.
5. Press `Tab` to enter ask-about mode. Sleep 0.3.
6. Type `rewrite this to be shorter`; sleep 1; press `Enter`.
7. Poll up to 3 min until pane shows the new content (proxy: section 1 text is visibly shorter than the original — easiest signal is `pre` contained the phrase `roasted coffee beans, which are the seeds of coffee cherries` and the post-patch capture does NOT). → capture `post`.
8. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`
   - `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.
   - `jq -r 'select(.payload.name=="patch_document_section" or .payload.name=="update_document_section") | .payload.arguments' "$SESS" > <patch_args>`. → capture `patch_args`.
9. Cleanup: press `q` to close the reader.

**Expect** (all must hold):

Pane — patch landed in section 1 only:
- `post` contains `Sections (n/p` — reader still open
- `post` contains `Slide 1` — section 1 title still visible
- `post` not contains `roasted coffee beans, which are the seeds of coffee cherries` — original section-1 long form is gone (proves it was actually rewritten)
- `post` contains `Slide 2: Why People Drink It` — section 2 title unchanged
- `post` contains `Slide 3: Common Brewing Styles` — section 3 title unchanged

Tool routing:
- `tool_counts` contains `present_reading_view` — reader was opened via the right tool
- `tool_counts` contains `patch_document_section` OR `update_document_section` — at least one scoped-edit tool was used (the agent may use both, or just one, depending on whether it interprets the selection as a literal patch target or as scope context)
- `tool_counts` not contains `apply_patch` — did NOT fall back to the raw apply-patch tool (which would bypass the section model and risk wrong-section edits)
- `tool_counts` not contains `shell` — did NOT shell out

Section scoping — the deep regression guard:
- `patch_args` contains `"section_index": 0` OR `"section_index":0` — every scoped-edit call targeted section index 0 (Slide 1, zero-indexed)
- `patch_args` not contains `"section_index": 1` — section 2 was untouched
- `patch_args` not contains `"section_index": 2` — section 3 was untouched
- `patch_args` contains `"document_id"` — argument shape includes a document id (proves the patch is scoped to this document, not the global namespace)

Selection-text fidelity:
- `patch_args` contains text from the user's selection (substring of `"Slide 1: What Coffee Is"`, e.g. `"Slide 1: What Coffee I"` reflecting the 25-char selection length) — proves the selected text actually reached the agent's prompt, not the whole section

---

## TR-032: Reading view — agent adds a new section with the right tool

Reading-view has 5 write tools (`present_reading_view`, `add_document_section`,
`append_to_section`, `patch_document_section`, `update_document_section`).
Each has different semantics; the agent must disambiguate based on the user's
intent. This test verifies that "add a new section" routes specifically to
`add_document_section` (NOT `patch` / `update`, which would mutate existing
content), and that the new section is inserted at the right position with
sections 1-3 untouched.

**Setup**: TR-003 setup + TR-001's reading-view precondition.

**Action**:
1. In ata, send: `give me 3 short slides on coffee in reading view, don't use any skills`. Sleep 1; `Enter`.
2. Poll up to 3 min until pane contains `Sections (n/p` (reader open) AND `Slide 3` (third section rendered). → capture `pre`.
3. Press `Tab` (from outside visual selection mode) to enter the ask prompt. Sleep 0.5.
4. Type `add a slide 4 about espresso`; sleep 1; press `Enter`.
5. Poll up to 3 min until pane contains `Slide 4` AND `4/4` (header section counter shows 4-of-4). → capture `post`.
6. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`
   - `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.
   - `jq -r 'select(.payload.name=="add_document_section") | .payload.arguments' "$SESS" > <add_args>`. → capture `add_args`.
7. Cleanup: press `q` to close the reader.

**Expect** (all must hold):

Pane — new section inserted at the end:
- `pre` contains `Slide 3` AND `Sections (n/p` — initial 3-section state
- `pre` not contains `Slide 4` — section 4 didn't exist before
- `post` contains `Slide 4` — new section visible
- `post` contains `4/4` — header section counter updated from `1/3` (or wherever) to `4/4`
- `post` contains `end of document` — slide 4 is now the last, so the bottom edge marker says end-of-document
- `post` contains `Slide 1: What Coffee Is` — section 1 title preserved
- `post` contains `Slide 2: Why People Drink It` — section 2 title preserved
- `post` contains `Slide 3: Common Brewing Styles` — section 3 title preserved

Tool routing:
- `tool_counts` contains `present_reading_view` — initial open
- `tool_counts` contains `add_document_section` — the dedicated add tool was used
- `tool_counts` not contains `patch_document_section` — did NOT mutate existing content
- `tool_counts` not contains `update_document_section` — did NOT replace an existing section
- `tool_counts` not contains `append_to_section` — did NOT append to an existing section

Argument fidelity — correct insertion point:
- `add_args` contains `"after_section_index": 2` OR `"after_section_index":2` — insert after section index 2 (zero-indexed Slide 3); new section becomes index 3 (Slide 4)
- `add_args` contains `"heading"` — heading argument present
- `add_args` contains `Slide 4` OR `Espresso` — heading reflects the user's request
- `add_args` contains `"content"` — content argument present
- `add_args` contains `espresso` (case-insensitive substring) — content about the requested topic
- `add_args` contains `"document_id"` — scoped to a document

---

## TR-033: Reading view — agent picks append_to_section (NOT update_document_section)

The trickiest reading-view tool disambiguation. `append_to_section` preserves
existing content and adds new text at the end; `update_document_section`
replaces the entire section. A user asking "add X to slide N" almost
always wants append semantics — but a model may incorrectly call update
with concatenated content, which works visually but is semantically
wrong (it sends the entire section content as a fresh write every time,
making any concurrent edits or partial-content history brittle).

This is THE classic "rendering looks fine, wrong tool got called"
regression that a pane-only test cannot catch.

**Setup**: TR-003 setup + TR-001's reading-view precondition.

**Action**:
1. In ata, send: `give me 3 short slides on coffee in reading view, don't use any skills`. Sleep 1; `Enter`.
2. Poll up to 3 min until pane contains `Sections (n/p` AND `Slide 3`. → capture `pre`.
3. Press `Tab` to enter ask mode. Sleep 0.5.
4. Type `add a fun fact about coffee to the end of slide 1`; sleep 1; press `Enter`.
5. Poll up to 3 min until the agent's patch is reflected — proxy: capture pane and check that `Fun fact` substring is now present somewhere.
6. Navigate back to Slide 1: `tmux send-keys -t <new> p p p` (or until `1/3` / `1/4` shows in the header). Sleep 1.
7. → capture `post`.
8. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`
   - `jq -r '.payload.name // empty' "$SESS" | sort | uniq -c > <tool_counts>`. → capture `tool_counts`.
   - `jq -r 'select(.payload.name=="append_to_section") | .payload.arguments' "$SESS" > <append_args>`. → capture `append_args`.
9. Cleanup: press `q` to close the reader.

**Expect** (all must hold):

Pane — original content preserved + new content appended:
- `pre` contains `roasted` (substring of original Slide 1 content) — original exists
- `post` contains `Slide 1: What Coffee Is` — title preserved
- `post` contains `roasted` — ORIGINAL Slide 1 content preserved (this is the key append-vs-update predicate)
- `post` contains `Fun fact` — new appended content present
- `post` contains `Slide 2: Why People Drink It` — section 2 unchanged
- `post` contains `Slide 3: Common Brewing Styles` — section 3 unchanged

Tool routing — append, NOT update:
- `tool_counts` contains `append_to_section` — correct append tool was used
- `tool_counts` not contains `update_document_section` — did NOT replace the whole section
- `tool_counts` not contains `patch_document_section` — did NOT use the find-and-replace patcher
- `tool_counts` not contains `add_document_section` — did NOT add a new section

Argument fidelity — right section, right scope:
- `append_args` contains `"section_index": 0` OR `"section_index":0` — append targets Slide 1 (zero-indexed)
- `append_args` not contains `"section_index": 1` — Slide 2 untouched
- `append_args` not contains `"section_index": 2` — Slide 3 untouched
- `append_args` contains `"content"` — content field present
- `append_args` contains `Fun fact` OR `fun fact` — content reflects the user's "fun fact" request
- `append_args` not contains `roasted` — content is ONLY the new fun fact, NOT a re-paste of the existing section (proves the agent used append semantics, not "update with concatenation")

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

## TR-036: Reading view navigation — n/p, t (TOC), and TOC jump

Reading view's section navigation has three modes that intersect:
section-level (`n`/`p`), TOC overlay (`t`), and intra-section scroll
(`j`/`k`). Each can break independently and silently. This test exercises
all of them on a single multi-section document, plus the read-indicator
(`✓`) state machine, plus header boundary affordances (no `◀` at section
1, no `▶` at the last section).

**Setup**: TR-003 setup + TR-001's reading-view precondition. Use a
6-section document for full nav coverage (TR-035's synthesis output
naturally produces one — alternatively, ask: `give me 6 short slides on
Rust async performance in reading view`).

**Action**:
1. Open a 6-section reading view (per Setup).
2. Wait for `1/6` in the header. → capture `at_1`.
3. Press `n`; sleep 0.5. → capture `at_2`.
4. Press `p`; sleep 0.5. → capture `back_at_1`.
5. Press `t`; sleep 0.5. → capture `toc_open`.
6. Press `j j j Enter`; sleep 0.5. → capture `jumped_to_4`.
7. Press `n n n`; sleep 1 (advance to the last section). → capture `at_last`.
8. Cleanup: press `q` to close the reader.

**Expect** (all must hold):

Section navigation:
- `at_1` contains `1/6 ▶` — header shows position + right arrow (more sections forward)
- `at_1` not contains `◀ 1/6` — no left arrow at section 1 (boundary affordance)
- `at_2` contains `◀ 2/6 ▶` — header now shows both arrows (sections in both directions)
- `back_at_1` contains `1/6 ▶` — back at section 1 cleanly
- `at_last` contains `◀ 6/6` — at the last section, header shows only left arrow
- `at_last` not contains `6/6 ▶` — no right arrow at the last section (boundary affordance)

TOC overlay:
- `toc_open` contains `Table of Contents` — dedicated TOC view rendered (not just the inline TOC at the bottom of section 1)
- `toc_open` contains `j/k to navigate | Enter to jump | t/Esc to dismiss` — TOC has its own help footer
- `toc_open` contains `▶ 1. Bottom Line` OR `▶ 1. ` — current section marked with `▶`

TOC jump:
- `jumped_to_4` contains `4/6` — landed on section 4 after `j j j Enter`
- `jumped_to_4` not contains `Table of Contents` — TOC overlay dismissed by the jump
- `jumped_to_4` contains a section-4-specific term (e.g. `Hacker News Signal` for TR-035's output, or whatever the section-4 title is) — actually rendered the target section

Read indicators:
- `back_at_1` contains `✓ 2. ` — section 2 marked as read after visiting it
- `at_last` contains `✓ 4. ` — section 4 also marked (visited via TOC jump)

---

## TR-037: Tab-to-ask in a reader section produces a scoped, foldable inline answer

When the user is reading a section and presses Tab to ask a follow-up,
ata's design is: agent receives ONLY this section as context, answer
gets appended INTO that section as a foldable Q&A subsection, and the
user's question does NOT render as a chat bubble. Three layers
intersect: pane (foldable `[-]` block visible with the answer), reader
state (footer gains the `f: fold` key), and protocol (the
`append_to_section` call has `foldable: true` and `section_index`
matching the focused section).

This is THE strongest reading-view regression guard because the entire
inline-Q&A pipeline collapses if any one piece breaks: wrong section
context = wrong answer; wrong patch target = answer in wrong section;
`foldable: false` = the Q&A is permanently expanded; `foldable: true`
on the wrong section_index = silent corruption.

**Setup**: TR-003 setup + TR-001's reading-view precondition. Reader
open on a multi-section document with content that supports a clearly
section-scoped question. The cleanest setup: run TR-035 first (produces
a `rust-async-performance-papers-hn` synthesis with HN data in section
4), then navigate to section 4. Alternative: any 3+ section document
where section N has a fact that's NOT in sections 1..N-1.

**Action**:
1. Open the reading view per Setup.
2. Navigate to a specific section using `n`/`p` (or TOC jump). For the
   TR-035-based setup: `t`, then `j j j Enter` to land on section 4
   "Hacker News Signal". → capture `pre`.
3. Press `Tab` to enter ask mode. Sleep 0.5.
4. Type a section-scoped question. For TR-035's section 4: `which thread on this page had the most upvotes?`. Sleep 1.
5. Press `Enter`. Sleep 1.
6. Poll up to 2 min until the section's content visibly changes (pane contains the answer phrase, e.g. `most-upvoted` or `446 points` for TR-035's section 4).
7. → capture `post`.
8. Inspect session JSONL:
   - `SESS=$(find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1)`
   - `jq -c 'select(.payload.name=="append_to_section") | .payload.arguments' "$SESS" | tail -1 > <append_args>` → capture `append_args` (the most recent append, which should be the Tab-ask one).

**Expect** (all must hold):

Pane — answer rendered inline with the right structure:
- `post` contains the correct factual answer for the section's content. For TR-035's section 4: `Async Rust never left the MVP state` AND `446 points`.
- `post` contains `┊` — left-margin indicator on the foldable Q&A block
- `post` contains `[-]` OR `[+]` — foldable marker on the Q&A heading
- `post` contains `f: fold` — help footer gained the fold key (proves dynamic key registration when a foldable element appears)
- `post` not contains `› which thread on this page` — user's question NOT rendered as a chat bubble (the TR-002 inline-only invariant carries over)
- `post` not contains `[The user is reading` — system-prompt wrapper didn't leak
- `post` not contains `<!-- READER_TOOL_INSTRUCTIONS` — instructions block didn't leak

Protocol — append targeted the right section as a foldable Q&A:
- `append_args` contains `"foldable":true` OR `"foldable": true` — Tab-ask path produces foldable Q&A entries (NOT the `foldable: false` that explicit content additions use, like TR-033's fun-fact)
- `append_args` contains the correct `section_index` for the section the user was on (e.g. `"section_index": 3` OR `"section_index":3` for section 4 in TR-035's doc)
- `append_args` not contains `"section_index": 0` — section 1 was NOT touched (proves scope respected)
- `append_args` contains a `"content"` field with the answer text — the answer payload is in the patch, not just generated client-side
- `append_args` contains a `"summary"` field — Tab-ask path generates summaries for collapsed view
- `append_args` contains `"document_id"` — scoped to the right document

---

# Adding tests

Append `## TR-<NNN>: <name>` sections following the same shape. Pick
**Expect** predicates that fail when the bug regresses and pass otherwise —
keep them narrow. A predicate like "contains 'Section'" is too loose; one
like "row 1 starts with '╭'" is right because that's a specific
property of the rendering.

# Inspecting session logs

Many TUI tests verify that the agent actually called the right tool, not
just that the rendering looks right. Use the JSONL session logs at
`~/.ata/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl` for that:

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

Cross-checking the session log against the rendered pane catches
"agent rendered the right text but didn't call the right tool"
regressions — for example a Tab-to-ask response that's rendered inline
by the model but didn't go through `patch_document_section` is broken
even if the chat looks correct.

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
- `out_empty` contains `No agent response to copy` — exact error string (verified 2026-05-22 on ata 0.7.0)
- `out_empty` not contains `Copied last message to clipboard`
- `clipboard_empty` not contains `No agent response to copy` — the error string itself was NOT pushed to the clipboard (would be a regression where ata writes its own UI error into the user's clipboard)

(Note: predicate `clipboard equals Scenario C value` is not used because external clipboard activity between scenarios can confound it. The invariant we actually care about is that ata's empty-case error doesn't leak into the clipboard.)

### Scenario E: copy from inside a /side conversation

**Precondition (critical)**: `/side` requires the current conversation
to have at least one completed user→agent turn since the most recent
`/clear` or session start. Hitting `/side` on a fresh or freshly-cleared
session prints `'/side' is unavailable until the current conversation
has started. Send a message first, then try /side again.` (verified
2026-05-22). So Scenario D's `/clear` leaves us in the "conversation not
started" state — we must re-prime with a turn before /side.

16. Re-prime the conversation: `tmux send-keys -t <new> "respond with just hello from side parent"`; sleep 1; `Enter`. Poll until pane matches `^• hello from side parent\b` (proves the turn completed).
17. `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`; sleep 2. Wait until side-conversation context label appears AND an agent response is rendered.
18. `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
19. → capture `out_side`; `pbpaste > <clipboard_side>`.

**Expect** (verified 2026-05-22 on ata 0.7.0):
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

**Expect** (verified 2026-05-22 on ata 0.7.0):
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

**What /ps tracks (verified 2026-05-22 via source + manual test)**:
- `/ps` enumerates `unified_exec_processes` — long-lived interactive shells started by `exec_command` with `ExecCommandSource::UnifiedExecStartup`. These are persistent processes the agent can `write_stdin` to (Python REPL, interactive bash, node REPL, etc.).
- `/ps` does NOT track: `monitor_start` rows (those go to `/scheduling` Monitors), `exec_command(background:true)` async commands (also Monitors), `cron_create*` (Cron section), or one-shot `exec_command` calls (no persistent process).
- Naming caveat: the heading reads "Background terminals" but in practice it's "Persistent interactive shells".

3. Send: `open a persistent python REPL and don't close it`. Poll up to 60s until pane contains both `Persistent Python REPL is open` AND `/ps to view · /stop to close` (the footer hint that appears when at least one shell is alive).
4. `tmux send-keys -t <new> "/ps"`; sleep 0.5; `Enter`; sleep 1.
5. → capture `populated`.

**Expect** (verified 2026-05-22 on ata 0.7.0):
- `populated` contains `Background terminals` — heading
- `populated` not contains `No background terminals running.` — empty-state copy is gone
- `populated` matches `^\s*•\s+python` — at least one bullet row for the python process
- `populated` contains `Python 3.` — recent stdout/stderr preview is rendered under the row (last N stream chunks)
- `populated` contains `>>>` — REPL prompt visible in the preview (proves recent_chunks is being captured live)
- `populated` contains `1 background terminal running` — footer counter
- `populated` contains `/ps to view · /stop to close` — footer hint

### Scenario B2: monitor-spawned process DOES NOT show in /ps (negative)

Verified 2026-05-22: starting a monitor with `start a monitor named tr039-bg that runs: sleep 90` puts the row in `/scheduling` under `Monitors (1) [Running]`, but `/ps` continues to report `No background terminals running.` Same for `exec_command(background:true)` — they appear in `/scheduling` as `Monitors` rows, `pgrep -fl` returns a live PID, and yet `/ps` shows empty. This is by design: `/ps` is scoped to `unified_exec` startup processes only.

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

**Expect** (verified 2026-05-22 on ata 0.7.0):
- `stop_out` contains `Stopping all background terminals.` — exact confirmation copy
- `ps_after_stop` contains `No background terminals running.` — empty state restored
- `ps_after_stop` not contains `python` — the REPL row is gone
- `<pgrep_after>` does NOT contain the python REPL PID from before — the OS process was actually killed (not just dropped from UI; to fully verify, snapshot `pgrep` before /stop and diff)

### Scenarios E / F: TODO — multi-shell, just-completed, PID cross-check

Future expansion:
- E: open TWO persistent REPLs (python + node), `/ps` shows both rows in stable order, both with their own recent_chunks.
- F: exit a REPL via `write_stdin "exit\n"`, verify the row drops from `/ps` (without `/stop`).
- PID accuracy: extract PID from `/ps` if exposed (current 0.7.0 output doesn't show PID directly — confirm with source); cross-check against system `ps`.

**Cleanup**:
- `/stop` (if not already done in Scenario D) — clears all unified_exec shells.
- `/scheduling` → `d` per row to remove monitor entries.
- Verify with `pgrep -fl "sleep 90"` and `pgrep -fl "python3.*REPL"` both returning nothing — if anything is alive, `pkill -f` the survivor.

---

## TR-040: /workspace — full behavior matrix

`/workspace` is a dispatcher. Bare invocation prints a one-line usage
hint; `current`, `list`, `use <selector>` are the three subcommands
exposed through the TUI. Beneath the TUI sits the full `ata workspace`
CLI (~30 subcommands, covered separately in TR-CLI tests). This matrix
exercises: usage hint, active-workspace display, multi-workspace list,
workspace switching with valid/invalid selectors, list reflecting
switched state, and behavior during in-flight turn.

**Setup**: TR-003 setup. A clean default `global` workspace.

### Scenario A: bare prints usage hint (not summary)

1. `tmux send-keys -t <new> "/workspace"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `bare`.

**Expect**:
- `bare` contains `Usage: /workspace`
- `bare` contains `[current|list|use <selector>]`
- `bare` contains `ata workspace --help` — points at the CLI for the full surface
- `bare` not contains `Current workspace:` — bare does NOT auto-show current state

### Scenario B: /workspace current with default global workspace

3. `tmux send-keys -t <new> "/workspace current"`; sleep 0.5; `Enter`; sleep 1.
4. → capture `current_global`.

**Expect**:
- `current_global` contains `Current workspace:` — heading
- `current_global` contains `global` — default name
- `current_global` matches `\b0 repos\b` — repo-count token present and accurate (default workspace has 0 repos)
- `current_global` contains `/workspace list` OR `/workspace use` — follow-up hint

### Scenario C: /workspace list with one workspace

5. `tmux send-keys -t <new> "/workspace list"`; sleep 0.5; `Enter`; sleep 1.
6. → capture `list_one`.

**Expect**:
- `list_one` contains `Workspaces (1)` — count header includes the total
- `list_one` contains `global` — the workspace itself
- `list_one` contains `current` — active marker on the row
- `list_one` matches `global\s+global\s+0 repos\s+current` — column layout: id, name, repo count, status

### Scenario D: create a second workspace via CLI, /workspace list shows both

Workspace creation is CLI-only — ata's TUI has no command to create a new workspace. So this scenario crosses process boundaries: spawn an external shell to run `ata workspace init`, then verify the TUI picks up the new entry.

7. From a separate shell (NOT through the ata TUI): `ata workspace init tr040-second > <init_out>`.
8. `tmux send-keys -t <new> "/workspace list"`; sleep 0.5; `Enter`; sleep 1.
9. → capture `list_two`.

**Expect** (verified 2026-05-22 on ata 0.7.0):
- `<init_out>` matches `^tr040-second-[0-9a-f]{8}\s*$` — the CLI prints ONLY the new workspace's id (name + 8-char hex suffix), nothing else. No "created" / "initialized" prose copy.
- `list_two` contains `Workspaces (2)` — count incremented
- `list_two` contains `global` AND `tr040-second` — both names present (`tr040-second-<suffix>` for the id column, `tr040-second` for the name column)
- `list_two` contains exactly one `current` token in the rightmost column (count `current` and verify it equals 1)
- `list_two` matches `tr040-second-[0-9a-f]{8}\s+tr040-second\s+0 repos` — column layout: id, name, repo count

### Scenario E: switch active workspace via /workspace use

10. `tmux send-keys -t <new> "/workspace use tr040-second"`; sleep 0.5; `Enter`; sleep 1.
11. → capture `switch`.
12. `tmux send-keys -t <new> "/workspace current"`; sleep 0.5; `Enter`; sleep 1.
13. → capture `current_after`.

**Expect** (verified 2026-05-22):
- `switch` contains `Selected workspace: tr040-second-` AND `(tr040-second, 0 repos)` — confirmation includes id + name + repo count
- `switch` contains `Workspace selection saved.` — separate confirmation line
- `switch` contains `Restart the TUI for the new workspace's sandbox roots and cwd to take effect.` — **important nuance**: the selection persists immediately, but the live session keeps the old workspace's sandbox roots and cwd. Sandbox/cwd switch requires restart.
- `current_after` contains `Current workspace: tr040-second-` — selection IS reflected immediately in `/workspace current` (manifest-level current, distinct from runtime-level)
- `current_after` not contains `Current workspace: global` — global is no longer the selection

### Scenario F: invalid selector → exact error, no state change

14. `tmux send-keys -t <new> "/workspace use does-not-exist-zzz"`; sleep 0.5; `Enter`; sleep 1.
15. → capture `invalid`.
16. `tmux send-keys -t <new> "/workspace current"`; sleep 0.5; `Enter`; sleep 1.
17. → capture `current_still`.

**Expect** (verified 2026-05-22):
- `invalid` contains `workspace selector 'does-not-exist-zzz' not found` — exact error string (note lowercase `workspace`, single-quoted selector)
- `invalid` not contains `Selected workspace:` — no confirmation copy (the switch did not happen)
- `invalid` not contains `Workspace selection saved.`
- `current_still` contains `tr040-second-` — active selection unchanged from Scenario E (the failed switch did not corrupt state)

### Scenario G: /workspace current during in-flight turn (non-blocking)

18. `tmux send-keys -t <new> "write me a 1000-word essay on espresso"`; sleep 0.3; `Enter`.
19. Tight-poll up to 10s for `esc to interrupt`. Within window: `tmux send-keys -t <new> "/workspace current"`; sleep 0.5; `Enter`; sleep 0.5.
20. → capture `during_turn`.
21. `tmux send-keys -t <new> Escape`; sleep 1 (cancel essay).
22. → capture `after_cancel`.

**Expect** (verified 2026-05-22 on ata 0.7.0):
- `during_turn` contains `Current workspace:` AND the active workspace name — `/workspace current` is ALLOWED during an in-flight turn (no blocking)
- `during_turn` not contains `unavailable` OR `wait` — no block error
- After `/workspace current` is issued during the turn: the essay keeps generating, `esc to interrupt` is still visible until explicit Escape (same non-cancelling behavior as `/copy` in TR-038 F)
- `after_cancel` contains `Conversation interrupted - tell the model what to do differently.` — the explicit Escape is what cancels (TR-022 invariant)

**Cleanup**:
- `/workspace use global` to restore default active.
- `ata workspace delete tr040-second --force` to remove the test workspace.
- Verify with `/workspace list` → `Workspaces (1)` and only `global` shown.

---

## TR-041: /agent and /subagents — full behavior matrix

Both commands route to `AppEvent::OpenAgentPicker`. `/subagents` is the
alias form of `/agent`. This matrix covers: picker layout + alias parity,
single-Main-agent state, multi-agent state after spawn, keyboard
navigation (↑/↓ and ⌥+←/⌥+→), agent switching with composer footer
update, message routing to the selected agent (cross-checked via JSONL),
behavior during in-flight turn, and the picker focus state on reopen.

**Setup**: TR-003 setup.

### Scenario A: picker layout on a fresh session (single Main agent)

1. `tmux send-keys -t <new> "/agent"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `picker_main_only`.

**Expect**:
- `picker_main_only` contains `Subagents` — heading
- `picker_main_only` contains `Select an agent to watch.` — body copy
- `picker_main_only` contains `⌥ + ←` AND `⌥ + →` — nav hint
- `picker_main_only` matches `1\.\s+•\s+Main \[default\] \(current\)` — Main row with bullet, default tag, current marker
- `picker_main_only` matches `[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}` — UUID-shaped thread id rendered
- `picker_main_only` contains `Press enter to confirm or esc to go back` — footer
- Only one row visible (count `^›` and `^  [0-9]\.` rows in the picker block; total must be exactly 1)

### Scenario B: alias parity — /subagents opens the same UI

3. `tmux send-keys -t <new> Escape`; sleep 1.
4. `tmux send-keys -t <new> "/subagents"`; sleep 0.5; `Enter`; sleep 1.
5. → capture `picker_via_subagents`.

**Expect**:
- `picker_via_subagents` contains `Subagents` — same heading
- `picker_via_subagents` contains `Main [default] (current)` — same content
- `picker_via_subagents` contains `Press enter to confirm or esc to go back`
- `picker_via_subagents` and `picker_main_only` are equal modulo timestamps and the thread id (compare with `diff` after stripping UUIDs)

### Scenario C: Escape dismisses without changing active agent

6. `tmux send-keys -t <new> Escape`; sleep 1.
7. → capture `after_escape`.
8. `tmux send-keys -t <new> "respond with just AAA"`; sleep 1; `Enter`. Poll until pane matches `^• AAA\b`.
9. → capture `chat_after_escape`.
10. `SESS_A=$(find ~/.ata/sessions -name "*.jsonl" -mmin -2 | xargs ls -t | head -1)`. → capture `sess_a_path` and the last user/agent turn's thread_id (jq).

**Expect**:
- `after_escape` not contains `Subagents` — picker fully gone
- `chat_after_escape` matches `^• AAA\b` — message landed and got a response
- `sess_a_path` is the same JSONL as before the picker opened — Escape did not start a new session
- The just-recorded turn's `thread_id` equals the Main agent's id from Scenario A — message routed to Main (the active agent didn't change)

### Scenario D: spawn a second agent, picker shows both

11. `tmux send-keys -t <new> "spawn a research subagent named tr041-helper that helps with documentation questions"`; sleep 1; `Enter`. Poll up to 60s until pane contains `Spawned` AND `tr041-helper` AND a thread id.
12. → capture `spawn_log`; record the new thread id as `HELPER_ID`.
13. `tmux send-keys -t <new> "/agent"`; sleep 0.5; `Enter`; sleep 1.
14. → capture `picker_multi`.

**Expect**:
- `spawn_log` contains `Spawned` AND `tr041-helper` — confirmation
- `picker_multi` matches `1\.\s+•\s+Main \[default\] \(current\)` — Main still listed and current
- `picker_multi` contains `tr041-helper` — new agent in the picker
- `picker_multi` contains the helper thread id `HELPER_ID`
- The picker shows exactly 2 rows total

### Scenario E: ↑/↓ keyboard navigation moves the focus marker (verified)

Standard arrow-key nav. Focus glyph is `›` (verified 2026-05-22).

15. `tmux send-keys -t <new> Down`; sleep 0.5. → capture `nav_down`.
16. `tmux send-keys -t <new> Up`; sleep 0.5. → capture `nav_up`.

**Expect**:
- `nav_down` matches `^› 2\.` — focus moved to row 2 (Boole)
- `nav_down` not contains `› 1.` — focus left row 1
- `nav_up` matches `^› 1\.` — focus back to row 1 (Main)

### Scenario F: ⌥+←/→ inside picker — no visible effect (verified)

Verified 2026-05-22: pressing `⌥+→` / `⌥+←` inside an already-open picker does nothing visible — the focus marker stays where the arrow keys put it. The hint text `⌥ + ← previous, ⌥ + → next` appears aspirational or applies to a state other than "picker open" (likely the "view a different agent's thread" shortcut from chat view — when the picker is NOT open, ⌥+arrow cycles which agent's chat history is currently displayed).

17. With focus on row 1, `tmux send-keys -t <new> M-Right`; sleep 0.5. → capture `opt_right_picker`.
18. `tmux send-keys -t <new> M-Left`; sleep 0.5. → capture `opt_left_picker`.

**Expect**:
- `opt_right_picker` matches `^› 1\.` — focus unchanged (no-op inside picker)
- `opt_right_picker` not contains `^› 2\.`
- `opt_left_picker` matches `^› 1\.` — also no-op
- Neither capture shows a `(current)` change — Main is still the active agent (no silent switch)

### Scenario F2: ⌥+←/→ from chat view (NOT picker) — switches the watched thread (verified)

When the picker is NOT open, the same `⌥+←/→` shortcut switches the currently-viewed agent's chat thread (just like pressing Enter on a picker row). The footer line updates to reflect the new active thread.

19. Close picker if open: `Escape`. Confirm in main chat view (footer shows `Main [default]`).
20. `tmux send-keys -t <new> M-Left`; sleep 0.5. → capture `opt_left_chat`.

**Expect** (verified 2026-05-22):
- `opt_left_chat` displays a different agent's chat history (e.g. Boole's `Understood. I'll stand by for documentation questions` greeting from spawn time)
- `opt_left_chat` matches the agent's footer (e.g. `Boole [default]` instead of `Main [default]` in the bottom-right)
- The view switch is silent — no banner like "switched to Boole" is printed

### Scenario G: select Boole via Enter → next message routes to Boole (verified)

Verified 2026-05-22: selecting a row via Enter switches the active agent. Subsequent prompts route to that agent's thread, NOT to Main. JSONL cross-check confirms.

21. Reopen `/agent`. Navigate to row 2 (Boole) via `Down`. Press `Enter`; sleep 2. → capture `selected_helper`.
22. `tmux send-keys -t <new> "respond with just BBB"`; sleep 1; `Enter`. Poll until `^• BBB\b`. → capture `chat_to_helper`.
23. `SESS_B=$(find ~/.ata/sessions -name "*.jsonl" -mmin -2 | xargs ls -t | head -1); jq -c 'select(.payload.text=="respond with just BBB") | {thread_id: .thread_id, ts: .timestamp}' "$SESS_B" > <route_check>`. → capture `route_check`.

**Expect**:
- `selected_helper` not contains `Subagents` — picker dismissed by Enter
- `selected_helper` contains Boole's earlier thread context (e.g. the spawn-time system prompt `You are tr041-helper. Stand by` and the agent's `Understood. I'll stand by` reply — proves Enter switched the view to Boole's thread, not just selection)
- `selected_helper` footer contains `Boole [default]` (NOT `Main [default]`)
- `chat_to_helper` matches `^• BBB\b` — message got a response
- `<route_check>` contains `"thread_id":"<HELPER_ID>"` — the BBB message landed in Boole's thread

### Scenario H: switch back to Main → CCC routes there (verified)

24. `/agent` → navigate to row 1 (Main) via `Up` → Enter. → capture `selected_main`.
25. `tmux send-keys -t <new> "respond with just CCC"`; sleep 1; `Enter`. Poll until `^• CCC\b`. → capture `chat_to_main`.
26. JSONL cross-check: confirm the CCC message landed in Main's thread, not Boole's.

**Expect**:
- `selected_main` footer contains `Main [default]` — back to Main's view
- `chat_to_main` matches `^• CCC\b`
- JSONL: the CCC turn's thread_id equals Main's id from Scenario A

### Scenario I: /agent during in-flight turn — picker opens immediately, overlay-style (verified)

Verified 2026-05-22 (re-verification correction): `/agent` invoked during an active turn opens the picker IMMEDIATELY, overlaying the running turn. The turn keeps running underneath the picker — `esc to interrupt` is hidden by the picker chrome but the turn continues. Pressing `Esc` to dismiss the picker also cancels the running turn (Esc's "interrupt-turn" handler takes precedence on dismissal).

This corrects an earlier observation that suggested `/agent` was queued/deferred — that was a timing artifact (the original turn had already completed in the gap between polling and sending `/agent`).

27. With at least one completed turn so `/agent` doesn't think conversation is fresh: send a 5000-word essay prompt to trigger a long-running turn.
28. Poll for `esc to interrupt`. As soon as it appears: `tmux send-keys -t <new> "/agent"`; sleep 0.3; `Enter`; sleep 0.5.
29. → capture `during_turn`.
30. `tmux send-keys -t <new> Escape`; sleep 2. → capture `after_cancel`.

**Expect**:
- `during_turn` contains `Subagents` — picker DID open during the turn (immediate, not queued)
- `during_turn` not contains `esc to interrupt` — the picker overlay hides the turn's interrupt indicator (turn is still running underneath, just not visible)
- `after_cancel` not contains `Subagents` — picker dismissed by Escape
- `after_cancel` contains `Conversation interrupted - tell the model what to do differently.` — the SAME Escape that dismissed the picker also cancelled the running turn

### Scenario J: picker focus state on reopen — resets to current agent (verified)

Verified 2026-05-22: when the picker is reopened, the focus marker `›` always lands on the `(current)` agent — last-focused position is NOT preserved across open/dismiss cycles.

31. `/agent` → `Down` to row 2 (Boole) → `Escape` (no selection). → capture `escaped_at_row2`.
32. `/agent` again. → capture `reopen_focus`.

**Expect**:
- `escaped_at_row2` not contains `Subagents` — picker dismissed
- `reopen_focus` matches `^› 1\.` AND `^  2\.` — focus is on row 1 (current = Main), NOT row 2 where it was last
- `reopen_focus` contains `Main [default] (current)` on row 1

### Additional finding worth a separate predicate

Verified 2026-05-22: the picker enumerates ALL recent agent threads in this session, not just ones the user explicitly named via `/agent` or spawn-prompts. Threads created implicitly by tool calls (e.g. `spawn_agent` during multi-source synthesis) appear as `• Agent` rows (no codename label) with their thread id. So the picker can show MORE rows than the user expects.

Predicate to add to Scenario D (multi-row picker): record the exact number of rows after spawn, expect at least one explicit-name row AND possibly extra `• Agent` rows from prior tool-spawned subagents.

**Cleanup**: tear down the helper agent. If there's a "delete agent" tool exposed in the picker (need to check — may require an explicit `kill_agent` tool call), use it. Otherwise the helper remains in this session — note in report and remove via session-level cleanup (`/clear` or session restart).

---

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

### Scenario C: rollout path matches the actual session

7. Extract the printed path from `debug_out` as `RPATH`.
8. Cross-check: `find ~/.ata/sessions -name "*.jsonl" -mmin -5 | xargs ls -t | head -1 > <newest_session>`.

**Expect**:
- `RPATH` equals `<newest_session>` — `/rollout` returns the same JSONL that `find -mmin` returns (proves `/rollout` is sourcing from the live session state, not a stale handle)

**Notes**:
- TR-015 (the original `/rollout` smoke) was failing against npm 0.7.0 — that is the expected Scenario A behavior, not a regression.
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

Verified 2026-05-22 (second pass): bare `/plan` only activates Plan mode. A second `/plan` does NOT turn it off — the footer still shows `Plan mode (shift+tab to cycle)`. The only way to turn off Plan mode is `Shift+Tab` (the binary toggle covered in Scenario C). Earlier-pass observations that suggested `/plan` was a toggle were measuring activation, not deactivation.

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

**Expect** (verified 2026-05-22 — only two states despite "to cycle" hint):
- `bt1` not contains `Plan mode (shift+tab to cycle)` — first Shift+Tab toggles OFF
- `bt2` contains `Plan mode (shift+tab to cycle)` — second toggles ON
- `bt3` not contains `Plan mode (shift+tab to cycle)` — third toggles OFF again
- No third or fourth mode ever appears — the "cycle" hint is misleading; it's a binary toggle

### Scenario D: Plan mode persists across /side trip

10. With Plan mode ON: `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`. Wait for `• 4` in the side context. → capture `in_side`.
11. `tmux send-keys -t <new> Escape`; sleep 1. → capture `back_main`.

**Expect** (verified 2026-05-22):
- `back_main` contains `Main [default]` AND `Plan mode (shift+tab to cycle)` — Plan mode survived the /side detour

**Cleanup**: toggle Plan mode off with Shift+Tab or another `/plan`.

---

## TR-044: /side — entry, context label, recursion block, command restriction, exit

`/side` opens an ephemeral fork of the current thread for a quick
follow-up. Commands inside /side are restricted to a small allowlist
(`/copy`, `/raw`, `/diff`, `/mention`, `/status`, `/ide` per source);
everything else (`/scheduling`, `/agent`, `/side`, etc.) is blocked
with an explicit error pointing the user back to main.

**Setup**: TR-003 setup. **Critical precondition**: the conversation
must have at least one completed user→agent turn since the most recent
`/clear` or session start. /side is unavailable on a fresh or freshly-
cleared session (see also TR-038 Scenario E2 for the negative case).

### Scenario A: bare /side opens an empty side conversation

1. Prime if needed: `tmux send-keys -t <new> "respond with just primed"`; sleep 1; `Enter`. Poll until `^• primed\b`.
2. `tmux send-keys -t <new> "/side"`; sleep 0.5; `Enter`; sleep 2.
3. → capture `side_open`.

**Expect** (verified 2026-05-22):
- `side_open` contains `Side from main thread · Esc to return` — context label in the footer
- `side_open` footer agent label is `Agent` (NOT `Main [default]`) — side is its own thread with a separate agent context
- `side_open` does NOT auto-submit any message (composer is empty / on a default placeholder)

### Scenario B: /side with inline args submits a question into the side

4. Exit side: `Escape`. Back in main.
5. `tmux send-keys -t <new> "/side what is 2+2?"`; sleep 0.5; `Enter`; sleep 2. Wait for `• 4` in the chat.
6. → capture `side_with_arg`.

**Expect**:
- `side_with_arg` contains `› what is 2+2?` — the inline question landed as user message
- `side_with_arg` contains `Side from main thread · Esc to return` — context label
- `side_with_arg` contains `• 4` — agent answered in the side scope

### Scenario C: /side blocked inside a side conversation (recursion guard)

7. Still inside the side from Scenario B: `tmux send-keys -t <new> "/side"`; sleep 0.5; `Enter`; sleep 1.
8. → capture `recursion`.

**Expect** (verified 2026-05-22):
- `recursion` contains `'/side' is unavailable in side conversations. Press Esc to return to the main thread first.` — exact error (lowercase `/side`, mentions Esc as the way back)

### Scenario D: most slash commands blocked inside /side (command restriction)

9. Still inside side: `tmux send-keys -t <new> "/scheduling"`; sleep 0.5; `Enter`; sleep 1.
10. → capture `block_scheduling`.

**Expect** (verified 2026-05-22):
- `block_scheduling` contains `'/scheduling' is unavailable in side conversations. Press Esc to return to the main thread first.` — same error pattern, name interpolated
- The error format is `'/X' is unavailable in side conversations. Press Esc to return to the main thread first.` — covered by source as a single blocked-command template

### Scenario E: allowed commands work in side (positive control)

11. Still inside side: `tmux send-keys -t <new> "/copy"`; sleep 0.5; `Enter`; sleep 1.
12. → capture `allowed_copy`.

**Expect**:
- `allowed_copy` contains `Copied last message to clipboard` — /copy is in the allowlist (see TR-038 Scenario E for full validation)

### Scenario F: Esc returns to main thread

13. `tmux send-keys -t <new> Escape`; sleep 1.
14. → capture `back_main`.

**Expect** (verified 2026-05-22):
- `back_main` contains `Main [default]` — agent label is back to Main
- `back_main` not contains `Side from main thread · Esc to return` — side label gone
- The main thread's chat history is intact (still has the AAA/CCC/etc messages from before the /side detour — /side does NOT pollute main thread's transcript)

**Cleanup**: none.

---

## TR-045: /fork — clones the session, parent stays resumable, child starts fresh-but-aware

`/fork` creates a new chat session branched from the current one. The
new session has:
- A fresh visible composer (default placeholder, no scrollback)
- Full semantic memory of the parent's conversation (verified via
  follow-up question)
- A printed "resume" hint with the parent's session id so the user can
  go back

This is distinct from `/clear` (wipes history but keeps the same
session) and `/compact` (summarizes history in place).

**Setup**: TR-003 setup with at least 1-2 turns of meaningful history
in the main thread (otherwise "what did we discuss" has nothing to
remember).

### Scenario A: /fork prints token-usage summary, resume hint, and resets composer

1. With existing chat history present: `tmux send-keys -t <new> "/fork"`; sleep 0.5; `Enter`; sleep 2.
2. → capture `forked`.

**Expect** (verified 2026-05-22):
- `forked` contains `Token usage:` — token-usage summary line
- `forked` contains `To continue this session, run ata resume` — resume hint
- `forked` matches `ata resume [0-9a-f-]{36}` — parent session id is a UUID
- `forked` footer is `gpt-5.5 medium · ~` — agent label dropped (no `Main [default]` shown immediately after fork)
- `forked` composer shows a default placeholder like `Implement {feature}` — fresh visible state

### Scenario B: forked session retains semantic memory of parent

3. In the forked session: `tmux send-keys -t <new> "what did we discuss earlier?"`; sleep 1; `Enter`. Poll up to 90s until agent answers.
4. → capture `recall`.

**Expect** (verified 2026-05-22):
- `recall` contains references to actual prior history items — the fork inherited the parent's conversation context, NOT a clean slate
- For our test session, `recall` contained references to `AAA`, `BBB`, `CCC`, `espresso`, `tr041-helper` etc. — exact terms vary by parent history, so the predicate is "matches at least 2 distinct topic terms from the parent's history" (test must record those terms during setup)

### Scenario C: parent session is resumable via printed id

5. Quit ata or open a second terminal. Take the UUID from Scenario A's resume hint as `PARENT_ID`.
6. `ata resume $PARENT_ID > <resume_out>` OR launch ata and run `/resume <PARENT_ID>` from the picker.

**Expect** (to verify on first full run):
- ata launches into the parent's history — visible scrollback shows the messages that existed before the fork (the AAA/BBB/CCC turns).
- The session is the SAME session id as `PARENT_ID` — the fork did not invalidate the parent.

---

## TR-046: /resume — picker UI, inline-name lookup, fuzzy vs exact

`/resume` has two paths: a rich picker (bare invocation) and an inline
name/id lookup (`/resume <token>`). Bare opens a multi-column fuzzy
finder with sort/filter toggles; inline does an EXACT match against
session id or saved chat name and errors otherwise.

**Setup**: TR-003 setup. At least 2-3 prior sessions must exist in
`~/.ata/sessions/` (most users have many after a day of use).

### Scenario A: bare /resume opens the picker

1. `tmux send-keys -t <new> "/resume"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `picker`.

**Expect** (verified 2026-05-22):
- `picker` contains `Resume a previous session` — heading
- `picker` contains `Type to search` — search input placeholder
- `picker` matches `Filter: \[Cwd\] All` — filter field present (`[Cwd]` is the focused token; `All` is the value)
- `picker` matches `Sort: \[Updated\] Created` — sort field present (two options: Updated / Created)
- `picker` contains a `❯` focus marker on the top row
- `picker` matches `[0-9]+ / [0-9]+` — pagination indicator (e.g. `1 / 46`)
- `picker` contains all of: `enter resume`, `esc exit`, `tab focus sort/filter`, `ctrl+o comfortable view`, `ctrl+t transcript`, `ctrl+e expand`, `↑/↓ browse` — footer hint lines

### Scenario B: rows show time-ago + first user message

3. Inspect `picker` body rows.

**Expect**:
- `picker` matches `[0-9]+ ago\s+\S` on multiple lines — format is `<N> <unit> ago     <first user message excerpt>`
- The top row's "time ago" is the smallest (most recent session at the top, matching default `Sort: Updated`)

### Scenario C: inline /resume needs exact match (NOT fuzzy)

4. Escape out of the picker.
5. `tmux send-keys -t <new> "/resume primed"`; sleep 0.5; `Enter`; sleep 1.  (`primed` is a substring of a prior message but not a session name/id)
6. → capture `no_match`.

**Expect** (verified 2026-05-22):
- `no_match` contains `No saved chat found matching 'primed'.` — exact error string (note single-quoted token)
- The picker did NOT open — inline-args mode skips the picker entirely

### Scenario D: inline /resume by session id loads that session

Verified 2026-05-22:
- Resuming the CURRENT session (the uuid currently in use) prints `Already viewing /Users/tim/.ata/sessions/<YYYY/MM/DD>/rollout-<timestamp>-<uuid>.jsonl.` — explicit no-op feedback.
- Resuming a DIFFERENT recent session swaps the view to that session's chat history; the prior session's messages become visible.

7. Get the UUID of the currently-active session for the no-op test, and one other recent session's UUID for the swap test.
8. `tmux send-keys -t <new> "/resume <CURRENT_UUID>"`; sleep 0.5; `Enter`; sleep 1.5. → capture `same_session`.
9. `tmux send-keys -t <new> "/resume <OTHER_UUID>"`; sleep 0.5; `Enter`; sleep 3. → capture `swapped`.

**Expect**:
- `same_session` matches `Already viewing /.+/rollout-.+\.jsonl\.` — no-op confirmation
- `same_session` not contains `No saved chat found matching` — lookup succeeded (the uuid is valid)
- `swapped` does NOT contain `Already viewing` — different session, actually loaded
- `swapped` shows the OTHER session's chat history (verify by content uniqueness — e.g. messages that don't exist in the current session)
- `swapped` may also contain a `Token usage:` summary line for the prior session

### Scenario E: /resume is BLOCKED during an in-flight task (verified)

Verified 2026-05-22: unlike `/copy` (allowed) and `/agent` (opens immediate overlay), `/resume` is hard-blocked while a tool call or turn is in progress.

10. Trigger a long-running turn (e.g. ask for a 5000-word essay) so `esc to interrupt` is visible.
11. Within that window: `tmux send-keys -t <new> "/resume"`; sleep 0.5; `Enter`; sleep 1.5.
12. → capture `blocked`.
13. `tmux send-keys -t <new> Escape`; sleep 2 (cancel essay so future tests work).

**Expect** (verified 2026-05-22 — exact error string):
- `blocked` contains `'/resume' is disabled while a task is in progress.` — exact block error
- `blocked` not contains `Resume a previous session` — picker did NOT open
- The in-flight turn keeps running until the explicit Escape cancels it

---

## TR-047: /compact — summarizes visible history but preserves semantic memory

`/compact` replaces the rendered chat history with a one-line
confirmation, but the agent still has full semantic memory of what
happened before — proven by asking "what did we discuss earlier" and
getting a list of prior topics.

This is distinct from `/clear` (wipes both visible history AND agent
memory of that history within the current view) and `/fork` (creates
a new session that retains parent context).

**Setup**: TR-003 setup with at least 2-3 meaningful turns of chat
history so the compaction has something to summarize.

### Scenario A: /compact prints a one-line confirmation, resets composer

1. `tmux send-keys -t <new> "/compact"`; sleep 0.5; `Enter`; sleep 3.
2. → capture `compacted`.

**Expect** (verified 2026-05-22):
- `compacted` contains `Context compacted` — confirmation line
- `compacted` composer shows a default placeholder (e.g. `Implement {feature}`) — visible chat is reset
- `compacted` does NOT contain the prior user messages (`respond with just AAA`, `respond with just BBB`, etc.) — visible scrollback is wiped
- `compacted` footer: `gpt-5.5 medium · ~` (agent label may be dropped, matching `/fork` Scenario A)

### Scenario B: agent retains semantic memory after compaction

3. `tmux send-keys -t <new> "what did we discuss earlier?"`; sleep 1; `Enter`. Poll up to 90s.
4. → capture `recall_after_compact`.

**Expect** (verified 2026-05-22):
- `recall_after_compact` contains references to actual prior topics (e.g. `AAA`, `espresso`, `python REPL`, `tr041-helper`) — the agent still knows what happened, just in summarized form, not as a verbatim transcript
- The recall content is broadly correct (the agent may hallucinate minor details — record any such cases as observations, not failures)

### Scenario C: contrast with /clear

5. Optional follow-up: `tmux send-keys -t <new> "/clear"`; sleep 0.5; `Enter`; sleep 2.
6. `tmux send-keys -t <new> "what did we discuss earlier?"`; sleep 1; `Enter`. Poll.
7. → capture `recall_after_clear`.

**Expect**:
- `recall_after_clear` contains a "no prior discussion" / "we just started" / equivalent phrasing — `/clear` wipes both visible AND semantic memory of the cleared turns (unlike `/compact` which retains semantic memory)

---

## TR-048: /goal is unrecognized on the public release (feature-gated, separately from /plan)

Source investigation indicated `/goal` is feature-gated on
`Feature::Goals`. Empirically, on ata 0.7.0 the public release ships
WITHOUT `/goal` registered, while `/plan` (allegedly sharing the same
gate per source) IS registered. So `/goal` and `/plan` are gated
separately, despite the source's comment.

**Setup**: TR-003 setup.

### Scenario A: /goal on public release = unrecognized

1. `tmux send-keys -t <new> "/goal"`; sleep 0.5; `Enter`; sleep 1.
2. → capture `out`.

**Expect** (verified 2026-05-22 on ata 0.7.0 from npm):
- `out` contains `Unrecognized command '/goal'.`
- `out` contains `Type "/" for a list of supported commands.`

### Scenario B: /plan on the same build IS recognized

3. `tmux send-keys -t <new> "/plan"`; sleep 0.5; `Enter`; sleep 1.
4. → capture `plan_works`.

**Expect**:
- `plan_works` contains `Plan mode (shift+tab to cycle)` — /plan IS recognized on the same build that rejected /goal
- Proves the gates are independent, not shared.

### Scenario C: /goal on a build with the Goals feature flag ON (TODO)

When someone has a build with `Feature::Goals` actually enabled (debug
build with the feature flag forced, or a future release that exposes
it), expand this TR to cover the 4 documented states:
- `/goal <text>` — set goal
- `/goal clear` — clear goal
- `/goal pause` — pause goal status
- `/goal resume` — resume goal status

Predicates for each state to be captured during that first validation run.

---

## TR-049: Reading view — vim-style scroll within a section

Reader supports vim-style scrolling within a section. The `?` help
overlay documents:
- `j / ↓` — scroll down one line
- `k / ↑` — scroll up one line
- `Ctrl+d / Ctrl+u` — half-page down / up
- `Ctrl+f / Ctrl+b` — full-page down / up
- `gg` — jump to top of section
- `G` — jump to end of section
- `w / b` — word forward / backward
- `h / l` — cursor left / right

Verified 2026-05-22: `j` scrolls 1 line, `k` scrolls 1 line up, `gg`
jumps to top. **`G` documented as "Jump to end of section" but observed
only scrolling 3 lines — possible bug or limited by visible scroll
position. Worth a finding.**

**Setup**: TR-003 + reading-view precondition + open a reader with at
least 3 sections of moderate length (TR-032's coffee 3-slide setup is
sufficient).

### Scenario A: j / k scroll one line

1. Wait for `Sections (n/p` in pane. Note the cursor at the top of section 1.
2. `tmux send-keys -t <new> "j"`; sleep 0.3. → capture `j1`.
3. `tmux send-keys -t <new> "k"`; sleep 0.3. → capture `k1`.

**Expect** (verified 2026-05-22):
- `j1` text region differs from initial by exactly one line shift (scroll position moved down 1)
- `k1` text region matches the initial position again

### Scenario B: gg jumps to top of section

4. `tmux send-keys -t <new> "j j j j j"`; sleep 0.5.  (scroll down 5 lines)
5. `tmux send-keys -t <new> "gg"`; sleep 0.3. → capture `top`.

**Expect**:
- `top` shows the section heading (`Slide 1: What Coffee Is`) at the top of the content frame — section scrolled back to top

### Scenario C: G is documented as "jump to end" but observed as 3-line scroll

6. From the top: `tmux send-keys -t <new> "G"`; sleep 0.3. → capture `g1`.
7. `tmux send-keys -t <new> "G"`; sleep 0.3. → capture `g2`.
8. `tmux send-keys -t <new> "G"`; sleep 0.3. → capture `g3`.

**Expect** (verified 2026-05-22 — discrepancy with documented behavior):
- `g1` scrolled DOWN, but NOT all the way to the end — only a few lines (observed ~3 lines per press)
- `g2` and `g3` either continue scrolling or stop if at end
- The documented behavior ("Jump to end of section") is NOT what 0.7.0 does
- Either the implementation is bounded by visible scroll position OR the help text is wrong. File as a finding.

### Scenario D: half-page scroll with Ctrl+d / Ctrl+u

9. `tmux send-keys -t <new> "gg"`; sleep 0.3. (back to top)
10. `tmux send-keys -t <new> C-d`; sleep 0.3. → capture `cd`.
11. `tmux send-keys -t <new> C-u`; sleep 0.3. → capture `cu`.

**Expect** (to verify exact line-count delta on first run):
- `cd` advanced by roughly half a page (~10-15 lines depending on terminal height)
- `cu` returned to the original position

### Scenario E: implicit section-as-read marker (✓) when scrolling

Verified 2026-05-22: simply scrolling past content within a section can result in the section being marked `✓` in the inline TOC, even without using `n`/`p`. Same likely applies to neighboring sections if the cursor crosses a section boundary via Ctrl+f or G.

12. Initial state: only section 1 has `✓` in the inline TOC.
13. After heavy scrolling within slide 1 (j j j ... or C-d a few times) → capture `tocstate`.

**Expect**:
- `tocstate` shows `✓` next to slide 1 AND possibly slide 2 even though no `n`/`p` navigation happened — proves the read-tracker is internal-scroll-based, not just section-jump-based.

---

## TR-050: Reading view — / search within document, n/N navigation, wrap

Reader's `/` opens a search input. `Enter` commits the query; matches
are highlighted in pink with underline. `n` / `N` navigate forward /
backward through all matches across all sections. `Esc` clears search.

**Setup**: TR-003 + reading-view precondition + open a 4-slide coffee
reader (TR-049 setup) so there are multiple search hits across sections.

### Scenario A: / opens search input

1. With reader open: `tmux send-keys -t <new> "/"`; sleep 0.3.
2. → capture `search_open`.

**Expect** (verified 2026-05-22):
- `search_open` contains `Enter: search | Esc: cancel` — search-mode footer
- `search_open` contains `/` on its own row — search input cursor visible

### Scenario B: typing query + Enter highlights and counts matches

3. `tmux send-keys -t <new> "brewing"`; sleep 0.3.
4. `tmux send-keys -t <new> Enter`; sleep 1.
5. → capture `search_committed`.

**Expect** (verified 2026-05-22 — match count varies by content):
- `search_committed` contains `/brewing` — query echoed in the search bar
- `search_committed` matches `\[[0-9]+/[0-9]+\]` — match counter present (e.g. `[1/3]` or `[1/4]`)
- `search_committed` highlights the word `brewing` (pink underline ANSI — capture with `tmux capture-pane -p -e` to see the escape codes)
- The first match is auto-focused (counter starts at `[1/N]`, not `[0/N]`)

### Scenario C: n advances next match across section boundaries

6. `tmux send-keys -t <new> "n"`; sleep 0.5. → capture `next1`.
7. `tmux send-keys -t <new> "n"`; sleep 0.5. → capture `next2`.
8. Continue until you've seen all matches.

**Expect** (verified 2026-05-22):
- Counter increments (`[2/N]`, `[3/N]`, ...) — each `n` advances to the next match
- The view scrolls to a different section when needed — matches in slide 2, 3, etc. all reachable by repeated `n`
- After reaching the last match, the NEXT `n` either wraps to `[1/N]` OR shows a "no more matches" state (to verify which behavior on first run)

### Scenario D: N backwards with wrap-around

9. `tmux send-keys -t <new> "N"`; sleep 0.5. → capture `prev1`.
10. `tmux send-keys -t <new> "N"`; sleep 0.5. → capture `prev2`.

**Expect** (verified 2026-05-22):
- `N` from match 1 wraps to the last match — counter goes `[1/N]` → `[N/N]` (e.g. `[1/4]` → `[4/4]`)
- Subsequent `N` continues backward — `[N/N]` → `[N-1/N]` → ...

### Scenario E: Esc clears search

11. `tmux send-keys -t <new> Escape`; sleep 0.3. → capture `cleared`.

**Expect**:
- `cleared` not contains `/brewing` — search bar removed
- `cleared` not contains `[N/N]` — match counter removed
- `cleared` footer reverts to the standard `↑↓/jk: scroll | n/p: section | ...` line (no `Enter: search | Esc: cancel`)

---

## TR-051: Reading view — folding (f, [, ], zM, zR) on a document with foldable regions

`f` toggles fold at cursor; `[ / ]` jump prev/next fold; `zM / zR`
collapse/expand all (vim-style). Foldable regions are produced
inside the reader by content with `<!-- CODEX_SECTION_META -->`
metadata or by Tab-ask answers (TR-037's `foldable: true` markers).

**Setup**: TR-003 + reading-view precondition + open a reader. **Add at
least one foldable region** by either:
- Doing a Tab-ask interaction (TR-002 / TR-037 style — produces a
  foldable Q&A subsection)
- Letting the agent create a reader with explicit foldable metadata
  (e.g. a long literature-review section)

### Scenario A: f on a section without foldable regions = no-op

1. Open a plain coffee-slides reader (no fold markers).
2. `tmux send-keys -t <new> "f"`; sleep 0.3. → capture `no_fold`.

**Expect** (verified 2026-05-22):
- `no_fold` is identical (modulo footer timer) to the pre-`f` capture — nothing happens, no error printed

### Scenario B: f toggles a fold at cursor on a document with foldable regions

3. With a foldable region in scope (use TR-002 / TR-037 setup to produce one): position cursor on the fold by pressing `j` / `k` until the fold marker `[+]` or `[-]` is on the cursor row.
4. `tmux send-keys -t <new> "f"`; sleep 0.3. → capture `toggled`.

**Expect** (to verify exact markers on first foldable-region run):
- `toggled` flips the fold marker from `[+]` to `[-]` (or vice versa) — content under the fold becomes hidden or visible accordingly

### Scenario C: [ and ] jump to prev / next fold

5. With multiple folds in the document: `tmux send-keys -t <new> "]"`; sleep 0.3. → capture `jump_next`.
6. `tmux send-keys -t <new> "["`; sleep 0.3. → capture `jump_prev`.

**Expect**:
- `jump_next` cursor lands on the next fold marker (verify by position OR by content nearby)
- `jump_prev` cursor lands on the previous fold (or no-op if at first fold)

### Scenario D: zM and zR collapse / expand all

7. `tmux send-keys -t <new> "zM"`; sleep 0.3. → capture `all_collapsed`.
8. `tmux send-keys -t <new> "zR"`; sleep 0.3. → capture `all_expanded`.

**Expect**:
- `all_collapsed` shows every fold marker as `[+]` — all folds collapsed
- `all_expanded` shows every fold marker as `[-]` — all folds expanded

**Status**: TODO — needs first run on a document that actually has foldable regions to lock in marker syntax. Plain text slides do NOT trigger folds (verified empty case in Scenario A).

---

## TR-052: Reading view — TTS narration (r, R, s, +/-)

`r` starts TTS narration of the current section. While narrating, the
footer expands with audio controls (`s: pause`, `+/-: speed`, `f: fold`).
On a system without an ElevenLabs API key configured, `r` errors out
with `TTS error: Invalid API key`. With a valid key, narration plays
audio and (probably) highlights words as they're spoken.

**Setup**: TR-003 + reading-view precondition + open any reader.

### Scenario A: r WITHOUT a TTS API key configured → graceful error

1. Confirm ElevenLabs is NOT configured (or temporarily blank out `~/.ata/config.toml`'s `[voice_mode.elevenlabs]` section).
2. With reader open: `tmux send-keys -t <new> "r"`; sleep 1.
3. → capture `r_no_key`.

**Expect** (verified 2026-05-22):
- `r_no_key` footer EXPANDS to show audio controls: `r: read | s: pause | +/-: speed | t: toc | f: fold | v: select | Tab: ask | q: close` — proves the audio-active state was entered before failing
- `r_no_key` momentarily shows `▶️T  Speaking...` (the active TTS row) — narration started
- `r_no_key` then shows `TTS error: Invalid API key` — exact error string
- The error does NOT crash ata — reader stays open, other keys still work

### Scenario B: pause / resume / speed controls (TODO — needs valid TTS key)

When someone runs this against a build with a valid ElevenLabs key:
- `r` starts narration; `▶️T Speaking...` row stays visible
- `s` pauses (verify by audio stop AND status row changes to `paused`)
- `r` resumes
- `+` increases speed by some increment; `-` decreases (capture displayed speed value)
- The narration likely highlights words as they're spoken (per source notes on word-offset tracking)

### Scenario C: r key not in /? help even though it's in the footer (doc gap)

Verified 2026-05-22: the `?` help overlay covers Navigation, Selection,
Questions, Search, Folds, and Other — but TTS / audio keys (`r`, `s`,
`+/-`, `R`) are NOT documented in any section. The bottom footer DOES
show `r: read`, so the binding is discoverable but the help is incomplete.

3. Open the help overlay via `?` and scroll through.

**Expect**:
- Help overlay text contains all of: `j / ↓`, `gg`, `G`, `v`, `Tab`, `/`, `f`, `gx`, `q`
- Help overlay does NOT contain `r` standalone (the only `r` mentions are inside other strings like `Cursor` or `Folds`)
- Help overlay does NOT contain `s: pause`, `+/-: speed`, `R` — the TTS control surface is missing from documentation

---

## TR-053: Reading view — visual selection (v / V / hjkl / Enter / Tab / Esc)

`v` starts character-level selection; `V` line-level. Once in selection
mode, `hjkl` extend the selection (vim style), `Enter` triggers
"explain selected text" (a new agent path), `Tab` triggers "ask about
selected text" (the TR-002 Tab-ask path, scoped to the selection),
and `Esc` cancels the selection.

**Setup**: TR-003 + reading-view precondition + open any reader.

### Scenario A: v enters character selection mode; footer shows selection keys

1. `tmux send-keys -t <new> "v"`; sleep 0.3.
2. → capture `vselect_on`.

**Expect** (verified 2026-05-22):
- `vselect_on` footer is `hjkl: select | Enter: explain | Tab: ask about | Esc: cancel` — completely replaces the standard footer
- A cursor / selection start marker is visible somewhere in the content area (exact glyph to verify on first run — likely reverse-video block or `▎`)

### Scenario B: hjkl extends the selection

3. `tmux send-keys -t <new> "l l l l l"`; sleep 0.5.  (extend selection right by 5 chars)
4. → capture `extended`.

**Expect** (to verify exact highlight style on first run):
- `extended` shows ~5 characters between the start position and current cursor highlighted in a different visual style (likely reverse-video or different background color)
- The footer still shows `hjkl: select | Enter: explain | Tab: ask about | Esc: cancel`

### Scenario C: V enters line-level selection mode

5. `tmux send-keys -t <new> Escape`; sleep 0.3.  (cancel previous selection)
6. `tmux send-keys -t <new> "V"`; sleep 0.3.
7. → capture `Vselect`.

**Expect** (to verify on first run):
- `Vselect` selects the WHOLE current line (no character-level cursor — the whole row is highlighted)
- Same footer as Scenario A
- `j` extends selection down by full lines (NOT characters)

### Scenario D: Enter (in selection mode) triggers "explain" — distinct from Tab-ask

8. From Scenario B with an active selection: `tmux send-keys -t <new> Enter`; sleep 1.
9. Poll up to 90s until the agent replies with content related to the selected text.
10. → capture `explain_resp`.
11. Inspect JSONL for the tool call.

**Expect** (to verify on first run — predict but verify):
- `explain_resp` contains agent commentary on the selected text — proves Enter sent the selection as context
- The JSONL tool call is likely `patch_document_section` or `append_to_section` with `foldable: true` (same path as Tab-ask in TR-037) OR a distinct path that calls a different tool — record on first run

### Scenario E: Esc cancels selection cleanly

12. From any selection state: `tmux send-keys -t <new> Escape`; sleep 0.3.
13. → capture `escaped`.

**Expect**:
- `escaped` footer reverts to the standard `↑↓/jk: scroll | n/p: section | ...` line
- No selection highlight visible
- Cursor returns to a standard reader cursor position

---

## TR-054: Reading view — ? help overlay and its documented surface

The `?` key opens a full-screen help overlay enumerating all
keybindings. Pressing `?` again (or `Esc`) closes it. The help text is
the source of truth for what users are TOLD is supported — even if
some bindings (`r`, `s`, `+/-`, `R`) work but aren't documented (see
TR-052 C).

**Setup**: TR-003 + reading-view precondition + open any reader.

### Scenario A: ? opens the help overlay; structure verified

1. `tmux send-keys -t <new> "?"`; sleep 0.5.
2. → capture `help`.

**Expect** (verified 2026-05-22):
- `help` contains `Reading View Help` — top heading
- `help` contains `Getting around` AND `Use ↑↓ or j/k to scroll within a section` AND `Press n/p to go to the next or previous section` — Getting Around section
- `help` contains `Ask about anything` AND `Select text with v, then press Enter to explain it` AND `Or press Tab to type your own question` — Ask section
- `help` contains `Search` AND `Press / to search within the document` — Search section
- `help` contains `All Keyboard Shortcuts` — main shortcut block header

### Scenario B: shortcuts table — exact rows in the Navigation block

3. Scroll through with `j` until the full Navigation section is visible. → capture `nav_block`.

**Expect**:
- `nav_block` contains `j / ↓   Scroll down one line`
- `nav_block` contains `k / ↑   Scroll up one line`
- `nav_block` contains `Ctrl+d / Ctrl+u   Half-page down / up`
- `nav_block` contains `Ctrl+f / Ctrl+b   Full-page down / up`
- `nav_block` contains `gg   Jump to top of section`
- `nav_block` contains `G   Jump to end of section`  (note: actual behavior in 0.7.0 diverges — see TR-049 C)
- `nav_block` contains `n   Next section`
- `nav_block` contains `p   Previous section`

### Scenario C: shortcuts table — Text Selection, Questions, Search, Folds, Other

4. Continue scrolling. → capture `rest_blocks`.

**Expect**:
- `rest_blocks` contains `Text Selection` with rows: `v   Start character selection`, `V   Start line selection`, `Enter   Explain selected text`, `Tab   Ask about selected text`, `Esc   Cancel selection`
- `rest_blocks` contains `Questions` with: `Tab   Open question composer`, `Enter   Send question`, `Esc   Back to reading`
- `rest_blocks` contains `Search` with: `/   Start search`, `n / N   Next / previous match`, `Esc   Clear search`
- `rest_blocks` contains `Folds` with: `f   Toggle fold at cursor`, `[ / ]   Jump to prev / next fold`, `zM / zR   Collapse / expand all`
- `rest_blocks` contains `Other` with: `w / b   Word forward / backward`, `h / l   Cursor left / right`, `gx   Open link at cursor`, `t   Table of contents`, `?   Toggle this help`, `q   Close reading view`

### Scenario D: ? closes the overlay (toggle behavior)

5. `tmux send-keys -t <new> "?"`; sleep 0.3. → capture `help_closed`.

**Expect**:
- `help_closed` not contains `Reading View Help`
- `help_closed` contains the standard reader footer `↑↓/jk: scroll | ...`

### Scenario E: documentation gaps to flag (verified missing entries)

The help DOES NOT document:
- `r` (start narration) — IS in the bottom footer but NOT in help
- `R` (auto-narrate / TTS karaoke per source notes)
- `s` (pause TTS)
- `+ / -` (TTS speed)

When the help is updated to include these, this scenario's negative
predicates flip and become positive.

**Expect** (the documentation gap, verified 2026-05-22):
- `help` does NOT contain `r   Start narration` OR similar
- `help` does NOT contain `s   Pause narration`
- `help` does NOT contain `+ / -   Adjust narration speed`
- The full footer (which DOES list `r: read`) is in `help_closed` not in `help` — the bottom footer documents more bindings than the help overlay does

---

## TR-055: `ata workspace` CLI — read-only inspection surface

The CLI's read-only commands (`list`, `read`, `validate`, `recipe`,
`mirror-path`, `check-host`, `audit-query`, `export-spec`,
`search-commands`) return structured data (JSON or text) without
mutating state. These are the commands that should always be safe to
run from CI or scripts.

**Setup**: ata 0.7.0 installed and on PATH. Default `global` workspace
exists. No prior repos/runs/audit entries (clean state).

### Scenario A: `list` outputs a JSON array of workspaces

1. Shell command: `ata workspace list > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` is valid JSON (parseable with `jq .`).
- `<out>` matches `\[\s*\{` — array of objects.
- Each entry has keys `id`, `name`, `updatedAt`, `repoCount`.
- For a clean install: exactly one entry with `id: "global"`, `name: "global"`, `repoCount: 0`.
- `updatedAt` is a Unix timestamp (10-digit integer).

### Scenario B: `read` outputs the full workspace manifest

2. Shell command: `ata workspace read > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` is valid JSON.
- `<out>` contains top-level keys: `schemaVersion`, `id`, `name`, `createdAt`, `updatedAt`, `manifestVersion`, `repos`, `runs`, `papers`, `datasets`, `artifacts`, `links`, `snapshots`, `indexes`, `policies`, `knowledgeBase`, `labels`.
- `<out>` contains `"schemaVersion": 2`, `"manifestVersion": 1`.
- `<out>` contains `policies.defaultClone` with `depth`, `singleBranch`, `noTags`, `filter`, `submodules`, `lfs` fields.
- `<out>` contains `knowledgeBase.path` (e.g. `"knowledge-base"`).
- On a fresh workspace, all collection arrays (`repos`, `runs`, etc.) are empty.

### Scenario C: `validate` reports manifest/on-disk consistency

3. Shell command: `ata workspace validate > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` is valid JSON.
- `<out>` contains keys `workspaceId`, `ok`, `missingRepos`, `missingRuns`, `orphanRepoDirs`, `orphanRunDirs`.
- On a clean workspace: `"ok": true` and all four list fields are empty arrays.
- `workspaceId` equals the active workspace's id.

### Scenario D: `recipe list` returns 16 named recipes

4. Shell command: `ata workspace recipe list > <out>`.

**Expect** (verified 2026-05-22 — list will grow over time):
- `<out>` first line is `Available recipes:`.
- `<out>` contains each of: `export`, `export_spec`, `import`, `index_build`, `link_add`, `materialize`, `repo_pin`, `repo_remove`, `repo_unpin`, `repo_update`, `resource_add`, `run_delete`, `run_exec`, `run_gc`, `snapshot_create`, `snapshot_restore`.
- Format is two-space-indented names, one per line.

### Scenario E: `recipe <name>` prints a step-by-step bash recipe

5. Shell command: `ata workspace recipe repo_pin > <out>`.

**Expect** (verified 2026-05-22 — exact body lines for repo_pin):
- `<out>` starts with a `# ...` comment header (e.g. `# Pin repository to a specific commit`).
- `<out>` contains env-var assignments like `ALIAS="<repo_alias>"`, `SHA="<commit_sha>"`.
- `<out>` contains `ata workspace repo-pin --alias "$ALIAS" --sha "$SHA" --workspace "$WID"` — the CLI invocation for the operation.
- `<out>` contains `ata workspace audit --workspace "$WID"` — recipes always include their audit step.
- `<out>` contains `"op":"repo_pin"` — audit JSON includes the operation name.

### Scenario F: `mirror-path <url>` returns a hashed local cache path

6. Shell command: `ata workspace mirror-path https://github.com/openai/codex > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` matches `^/.+/\.ata/caches/repo-mirrors/[0-9a-f]{16}\s*$` — absolute path under `.ata/caches/repo-mirrors/` ending in a 16-char hex hash of the URL.
- The same URL always returns the same path (deterministic hash).
- A different URL returns a different hash.

### Scenario G: `check-host` echoes the URL on pass (silent allowlist)

7. Shell command: `ata workspace check-host https://github.com/openai/codex > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` matches `^https://github\.com/openai/codex\s*$` — URL echoed back, no other text.
- Exit code is 0 (the host passed the allowlist check).
- Test a disallowed host on first run to capture the error format (TODO: pin error string for hosts that aren't in `policies.repoHostsAllowlist`).

### Scenario H: `audit-query` returns JSON array (empty if no audits)

8. Shell command: `ata workspace audit-query --workspace global > <out>`.

**Expect** (verified 2026-05-22 on a clean workspace):
- `<out>` is exactly `[]` (empty JSON array).
- After running any mutating operation (e.g. `repo-clone`), `audit-query` returns at least one entry — verify by performing a tiny audit operation and re-querying.

### Scenario I: `export-spec` returns the workspace as a spec object

9. Shell command: `ata workspace export-spec > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` is valid JSON.
- `<out>` top-level keys: `schemaVersion`, `name`, `repos`, `labels`.
- `<out>` contains `"schemaVersion": 1` (spec version 1).
- For an empty workspace: `repos: []`, `labels: {}`.
- The output is SIMPLER than `read` — it's the spec view (intended for reproducibility), not the full manifest with runs/papers/audit-relevant fields.

### Scenario J: `search-commands <query>` ranks matches and prints best-match manual

10. Shell command: `ata workspace search-commands repo > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` starts with `Matches:` heading.
- `<out>` contains a numbered list `1.`, `2.`, `3.` ... with format `<command-name> — <one-line description>`.
- For query `repo`: matches include `repo-clone`, `repo-pin`, `repo-remove`.
- `<out>` then has a blank line + `Best match manual:` heading.
- `<out>` then has a clap-style help block: `Command: <name>`, description, `Usage:`, `Arguments:`, `Options:`.
- The best-match manual covers the top-ranked match (first in the list).

---

## TR-056: `ata workspace` CLI — lifecycle (init, select, delete)

Workspace creation and switching. Init returns just the new id;
selection is set by the same CLI as `/workspace use` (TR-040 E covers
the TUI side). Delete removes the directory tree and requires
`--force`.

**Setup**: clean state — only `global` workspace exists.

### Scenario A: `init` creates a workspace and prints only the new id

1. Shell command: `ata workspace init tr056-test > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` matches `^tr056-test-[0-9a-f]{8}\s*$` — id is `<name>-<8-char-hex>`.
- No additional text (no "created" / "initialized" prose).
- Exit code 0.
- `ata workspace list` now returns 2 entries.

### Scenario B: `select <selector>` activates a workspace

2. Shell command: `ata workspace select tr056-test > <out>`.
3. `ata workspace read > <out2>`.

**Expect** (to verify exact output of `select` on first run):
- `<out>` confirms the selection (record exact phrasing on first run — likely JSON with the selected workspace id).
- `<out2>` (manifest read of new active) has `"id": "tr056-test-..."`.

### Scenario C: `delete <id> --force` removes the workspace

4. Shell command: `ata workspace select global` (switch back to global before deleting).
5. Shell command: `ata workspace delete tr056-test-<suffix> --force > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` matches `^deleted: tr056-test-[0-9a-f]{8}\s*$` — confirmation line.
- `ata workspace list` returns only `global`.
- The workspace directory is gone from disk.

### Scenario D: `delete` without `--force` (negative — should refuse)

6. Create another test workspace: `ata workspace init tr056-noforce`.
7. Shell command: `ata workspace delete tr056-noforce-<suffix>` (no --force).

**Expect** (to verify exact error string on first run):
- The CLI refuses to delete without `--force` and prints an explanatory error.
- Exit code is non-zero.
- The workspace is NOT removed (list still shows it).

**Cleanup**: `ata workspace delete tr056-noforce-<suffix> --force`.

---

## TR-057: `ata workspace` CLI — repo management (clone, pin, unpin, update-state, remove)

Repo management commands operate on the active workspace's `repos[]`
collection. `repo-clone` actually clones from the network and registers
the repo; the others mutate manifest state. Each emits an audit entry.

**Setup**: clean `global` workspace. Network access available.

### Scenario A: `repo-clone <url> <alias>` clones, registers, and audits

1. Shell command: `ata workspace repo-clone https://github.com/openai/openai-cookbook tr057-cb > <out>`. (Choose a small public repo; openai-cookbook is reasonable but adjust if too large.)
2. `ata workspace read > <manifest>`.
3. `ata workspace audit-query --workspace global > <audit>`.

**Expect** (to verify exact `<out>` shape on first run):
- `<out>` confirms the clone (likely JSON with repo metadata).
- `<manifest>.repos[]` contains an entry with `alias: "tr057-cb"`, `url: "..."`, `headSha: "..."`.
- `<audit>` contains an entry with `op: "repo_clone"` and `targets[].alias: "tr057-cb"`.
- The cloned repo exists on disk under the workspace directory.

### Scenario B: `repo-pin --alias <a> --sha <sha>` pins to a commit

4. Pick a known SHA from the cloned repo (e.g. `git -C ~/.ata/workspaces/global/repos/tr057-cb log -1 --format=%H`).
5. Shell command: `ata workspace repo-pin --alias tr057-cb --sha <sha>`.
6. Verify in `ata workspace read`.

**Expect**:
- The repo entry now has `pinnedSha: "<sha>"`.
- `ata workspace audit-query` includes `op: "repo_pin"`.

### Scenario C: `repo-unpin --alias <a>` reverts to tracking mode

7. Shell command: `ata workspace repo-unpin --alias tr057-cb`.

**Expect**:
- The repo entry no longer has `pinnedSha` (or it's null).
- `audit-query` includes `op: "repo_unpin"`.

### Scenario D: `repo-update-state` updates head SHA / ref

8. Shell command: `ata workspace repo-update-state --alias tr057-cb --head-sha <new-sha> --head-ref refs/heads/main`.

**Expect**:
- Manifest's `repos[].headSha` updated.
- `audit-query` includes `op: "repo_update_state"`.

### Scenario E: `repo-remove --alias <a>` deletes everything

9. Shell command: `ata workspace repo-remove --alias tr057-cb`.

**Expect**:
- Repo directory gone from disk.
- Manifest's `repos[]` no longer includes `tr057-cb`.
- `audit-query` includes `op: "repo_remove"`.

**Cleanup**: ensure repo dir is gone (`rm -rf ~/.ata/workspaces/global/repos/tr057-cb` if leftover).

---

## TR-058: `ata workspace` CLI — runs lifecycle (run-setup, run-update-status, run-remove)

Runs are individual execution environments within a workspace, sourced
from a repo. Each run has its own working directory and status field.

**Setup**: TR-057 A run first (so there's a repo with alias to point runs at).

### Scenario A: `run-setup --name <n> --source-alias <a>` creates a run

1. Shell command: `ata workspace run-setup tr058-run --source-alias tr057-cb`.
2. `ata workspace read > <manifest>`.

**Expect**:
- `<manifest>.runs[]` contains an entry with `name: "tr058-run"`, `sourceAlias: "tr057-cb"`, `status: "<initial>"` (verify exact initial status — likely `pending` or `setup`).
- Run directory exists on disk.
- Default strategy is likely `worktree` (verify on first run; can override with `--strategy <worktree|copy|clone>`).
- `audit-query` includes `op: "run_setup"`.

### Scenario B: `run-update-status` updates a run's status

3. Shell command: `ata workspace run-update-status --id <run-id> --status running`.

**Expect**:
- Manifest's run entry's status field updated to `running`.
- `audit-query` includes `op: "run_update_status"` with old + new status.

### Scenario C: `run-remove` deletes a run (worktree cleanup)

4. Shell command: `ata workspace run-remove --id <run-id>`.

**Expect**:
- Run dir gone from disk.
- Manifest's `runs[]` no longer includes this run.
- `audit-query` includes `op: "run_remove"`.
- If strategy was `worktree`: the parent repo's worktree list (`git worktree list`) no longer includes the run.

**Cleanup**: TR-057 E to remove the repo afterward.

---

## TR-059: `ata workspace` CLI — manifest mutation (set-field, add-entry, remove-entry, add-paper)

Direct manifest mutation. Each emits an audit entry. `set-field` takes
a dotted-path key + JSON value; `add-entry` appends to a named
collection (`papers`, `datasets`, etc.); `add-paper` is a higher-level
convenience for the papers collection that also copies a markdown
file in.

**Setup**: clean workspace.

### Scenario A: `set-field` mutates a manifest field at a dotted path

1. Shell command: `ata workspace set-field --path policies.defaultClone.depth --value 5`.
2. `ata workspace read > <manifest>`.

**Expect**:
- `<manifest>.policies.defaultClone.depth` equals 5 (was 1 in the default).
- `audit-query` includes `op: "set_field"` with `path` and `value`.

### Scenario B: `add-entry` appends to a named collection

3. Shell command: `ata workspace add-entry --collection links --json '{"id":"tr059-link","url":"https://example.com","title":"Test"}'`.
4. `ata workspace read > <manifest>`.

**Expect**:
- `<manifest>.links[]` contains an entry with `id: "tr059-link"`, `url: "https://example.com"`, `title: "Test"`.
- `audit-query` includes `op: "add_entry"` with collection name `links`.

### Scenario C: `remove-entry` deletes by id

5. Shell command: `ata workspace remove-entry --collection links --id tr059-link`.

**Expect**:
- Manifest's `links[]` no longer includes `tr059-link`.
- `audit-query` includes `op: "remove_entry"`.

### Scenario D: `add-paper <md-path> --alias <a>` copies paper + registers

6. Create a tiny markdown file: `echo "# Test paper\nbody" > /tmp/tr059-paper.md`.
7. Shell command: `ata workspace add-paper /tmp/tr059-paper.md --alias tr059-paper --title "Test paper"`.

**Expect**:
- The markdown file is copied into the workspace's papers dir.
- `<manifest>.papers[]` contains an entry with `alias: "tr059-paper"`, `title: "Test paper"`.
- `audit-query` includes `op: "add_paper"`.

**Cleanup**:
- `ata workspace remove-entry --collection papers --id <paper-entry-id>`.
- `ata workspace set-field --path policies.defaultClone.depth --value 1` (restore default).

---

## TR-060: `ata workspace` CLI — spec round-trip (export-spec, diff-spec, materialize)

Spec files capture the minimal workspace definition (name + repos +
labels) for reproducibility. Round-trip: export the current workspace
as a spec, diff it against another spec, materialize from a spec into
a new workspace.

**Setup**: TR-057 A first (so there's a repo to include in the spec).

### Scenario A: `export-spec` produces a spec JSON

1. Shell command: `ata workspace export-spec > /tmp/tr060-spec.json`.

**Expect** (verified 2026-05-22 for empty workspace; populated case TODO):
- File is valid JSON.
- Keys: `schemaVersion`, `name`, `repos`, `labels`.
- `schemaVersion: 1`.
- With a repo present: `repos[]` includes the cloned repo with url + alias + pinnedSha.

### Scenario B: `diff-spec <spec-file>` shows what `materialize` would do

2. Modify the spec slightly (e.g. add a new repo entry to `/tmp/tr060-spec.json`).
3. Shell command: `ata workspace diff-spec /tmp/tr060-spec.json > <out>`.

**Expect**:
- `<out>` lists planned operations: `repos to add`, `repos to pin`, `repos to skip` (or equivalent — TODO verify exact section headers on first run).
- No state is mutated (it's a dry-run).

### Scenario C: `materialize <spec> --dry-run` shows the plan without applying

4. Shell command: `ata workspace materialize /tmp/tr060-spec.json --dry-run > <out>`.

**Expect**:
- Output is similar to `diff-spec` but in materialize's planning format (TODO verify on first run).

### Scenario D: `materialize <spec>` (no --dry-run) actually applies the plan

5. Create a fresh workspace: `ata workspace init tr060-target`.
6. Shell command: `ata workspace --enable=workspaces materialize /tmp/tr060-spec.json --workspace tr060-target-<suffix>`.

**Expect**:
- The target workspace now has the repos from the spec.
- `ata workspace read --workspace tr060-target-<suffix>` shows them.
- `audit-query --workspace tr060-target-<suffix>` includes one audit entry per repo-clone.

**Cleanup**: `ata workspace delete tr060-target-<suffix> --force` and `rm /tmp/tr060-spec.json`.

---

## TR-061: `ata zotero` CLI — status, search-commands, and subcommand inventory

The CLI's `status` reports the effective Zotero mode (`local` vs
`cloud`), endpoint, auth, and scope. `search-commands <query>` ranks
matches and prints a clap-style manual for the top hit (same pattern
as `ata workspace search-commands`).

**Setup**: ata 0.7.0 installed. Zotero desktop may or may not be
running — `status` works either way.

### Scenario A: `status` reports effective mode and config

1. Shell command: `ata zotero status > <out>`.

**Expect** (verified 2026-05-22 with no API key):
- `<out>` contains `Effective mode: local` — local mode is the default fallback
- `<out>` contains `Base URL: http://localhost:23119/api` — Zotero desktop's local API endpoint
- `<out>` contains `API key configured: no` — no key in this shell
- `<out>` contains `Library scope: all accessible libraries`
- `<out>` contains `Default write scope: unconfigured`
- `<out>` contains `Note: The effective Zotero mode is local because no Zotero API key is configured for this shell.` — explanation line

### Scenario A2: `status` with API key configured (TODO)

When someone has a Zotero API key in env or config:
- `<out>` should contain `Effective mode: cloud` (or whatever the explicit configured mode is)
- `<out>` should contain `API key configured: yes`
- The `Base URL` should match the cloud endpoint (api.zotero.org)

### Scenario B: `--help` lists all 17 first-level subcommands

2. Shell command: `ata zotero --help > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` lists each of: `search-commands`, `status`, `resolve-paper`, `add-paper`, `find-repos`, `search`, `tags`, `recent`, `advanced-search`, `grep-text`, `search-notes`, `item`, `collections`, `collection`, `groups`, `items`, `attachment`, `help`.
- `<out>` starts with `Manage Zotero libraries, collections, items, and attachments`.

### Scenario C: `search-commands <query>` includes nested commands too

3. Shell command: `ata zotero search-commands paper > <out>`.

**Expect** (verified 2026-05-22):
- `<out>` first line is `Matches:`.
- `<out>` numbered list includes top-level subcommands AND nested ones (verified: `item citation` appears as a nested-subcommand match alongside top-level `add-paper` and `resolve-paper`).
- `<out>` includes `Best match manual:` block with clap-style help for the top hit.
- The top hit's help shows `Usage: ata zotero <command> [OPTIONS]` and its `Options:` table.

### Scenario D-Q: subcommands requiring a live Zotero (TODO)

The following subcommands need either Zotero desktop running OR a
cloud API key + working credentials. Until that setup is documented
and present in CI, predicates are TODO — pin them on first successful
run.

- `collections` — list accessible collections
- `groups` — list accessible groups
- `recent --limit 5` — recent items
- `search --query rust` — keyword search
- `advanced-search <json>` — multi-condition
- `grep-text <json>` — literal/regex match
- `search-notes --query <q>` — note text
- `tags` — list tags
- `item --key <K>` — fetch by item key
- `item citation --key <K>` — generate citation (nested subcommand)
- `collection <name> --items` — list items in a collection
- `resolve-paper --query <q>` — resolve + enrich
- `add-paper --doi <DOI> --collection <name>` — add to collection (write op)
- `find-repos --query <q>` — extract repo URLs from items
- `items <subcommand>` — batch create / update / delete
- `attachment <subcommand>` — attachment create / link / delete

For each, when first validated, capture: exact JSON shape returned (if `--json` is supported), the structure of `--compact` output, error format when Zotero is unreachable, and behavior when the requested item does not exist.

### Scenario R: error when Zotero desktop is not reachable (negative test)

4. With Zotero desktop NOT running: `ata zotero collections > <out> 2>&1`.

**Expect** (to verify exact error on first run):
- `<out>` contains a connection error referencing `http://localhost:23119/api` (the local endpoint).
- Exit code is non-zero.
- Error string is stable across versions (pin it).

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

**Expect** (verified 2026-05-22 on ata 0.7.0):
- `resp` contains a paper title and/or `arxiv.org`, `dblp.org`, or `dagstuhl.de` link — proves an actual paper landed
- `resp` not contains `I couldn't find` OR `no results` — agent didn't bail
- `tool_counts` matches `[0-9]+ paper_search` with the leading count ≥ 1 — paper_search WAS called
- `tool_counts` typically shows paper_search called 3-6 times (orchestrated paraphrasing across sources — observed 6 calls on first verification run)
- `tool_counts` does NOT contain `web_search` — agent did NOT fall back to generic web search
- `tool_counts` does NOT contain `shell` — no shell-out fallback
- `tool_counts` may contain `exec_command` (count 1-4) for KB grep / journal writes — these are skill-orchestrated side effects, not search fallbacks

### Scenario B: argument schema verified across calls

6. Inspect `<search_args>` (one JSON object per line, one per call).

**Expect** (verified 2026-05-22):
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

### Scenario D: bad query / no results path (TODO)

When network is unavailable OR a deliberately obscure query returns nothing:
- `resp` should explicitly say "no recent papers" or similar
- `tool_counts` may show paper_search calls with 0 results
- The agent should NOT silently fall back to fabrication

To validate, design a deliberately ridiculous prompt and verify the agent's negative path. Pin the exact "no results" phrasing.

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

**Expect** (verified 2026-05-22):
- Response contains paper title (`Asynchronous Rust`), authors, DOI — correct content
- `tool_counts` contains `exec_command` (≥1 — used to curl arxiv.org/abs/2505.21323)
- `tool_counts` does NOT contain `paper_get` — natural prompt didn't route to the dedicated tool

### Scenario B: explicit tool naming triggers paper_get

3. In ata: `use the paper_get tool to fetch the paper with arxiv id 2505.21323 and tell me the abstract`. Sleep 1; Enter. Poll.
4. Inspect JSONL.

**Expect** (verified 2026-05-22):
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

**Expect** (verified 2026-05-22):
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

**Expect** (verified 2026-05-22):
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

**Expect** (verified 2026-05-22):
- `tool_counts` contains `paper_recommendations` (count = 1)
- Args:
  - `positive_paper_ids: ["arXiv:2505.21323"]` — ARRAY of ids (note plural and array structure, distinct from `paper_id` in paper_get/citations/references)
  - `limit: 10` (default — actually returns more than the user asked for; the agent filters down in its response)
  - `fields[]`: title, authors, year, venue, abstract, doi, arxiv_id, url, citation_count
  - `max_chars_per_item: 1200`

---

## TR-067: patent_search — agent does NOT route here even with explicit naming (routing bug)

`patent_search` tool exists in source (`core/src/tools/handlers/research.rs:226`) and is registered as `"patent_search"` for the EPO/Espacenet backend. **However, in 0.7.0 the agent fails to route to it even when the user explicitly names the tool in the prompt.** The agent falls back to `exec_command` (curl/scrape Google Patents) and produces reasonable results — but the dedicated tool is never invoked.

This is either:
- A routing bug: the patent tools are registered but the agent's tool-selection model doesn't surface them
- A feature flag: patent tools require an opt-in that isn't set in the default config

Worth filing as a finding for whoever owns the research-tools registration.

### Scenario A: even explicit naming falls back to exec_command (current 0.7.0 behavior)

1. In ata: `use patent_search to find patents about rust compiler intermediate representation`. Sleep 1; Enter. Poll up to 3 min.
2. Inspect JSONL.

**Expect** (verified 2026-05-22):
- Response contains real patent numbers (e.g. `US12039033B2`, `CN120704658A`) and descriptions — the agent DOES return patent results
- `tool_counts` does NOT contain `patent_search` — dedicated tool was NOT invoked
- `tool_counts` contains `exec_command` (≥1) — agent shelled out instead
- This scenario PASSES when the routing-bug predicate flips: if a future build successfully routes to patent_search, then `tool_counts contains patent_search` becomes the new positive predicate.

### Scenario B: when patent_search routing works (future / TODO)

When the routing is fixed:
- `tool_counts` contains `patent_search`
- Args likely have `query` (string), `limit`, `fields[]`, possibly date range / jurisdiction filters

---

## TR-068: patent_get — same routing-bug pattern expected (TODO)

`patent_get` tool exists at `core/src/tools/handlers/research.rs:231`.
Given the TR-067 finding, the same weak-routing is expected here — the
agent likely falls back to `exec_command` even when explicitly told to
use `patent_get`. Not validated in this pass.

### Scenario A (TODO — likely same as TR-067)

1. In ata: `use patent_get to fetch patent US12039033B2 and tell me the abstract`.
2. → capture and inspect.

**Expect** (predicted from TR-067 pattern):
- Response contains patent metadata correctly
- `tool_counts` does NOT contain `patent_get` (routing bug)
- `tool_counts` contains `exec_command`

When the routing is fixed: predicates flip.

---
