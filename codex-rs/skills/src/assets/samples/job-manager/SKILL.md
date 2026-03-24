---
name: job-manager
description: "Create, manage, and monitor scheduled jobs. Use when a user wants to run something on a schedule (e.g., 'run this every 2 hours', 'set up a daily digest', 'create a cron job for...'), check job status, view run history, or manage the scheduler daemon. This skill covers the full job lifecycle: creation, scheduling, monitoring, and cleanup."
metadata:
  short-description: Manage scheduled background jobs
policy:
  allow_implicit_invocation: true
---

# Scheduled Jobs

Scheduled jobs run skills or prompts automatically on a cron schedule, at fixed intervals, or in response to events (file changes, HTTP polls, webhooks). The scheduler daemon runs in the background and fires jobs when due.

## Architecture

```
~/.ata/jobs/*.toml          Job definitions (one TOML file per job)
~/.ata/scheduler.sqlite     Job metadata, run history
~/.ata/scheduler.pid        Daemon PID file (single-instance guard)
~/.ata/scheduler/runs/      Full output files per run
```

Each job fires by spawning `ata exec --full-auto --ephemeral` as a subprocess. Any capability the agent has interactively, it also has in scheduled jobs.

## CLI Commands

All job management uses the `ata` CLI.

**CRITICAL: Always run `ata` directly (e.g. `ata jobs list`, `ata jobs run <name>`). NEVER use `cargo run -p codex-cli` — it recompiles the binary, is extremely slow, and bypasses the installed daemon. The `ata` command is already available in PATH.**

If unsure which subcommand matches the user's intent, use `ata jobs search-commands "<intent>"` or `ata scheduler search-commands "<intent>"` first.

### Job management

| Command | What it does |
|---|---|
| `ata jobs list` | List all jobs with status, run count, next run |
| `ata jobs show <name>` | Show job definition + recent runs |
| `ata jobs create <name>` | Create a template TOML in `~/.ata/jobs/` |
| `ata jobs delete <name>` | Delete job TOML + DB records |
| `ata jobs pause <name>` / `resume <name>` | Pause/resume scheduling |
| `ata jobs run <name>` | Trigger an immediate run now |
| `ata jobs history <name>` | Show run history with status and duration |
| `ata jobs logs <run-id>` | Show full output of a specific run |

### Scheduler daemon

| Command | What it does |
|---|---|
| `ata scheduler install` / `uninstall` | Install/remove daemon as a launchd service (macOS). One-time setup. |
| `ata scheduler start` | Start daemon foreground (debugging) |
| `ata scheduler start -d` | Start daemon background via launchd |
| `ata scheduler stop` | Graceful stop via PID |
| `ata scheduler status` | Check if daemon is running |

### Sandbox and daemon delegation

**Important**: When running inside an ata session (sandboxed), you **cannot** start the daemon or run jobs directly. The sandbox blocks network access for child processes.

`ata jobs run <name>` automatically delegates to the daemon if running: it inserts a pending run, the daemon (outside sandbox via launchd) picks it up, and the CLI polls until completion.

If the daemon is not running, tell the user to run `ata scheduler install` from their terminal (outside ata). **Do NOT run `ata scheduler start` from inside an ata session** — it will fail or create a sandboxed daemon that can't reach the network.

## Job TOML Format

Jobs are TOML files in `~/.ata/jobs/`. The filename (without `.toml`) is the job ID. Each job needs a `[task]` (with either `skill` or `prompt`), a `[schedule]`, and optionally `[execution]`.

### Example: cron schedule with inline prompt

```toml
name = "daily-summary"
description = "Summarize today's git activity"
enabled = true

[task]
prompt = """
Look at the git log for today in ~/projects/myapp and write
a summary of what changed. Save it to ~/summaries/today.md.
"""
cwd = "/Users/me/projects/myapp"

[schedule]
cron = "0 18 * * *"

[execution]
timeout_minutes = 15
```

### Example: skill-based job

