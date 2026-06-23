# Subagents / multi-agent — behavioral spec

This spec covers ata's multi-agent surface: the agent picker
(`/agent`, `/subagents`), thread switching, side conversations
(`/side`), delegated subagent work, and lifecycle cleanup. Like the
other specs it stays at capability level: exact strings, key glyphs,
and row layouts are discovered at run time. If a behavior here
changes, that is a product decision, not a refactor.

Why this component gets its own spec: subagents multiply every other
component's failure modes. LSP and PDF handling each worked in the
main thread while broken specifically inside subagents; subagent
notification timeouts and lifecycle bugs have their own fix history;
and `/side` once produced a stack-overflow class crash, fixed with a
dedicated runtime thread (larger stacks) plus a recursion guard. Each
of those past fixes is a regression class to probe.

## What multi-agent is for

1. **Delegation**: the main agent (or the user) spawns a subagent for
   a scoped task; the subagent runs in its own thread with its own
   session log, and its result comes back to the parent.
2. **Watching**: the user can list all agent threads in the session,
   see which is current, and switch which thread the chat view shows
   and which thread new messages route to.
3. **Quick detours**: `/side` forks the current thread into an
   ephemeral side conversation for a follow-up question, then returns
   to the main thread with its transcript and modes untouched. Side
   conversations are deliberately ephemeral: they leave no session log
   on disk and no thread behind after exit — absence of a side rollout
   is correct behavior, not a finding.

## Capabilities and required behavior

### The agent picker (`/agent` and `/subagents`)

- Both commands open the same picker; they are aliases and must stay
  in lockstep — same heading, same rows, same footer.
- On a fresh session the picker shows exactly one row: the main
  agent, marked as both default and current, with its thread id
  visible in a form that can be cross-checked against the session
  JSONL.
- Dismissing the picker without selecting changes nothing: the
  active agent stays the same, no new session or thread is created,
  and the next message still routes to the agent that was active
  before the picker opened (verify thread id in the JSONL, not the
  pane).
- After a subagent is spawned, the picker lists it alongside the main
  agent with its name and thread id. The picker enumerates ALL agent
  threads in the session, including ones created implicitly by tool
  calls during a turn — so it may legitimately show more rows than
  the user explicitly created. Unnamed tool-spawned threads render as
  generic agent rows.
- Keyboard navigation moves a visible focus marker between rows;
  reopening the picker resets focus to the current agent, not to the
  last-focused row.
- A separate shortcut cycles the watched thread directly from the
  chat view without opening the picker; the footer label updates to
  the newly watched agent. The picker's nav hint refers to this
  chat-view shortcut, not to in-picker movement.
- Opening the picker during an in-flight turn works immediately,
  overlay-style: the turn keeps running underneath. Known sharp edge:
  the same key that dismisses the picker can also interrupt the
  running turn. Whatever the current behavior, it must be consistent
  and must never wedge the turn or the composer.

### Thread switching and routing

