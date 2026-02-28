---
name: job-manager
description: "Create, manage, and monitor scheduled jobs. Use when a user wants to run something on a schedule (e.g., 'run this every 2 hours', 'set up a daily digest', 'create a cron job for...'), check job status, view run history, or manage the scheduler daemon. This skill covers the full job lifecycle: creation, scheduling, monitoring, and cleanup."
metadata:
  short-description: Manage scheduled background jobs
policy:
  allow_implicit_invocation: true
---

# Scheduled Jobs

Scheduled jobs let users run skills or prompts automatically on a cron schedule, at fixed intervals, or in response to events (file changes, HTTP polls, webhooks). The scheduler daemon runs in the background and fires jobs when they are due.

## Architecture

```
~/.ata/jobs/*.toml          Job definitions (one TOML file per job)
~/.ata/scheduler.sqlite     Job metadata, run history
~/.ata/scheduler.pid        Daemon PID file (single-instance guard)
~/.ata/scheduler/runs/      Full output files per run
```

Each job fires by spawning `ata exec --full-auto --ephemeral` as a subprocess. The agent handles everything: executing the task, using tools, and producing output. This means any capability the agent has interactively, it also has in scheduled jobs.

## CLI Commands

All job management uses the `ata` CLI.

**CRITICAL: Always run `ata` directly (e.g. `ata jobs list`, `ata jobs run <name>`). NEVER use `cargo run -p codex-cli` — it recompiles the binary, is extremely slow, and bypasses the installed daemon. The `ata` command is already available in PATH.**

### Job management

| Command | What it does |
|---|---|
| `ata jobs list` | List all jobs with status, run count, next run |
| `ata jobs show <name>` | Show job definition + recent runs |
| `ata jobs create <name>` | Create a template TOML in `~/.ata/jobs/` |
| `ata jobs delete <name>` | Delete job TOML + DB records |
| `ata jobs pause <name>` | Pause scheduling (job stays in DB) |
| `ata jobs resume <name>` | Resume a paused job |
| `ata jobs run <name>` | Trigger an immediate run now |
| `ata jobs history <name>` | Show run history with status and duration |
| `ata jobs logs <run-id>` | Show full output of a specific run |

### Scheduler daemon

| Command | What it does |
|---|---|
| `ata scheduler install` | Install daemon as a launchd service (macOS). One-time setup. |
| `ata scheduler uninstall` | Remove daemon from launchd |
| `ata scheduler start` | Start the daemon (foreground, for debugging) |
| `ata scheduler start -d` | Start the daemon in the background via launchd |
| `ata scheduler stop` | Graceful stop via PID |
| `ata scheduler status` | Check if daemon is running |

### Sandbox and daemon delegation

**Important**: When running inside an ata session (which is sandboxed), you **cannot** start the daemon or run jobs directly. The sandbox blocks network access for child processes.

Instead, `ata jobs run <name>` automatically delegates to the daemon if it is running:
- It inserts a pending run into the database
- The daemon (running outside the sandbox via launchd) picks it up and executes it
- The CLI polls until the job completes and shows the result

If the daemon is not running, `ata jobs run` will warn you and attempt direct execution (which will fail inside a sandbox). Tell the user to run `ata scheduler install` from their terminal (outside ata) to set up the daemon.

**Do NOT run `ata scheduler start` from inside an ata session** — it will fail or create a sandboxed daemon that can't reach the network.

## Job TOML Format

Jobs are TOML files in `~/.ata/jobs/`. The filename (without `.toml`) is the job ID.

### Minimal example — cron schedule with inline prompt

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

### Skill-based job

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

### Interval-based job

```toml
name = "health-check"
description = "Check if API is responding"
enabled = true

[task]
prompt = "curl https://api.example.com/health and report the status"

[schedule]
interval_minutes = 30

[execution]
timeout_minutes = 5
skip_if_running = true
```

### Event-triggered jobs

#### File watcher

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

#### HTTP poll

```toml
name = "price-alert"
description = "Alert when BTC price changes significantly"
enabled = true

[task]
prompt = "Check the latest BTC price from the data and alert if it moved more than 5% in the last hour"

[schedule]
[schedule.trigger]
type = "http_poll"
url = "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd"
interval_seconds = 300
change_path = "bitcoin.usd"

[execution]
timeout_minutes = 5
```

