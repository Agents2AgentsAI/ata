# ATA agentic behavioral tests

ATA is tested by an agent, not by scripts.

Each component has a behavioral spec in `specs/` that explains what the
component contains, what it is for, and how it is supposed to behave. A
live agent (Claude or any agent with tmux + shell access) reads the
spec, drives the real ata binary — CLI commands directly, the TUI
through tmux, and the in-app agent through real model prompts — and
compares what it observes against the spec. The agent is expected to
probe beyond the listed behaviors: try edge cases, misuse, concurrency,
and malformed input on its own judgment. Anything that contradicts the
spec, fails silently, corrupts state, escapes containment, or misleads
the user is an issue.

There are no assertion scripts. A scripted test can only re-check what
its author thought of; the spec + live agent can find the bug class the
author missed. The spec is the contract, the agent run is the test, the
report is the deliverable.

## Run

```sh
./tests/tui/run-agentic.sh workspace
```

The runner builds ata if needed and launches Claude Code headlessly
with the spec-run prompt, then prints the report path and its issues
section. Extra args pass through to `claude` (e.g. `--model opus`).

Or ask an interactive agent directly:

> Run the behavioral spec at `codex-rs/tests/tui/specs/workspace.md`
> against `./target/debug/ata` and write the report.

The agent should:

1. Build `./target/debug/ata` if needed.
2. Read the spec top to bottom. Each section states expected behavior;
   the "How to test" section gives the probing strategy.
3. Exercise every behavior section, then spend real effort on
   adversarial probes the spec doesn't enumerate.
4. Write `reports/<ISO-date>-<component>.md` with: what was exercised,
   issues found (each with a reproduction), divergences between spec
   and binary, and what was not covered and why.
5. Never modify product code during a run. The report is the
   deliverable; the user decides what to fix.
6. Clean up everything it created (workspaces, tmux sessions, cron
   entries, config edits) regardless of pass/fail.

## Driving the TUI

Use tmux. The recipe:

```sh
tmux new-session -d -s probe -x 132 -y 40 "/abs/path/to/ata --yolo"
# poll until the pane shows "Agents2Agents ata" and no "esc to interrupt"
tmux send-keys -t probe "/workspace list" ; sleep 0.5 ; tmux send-keys -t probe Enter
tmux capture-pane -t probe -p          # visible pane
tmux capture-pane -t probe -p -S -3000 # with scrollback
tmux kill-session -t probe
```

When a probe involves the in-app model, the deterministic anchor is the
session JSONL (`~/.ata/sessions/.../rollout-*.jsonl`): one line per
event, including every tool/function call with arguments. Rendered text
can look right while the wrong tool was called — check the JSONL, not
just the pane.

## Structured verdict, rollup, and suite

The prose report is the human deliverable, but the agent run is only a *test*
if it produces a pass/fail. Every run also writes a machine-readable verdict
next to its report: `reports/<date>-<component>.verdict.json`.

Schema (all keys required):

```json
{
  "component": "reading-view",
  "spec": "tests/tui/specs/reading-view.md",
  "report": "tests/tui/reports/2026-06-12-reading-view.md",
  "verdict": "pass | partial | fail",
  "timestamp_utc": "2026-06-12T19:30:00Z",
  "binary_version": "ata 0.0.0",
  "capabilities_total": 11,
  "capabilities_passed": 11,
  "findings": [
    { "id": "rv-1", "severity": "high | medium | low",
      "capability": "Containment between reader and chat",
      "title": "intermediate exec cell leaks behind the reader",
      "status": "open" }
  ],
  "notes": "one line"
}
```

Verdict rule (the agent applies it, and must not soften a real defect to go
green): **fail** if any finding is `high`; **partial** if there are only
medium/low findings or some capabilities could not be exercised; **pass** only
if `findings` is empty and every spec capability was exercised.

**Fallbacks are failures.** A capability must work through its intended path.
If that path errors, returns nothing, or is unavailable and the agent only
reaches a good-looking result by working around it — shelling out, using a
different tool, doing the work by hand, retrying onto another strategy — that
is a `high` finding, *even when the final answer is correct*. A feature that
"works" only via a workaround is broken. Judge from the JSONL, not the
outcome: scan for failures the agent silently recovered from (tool or
sub-agent spawn errors, `could not …` / `failed to …`, non-zero exits, a
strategy switch mid-task) and file each one against the capability that
failed. A correct outcome never excuses a recovered failure. (This is not the
same as the agent freely *choosing* a path the spec permits — that is fine;
the defect is the intended path *failing* and being silently substituted.)

