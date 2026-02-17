# Configuration

For basic configuration instructions, see [this documentation](https://github.com/Agents2AgentsAI/ata/blob/main/docs/config.md).

For advanced configuration instructions, see [this documentation](https://github.com/Agents2AgentsAI/ata/blob/main/docs/config.md).

For a full configuration reference, see [this documentation](https://github.com/Agents2AgentsAI/ata/blob/main/docs/config.md).

## Connecting to MCP servers

Ata can connect to MCP servers configured in `~/.ata/config.toml`. See the configuration reference for the latest MCP server options:

- https://github.com/Agents2AgentsAI/ata/blob/main/docs/config.md

## Apps (Connectors)

Use `$` in the composer to insert a ChatGPT connector; the popover lists accessible
apps. The `/apps` command lists available and installed apps. Connected apps appear first
and are labeled as connected; others are marked as can be installed.

## Notify

Ata can run a notification hook when the agent finishes a turn. See the configuration reference for the latest notification settings:

- https://github.com/Agents2AgentsAI/ata/blob/main/docs/config.md

## JSON Schema

The generated JSON Schema for `config.toml` lives at `codex-rs/core/config.schema.json`.

## Notices

Ata stores "do not show again" flags for some UI prompts under the `[notice]` table.

Ctrl+C/Ctrl+D quitting uses a ~1 second double-press hint (`ctrl + c again to quit`).
