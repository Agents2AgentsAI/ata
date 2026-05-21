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
3. `tmux capture-pane -t <new> -p > <narrow_capture>`.
   → capture `narrow`.
4. `tmux resize-pane -t <new> -x $BASE_W`, then sleep 4.
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

> Updated 2026-05-21: predicate `No previous message to edit` was too
> narrow — that string only appears when chat history is empty. With
> prior messages, Escape shows `esc again to edit previous message`
> instead. Both forms contain `previous message`, which is now the
> shared substring. The toggle-off predicates also got tightened: instead
> of `not contains "Hold Space to speak"` (which matched announcement
> text still in scrollback after exit), assert the announcement
> `Voice mode off.` AND the default composer placeholder
> `Find and fix a bug` are visible — both unambiguously prove voice
> mode is gone.

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
- `after_escape` contains `🎤  Hold Space to speak` — still in voice mode (the core assertion)
- `after_escape` contains `previous message` — Escape ran its real binding (matches both "No previous message to edit" and "esc again to edit previous message")
- `after_toggle_off` contains `Voice mode off.` — confirmation announcement
- `after_toggle_off` contains `Find and fix a bug` — composer reverted to default placeholder (proves voice composer is gone)

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
- `saved` contains the composer placeholder (e.g. `Find and fix a bug`) — back in chat
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
