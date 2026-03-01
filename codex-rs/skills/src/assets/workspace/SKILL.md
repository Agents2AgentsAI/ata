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

Required: `git`. The `ata` binary is already in PATH.

## Containment Rules

- **ALWAYS** use `ata workspace resolve '@spec'` for workspace paths — never construct paths manually
- **ALWAYS** use `ata workspace check-host <url>` before any clone operation
- **ALWAYS** use typed mutation commands (e.g. `repo-pin`, `add-entry`, `set-field`) for manifest changes (atomic lock + version bump)
- **ALWAYS** audit significant operations via `ata workspace audit`
- For repo cloning, prefer the thick command `ata workspace repo-clone <url> <alias>` which enforces all of the above

## Command Reference

| Command | Description |
|---------|-------------|
| `ata workspace init <name>` | Create workspace, print ID |
| `ata workspace list` | List workspaces as JSON |
| `ata workspace read [--workspace ID]` | Print manifest JSON |
| `ata workspace select <id>` | Set active workspace (session-aware) |
| `ata workspace delete <id>` | Remove workspace tree |
| `ata workspace resolve '<@spec>' [--workspace ID]` | Resolve @-path to absolute path |
| `ata workspace check-host <url> [--workspace ID]` | Validate URL + host allowlist |
| `ata workspace audit '<json>' [--workspace ID]` | Append audit entry |
| `ata workspace audit-query [--workspace ID] [--since TS] [--until TS] [--ops OPS] [--limit N]` | Query audit log |
| `ata workspace run-locked --level <lvl> [--target-id ID] -- <cmd>` | Run under fine-grained lock |
| `ata workspace mirror-path <url>` | Print shared mirror cache path |
| **`ata workspace repo-clone <url> <alias> [--workspace ID] [--full]`** | **Full repo_add: validate + clone + register + audit** |
| **`ata workspace run-setup <name> --source-alias <alias> [--strategy S] [--workspace ID]`** | **Full run creation: dirs + materialize + register + audit** |
| `ata workspace repo-update-state --alias X --head-sha Y [--head-ref Z] [--workspace ID]` | Update repo HEAD state |
| `ata workspace repo-pin --alias X --sha Y [--workspace ID]` | Pin repo to SHA |
| `ata workspace repo-unpin --alias X [--workspace ID]` | Unpin repo (tracking mode) |
| `ata workspace repo-remove --alias X [--workspace ID]` | Remove repo dir + manifest entry + audit |
| `ata workspace run-update-status --id X --status Y [--workspace ID]` | Update run status |
| `ata workspace run-remove --id X [--workspace ID]` | Remove run + worktree cleanup + audit |
| `ata workspace add-entry --collection <type> --json '{...}' [--workspace ID]` | Append to collection (papers/datasets/artifacts/links/snapshots/indexes) |
| `ata workspace remove-entry --collection <type> --id X [--workspace ID]` | Remove by ID from collection |
| `ata workspace set-field --path <dotted.path> --value '<json>' [--workspace ID]` | Set manifest field at dotted path |
| `ata workspace index-update-status --id X --status Y [--workspace ID]` | Update index status |
| `ata workspace recipe <operation>` | Print step-by-step recipe for an operation |
| `ata workspace recipe list` | List all available recipes |

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

Alternatively, use `ata workspace recipe <operation>` for quick inline recipes.
