# Agent Coordination Setup

Agent coordination lets multiple ATA instances communicate with each other — both on the same machine and across the network. Agents can send team messages, see what peers are working on, and respond to incoming messages automatically.

## Quick Start

### 1. Build with relay support

```bash
cargo build -p codex-cli --features relay
```

Or with all features:

```bash
cargo build -p codex-cli --all-features
```

### 2. Enable coordination in config

Add to `~/.ata/config.toml`:

```toml
[features]
coordination = true
```

That's it. No relay URL, no server, no ports to configure.

## What happens automatically

1. **First ATA instance** auto-starts a relay server on `127.0.0.1:7800` in the background
2. **Subsequent instances** detect the port is taken and connect as clients
3. **Messages flow through the relay** — when an agent calls `team_post`, the message goes to the relay and is pushed to all connected agents via SSE
4. **Incoming messages appear in chat** and are submitted to the agent so it can see and respond to them
5. **Project scoping** — messages are scoped per git remote (the origin URL is hashed), so agents in different repos don't see each other's messages

## CLI Commands

| Command                    | Description                                             |
| -------------------------- | ------------------------------------------------------- |
| `ata team agents`          | List active coordination agents in the current repo     |
| `ata team messages`        | Show recent coordination messages                       |
| `ata team messages <name>` | Filter messages by agent name prefix                    |
| `ata team relay`           | Start a standalone relay server (for cross-machine use) |
| `ata team relay-logs`      | Tail coordination-related logs from the TUI log file    |

## Cross-Machine Setup

For agents on different laptops to communicate:

### On the relay host machine

Either let ATA auto-start the relay (it binds to `127.0.0.1`), or start a standalone relay that listens on all interfaces:

```bash
ata team relay --port 7800
```

### On all machines

Add to `~/.ata/config.toml`:

```toml
[coordination]
relay_url = "http://<relay-host-ip>:7800"
```

Optional shared secret for authentication:

```toml
[coordination]
relay_url = "http://<relay-host-ip>:7800"
relay_secret = "my-shared-secret"
```

When a secret is set, all agents must use the same secret to connect.

## How It Works

```
Agent A calls team_post
  → writes to local SQLite (for history/queries)
  → HTTP POST to relay server
  → relay broadcasts via SSE to all subscribers
  → Agent B's watcher receives the message
  → message appears in Agent B's TUI
  → message is submitted to Agent B's LLM as input
```

- **Local SQLite** is used for persistence (`ata team messages`, coordination context in prompts)
- **Relay server** handles all push notifications via Server-Sent Events (SSE)
- **Auto-start**: the first ATA instance to bind port 7800 becomes the relay host; others connect as clients
- **Reconnection**: if the relay goes down, agents retry every 5 seconds
- **Self-filtering**: agents never see their own messages echoed back

## Debugging

Tail coordination logs:

```bash
ata team relay-logs
```

For verbose debug output, start ATA with:

```bash
RUST_LOG=codex_coordination=debug,codex_coordination_relay=debug ata
```

Log file location: `~/.ata/log/codex-tui.log`
