# Ata CLI (Rust Implementation)

We provide Ata CLI as a standalone, native executable to ensure a zero-dependency install.

## Installing Ata

Today, the easiest way to install Ata is via `npm`:

```shell
npm i -g @a2a-ai/ata
ata
```

You can also install via Homebrew (`brew install --cask ata`) or download a platform-specific release directly from our [GitHub Releases](https://github.com/Agents2AgentsAI/ata/releases).

For macOS/Linux, you can also install via:

```shell
curl -fsSL https://agents2agents.ai/ata/install.sh | sh
```

## Documentation quickstart

- First run with Ata? Start with [`docs/getting-started.md`](../docs/getting-started.md) (links to the walkthrough for prompts, keyboard shortcuts, and session management).
- Want deeper control? See [`docs/config.md`](../docs/config.md) and [`docs/install.md`](../docs/install.md).

## What's new in the Rust CLI

The Rust implementation is now the maintained Ata CLI and serves as the default experience. It includes a number of features that the legacy TypeScript CLI never supported.

### Config

Ata supports a rich set of configuration options. Note that the Rust CLI uses `config.toml` instead of `config.json`. See [`docs/config.md`](../docs/config.md) for details.

### Model Context Protocol Support

#### MCP client

Ata CLI functions as an MCP client that allows the Ata CLI to connect to MCP servers on startup. See the [`configuration documentation`](../docs/config.md#connecting-to-mcp-servers) for details.

#### MCP server (experimental)

Ata can be launched as an MCP _server_ by running `ata mcp-server`. This allows _other_ MCP clients to use Ata as a tool for another agent.

Use the [`@modelcontextprotocol/inspector`](https://github.com/modelcontextprotocol/inspector) to try it out:

```shell
npx @modelcontextprotocol/inspector ata mcp-server
```

Use `ata mcp` to add/list/get/remove MCP server launchers defined in `config.toml`, and `ata mcp-server` to run the MCP server directly.

### Notifications

You can enable notifications by configuring a script that is run whenever the agent finishes a turn. The [notify documentation](../docs/config.md#notify) includes a detailed example that explains how to get desktop notifications via [terminal-notifier](https://github.com/julienXX/terminal-notifier) on macOS. When Ata detects that it is running under WSL 2 inside Windows Terminal (`WT_SESSION` is set), the TUI automatically falls back to native Windows toast notifications so approval prompts and completed turns surface even though Windows Terminal does not implement OSC 9.

### `ata exec` to run Ata programmatically/non-interactively

To run Ata non-interactively, run `ata exec PROMPT` (you can also pass the prompt via `stdin`) and Ata will work on your task until it decides that it is done and exits. Output is printed to the terminal directly. You can set the `RUST_LOG` environment variable to see more about what's going on.
Use `ata exec --ephemeral ...` to run without persisting session rollout files to disk.

### Experimenting with the Ata Sandbox

To test to see what happens when a command is run under the sandbox provided by Ata, we provide the following subcommands in Ata CLI:

```
# macOS
ata sandbox macos [--full-auto] [--log-denials] [COMMAND]...

# Linux
ata sandbox linux [--full-auto] [COMMAND]...

# Windows
ata sandbox windows [--full-auto] [COMMAND]...

# Legacy aliases
ata debug seatbelt [--full-auto] [--log-denials] [COMMAND]...
ata debug landlock [--full-auto] [COMMAND]...
```

### Selecting a sandbox policy via `--sandbox`

The Rust CLI exposes a dedicated `--sandbox` (`-s`) flag that lets you pick the sandbox policy **without** having to reach for the generic `-c/--config` option:

```shell
# Run Ata with the default, read-only sandbox
ata --sandbox read-only

# Allow the agent to write within the current workspace while still blocking network access
ata --sandbox workspace-write

# Danger! Disable sandboxing entirely (only do this if you are already running in a container or other isolated env)
ata --sandbox danger-full-access
```

The same setting can be persisted in `~/.ata/config.toml` via the top-level `sandbox_mode = "MODE"` key, e.g. `sandbox_mode = "workspace-write"`.
In `workspace-write`, Ata also includes `~/.ata/memories` in its writable roots so memory maintenance does not require an extra approval.

## Code Organization

This folder is the root of a Cargo workspace. It contains quite a bit of experimental code, but here are the key crates:

- [`core/`](./core) contains the business logic for Ata. Ultimately, we hope this to be a library crate that is generally useful for building other Rust/native applications that use Ata.
- [`exec/`](./exec) "headless" CLI for use in automation.
- [`tui/`](./tui) CLI that launches a fullscreen TUI built with [Ratatui](https://ratatui.rs/).
- [`cli/`](./cli) CLI multitool that provides the aforementioned CLIs via subcommands.

If you want to contribute or inspect behavior in detail, start by reading the module-level `README.md` files under each crate and run the project workspace from the top-level `codex-rs` directory so shared config, features, and build scripts stay aligned.