```toml
name = "research-digest"
description = "Weekly research paper digest"
enabled = true

[task]
skill = "paper-discovery"
context = "Focus on recent advances in reinforcement learning for robotics"

[schedule]
cron = "0 9 * * 1"

[task.config]
model = "o3"
sandbox_mode = "danger-full-access"

[execution]
timeout_minutes = 45
max_retries = 1
```

### Example: event trigger (file watcher)

```toml
name = "on-save-lint"
description = "Lint when source files change"
enabled = true

[task]
prompt = "Run cargo clippy on the workspace and report any warnings"
cwd = "/Users/me/projects/myapp"

[schedule]
[schedule.trigger]
type = "file_watch"
path = "/Users/me/projects/myapp/src"
pattern = "*.rs"
debounce_seconds = 10

[execution]
timeout_minutes = 10
skip_if_running = true
```

Other trigger types: `http_poll` (fields: `url`, `interval_seconds`, `change_path`) and `webhook` (fields: `path`). Use `interval_minutes` in `[schedule]` for fixed-interval jobs instead of cron.

### Key fields not obvious from examples

- `[task]`: use `skill` (name from `~/.ata/skills/`) OR `prompt` (inline text), plus optional `context` for extra instructions
- `[task.config]`: `model` (LLM model), `sandbox_mode` (`"read-only"` | `"workspace-write"` | `"danger-full-access"`)
- `[execution]`: `timeout_minutes` (default 30), `max_retries` (default 0), `retry_delay_seconds` (default 60), `skip_if_running` (default false)

## Workflow: Creating a Job

When a user asks you to schedule something:

1. **Check daemon status**: `ata scheduler status`. If not running, tell the user to run `ata scheduler install` from their terminal (outside this session).
2. **Create the TOML file** by writing directly to `~/.ata/jobs/<job-name>.toml`.
3. **Validate**: `ata jobs show <job-name>` to confirm it parses.
4. **Test**: `ata jobs run <job-name>` to verify. The daemon handles execution.
5. **Confirm** to the user: schedule, next run time, and how to check results.

**CRITICAL: NEVER run job scripts, curl commands, or any job logic directly from inside this session.** You are inside a sandbox. **Always use `ata jobs run <name>`** to test — this delegates to the daemon which runs outside the sandbox.

### Choosing schedule type

- "every N hours/minutes" -> `interval_minutes`
- "daily at 7am" or a time pattern -> `cron`
- "when files change" -> `file_watch` trigger
- "check this URL for changes" -> `http_poll` trigger
- "when I hit an endpoint" -> `webhook` trigger

### Sandbox mode

- Read/write local files only: default sandbox is fine
- Network access needed (APIs, web scraping): `sandbox_mode = "danger-full-access"`
- Write outside working directory: `"workspace-write"` or `"danger-full-access"`

## Connecting External Services

When a job needs an external service (Slack, GitHub, etc.), minimize user friction: do everything possible locally, give the most automated option first.

### Browser automation (preferred)

Playwright MCP (`mcp__playwright__*` tools) connects to the user's real Chrome via the extension, with their existing authenticated sessions. Use it for multi-step web UI flows (creating Slack apps/webhooks, GitHub tokens, etc.): navigate, snapshot, click, fill forms, extract results. If extension mode is unavailable, fall back to manual.

### Manual fallback

1. Open URLs in the user's browser with `open <url>` (macOS) / `xdg-open` (Linux) -- they're already authenticated
2. Provide ready-to-paste config/manifests instead of step-by-step UI walkthroughs
3. Never give more than 4-5 user-facing steps
4. Do all local work silently (write configs, set permissions), then present only what requires manual action
5. API keys: offer to store in `~/.ata/secrets/` with locked permissions

## Checking Job Results

1. `ata jobs history <name>` — see all runs with status (success/failed/timeout/skipped)
2. `ata jobs logs <run-id>` — full output of a specific run (first 8 chars of run ID)
3. Output files also at `~/.ata/scheduler/runs/<run-id>.md`
