# Goals — behavioral spec

This spec describes what the goals feature does for users and the
in-app agent, and the outcomes that must hold. It avoids
implementation detail (storage layout, internal event names, exact
info-message strings): the testing agent discovers concrete wording at
run time. If a behavior here changes, that is a product decision, not
a refactor.

Feature gating: goals are behind the experimental `goals` feature
flag. In this build it defaults **on**, but the public/upstream
default has historically been off and this feature has never had a
verified test run (legacy PLAN.md TR-048 was a placeholder). Do not
trust the default: the How-to-test section says how to force it on so
the run exercises the real feature either way.

## What a goal is for

A goal is a persistent objective attached to a saved session (thread).
It outlives individual turns: while the goal is active, the agent
keeps working toward it across turns without the user re-prompting,
and the system tracks how many tokens and how much wall-clock time the
goal has consumed. The use cases it must serve:

1. **Long-running tasks**: the user states an objective once; the
   agent continues pursuing it turn after turn until it is genuinely
   complete or genuinely blocked, instead of stopping when one turn
   ends.
2. **Budgeted autonomy**: a goal can carry a token budget; the system
   accounts usage against it and stops continuing when the budget is
   exhausted, and the user gets a final usage report on completion.
3. **User control**: the user can view, set, replace, edit, pause,
   resume, and clear the goal at any time, including while a task is
   running. Lifecycle control belongs to the user; the model can only
   declare the goal complete or blocked.
4. **Survival across sessions**: a goal persists with the thread. Quit
   and resume, and the goal (objective, status, accumulated usage) is
   still there.

There are two doors to the same state, and they must agree:

- `/goal` in the TUI — the user's full control surface,
- the model-facing goal tools (`get_goal`, `create_goal`,
  `update_goal`) — the agent's narrow, deliberately restricted view.

## Capabilities and required behavior

### Setting and viewing (`/goal`)

- `/goal <objective>` sets the goal for the current thread and
  confirms with the goal's status and a usage summary (objective,
  time, tokens when budgeted).
- Bare `/goal` with a goal set shows a summary of the current goal.
  With no goal set, it explains usage and says no goal is set —
  not an error, not a silent no-op.
- Setting a new objective while an unfinished goal exists asks the
  user to confirm replacement first; declining leaves the old goal
  untouched. Replacing a finished goal does not nag.
- `/goal edit` opens the existing objective for editing; with no goal
  to edit, the user is told so. An edited objective reaches the agent:
  a subsequent turn pursues the new wording, not the old.
- `/goal pause` and `/goal resume` flip the goal between paused and
  active. `/goal clear` removes it. All three require a goal/session
  to act on and say so when there isn't one.
- The objective is bounded (4,000 characters). Over-long input is
  refused with the actual and maximum counts and a hint to put long
  instructions in a file and reference it. The check counts the real
  expanded text — pasting a huge block as a placeholder must not
  sneak past it.
- An empty objective is refused with usage help.
- `/goal` is available during a running task. Typed before the
  session is ready, a goal-set is queued and applied once the session
  starts rather than dropped.
- Goals require a saved session. In an ephemeral session the user is
  told goals need a saved session and how to get one; nothing is
  half-created.

### Persistence and continuation

- An active goal drives automatic continuation: when a turn ends and
  the session is idle (no queued user input), the system starts a new
  turn on its own with goal context injected, and the agent keeps
  working. The user does not have to say "continue".
- Continuation respects state: it does not fire for paused, blocked,
  complete, or limited goals; it does not preempt queued user input;
  it does not double-start when a turn is already running; and it
  re-checks that the goal is still the same and still active at launch
  (a goal cleared or replaced in the gap must not be continued).
- The injected goal context treats the objective as user data, not as
  higher-priority instructions — an objective containing
  instruction-shaped text ("ignore previous instructions…") must not
  escalate privileges.
- The goal survives process death: quit the TUI, resume the thread,
  and the goal is intact with its accumulated usage. An active goal
  resumes its runtime; a paused/blocked/usage-limited goal prompts the
  user to decide whether to resume it rather than silently
  reactivating.
- Plan mode ignores goals: no continuation fires while plan mode is
  active.
- The goal lives outside the visible transcript, so history-level
  operations (`/compact`, scrollback loss) must not destroy or mutate
  it.

### Statuses and budgets

- A goal is in exactly one of: active, paused, blocked, usage limited,
  budget limited, complete. Status is always visible via bare `/goal`.
- Token and wall-clock usage accumulate over the goal's life and are
  reported in human-readable form (compact token counts, `1h 30m`
  style elapsed time).