```sh
./tests/tui/run-agentic.sh reading-view --isolated   # one spec → report + verdict (exit 3 on fail)
./tests/tui/run-suite.sh reading-view goals workspace # several, serial, isolated → rollup
./tests/tui/run-suite.sh all                          # every spec (expensive, quota-bound)
./tests/tui/rollup.sh                                 # regenerate STATUS.md from latest verdicts
./tests/tui/run-due.sh --isolated                     # let an agent pick specs off the diff, run them
./tests/tui/run-due.sh --dry-run                      # print the agent's spec picks, run nothing
./tests/tui/run-due.sh --all --isolated               # drift sweep: every spec (put this on a timer)
```

`run-due.sh` is the cheap-cadence entry point: rather than a path->spec mapping
table, it shows an agent the diff since the last sweep (a cursor in the
gitignored `.last-swept`) and lets it choose which specs the change could
affect. Its judgment reads intent a glob can't, and a feature added/changed/
removed surfaces in the diff so the affected spec gets picked with no rule to
maintain. Run it per push. The case it cannot see — drift with no local diff
(an upstream merge, a model swap) — is covered by `--all` on a timer
(nightly/weekly), which re-judges every spec against its contract regardless of
what the cheap layer says. One selector turn per push, one full sweep on a
timer.

`rollup.sh` reads the newest verdict per component and regenerates
`STATUS.md` — a table of every spec with its verdict, date, and finding
counts, so coverage gaps (specs never run) and regressions are visible at a
glance. Runs are serial because the driver agent and ata's own model share one
credential; parallel runs exhaust quota.

These do not replace the deterministic Rust unit tests. Unit tests are the
fast regression guard for specific cases; the agentic suite is the behavioral
layer where the agent decides what to probe and judges against the spec. Both
matter; the agentic layer is the one that catches the bug class the unit-test
author didn't think of.

## How fixes must be written

Every fix that comes out of a test report MUST follow
`ENGINEERING-PRINCIPLES.md` in this directory: fix at the choke point, prefer
native/platform capability over workarounds, design the failure out rather
than handle it, centralize boundary transforms, fail fast and typed,
root-cause before patching, and never weaken a test or spec to go green. Read
it before fixing anything.

**Spec-first discipline.** The agent only tests what the spec says. So when a
fix changes or clarifies a behavior — or when a bug existed because the
contract was never written down — update the relevant `specs/*.md` *first* (or
in the same change), then fix the code. A fix that lands without its spec
update means the agentic suite can never catch a regression of it. This is how
the suite's coverage stays honest instead of drifting behind the code.

## Files

- `specs/` — one behavioral spec per component. The contract.
- `reports/` — one report per agent run. Gitignored by default;
  selectively commit notable runs (baselines, regression evidence).
- `PLAN.md` — legacy keystroke-level test plan from the old scripted
  system. Kept only as raw material: when writing a new spec for a
  component, mine its TR sections for known behaviors and divergences,
  then delete those sections. Workspace is already converted.

## Specs so far

| Component | Spec |
|---|---|
| Workspace | `specs/workspace.md` |
| Reading view / document reader | `specs/reading-view.md` |
| Knowledge base | `specs/knowledge-base.md` |
| Code intelligence | `specs/code-intel.md` |
| Research tools | `specs/research.md` |
| Repo analysis | `specs/repo-analysis.md` |
| Scheduling | `specs/scheduling.md` |
| Goals | `specs/goals.md` |
| Model providers + auth | `specs/model-providers.md` |
| Feature flags | `specs/feature-flags.md` |
| Session continuity (resume / fork / clear / compaction) | `specs/session-continuity.md` |
| PDF ingestion (attach, extraction, providers, subagents) | `specs/pdf-ingestion.md` |
| Trajectory fork cards + live whiteboard | `specs/cards-whiteboard.md` |
| Skills system (roots, advertising, injection, /skills) | `specs/skills-system.md` |
| Voice mode + TTS / karaoke | `specs/voice-tts.md` |
| Subagents / multi-agent (/agent, /subagents, /side) | `specs/subagents.md` |
| ATA account / Supabase / mobile pairing | `specs/account-supabase.md` |

Everything else (slash commands, scheduling internals, …) still
lives in `PLAN.md` form and needs conversion.
