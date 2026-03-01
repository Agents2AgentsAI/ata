# Execution Runs

## Create Run (Preferred: thick command)

Use `run-setup` for the full safe flow:

```bash
python3 $WS run-setup "experiment-1" --source-alias my-repo --workspace "$WID"
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
RUN_ROOT=$(python3 $WS resolve "@run/$RUN_ID" --workspace "$WID")

# Mark as running
python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | if .id == \"$RUN_ID\" then .status = \"running\" | .updatedAt = $(date +%s) else . end]"

# Execute with timeout and log capture
cd "$RUN_ROOT/root"
LOG_FILE="$RUN_ROOT/logs/$(date +%s).txt"
timeout 900 <command> 2>&1 | head -c 65536 | tee "$LOG_FILE"
EXIT_CODE=${PIPESTATUS[0]}

# Update status
STATUS=$( [ "$EXIT_CODE" -eq 0 ] && echo "completed" || echo "failed" )
python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | if .id == \"$RUN_ID\" then .status = \"$STATUS\" | .updatedAt = $(date +%s) else . end]"
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
export PIP_CACHE_DIR=$(python3 $WS resolve '@cache/pip' --workspace "$WID")
export HF_HOME=$(python3 $WS resolve '@cache/huggingface' --workspace "$WID")
export TORCH_HOME=$(python3 $WS resolve '@cache/torch' --workspace "$WID")
mkdir -p "$PIP_CACHE_DIR" "$HF_HOME" "$TORCH_HOME"
```

## List Runs

```bash
python3 $WS read --workspace "$WID" | jq '.runs[] | {id, name, status, createdAt}'
```

## Delete Run

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')

# Clean up worktree if applicable
REPO_PATH=$(git -C "$WS_ROOT/runs/$RUN_ID/root" rev-parse --git-common-dir 2>/dev/null | xargs dirname 2>/dev/null || true)
if [ -n "$REPO_PATH" ]; then
  git -C "$REPO_PATH" worktree remove "$WS_ROOT/runs/$RUN_ID/root" --force 2>/dev/null || true
fi
rm -rf "$WS_ROOT/runs/$RUN_ID"
python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | select(.id != \"$RUN_ID\")]"
python3 $WS audit --workspace "$WID" \
  '{"op":"run_delete","targets":[{"type":"run","id":"'"$RUN_ID"'"}]}'
```

## Garbage Collect Stale Runs

```bash
# List runs older than 7 days with completed/failed status
python3 $WS read --workspace "$WID" | jq -r \
  '.runs[] | select(.status == "completed" or .status == "failed") | select(.updatedAt < (now - 604800)) | .id'
```

Then delete each with the run delete flow above.