#### Webhook

```toml
name = "deploy-hook"
description = "Run post-deploy checks when webhook fires"
enabled = true

[task]
prompt = "Run the smoke test suite and report results"
cwd = "/Users/me/projects/myapp"

[schedule]
[schedule.trigger]
type = "webhook"
path = "/hooks/deploy"

[execution]
timeout_minutes = 20
```

## TOML Field Reference

### Top-level

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | string | yes | — | Display name |
| `description` | string | no | `""` | Description |
| `enabled` | bool | no | `true` | Whether the job is active |

### `[task]`

| Field | Type | Required | Description |
|---|---|---|---|
| `skill` | string | one of skill/prompt | Skill name from `~/.ata/skills/` |
| `prompt` | string | one of skill/prompt | Inline prompt text |
| `context` | string | no | Extra instructions prepended to the prompt |
| `cwd` | path | no | Working directory for execution |

### `[task.config]`

| Field | Type | Default | Description |
|---|---|---|---|
| `model` | string | (default model) | LLM model to use |
| `sandbox_mode` | string | (default sandbox) | `"read-only"`, `"workspace-write"`, or `"danger-full-access"` |

### `[schedule]` — exactly one of these three

| Field | Type | Description |
|---|---|---|
| `cron` | string | Standard 5-field cron: `"min hour dom month dow"` |
| `interval_minutes` | integer | Fixed interval in minutes (must be > 0) |
| `[schedule.trigger]` | table | Event-based trigger (see examples above) |

### `[execution]`

| Field | Type | Default | Description |
|---|---|---|---|
| `timeout_minutes` | integer | `30` | Max run time before kill |
| `max_retries` | integer | `0` | Retry count on failure |
| `retry_delay_seconds` | integer | `60` | Wait between retries |
| `skip_if_running` | bool | `false` | Skip if previous run still active |

## Cron Syntax

Standard 5-field Unix cron is supported:

```
┌───────────── minute (0-59)
│ ┌───────────── hour (0-23)
│ │ ┌───────────── day of month (1-31)
│ │ │ ┌───────────── month (1-12)
│ │ │ │ ┌───────────── day of week (0-6, Sun=0)
│ │ │ │ │
* * * * *
```

| Expression | Meaning |
|---|---|
| `0 7 * * *` | Daily at 7:00 AM |
| `0 9 * * 1` | Every Monday at 9:00 AM |
| `*/15 * * * *` | Every 15 minutes |
| `0 0 1 * *` | First day of each month at midnight |
| `0 8-17 * * 1-5` | Every hour 8 AM–5 PM on weekdays |
| `30 6,18 * * *` | At 6:30 AM and 6:30 PM daily |

## Workflow: Creating a Job for the User

When a user asks you to schedule something:

1. **Understand what they want**: the task, frequency, and any delivery/output requirements.
2. **Check daemon status**: run `ata scheduler status`. If not running, tell the user to run `ata scheduler install` from their terminal (outside this session). Do NOT attempt to start it yourself from inside a sandboxed session.
3. **Create the TOML file** by writing directly to `~/.ata/jobs/<job-name>.toml`. Use the format above.
4. **Validate** by running `ata jobs show <job-name>` to confirm it parses correctly.
5. **Test** by running `ata jobs run <job-name>` to verify it works. If the daemon is running, this delegates to it automatically.
6. **Confirm** to the user: show them the schedule, next run time, and how to check results.

**CRITICAL: NEVER run job scripts, curl commands, or any job logic directly from inside this session.** You are inside a sandbox — network access, browser automation, and external API calls will all fail or hang. **Always use `ata jobs run <name>`** to test jobs. This delegates execution to the daemon which runs outside the sandbox.

### Choosing the schedule type

- User says "every N hours/minutes" → use `interval_minutes`
- User says "daily at 7am" or gives a time pattern → use `cron`
- User says "when files change" → use `file_watch` trigger
- User says "check this URL for changes" → use `http_poll` trigger
- User says "when I hit an endpoint" → use `webhook` trigger

