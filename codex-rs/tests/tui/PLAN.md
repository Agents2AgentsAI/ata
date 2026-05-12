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

---

## TR-001: Reading view survives a resize cycle

This is the canonical regression test for the v0.129.0 merge resize-corruption
bug. It opens a reading view, shrinks the pane, grows it back, and verifies
that no welcome-banner / chat-composer cells leak into the reader.

**Setup**:
1. `cargo build -p codex-cli` and confirm the binary at
   `./target/debug/ata` is newer than `tui/src/tui.rs`.
2. Find the user's tmux session/window via `tmux list-clients` +
   `tmux display-message`.
3. Record the original pane width as `BASE_W` (typically 200+).
4. `tmux split-window -h -t <session>:<window>.<pane> -c $(pwd) './target/debug/ata --yolo'`.
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

# Adding tests

Append `## TR-<NNN>: <name>` sections following the same shape. Pick
**Expect** predicates that fail when the bug regresses and pass otherwise —
keep them narrow. A predicate like "contains 'Section'" is too loose; one
like "row 1 starts with '╭'" is right because that's a specific
property of the rendering.
