# Reading view — behavioral spec

This spec describes what the reading view (document reader) does for
users and agents, and the outcomes that must hold. It deliberately
avoids implementation detail (exact key labels, error strings, tool
schemas): the testing agent discovers concrete syntax at run time from
the reader's own footer and help overlay, and judges agent behavior
from the session JSONL. If a behavior here changes, that is a product
decision, not a refactor.

This component has the richest regression history in the repo
(~120 fix commits): section duplication, fold loss, resume replay,
chat leaks, bounds panics, browser-mode buffering. The probes below
encode that history; treat each one as a regression that has already
happened once.

## What the reading view is for

When the agent produces structured, document-shaped content (slides, a
report, a synthesis, an explained paper), it can present it in a
dedicated reader instead of dumping it into chat. The reader gives the
user a focused surface: navigate sections, scroll, search, fold,
select text, ask scoped questions, and have sections narrated aloud —
while the chat transcript and the model prompt stay clean. The same
document can render in the terminal or in a browser tab, per
configuration.

The contract has two halves:

- **the user-facing surface** — a modal reader with its own keymap,
  discoverable from a footer hint line and a help overlay;
- **the agent-facing surface** — a small set of document tools with
  distinct semantics (present a document; add a section; append to a
  section; replace a section; make a scoped find-and-replace patch
  inside a section), every call scoped to a document and a section.

## Capabilities and required behavior

### Gating and modes

- The reading view is opt-in: a feature flag plus a display mode
  (terminal / browser / disabled), both discoverable in the config and
  the setup popup. With the feature off or mode disabled, the agent
  answers in chat — no half-open reader, no orphaned state.
- Changing the mode in the setup popup takes effect immediately and
  persists across restarts.
- Plan mode suppresses the reader; document-shaped answers stay in
  chat there.

### Document tool routing (judged from session JSONL)

The write operations have different semantics and the agent must pick
the one matching the user's intent. The pane can look correct while
the wrong operation ran, so every probe here is judged from the
session JSONL — operation name, target document, target section,
payload — not the rendering.

- "add a new slide/section about X" routes to the add-section
  operation, inserted at the right position; existing sections are
  untouched (verify their indices never appear as write targets).
- "add Y to the end of section N" routes to append. The original
  section content survives on screen AND the appended payload contains
  only the new text — an append whose payload re-pastes the whole
  section is the classic wrong-tool failure even though it renders
  fine.
- A question or rewrite scoped by a visual selection patches only the
  focused section. The selected text itself must reach the agent (the
  payload reflects the selection, not the whole section), and no write
  targets a neighboring section.
- The agent never falls back to raw file-patching or shelling out to
  edit a presented document; those bypass the section model.

### Re-presentation: no duplication, no fold loss

Presenting the same document again — in the same session or after the
agent revises it — must not duplicate sections, and fold state set by
the user must survive the re-present. A surviving fold must stay
anchored to its own body text: when the re-presented content shifts the
fold's bytes (revised prose, headers the core cache never saw), the fold
re-anchors to where its body actually is, or is dropped — it must never
keep stale offsets that render a collapsed `[+]` over unrelated prose
while the real body leaks. This regressed before; probe it explicitly by
asking for the same document twice and re-checking section count and
folds.

### Resume and replay

- Resuming a session that contained a reader restores the documents so
  follow-up tool calls still work, but must not replay the reader open
  uninvited: the user lands in chat, not in a re-opened reader, and in
  browser mode no browser tab opens on its own.
- Historical reader events must not double-render in the resumed
  transcript.

### Containment between reader and chat

- Document content enters the model prompt only as the gating intends.
  Check the JSONL for what was actually sent, not the pane.