- Selecting an agent switches both the displayed transcript (the
  selected agent's history, including its spawn-time context) and the
  routing of subsequent messages. The footer reflects the selected
  agent.
- The JSONL is the arbiter: after a switch, a new prompt's recorded
  thread id must be the selected agent's, and after switching back,
  the next prompt's thread id must be the main agent's. The pane can
  look right while routing is wrong.
- Switching is non-destructive: no thread loses history because the
  user looked at a different one.

### Side conversations (`/side`)

- `/side` requires at least one completed turn since session start or
  the last `/clear`; on a fresh session it is unavailable with a
  clear message, not a crash or a silent no-op.
- Entering a side conversation is visibly labeled as such, with the
  way back stated. The side runs as its own thread (its own agent
  context), seeded from the main thread's content.
- Bare `/side` opens with an empty composer; `/side <question>`
  submits the question into the side scope and the answer arrives
  there, not in main.
- Inside a side conversation, slash commands are restricted to a
  small read-only allowlist; everything else is blocked with one
  consistent error template that names the command and points back to
  the main thread. `/side` itself is blocked inside a side
  conversation — this recursion guard is part of the stack-overflow
  fix and must never regress to allowing nested sides.
- Starting a side is idempotent: while one side is open or a side
  start is still in flight, a second `/side` is rejected with the
  "already open" error and its command text is dropped, never restored
  into the composer. Rapid `/side` enter/exit cycles must not queue
  duplicate side starts, accumulate command text, or leave the UI
  wedged in a half-side state where Esc no longer returns to main.
- Exiting the side returns to the main thread: the side label is
  gone, the main transcript is intact and unpolluted by the side
  exchange, and modes set before the detour (e.g. plan mode) survive
  it.

### Subagent capability parity

A subagent must have the same working capabilities as the main agent
within its scope. Two capabilities broke specifically inside
subagents in the past and are mandatory probes:

- **Code intel / LSP**: a subagent given a task that needs code
  intelligence must actually get working code-intel tools, and a
  code-intel failure inside the subagent must surface as an error in
  the parent, not hang it.
- **PDF handling**: a subagent given a PDF to read must either read
  it or report the failure to the parent. Errors were previously
  swallowed inside subagents — a subagent that "completes" while its
  JSONL shows an unreported tool failure is exactly the historical
  bug.

In both cases judge the subagent's own session JSONL, not the
parent's prose: which tools were called, with what arguments, what
they returned, and whether failures made it into the result the
parent received.

### Lifecycle and cleanup

- A spawn is recorded in the parent's JSONL; progress flows while the
  subagent works; the completion result returns to the parent.
- Parallel subagents on tasks touching the same directory stay
  isolated: distinct session logs, no interleaved writes, both
  results reach the parent.
- A subagent that fails hard (or is killed mid-task) must not take
  the parent down: the parent reports the failure and remains usable.
- No orphans: after the parent session exits, no subagent process,
  thread, or stray session activity survives. The same holds for side
  conversations — exiting a side leaves nothing running.
- Containment: a subagent inherits the parent's sandbox and workspace
  roots and cannot write outside them. Delegation is not an escape
  hatch.

## How to test it

Drive the TUI through tmux (recipe in the README), with the session
JSONL as the deterministic anchor for every routing and delegation
claim. Use a build of `./target/debug/ata --yolo` and a throwaway
working directory.

Work through the picker first: fresh-session single row, alias
parity between the two commands, dismiss-changes-nothing (send a
distinctive message after dismissing and check its thread id), spawn
a named subagent and confirm it appears, switch to it and back with a
distinctive message at each step, cross-checking every routing claim
in the JSONL. Then the chat-view thread-cycling shortcut, focus reset
on reopen, and the picker during a deliberately long turn.

Then `/side`: the fresh-session negative case, bare entry, inline-arg
entry, the blocked-command template on several commands, the
recursion guard, an allowlisted command as positive control, and exit
with main-transcript and mode-persistence checks.

**The stack-overflow probe deserves dedicated effort.** The historic
crash class was process-fatal (stack exhaustion), fixed by a
dedicated runtime thread and the recursion guard — so probe the
class, not just the one known trigger:

- Rapid `/side` entry-exit cycles: dozens of enter/exit round trips
  as fast as the TUI accepts them, including cycles with a question
  submitted each time. The process must never die; watch for the
  tmux pane dying, a shell prompt where the TUI was, or
  abort/SIGSEGV in the wrapper shell's exit status.
- `/side` attempts at hostile moments: immediately at startup
  (before the precondition is met), during an in-flight turn, twice
  in quick succession before the first finishes opening, and from a
  session with a very long history.
- Repeated recursion attempts inside a side (the guard must hold on
  the 50th try as on the first).

For delegation, give the main agent a real task to hand off — in
your own words, varied between runs — that forces a tool-heavy
subagent: e.g. "delegate to a subagent: find where <symbol> is
defined in this repo and summarize its callers" (code intel) and
"have a subagent read <local PDF> and report the figure count" (PDF).
Locate the subagent's JSONL under `~/.ata/sessions/`, and judge the
trajectory: were the tools available inside the subagent, were they
called, did errors propagate to the parent's result. Repeat the PDF
probe with a corrupt file so there is a guaranteed error to trace.

For lifecycle: run two subagents in parallel against the same
directory and diff their session logs for interleaving; kill a
subagent's process mid-task and confirm the parent stays usable;
after exiting the TUI, sweep for orphans (`pgrep` for ata children,
recent-mtime session files still growing). For containment, delegate
a task whose easy shortcut writes outside the sandbox and verify the
subagent refused or was blocked — in its JSONL, not its prose.

Clean up everything: kill spawned subagents where a teardown exists,
otherwise note the leftover in the report; kill tmux sessions; remove
scratch files.

Report per the README: issues with exact reproductions, divergences
citing the section above, JSONL excerpts for routing/delegation
claims, and coverage notes.