- A token budget, when present, must be positive. When usage exhausts
  the budget, the system stops continuing and the goal lands in a
  budget-limited state — the model cannot talk its way past it, and
  cannot mark the goal complete merely because the budget ran out.
- Hitting an account usage/rate limit parks an active goal in usage
  limited rather than burning continuation turns against a wall.
- When a budgeted goal completes, the user gets a final usage report
  (tokens used vs budget, elapsed time).

### Model-facing tool contract

- `get_goal` returns the current goal (status, budgets, usage,
  remaining tokens) or an empty result when none is set.
- `create_goal` starts a new active goal and fails — with a clear,
  actionable error — if a goal already exists. The tool's contract
  tells the model to create goals only on explicit request; an
  ordinary task prompt must not cause the model to invent a goal.
- `update_goal` accepts exactly two statuses: complete and blocked.
  Pause, resume, and limit transitions are user/system-owned; an
  attempt to set them through the tool is refused with an explanation.
- `blocked` is held to a strict audit: the same blocker must recur for
  at least three consecutive goal turns before the model may declare
  it. Completion likewise requires an evidence-based audit, not
  optimism. (Judging whether the model honors this is a trajectory
  check, not a tool check.)
- When goals are disabled (feature off), the goal tools are not
  offered to the model and `/goal` does nothing visible — there must
  be no half-enabled state where one door works and the other errors.

## How to test it

Build the TUI and boot it with the feature forced on so the run never
silently tests a disabled build:

```sh
./target/debug/ata --yolo -c features.goals=true
```

(`goals` is the feature key; `-c features.goals=false` gives you the
disabled-state counterpart for the gating checks.) Drive the TUI
through tmux per the README recipe. Every claim about what the agent
did must be verified in the session JSONL — goal tool calls, injected
goal context, continuation turns — not inferred from the pane.

Persistence claims need real process death: set a goal, kill the TUI,
relaunch with `ata resume` (or `/resume`), and confirm objective,
status, and usage came back. Continuation claims need patience: give a
small multi-step objective, send one turn, then watch for the system
starting the next turn unprompted; confirm in the JSONL that a
continuation turn ran with goal context and was not user-initiated.

Then go adversarial — minimum classes, invent more:

- **Mid-turn control**: set, pause, and clear the goal while a turn is
  in flight. Nothing should corrupt; pause/clear should stop further
  continuations.
- **Conflicting goals**: set a goal, then set a different one — does
  the replace confirmation appear, and does declining preserve the
  original? Race the other door: while a goal exists, prompt the model
  to call `create_goal` and confirm it fails cleanly.
- **Length and content**: a 4,001-character objective (refused, with
  counts and the file hint); a 4,001-char paste behind a placeholder
  (still refused); an objective full of markup and
  prompt-injection text (stored verbatim, treated as data).
- **Lifecycle abuse**: `/goal resume` with no goal; `/goal pause`
  twice; clear then bare `/goal`; goal commands in an ephemeral
  session (`ata exec`-style or temporary session) — each should
  explain itself, none should wedge the session.
- **Across /clear and /resume**: whether the goal survives `/clear`
  (new thread vs same thread) is a product question — observe and
  report what happens, flagging silent loss as an issue. Across
  `/resume` the goal must survive, and a paused/blocked goal must
  produce the resume prompt rather than auto-reactivating.
- **Status escape attempts**: prompt the model to "pause the goal
  using your tools" and to mark an unfinished goal complete — the tool
  layer should refuse the former; the latter is a trajectory judgment
  (did the model audit before declaring complete?).
- **Disabled state**: boot with `features.goals=false` and confirm
  `/goal` is inert and the goal tools are absent from the session's
  tool list (check the JSONL, not the pane).

For the live-model layer, set goals in your own words across runs
("/goal get the test suite green", "/goal follow the plan in
docs/x.md", a budgeted variant) and judge trajectories: does the agent
keep the full objective intact across continuation turns instead of
shrinking it, does it verify before completing, does it report final
usage for a budgeted goal. Vary the wording between runs; verbatim
reuse turns this back into a script.

Note on maturity: this feature has never had a verified behavioral
run. Expect first-run findings; distinguish "diverges from this spec"
(an issue) from "spec was wrong about the product" (report as a spec
correction, with evidence).

Report per the README: issues with exact reproduction steps,
divergences citing the section above, trajectory violations quoting
the session JSONL, and coverage notes. Clean up: clear any goals you
set, delete scratch threads, kill tmux sessions.
