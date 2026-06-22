# Session continuity — behavioral spec

This spec covers the four operations that transform a session's
lifetime — resume, fork, /clear, and compaction — not as four isolated
commands but as a cross-cutting axis. The history shows why: every
feature that holds state has broken under one of these operations at
least once (reading view replayed on resume, voice lost after /clear,
Gemini history gone on resume, compaction mangled by PDFs three
separate times). The unit of testing here is therefore the pair
(operation x content type), and the testing agent's job is to sweep
that matrix, not to check four commands in a vacuum.

Like the other specs, this stays at capability level: discover exact
commands, picker layouts, and wording at run time via `--help`, the
slash menu, and the session JSONL. The JSONL is the ground truth
throughout — the pane can look continuous while the rebuilt prompt is
broken, and vice versa.

## The four operations and their contracts

Each operation makes a distinct promise about what survives:

- **Resume** restores a prior session in full. Conversation history,
  the selected model and provider, and the session's accumulated
  context all come back; the next turn works as if the process had
  never exited. Resume must restore state without replaying side
  effects: a session that had a reading view open must not re-open it
  uninvited, narration must not restart, tool calls must not re-fire.
  There are two entry paths — a browsable picker over prior sessions
  and a direct lookup by id or saved name — and they must agree on
  which sessions exist. Direct lookup is exact, never fuzzy: a
  near-miss is an error, not a guess. Resuming the session you are
  already in is an explicit no-op, not a reload.
- **Fork** branches a new session off the current one. The child
  starts with a fresh visible surface but full semantic memory of the
  parent's conversation. The parent is untouched and stays resumable
  under its own id (the fork tells you how to get back). The two
  branches are thereafter independent: turns in one must never appear
  in the other, and both must survive their own resume.
- **/clear** starts a fresh conversation thread. In the live session
  that follows, rendered history and the agent's memory of the
  pre-clear turns are gone — a "what did we discuss" probe after /clear
  comes back empty, in the new thread's JSONL too, not only the pane.
  What survives /clear is configuration, not conversation: settings and
  saved defaults persist per their own contracts (voice defaults were
  once lost here — that was a bug). Clearing an empty session does
  nothing noisy.
  > **Design note (resume-after-clear):** /clear forks a NEW thread;
  > the PRE-clear session is left intact and resumable, so resuming the
  > id /clear prints brings the old conversation back. This is recovery
  > by design, not a leak — the live post-clear thread stays empty.
  > (If the intended contract is instead "cleared turns are
  > unrecoverable even by resuming the old id," that's a product change
  > to make /clear redact or detach the old thread; flagged for a
  > decision.)
- **Compaction** trades verbatim history for a summary while the
  session continues. After compacting, the visible scrollback
  persists and a compaction marker is appended to it; what changes is
  the rebuilt prompt, which is compacted to keep user and assistant
  text while dropping tool traffic — recall probes succeed in
  summarized form. The compacted prompt must be well-formed for the
  active provider, and the tail of the conversation (the turns nearest
  the compaction point) must not be dropped. Compaction may also fire
  automatically as context fills; the same contract applies.

All four are conversation-lifecycle operations and are blocked while a
turn is in flight; the in-flight turn must be untouched by the
rejected attempt.

The contrast between the four is itself part of the contract: /clear
forgets, /compact remembers in summary, /fork remembers in full in a
new session, resume remembers in full in the same session. A probe
that cannot distinguish them is too weak.

## Continuity per content type

This is the spec's distinctive job. After each operation, do not ask
"did it work" — ask, for each kind of state the session held, "did
this survive, and if it degraded, did it degrade honestly?" Honest
degradation means the user is told; dishonest degradation is state
silently missing, mangled, or replayed.

Sweep at least these content types against each operation:

- **Conversation history.** The baseline. After resume and fork,
  recall probes must reach pre-operation turns; after /clear they must
  not; after compaction they must succeed in summary. Verify against
  the JSONL, since rendered scrollback and the rebuilt prompt have
  diverged before.
- **Model and provider selection.** The selected model must stick
  across all four operations, and resume of a session that used a
  non-default provider must bring back both the selection and the
  history. Gemini sessions losing their chat history on resume was a
  real shipped bug; run the resume probe on at least one non-OpenAI
  provider.