- Asking a question from inside the reader keeps the whole exchange
  inline: the question does not render as a chat bubble, the answer
  lands inside the focused section as a foldable Q&A block (with a
  collapsed summary), and the system wrappers that carry the question
  to the agent are never visible in the pane or recallable from
  composer history. The Q&A block is self-contained: its collapsible
  body holds only the answer — the question is not echoed back into the
  prose, since the fold summary (the model-supplied title) already
  labels what the answer addresses. Collapsing it
  must leave the original section prose — including the exact text the
  user selected and the sentence that follows it — fully readable. A
  fold that hides adjacent original prose when collapsed is a defect.
- The follow-up turn's intermediate work stays out of the transcript:
  while the reader owns the screen, the agent's shell/command cells,
  tool calls, and streamed reasoning must not leak into the scrollback
  behind the reader. Only the answer reaches the reader (through the
  document tool). If a follow-up runs a shell command, no `Ran …` cell
  appears in the chat — the JSONL still records the call, but the TUI
  transcript shows nothing for it.
- Closing the reader is clean: chat resumes with no leftover frame
  fragments or garbled output, and later turns render normally. The
  close sends the agent a silent system note about what was viewed
  (it may trigger an invisible follow-up turn); that note appears in
  the JSONL as a system-injected message, never in up-arrow history.
- Closing from inside a sub-mode (e.g. an active visual selection)
  closes in one step and drops the selection silently — the discarded
  selection must never reach the agent.
- The dismiss keys behave asymmetrically by design: the close key and
  interrupt key both close the reader; plain Escape does not (it is
  reserved for cancelling sub-modes like selection, search, and the
  help overlay).

### Navigation, search, folds, selection, help

- Section navigation: next/previous keys, a table-of-contents overlay
  with jump-to-section, read indicators on visited sections (including
  sections visited by jump or by scrolling through them), and boundary
  affordances — no back affordance on the first section, no forward
  affordance on the last.
- Intra-section movement is vim-style: line scroll, half- and
  full-page, jump to top/end, word and character motion. Both ends are
  hard bounds; overscrolling never panics or corrupts the frame.
- Search spans the whole document: a query input, a current/total
  match counter starting at the first match, forward/backward stepping
  that crosses section boundaries and wraps at the ends, and a cancel
  that removes the query, the counter, and the highlights, restoring
  the normal footer.
- Folding: toggle at cursor, jump to previous/next fold, collapse-all
  and expand-all. On a document with nothing foldable every fold key
  is a silent no-op — bound, harmless, no error.
- Visual selection: character- and line-level modes with their own
  footer, motion keys to extend, one key to have the selection
  explained, another to ask a typed question about it, cancel to drop
  it. Selection drives the scoped-write contract above.
- The help overlay toggles open/closed and enumerates the keymap. The
  footer and the help overlay should agree on the supported surface;
  any binding present in one but not the other is a reportable
  documentation gap (narration keys have historically been missing
  from the help).

### TTS narration

- A key starts narration of the current section. While narrating, the
  footer gains audio controls (pause/resume toggle, speed up/down);
  when idle those controls are absent and pressing them is a no-op.
- With no speech backend configured, starting narration fails with a
  visible error and the reader stays fully usable. It must never crash
  or wedge the reader.
- With a backend, pause, resume, and speed changes take effect
  immediately. Word-tracking (karaoke) correctness is covered in depth
  by the voice spec; here, verify narration starts, controls respond,
  and stopping or closing the reader stops the audio.

### Resilience

- Resize survival: shrink the terminal hard and grow it back with the
  reader open. The reader frame stays intact throughout and no
  chat-layer content (welcome banner, composer, tips) bleeds into the
  reader cells. This is the canonical historical corruption bug.
- Concurrency: navigating, scrolling, and searching while the agent is
  still streaming content into the document, or while a reader
  question is being answered, must work — browsing is not locked out
  by an in-flight turn, and no panic or misrender results.
- Compaction with an open reader: in a session whose prompt contains
  reader content, force a compaction while the reader is open, then
  keep conversing and keep using the reader. The session stays usable
  and the post-compaction prompt is not mangled — judge from the
  JSONL. (Compaction × rich content is a recurring regression class.)

### Browser mode

