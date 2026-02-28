# Browser Automation for External Service Setup

## Problem

When the agent needs the user to set up an external service (Slack webhook, GitHub token, etc.), the user has to manually navigate a web UI. We investigated whether Playwright (via MCP) could automate this — clicking through the setup flow on behalf of the user.

## Solution: Playwright MCP Extension Mode

Playwright MCP supports an `--extension` flag that uses a **Chrome extension bridge** to connect to the user's real browser. This gives the agent full Playwright API access (navigate, click, fill, screenshot, evaluate) with the user's authenticated sessions — no sandboxed browser, no CDP port, no Chrome restart.

### How it works

1. User installs the **Playwright MCP Bridge** Chrome extension (one-time)
2. Playwright MCP server is configured with `--extension` flag
3. When browser tools are called, Playwright connects via the extension to the user's running Chrome
4. The agent operates in the user's actual browser context — logged into Slack, GitHub, Google, etc.

### Configuration

Playwright MCP is configured as an MCP server in ATA's config (`~/.ata/config.toml`):

```toml
[mcp_servers.playwright]
command = "npx"
args = ["@playwright/mcp@latest", "--extension"]
```

When ATA starts (including `ata exec` for scheduled jobs), it connects to Playwright MCP and gains tools like `mcp__playwright__browser_navigate`, `mcp__playwright__browser_click`, `mcp__playwright__browser_snapshot`, etc.

### One-time setup

The user must install the **Playwright MCP Bridge** Chrome extension:
https://chromewebstore.google.com/detail/playwright-mcp-bridge/mmlmfjhmonkocbjadbfplnigmagldckm

This extension bridges Playwright into the user's real Chrome. Without it, Playwright falls back to a sandboxed Chromium with no authenticated sessions.

## What We Tried Before (and Why It Failed)

Before finding extension mode, we tried default Playwright (sandboxed Chromium). The Slack page said:

> "You'll need to sign in to your Slack account to create an application."

The user was not logged in because Playwright's default browser is completely isolated.

### Why other approaches fail

| Approach                             | Why It Fails                                                                                                                                                           |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Playwright default**               | Opens sandboxed Chromium with no cookies/auth.                                                                                                                         |
| **CDP (`--remote-debugging-port`)**  | Chrome 136+ requires a separate `--user-data-dir` for CDP, so you get a clean profile with no sessions. This was a deliberate security change to prevent cookie theft. |
| **Playwright + Chrome profile copy** | Chrome locks its profile directory. Copying a locked profile gives inconsistent snapshots.                                                                             |
| **Cookie extraction**                | Chrome encrypts cookies via macOS Keychain. Decrypting requires Keychain access or the encryption key.                                                                 |

### Why extension mode works where CDP doesn't

Chrome 136 blocked CDP on the default profile as a security measure against malware. But the extension API is Chrome's _intended_ mechanism for browser automation — it's audited, sandboxed by Chrome's extension security model, and doesn't expose raw debugging protocol access. The Playwright MCP Bridge extension communicates with the MCP server over a local channel, and executes actions via Chrome's `chrome.scripting` and `chrome.tabs` APIs.

## Fallback: Manual Flow

If extension mode is not available (extension not installed, non-Chrome browser), fall back to `open <url>` + minimal manual steps:

```sh
open "https://api.slack.com/apps?new_app=1"
```

Provide a ready-to-paste manifest and limit user actions to 3-4 steps max. See the job-manager skill for the full least-friction pattern.

## macOS Alternative: AppleScript

On macOS, Chrome supports `execute javascript` via AppleScript as a zero-install fallback:

```bash
# Navigate
osascript -e 'tell application "Google Chrome" to set URL of active tab of front window to "https://example.com"'

# Execute JS
osascript -e 'tell application "Google Chrome" to execute javascript "document.title" in active tab of front window'
```

Requires one-time opt-in: Chrome → View → Developer → "Allow JavaScript from Apple Events".

This gives full JS execution in the user's authenticated Chrome without any extension or CDP. Useful as a fallback when the Playwright extension isn't installed.

## Verified: Slack Webhook Setup (Full Automation)

This flow has been tested end-to-end and works:

1. `browser_navigate` → `https://api.slack.com/apps?new_app=1`
2. Select "From a manifest" tab
3. **CodeMirror gotcha**: Slack's manifest editor uses CodeMirror, not a plain `<textarea>`. Direct `locator('textarea').click()` fails because the CodeMirror overlay intercepts pointer events. Use `browser_run_code` with `page.evaluate()`:
   ```js
   const cmEl = document.querySelector(".CodeMirror");
   cmEl.CodeMirror.setValue(manifestJson);
   cmEl.CodeMirror.focus();
   ```
4. Click Next → review summary → Create
5. Navigate to Incoming Webhooks → Add New Webhook
6. Select channel from dropdown → Allow
7. Extract webhook URL via `browser_evaluate` on the URL input element
8. Store in `~/.ata/secrets/` and wire into job config

Total time: ~2.5 minutes. Zero user interaction required.

## Automation Capability Tiers

| Tier         | Method                           | Setup                       | Capabilities                                         | Platform       |
| ------------ | -------------------------------- | --------------------------- | ---------------------------------------------------- | -------------- |
| **1 (best)** | Playwright MCP `--extension`     | Install Chrome extension    | Full Playwright API, screenshots, selectors, waiting | Cross-platform |
| **2**        | AppleScript `execute javascript` | Enable JS from Apple Events | JS execution, navigation, form filling               | macOS only     |
| **3**        | `open <url>` + manual steps      | None                        | Opens page, user does the rest                       | Cross-platform |

The agent should try tier 1 first, fall back to tier 2 on macOS, and use tier 3 as a last resort.
