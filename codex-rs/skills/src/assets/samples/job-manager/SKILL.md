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

All job management uses the `ata` CLI. Run these via the shell tool.

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
| `ata scheduler start` | Start the daemon (foreground) |
| `ata scheduler stop` | Graceful stop via PID |
| `ata scheduler status` | Check if daemon is running |

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

## IMPORTANT: Sandbox Limitations

**You are running inside a sandboxed ata session.** The sandbox blocks network access and kills child processes when the session ends. This means:

- **DO NOT run `ata jobs run`** from inside this session — it spawns `ata exec` which needs network to reach the LLM API. It will always fail with "stream disconnected" or "process exited with code 1".
- **DO NOT run `ata scheduler start`** from inside this session — the daemon process will be killed when the sandbox tears down, even with `--daemon`.
- **DO** create the job TOML file, validate it with `ata jobs show`, and then tell the user to run these commands in their normal terminal:

```
ata scheduler start --daemon
ata jobs run <job-name>        # optional: test one run
ata scheduler status           # verify daemon is running
```

## Workflow: Creating a Job for the User

When a user asks you to schedule something:

1. **Understand what they want**: the task, frequency, and any delivery/output requirements.
2. **Create the TOML file** by writing directly to `~/.ata/jobs/<job-name>.toml`. Use the format above.
3. **Validate** by running `ata jobs show <job-name>` to confirm it parses correctly.
4. **Tell the user** to run these in their normal terminal (NOT from this session):
   - `ata scheduler start --daemon` (if not already running)
   - `ata jobs run <job-name>` (to test)
   - `ata jobs history <job-name>` (to check results)
5. **Confirm** to the user: show them the schedule, next run time, and how to check results.

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

When a job needs to interact with an external service (Slack, email, webhooks, etc.), **always use browser automation first**. You have Playwright MCP tools that can automate the entire setup flow in the user's real Chrome browser.

### IMPORTANT: Always offer automation first

When external service setup is needed, you MUST proactively ask the user:

> "I can set this up automatically using browser automation — I'll navigate your Chrome, fill in the forms, and extract the credentials. You just watch. Want me to do that, or would you prefer to do it manually?"

**Default to automation.** Never give the user manual browser steps without first offering to do it for them with Playwright. The manual approach is a fallback, not the default.

### Browser automation (default)

Playwright MCP is configured in `~/.ata/config.toml` as `[mcp_servers.playwright]` with `--extension` mode. This connects to the **user's real Chrome** via the Playwright MCP Bridge extension — the user's Slack, GitHub, Google sessions are all available.

**Tools available** (prefixed `mcp__playwright__`):

| Tool | Use for |
|---|---|
| `browser_navigate` | Open a URL |
| `browser_snapshot` | Read current page state (accessibility tree) |
| `browser_click` | Click elements by ref |
| `browser_fill_form` | Fill input fields |
| `browser_evaluate` | Extract values from the DOM |
| `browser_run_code` | Run arbitrary Playwright code (for complex interactions like CodeMirror editors) |
| `browser_wait_for` | Wait for page transitions |

**Automation flow:**
1. Navigate to the service setup page
2. Snapshot to see current state
3. Click, fill, and interact to complete the flow
4. Extract credentials/URLs via `browser_evaluate`
5. Store credentials locally and wire into the job — zero user effort

**Tips from real usage:**
- Slack's manifest editor uses CodeMirror, not a plain `<textarea>`. Use `browser_run_code` with `page.evaluate()` to call `document.querySelector('.CodeMirror').CodeMirror.setValue(text)` instead of trying to click the textarea directly.
- Always snapshot after each action to confirm the page transitioned correctly.
- If a click times out, check the snapshot — an overlay or modal may be intercepting pointer events.

**If Playwright tools are not available** (extension not installed, server not configured), fall back to the manual approach below.

### Fallback: manual with least-friction

Only use this if browser automation is unavailable. Minimize manual steps:

1. **Open URLs in the user's browser** using `open <url>` (macOS) or `xdg-open <url>` (Linux).
2. **Provide app manifests / config files** that can be pasted in one shot.
3. **Never give more than 4-5 user-facing steps.**
4. **Do all local work silently** — then present only what the user must do manually.

### Slack webhook setup

**Automated (with Playwright — always try this first):**
1. `browser_navigate` to `https://api.slack.com/apps?new_app=1`
2. Select "From a manifest" tab, use `browser_run_code` with `page.evaluate` to set the JSON manifest in the CodeMirror editor
3. Click Next → review summary → click Create
4. Navigate to Incoming Webhooks → Add New Webhook → select channel → Allow
5. Extract the webhook URL via `browser_evaluate` on the URL textbox
6. Store it securely and wire it into the job — zero user effort

**Manual fallback (only if automation unavailable):**
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