- The browser view serves on a stable port; closing and reopening the
  tab mid-session reconnects automatically; a browser that connects
  late receives the buffered events and catches up to the current
  document state rather than starting blank.
- Resume never auto-opens a browser tab.
- Figure extraction: on the first use in a fresh session, asking the
  agent to extract a figure from a real PDF must produce an image that
  actually renders in the browser view (asset serving works from the
  very first extraction) and whose crop contains the figure.

### Knowledge-base persistence

- A reading session that teaches a concept feeds the knowledge base.
  The terminal reader persists on close; browser mode has no close key,
  so it persists after a follow-up question and after the next genuine
  response in the TUI. Either way, new insight from the session's
  follow-up Q&A lands in the document's KB card.
- Persistence is checked against what the card already holds: a session
  that produced nothing new writes nothing. It must never block on the
  persist (no waiting), and a failed or skipped persist must not be
  silently assumed to have succeeded.
- Persistence runs as a detached background sub-agent the reading view
  pokes directly at the close / follow-up boundary; it never rides the
  foreground model turn. After the reader closes (or after a browser
  follow-up), the composer is immediately usable and the TUI does not
  enter a visible "Working …" state for the persist. The card is still
  written in the background. A close that leaves the composer busy for
  the duration of a KB write is a defect.
- It is invisible: the persistence trigger never surfaces as a chat
  bubble, a recalled history entry, or an announced action, and it does
  not stop the reading view from serving.
- Judge real effect, not just the spawn: after a browser-mode reading
  session with substantive follow-up Q&A, the active KB has card content
  for the document. An empty KB after such a session is a defect.

## How to test it

Drive the TUI through tmux per the README recipe. First flip the
feature flag and display mode on in the config (record the prior
values and restore them at the end). Open readers with natural
prompts in your own words ("give me N short slides on … in reading
view"); vary topic and wording between runs — verbatim reuse turns
this back into a script. Reader opens can take a couple of minutes;
poll the pane.

Discover the keymap from the footer and help overlay rather than this
spec, then exercise every behavior section above. For all routing and
containment claims, the session JSONL is the anchor: extract the
document tool calls and their arguments, and judge operation choice,
section targets, and payloads against the contract. Tool routing is
model-dependent — on a surprising miss, rephrase and retry once before
filing; a miss that persists under a direct instruction is a defect.

Adversarial minimum (invent more):

- Re-present the same document twice; resume the session and confirm
  no replay, no auto-open, no duplicated sections, folds intact.
- Overscroll both ends; mash navigation during streaming; resize
  during render; close from every sub-mode.
- Search for a term with zero hits, one hit, and hits in every
  section; step past both ends.
- Ask a question whose honest answer requires content from a
  different section — does the scoped write still respect the focused
  section?
- Force a compaction with the reader open, then keep working.
- Start narration with no backend configured; press the audio keys
  while idle.
- In browser mode: connect the browser late, kill the tab mid-stream,
  reconnect.
- KB persistence: in browser mode, open a reader that teaches a concept,
  ask a follow-up that yields a genuinely new insight, then respond in
  the TUI. Confirm the active KB gains a card for the document (inspect
  `CODEX_KB_PATH` / the workspace knowledge-base — not empty), and that
  nothing about the persist surfaced as a visible chat bubble, history
  recall, or announcement in the session JSONL. Repeat with a follow-up
  that adds nothing new and confirm the KB is left unchanged. Also close
  a terminal-mode reader after follow-up Q&A and confirm the same
  persistence.

Known divergences to verify and report (do not silently accept or
silently fix): the jump-to-end motion has been observed scrolling a
few lines instead of jumping; the help overlay has omitted the
narration keys that the footer shows. If still present, report each
as a divergence citing this section.

Clean up everything: close readers, kill panes, restore the config to
its prior state, pass or fail. Report per the README: issues with
exact reproductions, divergences citing sections above, routing
violations quoting the JSONL, and coverage notes.
