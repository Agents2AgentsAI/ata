# AGENTS.md

For information about AGENTS.md, see [this documentation](https://github.com/Agents2AgentsAI/ata/blob/main/docs/agents_md.md).

## Hierarchical agents message

When the `child_agents_md` feature flag is enabled (via `[features]` in `config.toml`), Ata appends additional guidance about AGENTS.md scope and precedence to the user instructions message and emits that message even when no AGENTS.md is present.
