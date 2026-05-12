# ATA TUI tests

Agent-driven functional regression tests for the TUI.

## Run

Ask any Claude / agent that has the `ata-tmux-test` skill installed:

> Run the TUI test plan at `codex-rs/tests/tui/PLAN.md` and write the report.

The agent will:

1. Build `./target/debug/ata`.
2. For each `## TR-NNN:` section in `PLAN.md`, follow Setup → Action → Expect.
3. Write `reports/<ISO-datetime>.md` with per-test status, the captured pane
   snapshots for any failing test, and a summary header.
4. Update `reports/latest.md` to point at the new file.

The agent should NOT modify code while running tests. If a test fails, the
report is the deliverable; the user decides what to fix.

## Files

- `PLAN.md` — test definitions. Committed.
- `reports/` — per-run output. Gitignored by default (see `.gitignore`).
  Selectively `git add reports/<file>.md` to share a notable run (baseline,
  regression evidence, etc.).
- `reports/.gitkeep` — keeps the dir in git.
- `reports/latest.md` — symlink to the most recent report (gitignored).

## Why markdown instead of `cargo test`

These tests need a real terminal, real ratatui rendering, real tmux pane
behavior, and a real network call to the agent backend. `cargo test`
can't provide any of those. The markdown plan + tmux capture is the
cheapest way to get end-to-end coverage.
