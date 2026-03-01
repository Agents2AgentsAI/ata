# Audit Trail & Knowledge Base

## Audit Trail

All operations append to `notes/workspace/audit.ndjson`. Each line is a JSON object with `schemaVersion`, `ts`, `workspaceId`, `actor`, `op`, `status`, `targets`.

### Query Audit Log

```bash
# All entries (last 200)
python3 $WS audit-query --workspace "$WID"

# With filters
python3 $WS audit-query --workspace "$WID" --ops "repo_add,repo_remove" --limit 50
python3 $WS audit-query --workspace "$WID" --since 1700000000 --until 1710000000
```

### Raw NDJSON Access

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
cat "$WS_ROOT/notes/workspace/audit.ndjson" | jq -s '.'
```

### Filter by Target

```bash
cat "$WS_ROOT/notes/workspace/audit.ndjson" | jq -s '[.[] | select(.targets[] | .alias == "my-repo")]'
```

## Knowledge Base

KB files live under `<workspace_root>/knowledge-base/`. Use `$kb` skill for card operations.

### Resolve KB Path

```bash
python3 $WS resolve '@kb' --workspace "$WID"
```

### KB Write Operations (with locking)

```bash
python3 $WS run-locked --level kb --workspace "$WID" -- <kb-write-command>
```

## Locking

`ws.py` supports fine-grained locking via `run-locked`. Lock ordering: workspace < kb < run < index.

| Lock | Path | Usage |
|------|------|-------|
| Workspace | `locks/workspace.lock` | Manifest mutations (automatic via `mutate`) |
| KB | `knowledge-base/kb.lock` | `run-locked --level kb -- <cmd>` |
| Run | `runs/<runId>/run.lock` | `run-locked --level run --target-id <id> -- <cmd>` |
| Index | `indexes/<indexId>/index.lock` | `run-locked --level index --target-id <id> -- <cmd>` |

All locks use exclusive flock with 30s timeout.

## Graceful Degradation

- **`jq` not found**: `ws.py mutate` will error. Install jq or use `ws.py read` + manual JSON editing.
- **`python3` not found**: Fall back to manual JSON editing. Lock by creating the lock file manually.
- **`git` not found**: Cannot clone repos or create worktree-based runs. Use copy-based runs.
- **Lock contention (30s timeout)**: Another process holds the lock. Wait and retry, or investigate with `lsof <lock_path>`.
