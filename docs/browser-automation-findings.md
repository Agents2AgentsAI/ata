# Browser Automation for External Service Setup

## Problem

When the agent needs the user to set up an external service (Slack webhook, GitHub token, etc.), the user has to manually navigate a web UI. We investigated whether Playwright (via MCP) could automate this — clicking through the setup flow on behalf of the user.

## What We Tried

We have Playwright MCP tools available (`browser_navigate`, `browser_click`, `browser_snapshot`, `browser_run_code`, etc.). We tried navigating to `https://api.slack.com/apps` to create a Slack app.

## What Happened

Playwright opened its **own sandboxed Chromium browser**. The Slack page said:

> "You'll need to sign in to your Slack account to create an application."

The user was not logged in because Playwright's browser is completely isolated from the user's actual browser.

## Why It Can't Work

| Approach | Why It Fails |
|---|---|
| **Playwright default** | Opens sandboxed Chromium with no cookies/auth. User is not logged into anything. |
| **Playwright + Chrome profile** | Chrome locks its profile directory while running (`~/Library/Application Support/Google/Chrome/Default/`). Two Chrome instances cannot share one profile — the second will crash or corrupt session data. |
| **Playwright + copy of Chrome profile** | Copying a locked profile gives an inconsistent snapshot. Slack auth cookies are short-lived and may not survive the copy. |
| **Playwright + Chrome remote debugging** | Requires Chrome to have been launched with `--remote-debugging-port=<port>`. Normal user Chrome isn't started this way. We can't retroactively enable it on a running browser. |
| **Cookie extraction** | Chrome encrypts cookies via macOS Keychain (`CryptoAPI` / `v10` prefix). Decrypting them requires either: (1) Keychain access prompt (user interaction), or (2) the Keychain encryption key (requires root or user password). Even if extracted, injecting them into Playwright's different browser storage format is fragile. |

## The Right Solution

Use `open <url>` (macOS) or `xdg-open <url>` (Linux) to open the URL in the user's **default browser**. This is the correct approach because:

1. **Already authenticated** — the user's default browser already has active sessions for Slack, GitHub, Google, etc.
2. **Zero setup** — no installation, no configuration, no profile copying.
3. **One command** — `open "https://api.slack.com/apps?new_app=1"` opens the exact page they need.

Combined with providing a copy-pasteable manifest (for Slack) or pre-filled URL parameters (for GitHub tokens), this minimizes the user's manual effort to 3-4 clicks.

### Concrete example: Slack webhook setup

```sh
# Agent runs this — Chrome opens the right page
open "https://api.slack.com/apps?new_app=1"
```

Agent provides:
```yaml
display_information:
  name: My Alert Bot
  description: Posts alerts to Slack
features:
  bot_user:
    display_name: My Alert Bot
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

User does: paste manifest, click Create, Add Webhook, pick channel, copy URL, paste back. **4 actions total.**

## When Playwright IS Useful

Playwright MCP is still valuable for:
- **Web scraping** — fetching data from public pages (no auth needed)
- **Testing** — validating that a web UI or API endpoint returns expected content
- **Form automation on public sites** — filling out forms that don't require login
- **Screenshots** — capturing the state of a page for debugging or documentation

It's the wrong tool when you need the user's **existing browser session**.

## Future Possibilities

1. **Slack CLI** — `slack create --manifest manifest.yaml` can create apps entirely from the terminal if the user has the Slack CLI installed. Worth checking with `which slack` before falling back to browser flow.

2. **Chrome CDP via launched profile** — if the user is willing to restart Chrome with `--remote-debugging-port=9222`, Playwright could connect to their actual browser session. But this is disruptive and not practical for casual use.

3. **Browser extension** — a browser extension could bridge between the agent and the user's browser, receiving instructions via local WebSocket and performing actions in the authenticated context. This is a large engineering effort but would fully solve the problem.
