# Scheduling — behavioral spec

This spec describes what ata's scheduling feature does for users and
agents, and the outcomes that must hold. It deliberately avoids
implementation detail (field names, exact error strings, on-disk
layout): the testing agent discovers concrete syntax at run time by
reading the tools' own descriptions and the panel's own copy, so
code-level churn does not invalidate this document. If a behavior here
changes, that is a product decision, not a refactor.

**Read the teardown section before creating anything.** OS cron tests
leave entries that fire every minute on the user's real machine; the
cleanup ritual is mandatory, pass or fail.

## What scheduling is for

Scheduling lets the in-app agent run work that is not tied to the
current turn. Three task kinds, chosen by intent:

1. **OS cron jobs**: clock-aligned recurring prompts that must outlive
   ata. The schedule is owned by the operating system's crontab; each
   firing spawns a fresh ata subprocess with no memory of any chat,
   and its output goes to a per-task log file, not to chat. Minimum
   granularity is one minute.
2. **In-session crons**: clock-aligned recurring prompts scoped to the
   current chat session. Each firing injects a new user turn into this
   conversation, with full conversation memory, rendered in chat
   (visibility is tunable). Sub-minute schedules are allowed, as are
   bounds like a maximum firing count, an end time, and a timezone
   offset. The schedule dies when ata closes (and may resume with a
   resumed session).
3. **Monitors**: a background shell command spawned in this session,
   with its output streamed line by line. There is no clock; work
   happens when output appears or the process exits. Two blocking
   primitives let the agent react: one waits for termination and
   returns final status plus an output tail (`monitor_wait`), the
   other waits for a specific line to appear mid-stream
   (`monitor_watch_for`), with an optional batch mode that collects
   every match.

One TUI surface, `/scheduling`, shows all of the session's tasks: cron
jobs and monitors, with names, statuses, fire/line counts, and a
delete key.

## Capabilities and required behavior

### Routing: the agent must pick the right kind

The tool family is large and the model chooses among the tools by
intent. These contracts are judged from the session JSONL (which tool
was actually called, with what arguments), never from the rendered
prose, because the pane can narrate a plausible success while the
wrong primitive ran underneath:

- Persistent phrasing ("every morning at 9", "keep doing X after I
  close this") must route to the OS cron creator, not the in-session
  one.
- Session-bound phrasing ("while I'm working today", "in this
  session", sub-minute intervals, or any firing that needs the current
  conversation's context) must route to the in-session creator, not OS
  cron.
- A prompt asking to **react to a specific pattern in output** ("tell
  me when it prints X", "alert me when 'ready' appears") must use
  `monitor_watch_for`, not `monitor_wait`, and not a shell fallback
  (grep/tail loops). The pattern argument must carry the user's
  literal phrase, not a paraphrase.
- A prompt asking for the **final result** of a command ("run the
  build and tell me if it fails") is `monitor_wait` territory.
- None of these should fall back to the generic shell tool when the
  dedicated tool exists and applies.

### OS cron jobs

- Creating a job writes a real entry to the user's system crontab,
  observable via `crontab -l`, and reports a task id, the next fire
  time, and the log path. The recorded schedule must match what is in
  the crontab.
- The job **actually fires**: within roughly a minute of the schedule,
  a fresh ata subprocess runs the prompt and the per-task log file
  appears and grows. The firing has no memory of the creating chat.
- Jobs are user-scoped, not session-scoped: a job created in one
  session is listed from another, and from a fresh launch.
- Jobs survive ata exit: with ata closed entirely, the OS keeps
  firing the job, and on the next launch the panel shows the job again
  with its accumulated fire count (non-zero if it fired while ata was
  off).
- Listing only reports ata's own jobs. Foreign crontab lines the user
  wrote by hand must be neither listed nor touched by ata's list or
  delete operations.
- Sub-minute or otherwise impossible schedules for OS cron must be
  refused or redirected, not silently accepted.
- **Deletion is total** (this is a past regression, treat it as the
  headline check): deleting an OS cron job, whether via the delete
  tool or the panel's delete key, must (a) remove the crontab entry,
  verifiably via `crontab -l`, and (b) kill every subprocess of that
  job that is in flight at delete time. Verify (b) by PID: record the
  job's live process tree before deleting, then confirm each recorded
  PID is dead afterward. A disappearing panel row with a still-running
  subprocess is the exact bug this guards against.
- Known caveat (macOS): the cron daemon may have already enqueued a
  fire from its in-memory schedule in the same minute as the delete;
  that fire runs with *new* PIDs and finishes naturally. The invariant
  is "every pre-delete PID dies", not "no process ever matches the
  task id again". Do not flag the OS race as a failure.
- Deleting an unknown task id reports that nothing was deleted rather
  than erroring.

### In-session crons

- Creation registers the schedule in this session only and reports a
  task id. No crontab entry appears.
- The cron actually fires: at the scheduled time the prompt arrives in
  chat as a new user turn and the agent answers it with full
  conversation memory. The panel's fire counter increments per fire,
  and a next-fire countdown is shown between fires.
- The visibility flag does what it says: a "report back each time"
  cron renders its replies in chat; an "only alert me if" cron keeps
  quiet between alerts.
- Bounds are honored: a one-shot fires exactly once and stops; an end
  time stops further firings.
- Deletion stops future firings; the row leaves the panel. Closing ata
  ends the schedule (no crontab residue, nothing fires after exit).

### Monitors

- Starting a monitor spawns the command in the background and returns
  a task id immediately. With live streaming on, every output line is
  delivered to the agent itself as it happens — pushed into the
  conversation without the agent polling for it — and also reaches the
  user's chat; with it off, per-line output is suppressed for both the
  agent and the chat, and only the panel's line counter climbs until the
  termination summary fires.
- Live streaming to the agent is push, not pull: the agent that started
  the monitor receives the output lines on its own turn loop and reacts
  to them without ever calling `monitor_wait`. The agent's command is
  the filter — it should narrow output to the lines worth acting on
  (e.g. grep for errors), since each delivered line is a message, and it
  must keep that filter line-buffered (`grep --line-buffered`, `sed -u`,
  `awk` with `fflush()`) so matches stream as they happen; an
  un-line-buffered filter block-buffers and dumps every match in one
  burst when the command exits, which is an agent-command defect, not a
  streaming failure. Bursts
  of lines are coalesced into batched deliveries rather than one per
  line, and a chatty monitor that would flood the agent has its live
  streaming capped (the monitor keeps running and the user still sees
  output; the agent is told streaming was capped and can fall back to
  `monitor_watch_for`/`monitor_wait`).
- When the process exits, a termination summary surfaces (status plus
  output tail) to the agent and the panel, ordered after any streamed
  lines, and the panel retains the row in a completed state with an
  accurate line count until the user dismisses it.
- `monitor_wait` is a fallback for blocking until the process exits to
  use its final status and tail in the same turn; it is not required to
  receive streamed output. An explicit timeout returns early and says so.
- `monitor_watch_for` returns the moment a line containing the literal
  pattern appears, identifying the matching line and stream. Matches
  that happened before the call started are not missed (the existing
  buffer is scanned). The three exits are distinguishable: matched,
  process terminated without ever matching, timed out. Batch mode
  collects every match across the process lifetime.
- A watch returning early must **not** disturb the underlying process:
  it runs to completion and the final line count reflects all output,
  not just up to the match.
- Stopping a monitor kills the process; no further output is injected.
- Monitors are session-bound. After an ata restart, a monitor that was
  running is reported as interrupted rather than silently vanishing.
  The opt-in restart-on-resume behavior exists for idempotent
  long-running commands only; a one-shot build must never be silently
  re-run on resume.

### The `/scheduling` panel

- The panel shows the session's scheduling state: a cron section and a
  monitors section, each with a count, plus a freshness timestamp and
  key hints. Empty sections say so rather than rendering nothing.
- Rows carry a recognizable name (the human label when one was given),
  a status that distinguishes the task kinds and lifecycle stages
  (an OS cron scheduled, an in-session cron pending with a countdown
  and fire count, a monitor running/completed with a line count), and
  both sections render correctly when populated simultaneously.
- The panel reads live state: a fire that happened while the panel was
  closed shows in the counters on next open; a job created from
  another session shows up for OS cron.
- Selecting a row and confirming opens a detail view with the full
  story the list truncates: id, status, the complete command or
  prompt, counts, and a recent-output tail tagged by stream.
- Pressing the delete key on a row performs the real deletion for that
  task kind, with all the consequences above (crontab entry removed,
  in-flight subprocesses killed, future firings stopped). It is not a
  cosmetic row removal.
- Dismissing the panel (from the list, or from the detail view, which
  exits the whole panel rather than returning to the list; a
  documented quirk, not a bug) leaves no residue: panel content fully
  gone, composer focused, next chat round-trip works.
- Panel keys must not leak into chat or fire while the user is doing
  something else; an accidental delete from a stray keypress is an
  issue.

## How to test it

This spec demands **real effects**. The old scripted suite skipped
firings and restarts as too flaky for CI; an agentic run has the
judgment to wait, retry once, and tell flake from failure, so do test
them. For every claim of an effect, check the substrate, not the UI:
the crontab via `crontab -l`, the process table via recorded PIDs, the
log file on disk, the session JSONL for tool routing. A row
disappearing from the panel proves nothing by itself.

Practicalities:

- Prefix every task name (e.g. `schedtest-`) so teardown can find
  strays. Use cheap, recognizable commands (short echo loops, sleeps
  with markers) so processes are easy to find and harmless.
- Timing: OS cron fires on minute boundaries; allow up to ~70s for the
  first fire and poll the log file size rather than sleeping blind.
  In-session crons can be sub-minute; use 20–30s intervals to iterate
  fast. After a delete that should kill processes, give the kill a
  ~10s settle window before asserting PIDs are gone (the kill is
  asynchronous and process reaping takes a beat).
- The PID discipline for the deletion check: record the job's live
  PIDs into a file *before* pressing delete, then assert each recorded
  PID is dead afterward. Re-querying by name post-delete is wrong both
  ways (misses the regression, flags the macOS race).
- Restart story: create an OS cron, quit ata fully, wait through at
  least one minute boundary, confirm the log grew while ata was off,
  relaunch, and check the panel shows the job with a non-zero fire
  count. Run the in-session counterpart too and confirm the opposite:
  nothing fires after exit.
- macOS precondition: the terminal needs Full Disk Access or crontab
  writes fail; verify `crontab -l` works from your shell first.

Then go adversarial — minimum classes, invent more:

- **Bad schedules**: garbage expressions, wrong field counts,
  sub-minute requests aimed at OS cron, end times in the past. Each
  should be a clear refusal or redirection, never a silent acceptance.
- **Identity**: deleting/stopping/waiting on unknown or already-dead
  task ids; deleting the same task twice; an in-session delete aimed
  at an OS task id and vice versa.
- **Tampering**: hand-remove the crontab entry behind ata's back, then
  list and delete through ata (degrade gracefully?); add a foreign
  crontab line and confirm ata's operations never touch it.
- **Watch edges**: a pattern that already scrolled by before the watch
  call (must still match from the buffer); a pattern that never
  appears (terminated-without-match, not a hang); a watch with a short
  timeout against a slow process; killing the monitored process
  externally while a wait is blocked.
- **Concurrency**: several monitors streaming at once; the panel open
  while a cron fires; delete pressed during an in-flight firing.
- **Silent failure hunting**: wherever creation reports success,
  verify the implied substrate effect actually exists (entry in
  crontab, log file appearing, process spawned). This class is where
  scheduling bugs hide.

For the routing contracts, boot the TUI and give the in-app agent
scheduling tasks in your own words, varying the phrasing between runs
(verbatim reuse turns this back into a script): a persistent daily
job; a "while I'm here" poll; a long command with "tell me when it
prints <phrase>"; a "run this and tell me if it fails". After each
turn, read the session JSONL and judge the trajectory: which tool was
called, with what arguments, whether the pattern is the user's literal
phrase, and whether the agent shelled out instead of using the family.
A correct-sounding reply on top of the wrong tool call is a failure.

Report per the README: issues with exact reproduction commands,
divergences citing the section above, routing violations quoting the
session log, and coverage notes.

## Mandatory teardown — run regardless of pass/fail

OS cron entries created here fire **every minute** on the user's real
machine. On macOS, every fire of an unsigned dev binary triggers a TCC
permission popup that a known OS bug prevents from dismissing, so a
forgotten entry spams the screen until removed. Therefore, at the end
of every run, even after a crash or interrupt, and again manually if
the run aborted mid-way:

1. Remove every ata-created entry from the user's crontab (filter
   ata's tagged lines out and rewrite the crontab; leave foreign lines
   intact).
2. Kill any orphan ata subprocesses spawned by cron firings.
3. Verify both: `crontab -l` shows no ata entries, and the process
   table shows no ata cron children. Both checks must come back empty
   before the run is considered finished.

Also dismiss leftover panel state, delete any surviving in-session
tasks and monitors, and kill the tmux session. A passing report with a
live crontab entry left behind is a failed run.