- **Open reading-view documents.** Resume a session that had a
  document open: the document's events must have persisted (they once
  were not), and the view must be restorable without replaying or
  auto-opening — a resumed session once popped the browser reading
  view uninvited. Fold state and section structure must come back
  without duplication.
- **PDFs in context.** Compact a session whose context contains PDF
  content (attached file and fetched URL), then continue the
  conversation. This exact combination broke three separate times.
  The post-compaction prompt must be valid for the provider, the
  session must stay usable, and the PDF content must be either
  summarized or honestly absent — never mangled bytes in the prompt.
  Then resume that compacted session and continue again.
- **Goals.** If the build exposes goal state, an active goal must
  survive resume and fork per the goals spec's own contract, and
  /clear must treat it per the documented boundary (goal is
  session-scoped or not — whichever the product says, verify it and
  flag ambiguity).
- **Queued and in-progress state.** Queued user messages, running
  background tasks, and pending approvals at the moment of exit:
  after resume, each must either be restored or be reported as
  dropped. Silently vanished queued input is a failure.

When a feature outside this list holds session state (voice, forks of
forks, whiteboard, anything new since this spec was written), add it
to the sweep. The pattern in the history is that every new stateful
feature breaks under resume first; assume the matrix grows.

## Session discovery and storage hygiene

- Session discovery must be robust to a polluted sessions directory:
  non-rollout JSONL files and stray files alongside real sessions must
  not confuse the picker, the resume lookup, or session counting.
  This was a shipped bug.
- A session id printed by any of the four operations (the resume hint
  after /clear or fork) must actually resolve — the hint is a
  contract, not decoration.
- Killing the TUI uncleanly (mid-turn, mid-fork, mid-compaction) must
  leave the sessions directory in a state where discovery still works
  and the damaged session either resumes or fails with a clear error,
  never corrupts the picker for everyone else.

## Post-upstream-merge smoke

Run this spec first after any upstream merge. Every upstream merge in
the repo's history was followed by a wave of fix commits, and session
continuity is where the breakage surfaces: the merge moves session,
prompt-assembly, or rollout code, and resume/compaction are the paths
that exercise all of it at once. A single pass of the content-type
matrix above doubles as the cheapest whole-system smoke test ATA has.
If only one behavioral run fits in the post-merge budget, it is this
one.

## How to test it

Drive the real binary through tmux (recipe in the README). The
general shape of every probe is: **build state, operate, diff**.

1. Build a session that holds several content types at once: a few
   distinctive conversational turns (plant unique recall tokens), a
   non-default model selection, a document open in the reading view, a
   PDF in context, a goal if available, and something queued or
   running at exit time. Record what the session holds — the recall
   tokens, the model name, the document, the JSONL path.
2. Apply one operation.
3. Diff observed behavior against the recorded pre-state, content
   type by content type, using both the pane and the JSONL. Then send
   a real next turn: the session must not just look right, it must
   work.

Run that loop for each of the four operations, and run the
compositions the history flags as fragile: compact → resume →
continue (the post-resume prompt must be built from the compacted
state without losing the conversation tail), fork → mutate both
branches → resume both (independence and survivability), clear →
resume the cleared session, fork of a fork.

For resume specifically, exercise both entry paths and their
disagreements: picker vs direct lookup, exact id, saved name,
near-miss token (must error, not guess), the current session's own id
(explicit no-op), and a session from a polluted directory.

Then go adversarial — minimum classes, invent more:

- **Replay hunting**: after every resume, scan the JSONL for re-fired
  tool calls, re-opened views, restarted narration, or duplicated
  events. Side effects must not run twice.
- **Interrupted operations**: kill the process mid-compaction and
  mid-fork; force the four operations during an in-flight turn (each
  must be refused and leave the turn intact); fire two operations
  back-to-back faster than they can settle.
- **Storage tampering**: stray and non-rollout files in the sessions
  directory, a truncated rollout file, a rollout from an older binary
  version.
- **Cross-provider**: repeat the resume and compaction probes on at
  least two providers; provider-specific prompt rebuilding is where
  history-loss bugs lived.
- **Honesty checks**: wherever state did not survive, confirm the
  user was told. A clean pane over a lossy resume is the worst
  outcome this spec exists to catch.

Report per the README: issues with exact reproductions, divergences
citing the contract sections above, a filled-in operation x content
type matrix showing what was swept and what was not, and coverage
notes. If run as post-merge smoke, say so in the report header and
prioritize breadth of the matrix over depth of any one cell.
