---
name: workspace
description: >-
  Multi-repo workspace management: clone repos, create execution runs,
  track papers/datasets/artifacts, manage snapshots, and review audit
  logs. Use when the user wants to organize multi-repo work, run
  experiments in sandboxes, or track research resources.
metadata:
  short-description: Multi-repo workspace management
policy:
  allow_implicit_invocation: true
---

# Workspace Management

Workspaces organize multi-repo work, execution runs, and research resources under `~/.ata/workspaces/<id>/`. Each workspace has a JSON manifest (`workspace.json`) tracking repos, runs, papers, datasets, artifacts, indexes, snapshots, and links.

## Prerequisites

Required: `python3`, `jq`, `git`. The helper script:

```
WS="<skill_dir>/scripts/ws.py"
```

## Containment Rules

- **ALWAYS** use `python3 $WS resolve '@spec'` for workspace paths — never construct paths manually
- **ALWAYS** use `python3 $WS check-host <url>` before any clone operation
- **ALWAYS** use `python3 $WS mutate` for manifest changes (atomic lock + version bump)
- **ALWAYS** audit significant operations via `python3 $WS audit`
- For repo cloning, prefer the thick command `python3 $WS repo-clone <url> <alias>` which enforces all of the above

## Command Reference

| Command | Description |
|---------|-------------|
| `python3 $WS init <name>` | Create workspace, print ID |
| `python3 $WS list` | List workspaces as JSON |
| `python3 $WS read [--workspace ID]` | Print manifest JSON |
| `python3 $WS mutate '<jq>' [--workspace ID] [--expect-version N]` | Atomic lock + jq + version bump |
| `python3 $WS select <id>` | Set active workspace (session-aware) |
| `python3 $WS delete <id>` | Remove workspace tree |
| `python3 $WS resolve '<@spec>' [--workspace ID]` | Resolve @-path to absolute path |
| `python3 $WS check-host <url> [--workspace ID]` | Validate URL + host allowlist |
| `python3 $WS audit '<json>' [--workspace ID]` | Append audit entry |
| `python3 $WS audit-query [--workspace ID] [--since TS] [--until TS] [--ops OPS] [--limit N]` | Query audit log |
| `python3 $WS run-locked --level <lvl> [--target-id ID] -- <cmd>` | Run under fine-grained lock |
| `python3 $WS mirror-path <url>` | Print shared mirror cache path |
| **`python3 $WS repo-clone <url> <alias> [--workspace ID] [--full]`** | **Full repo_add: validate + clone + register + audit** |
| **`python3 $WS run-setup <name> --source-alias <alias> [--strategy S] [--workspace ID]`** | **Full run creation: dirs + materialize + register + audit** |
| `python3 $WS recipe <operation>` | Print step-by-step recipe for an operation |
| `python3 $WS recipe list` | List all available recipes |

Workspace resolution: `--workspace` > project pin (`.codex/workspace.json`) > session > `global`.

## Path Aliases

| Alias | Resolves to |
|-------|------------|
| `@<alias>/path` | `repos/<alias>/path` |
| `@run/<id>/path` | `runs/<id>/path` |
| `@kb/path` | `knowledge-base/path` |
| `@notes/path` | `notes/workspace/path` |
| `@notes/<category>/path` | `notes/<category>/path` |
| `@artifacts/<id>/path` | `artifacts/<id>/path` |
| `@cache/path` | `cache/path` |
| `@index/<id>/path` | `indexes/<id>/path` |
| `@ws/<other_id>/path` | Cross-workspace path |

Reserved aliases: `run`, `notes`, `ws`, `artifacts`, `kb`, `index`, `cache`.

## Directory Layout

```
~/.ata/workspaces/<id>/
  workspace.json          # Manifest (schema v2, camelCase)
  repos/<alias>/          # Cloned repositories
  runs/<run-id>/          # Execution runs (run.json, root/, logs/)
  artifacts/              # Materialized artifacts
  indexes/                # Search indexes
  cache/                  # Workspace cache
  locks/workspace.lock    # Exclusive flock (30s timeout)
  knowledge-base/         # cards/, topics/, briefings/, explanations/
  notes/                  # workspace/, repos/, papers/, datasets/, runs/
    workspace/audit.ndjson
```

## Detailed Instructions

For detailed step-by-step instructions on specific operations, read the appropriate reference file:

| Category | File | Covers |
|----------|------|--------|
| Workspace lifecycle | `<skill_dir>/scripts/references/workspace-management.md` | init, list, select, delete, resolution |
| Repository ops | `<skill_dir>/scripts/references/repo-management.md` | repo-clone, update, pin, remove, mirrors |
| Execution runs | `<skill_dir>/scripts/references/run-management.md` | run-setup, exec, list, delete, GC |
| Research resources | `<skill_dir>/scripts/references/resources.md` | papers, datasets, artifacts, links |
| Snapshots & export | `<skill_dir>/scripts/references/snapshots-and-export.md` | snapshot, restore, export, import |
| Audit & KB | `<skill_dir>/scripts/references/audit-and-kb.md` | audit, audit-query, KB, locking |

Alternatively, use `python3 $WS recipe <operation>` for quick inline recipes.
