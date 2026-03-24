# Rollout Analyzer

Analyze ATA session rollout files for context pollution and token usage.

## Usage

```bash
# Analyze a specific rollout file
python3 analyze_rollout.py <rollout.jsonl>

# Analyze the most recent rollout
python3 analyze_rollout.py latest

# Analyze the most recent parent (non-subagent) session
python3 analyze_rollout.py latest --parent

# List recent sessions
python3 analyze_rollout.py --list

# Verbose mode (subcategory breakdown)
python3 analyze_rollout.py <file> -v

# JSON output for scripting
python3 analyze_rollout.py <file> --json
```

## What It Reports

- **Category breakdown**: system, skills, tool descriptions, user messages, assistant messages, tool calls, tool outputs, reasoning — as characters, estimated tokens, and percentage
- **Skill injections**: which skills were injected and their sizes
- **Tool calls**: call counts, argument sizes, output sizes per tool
- **Subagent tracking**: which subagents were spawned, whether they produced staging files
- **Turn-by-turn API token usage**: actual input/cached/output/reasoning tokens from the API
- **Context accumulation chart**: visual representation of how context grows over turns
- **Warnings**: automatic detection of context pollution (large skill injections, tool output dominance, missing staging files, duplicate injections)

## Rollout File Location

Session rollouts are stored in `~/.ata/sessions/YYYY/MM/DD/rollout-*.jsonl`.