### Choosing skill vs prompt

- If the task matches an existing skill (e.g., paper-discovery, kb), use `skill`
- For custom one-off tasks, use `prompt` with detailed instructions
- The `context` field adds extra instructions to either approach

### Sandbox mode

- Jobs that only read/write local files: default sandbox is fine
- Jobs that need network access (APIs, web scraping): use `sandbox_mode = "danger-full-access"`
- Jobs that need to write outside the working directory: use `sandbox_mode = "workspace-write"` or `"danger-full-access"`

## Connecting External Services (Slack, etc.)

When a job needs to interact with an external service (Slack, email, webhooks, etc.), **always minimize user friction**. Follow this principle: do everything you can locally, and for the part that requires the user, give them the most automated single-step option first.

### Browser automation (preferred)

Playwright MCP is configured with `--extension` mode, which connects to the **user's real Chrome** via the Playwright MCP Bridge extension. This means the agent can automate authenticated browser flows — the user's Slack, GitHub, Google sessions are all available.

**When to use browser automation:**
- Setting up Slack apps/webhooks
- Creating GitHub tokens
- Any multi-step web UI flow where the user is already logged in

**How to use it:**

Playwright MCP is configured in `~/.ata/config.toml` as `[mcp_servers.playwright]`. The agent has access to tools prefixed with `mcp__playwright__`:

1. Use `mcp__playwright__browser_navigate` to open the service URL
2. Use `mcp__playwright__browser_snapshot` to see the current page state
3. Use `mcp__playwright__browser_click`, `mcp__playwright__browser_fill_form`, `mcp__playwright__browser_evaluate` to interact
4. The user watches their Chrome as the agent clicks through the flow

**If extension mode is not available** (extension not installed), the tools will fall back to launching a sandboxed browser. If that happens, fall back to the manual approach below.

### Fallback: manual with least-friction

If browser automation isn't available, minimize manual steps:

Rules:
1. **Open URLs in the user's browser** using `open <url>` (macOS) or `xdg-open <url>` (Linux). Their default browser is already authenticated.
2. **Provide app manifests / config files** that can be pasted in one shot, instead of step-by-step UI walkthroughs.
3. **Never give more than 4-5 user-facing steps.** If you're writing more, you're not automating enough.
4. **Do all local work silently** — write config files, set permissions, wire scripts — then present only what the user must do manually.

### Slack webhook setup

**Automated (with Playwright extension):**
1. `browser_navigate` to `https://api.slack.com/apps?new_app=1`
2. Select "From a manifest" → paste the YAML manifest via `browser_fill_form`
3. Click through creation, enable Incoming Webhooks, add to workspace
4. Extract the webhook URL from the page via `browser_evaluate`
5. Store it securely and wire it into the job — zero user effort

**Manual fallback:**
1. `open "https://api.slack.com/apps?new_app=1"`
2. Give them a ready-to-paste app manifest:
   ```yaml
   display_information:
     name: <descriptive name>
     description: <what it does>
   features:
     bot_user:
       display_name: <name>
       always_online: false
   oauth_config:
     scopes:
       bot:
         - incoming-webhook
   settings:
     org_deploy_enabled: false
     socket_mode_enabled: false
     token_rotation_enabled: false
   ```
3. Tell them: "Create from manifest → Incoming Webhooks → Add → pick channel → copy URL → paste here"
4. Once they paste the URL, store it securely and wire it into the job.

### Other services

Apply the same automation-first pattern:
- **GitHub tokens**: automate via Playwright, or fallback to `open "https://github.com/settings/tokens/new?scopes=repo&description=ata-job"` with pre-filled scope
- **API keys**: offer to store in `~/.ata/secrets/` with locked permissions
- **Email**: check if `sendmail`/`msmtp` is configured before suggesting external services

## Checking Job Results

After a job runs:

1. `ata jobs history <name>` — see all runs with status (success/failed/timeout/skipped)
2. `ata jobs logs <run-id>` — see full output of a specific run (use the first 8 chars of the run ID)
3. Output files are also stored at `~/.ata/scheduler/runs/<run-id>.md`
