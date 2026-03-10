# Execution Runs

## Create Run (Preferred: thick command)

Use `run-setup` for the full safe flow:

```bash
ata workspace run-setup "experiment-1" --source-alias my-repo --workspace "$WID"
# Options:
#   --strategy worktree  (default, fast, shares object store)
#   --strategy copy      (no git needed)
#   --strategy clone     (independent git repo)
```

This single command handles:
- Verifying source repo exists
- Creating run directory structure (`logs/`, `outputs/`, `tmp/`, `env/`)
- Materializing code via chosen strategy (falls back to copy if worktree fails)
- Writing `run.json` metadata
- Registering in workspace manifest with optimistic concurrency
- Audit logging

Output: JSON with `runId`, `name`, `rootPath`, `codePath`, `strategy`, `source`.

## Execute in Run

```bash
RUN_ID="<run_id>"
RUN_ROOT=$(ata workspace resolve "@run/$RUN_ID" --workspace "$WID")

# Mark as running
ata workspace run-update-status --id "$RUN_ID" --status running --workspace "$WID"

# Execute with timeout and log capture
cd "$RUN_ROOT/root"
LOG_FILE="$RUN_ROOT/logs/$(date +%s).txt"
timeout 900 <command> 2>&1 | head -c 65536 | tee "$LOG_FILE"
EXIT_CODE=${PIPESTATUS[0]}

# Update status
STATUS=$( [ "$EXIT_CODE" -eq 0 ] && echo "completed" || echo "failed" )
ata workspace run-update-status --id "$RUN_ID" --status "$STATUS" --workspace "$WID"
```

**Notes:**
- Output truncation: 64KB max per stream
- Command history: bounded at 200 entries
- Status transitions: `created` → `running` → `completed` / `failed` / `cancelled`
- Timeout: 900s default; killed on timeout (exit 124)

## Environment Setup

```bash
# Network isolation (optional)
export CODEX_SANDBOX_NETWORK_DISABLED=1

# Route caches through workspace
export PIP_CACHE_DIR=$(ata workspace resolve '@cache/pip' --workspace "$WID")
export HF_HOME=$(ata workspace resolve '@cache/huggingface' --workspace "$WID")
export TORCH_HOME=$(ata workspace resolve '@cache/torch' --workspace "$WID")
mkdir -p "$PIP_CACHE_DIR" "$HF_HOME" "$TORCH_HOME"
```

## List Runs

```bash
ata workspace read --workspace "$WID" | jq '.runs[] | {id, name, status, createdAt}'
```

## Delete Run

```bash
ata workspace run-remove --id "$RUN_ID" --workspace "$WID"
```

This single command handles worktree cleanup, directory removal, manifest update, and audit logging.

## Garbage Collect Stale Runs

```bash
# List runs older than 7 days with completed/failed status
ata workspace read --workspace "$WID" | jq -r \
  '.runs[] | select(.status == "completed" or .status == "failed") | select(.updatedAt < (now - 604800)) | .id'
```

Then delete each with the run delete flow above.
