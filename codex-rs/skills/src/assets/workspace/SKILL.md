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

Workspaces organize multi-repo work, execution runs, and research resources under a single directory tree. Each workspace has a JSON manifest tracking repos, runs, papers, datasets, artifacts, indexes, snapshots, and resource links.

All mutations go through `ws.py` (atomic lock + jq + version bump). Reads use standard file tools. Git commands handle repo operations directly.

## Prerequisites

Required: `python3`, `jq`, `git`. Check availability:

```
which python3 jq git
```

The helper script is at `<skill_dir>/scripts/ws.py` where `<skill_dir>` is the directory containing this SKILL.md. Refer to it as `WS` in commands below:

```
WS="<skill_dir>/scripts/ws.py"
```

## Helper Script Reference

| Command | Description |
|---------|-------------|
| `python3 $WS init <name>` | Create workspace, print workspace ID |
| `python3 $WS list` | List all workspaces as JSON array |
| `python3 $WS read [--workspace ID]` | Print manifest JSON to stdout |
| `python3 $WS mutate '<jq_expr>' [--workspace ID]` | Lock, apply jq, bump version, atomic write |
| `python3 $WS select <id>` | Set active workspace selection |
| `python3 $WS audit '<json>' [--workspace ID]` | Append entry to audit.ndjson |
| `python3 $WS delete <id>` | Remove workspace directory tree |
| `python3 $WS resolve '<@spec>' [--workspace ID]` | Resolve @-path alias to absolute path |

Default workspace is `global` when `--workspace` is omitted.

## Directory Layout

```
~/.ata/workspaces/<workspace-id>/
  workspace.json              # Manifest (schema v2, camelCase)
  repos/                      # Cloned repositories by alias
    <alias>/
  runs/                       # Execution runs
    <run-id>/
      run.json                # Run manifest
      root/                   # Working directory for the run
      logs/                   # Command output logs
  artifacts/                  # Materialized artifact files
  indexes/                    # Search indexes
  cache/                      # Workspace cache
  locks/
    workspace.lock            # Exclusive flock (30s timeout)
  knowledge-base/
    cards/
    topics/
    briefings/
    explanations/
    assets/
    staging/
  notes/
    workspace/
      audit.ndjson            # All operations logged
      snapshots/              # Manifest snapshots
    repos/
    papers/
    datasets/
    artifacts/
    runs/
    indexes/
```

`CODEX_HOME` defaults to `~/.ata`. Override with the `$CODEX_HOME` environment variable.

## Manifest Schema

The workspace manifest (`workspace.json`) uses **camelCase** keys. Key fields:

| Field | Type | Description |
|-------|------|-------------|
| `schemaVersion` | int | Always `2` |
| `id` | string | Workspace ID (e.g., `my-project-a1b2c3d4`) |
| `name` | string | Human-readable name |
| `createdAt` | int | Unix seconds |
| `updatedAt` | int | Unix seconds (bumped on every mutation) |
| `manifestVersion` | int | Incremented on every mutation |
| `repos` | array | Repository entries |
| `runs` | array | Run summaries |
| `papers` | array | Paper references |
| `datasets` | array | Dataset references |
| `artifacts` | array | Artifact entries |
| `links` | array | Resource links between entries |
| `snapshots` | array | Manifest snapshots |
| `indexes` | array | Search index entries |
| `policies` | object | Clone defaults, host allowlist |
| `knowledgeBase` | object | KB config (`{"path": "knowledge-base"}`) |
| `labels` | object | User-defined key-value labels |

## Path Aliases

Use `ws.py resolve` to convert @-paths to absolute paths:

| Alias | Resolves to |
|-------|------------|
| `@<repo_alias>/path` | `repos/<alias>/path` |
| `@run/<run_id>/path` | `runs/<run_id>/path` |
| `@kb/path` | `knowledge-base/path` |
| `@notes/path` | `notes/workspace/path` |
| `@notes/<category>/path` | `notes/<category>/path` (if category is known) |
| `@artifacts/<id>/path` | `artifacts/<id>/path` |
| `@cache/path` | `cache/path` |
| `@index/<id>/path` | `indexes/<id>/path` |
| `@ws/<other_id>/path` | Cross-workspace: `workspaces/<other_id>/path` |

Known notes categories: `workspace`, `repos`, `papers`, `datasets`, `runs`, `artifacts`, `indexes`.

Reserved aliases (cannot be used as repo aliases): `run`, `notes`, `ws`, `artifacts`, `kb`, `index`, `cache`.

---

## Operations

### Workspace Management

#### Create Workspace

```bash
WID=$(python3 $WS init "My Project")
echo "Created workspace: $WID"
```

Generates a workspace ID (`slugified-name-<hash>`), creates the full directory tree, and writes an initial manifest.

#### List Workspaces

```bash
python3 $WS list
```

Returns JSON array: `[{"id", "name", "updatedAt", "repoCount"}, ...]`

#### Get Workspace Details

```bash
python3 $WS read --workspace "$WID"
```

#### Select Active Workspace

```bash
python3 $WS select "$WID"
```

Writes workspace ID to `$CODEX_HOME/.workspace_selected`.

#### Delete Workspace

```bash
python3 $WS delete "$WID"
```

Removes the entire workspace directory tree. Refuses to delete `global`.

---

### Repository Management

#### Security: URL Validation

Before cloning any repository, validate the URL to prevent credential leakage:

- Must use `https://` (reject `http://`, `git://`, `ssh://`, `file://`)
- Must not contain embedded credentials (`user:pass@`)
- Must not contain tokens in query params (`?token=`, `?access_token=`)
- Must not contain GitHub PAT patterns (`ghp_`, `gho_`, `github_pat_`)

```bash
# Validate URL before cloning
validate_repo_url() {
  local url="$1"
  if ! echo "$url" | grep -qE '^https://'; then
    echo "error: only https:// URLs allowed" >&2; return 1
  fi
  if echo "$url" | grep -qE '://[^/@]+:[^/@]+@'; then
    echo "error: embedded credentials not allowed" >&2; return 1
  fi
  if echo "$url" | grep -qiE '\?(token|access_token)=|ghp_|gho_|github_pat_'; then
    echo "error: tokens/PATs in URL not allowed" >&2; return 1
  fi
}
validate_repo_url "$REPO_URL" || exit 1
```

For host restriction, configure `policies.repoHostsAllowlist` in the manifest (e.g., `["github.com", "gitlab.com"]`). Only store sanitized URLs in the manifest.

#### Add Repository

```bash
# 1. Determine workspace root and clone defaults
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
ALIAS="my-repo"
REPO_URL="https://github.com/org/repo.git"

# 2. Read clone policy from manifest (falls back to defaults)
CLONE_POLICY=$(python3 $WS read --workspace "$WID" | jq -r '.policies.defaultClone // empty')
CLONE_DEPTH=$(echo "$CLONE_POLICY" | jq -r '.depth // 1')
CLONE_FILTER=$(echo "$CLONE_POLICY" | jq -r '.filter // "blob:limit=1m"')
CLONE_SINGLE=$(echo "$CLONE_POLICY" | jq -r '.singleBranch // true')
CLONE_NOTAGS=$(echo "$CLONE_POLICY" | jq -r '.noTags // true')
CLONE_SUBS=$(echo "$CLONE_POLICY" | jq -r '.submodules // "none"')
CLONE_LFS=$(echo "$CLONE_POLICY" | jq -r '.lfs // "auto"')

# 3. Build clone command from policy
#    Override with --full to skip depth/filter (full clone)
#    Override individual options: --depth N, --submodules, --lfs
CLONE_ARGS=()
if [ "$CLONE_DEPTH" != "null" ] && [ "$CLONE_DEPTH" -gt 0 ] 2>/dev/null; then
  CLONE_ARGS+=(--depth "$CLONE_DEPTH")
fi
if [ "$CLONE_SINGLE" = "true" ]; then CLONE_ARGS+=(--single-branch); fi
if [ "$CLONE_NOTAGS" = "true" ]; then CLONE_ARGS+=(--no-tags); fi
if [ -n "$CLONE_FILTER" ] && [ "$CLONE_FILTER" != "null" ]; then
  CLONE_ARGS+=(--filter="$CLONE_FILTER")
fi
if [ "$CLONE_SUBS" = "recursive" ]; then CLONE_ARGS+=(--recurse-submodules); fi

git clone "${CLONE_ARGS[@]}" "$REPO_URL" "$WS_ROOT/repos/$ALIAS"

if [ "$CLONE_LFS" = "always" ] || [ "$CLONE_LFS" = "auto" ]; then
  (cd "$WS_ROOT/repos/$ALIAS" && git lfs pull 2>/dev/null || true)
fi

# 4. Read git state
HEAD_SHA=$(git -C "$WS_ROOT/repos/$ALIAS" rev-parse HEAD)
HEAD_REF=$(git -C "$WS_ROOT/repos/$ALIAS" symbolic-ref --short HEAD 2>/dev/null || echo "")
DEFAULT_BRANCH=$(git -C "$WS_ROOT/repos/$ALIAS" rev-parse --abbrev-ref origin/HEAD 2>/dev/null | sed 's|origin/||' || echo "main")
REPO_ID=$(python3 -c "import uuid; print(f'repo-{int(__import__(\"time\").time())}-{uuid.uuid4().hex}')")

# 5. Record actual clone options used in manifest
CLONE_RECORD="{\"depth\":$CLONE_DEPTH,\"singleBranch\":$CLONE_SINGLE,\"noTags\":$CLONE_NOTAGS,\"filter\":\"$CLONE_FILTER\",\"submodules\":\"$CLONE_SUBS\",\"lfs\":\"$CLONE_LFS\"}"

python3 $WS mutate --workspace "$WID" \
  ".repos += [{
    \"id\": \"$REPO_ID\",
    \"alias\": \"$ALIAS\",
    \"repoKey\": \"org/repo\",
    \"remoteUrl\": \"$REPO_URL\",
    \"checkoutPath\": \"repos/$ALIAS\",
    \"notesPath\": \"notes/repos/$ALIAS\",
    \"clone\": $CLONE_RECORD,
    \"pin\": {\"mode\":\"tracking\"},
    \"state\": {\"headSha\":\"$HEAD_SHA\",\"headRef\":\"$HEAD_REF\",\"defaultBranch\":\"$DEFAULT_BRANCH\",\"shallow\":$([ \"$CLONE_DEPTH\" -gt 0 ] 2>/dev/null && echo true || echo false)}
  }]"

# 6. Create notes dir and audit
mkdir -p "$WS_ROOT/notes/repos/$ALIAS"
python3 $WS audit --workspace "$WID" \
  "{\"op\":\"repo_add\",\"targets\":[{\"type\":\"repo\",\"id\":\"$REPO_ID\",\"alias\":\"$ALIAS\"}]}"
```

#### List Repositories

```bash
python3 $WS read --workspace "$WID" | jq '.repos[] | {alias, remoteUrl, state}'
```

#### Update Repository (fetch latest)

```bash
ALIAS="my-repo"
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
REPO_PATH="$WS_ROOT/repos/$ALIAS"

git -C "$REPO_PATH" fetch --depth 1
git -C "$REPO_PATH" reset --hard "origin/$(git -C "$REPO_PATH" rev-parse --abbrev-ref origin/HEAD | sed 's|origin/||')"

HEAD_SHA=$(git -C "$REPO_PATH" rev-parse HEAD)
NOW=$(date +%s)

python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .state.headSha = \"$HEAD_SHA\" | .state.lastUpdatedAt = $NOW else . end]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"repo_update\",\"targets\":[{\"type\":\"repo\",\"id\":\"\",\"alias\":\"$ALIAS\"}]}"
```

#### Pin Repository to Commit

```bash
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .pin = {\"mode\":\"pinned\",\"pinnedSha\":\"$SHA\"} else . end]"
```

#### Unpin Repository (switch to tracking)

```bash
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .pin = {\"mode\":\"tracking\"} else . end]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"repo_track\",\"targets\":[{\"type\":\"repo\",\"id\":\"\",\"alias\":\"$ALIAS\"}]}"
```

#### Remove Repository

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
rm -rf "$WS_ROOT/repos/$ALIAS"

python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | select(.alias != \"$ALIAS\")]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"repo_remove\",\"targets\":[{\"type\":\"repo\",\"id\":\"\",\"alias\":\"$ALIAS\"}]}"
```

---

### Execution Runs

#### Create Run

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
RUN_ID=$(python3 -c "import uuid,time; print(f'run-{int(time.time())}-{uuid.uuid4().hex}')")
REPO_ALIAS="my-repo"
REPO_PATH="$WS_ROOT/repos/$REPO_ALIAS"
RUN_ROOT="$WS_ROOT/runs/$RUN_ID"
RUN_NAME="experiment-1"  # optional human-readable name

mkdir -p "$RUN_ROOT/logs" "$RUN_ROOT/outputs" "$RUN_ROOT/tmp" "$RUN_ROOT/env"

# --- Materialization strategy (pick one) ---

# Strategy 1: Worktree (preferred — fast, shares object store)
git -C "$REPO_PATH" worktree add "$RUN_ROOT/root" HEAD

# Strategy 2: Clone (if worktree unavailable, e.g. bare repo)
# git clone "$REPO_PATH" "$RUN_ROOT/root"

# Strategy 3: Copy (no git needed)
# cp -R "$REPO_PATH" "$RUN_ROOT/root"

# --- Write run.json metadata (RunManifestV2) ---
NOW=$(date +%s)
HEAD_SHA=$(git -C "$RUN_ROOT/root" rev-parse HEAD 2>/dev/null || echo "")
cat > "$RUN_ROOT/run.json" <<RUNJSON
{
  "schemaVersion": 2,
  "id": "$RUN_ID",
  "name": "$RUN_NAME",
  "createdAt": $NOW,
  "updatedAt": $NOW,
  "status": "created",
  "source": {"repoAlias": "$REPO_ALIAS", "sha": "$HEAD_SHA"},
  "commands": [],
  "env": {}
}
RUNJSON

# --- Register in workspace manifest ---
python3 $WS mutate --workspace "$WID" \
  ".runs += [{
    \"id\": \"$RUN_ID\",
    $([ -n "$RUN_NAME" ] && echo "\"name\": \"$RUN_NAME\",")
    \"createdAt\": $NOW,
    \"updatedAt\": $NOW,
    \"rootPath\": \"runs/$RUN_ID\",
    \"status\": \"created\",
    \"source\": {\"repoAlias\": \"$REPO_ALIAS\"}
  }]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"run_create\",\"targets\":[{\"type\":\"run\",\"id\":\"$RUN_ID\"}]}"
```

#### Execute in Run

```bash
RUN_ROOT="$WS_ROOT/runs/$RUN_ID"

# --- Environment setup ---
# Network isolation (default: disabled)
export CODEX_SANDBOX_NETWORK_DISABLED=1

# Route caches through workspace cache (resolve via ws.py)
export PIP_CACHE_DIR=$(python3 $WS resolve '@cache/pip' --workspace "$WID")
export HF_HOME=$(python3 $WS resolve '@cache/huggingface' --workspace "$WID")
export TORCH_HOME=$(python3 $WS resolve '@cache/torch' --workspace "$WID")
mkdir -p "$PIP_CACHE_DIR" "$HF_HOME" "$TORCH_HOME"

# --- Mark run as running ---
python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | if .id == \"$RUN_ID\" then .status = \"running\" | .updatedAt = $(date +%s) else . end]"

# --- Execute with timeout (900s default) and log capture ---
cd "$RUN_ROOT/root"
LOG_FILE="$RUN_ROOT/logs/$(date +%s).txt"

timeout 900 <command> 2>&1 | head -c 65536 | tee "$LOG_FILE"
EXIT_CODE=${PIPESTATUS[0]}

# --- Update status (created → running → completed/failed/cancelled) ---
if [ "$EXIT_CODE" -eq 0 ]; then
  STATUS="completed"
elif [ "$EXIT_CODE" -eq 124 ]; then
  STATUS="failed"  # timeout
else
  STATUS="failed"
fi

python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | if .id == \"$RUN_ID\" then .status = \"$STATUS\" | .updatedAt = $(date +%s) else . end]"
```

**Notes:**
- Output truncation: 64KB max per stream recorded in run manifest
- Command history: bounded at 200 entries; oldest evicted when limit reached
- Status transitions: `created` → `running` → `completed` / `failed` / `cancelled`
- Timeout: 900s default; process killed on timeout (exit code 124)

#### List Runs

```bash
python3 $WS read --workspace "$WID" | jq '.runs[] | {id, status, createdAt}'
```

#### Delete Run

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
  "{\"op\":\"run_delete\",\"targets\":[{\"type\":\"run\",\"id\":\"$RUN_ID\"}]}"
```

#### Garbage Collect Stale Runs

```bash
# List runs older than 7 days with status completed/failed
python3 $WS read --workspace "$WID" | jq -r \
  ".runs[] | select(.status == \"completed\" or .status == \"failed\") | select(.updatedAt < (now - 604800)) | .id"
```

Then delete each with the run_delete flow above.

---

### Resources (Papers / Datasets / Artifacts)

These three resource types follow the same pattern. Replace `<type>` with `papers`, `datasets`, or `artifacts` and `<prefix>` with `paper`, `dataset`, or `artifact`.

#### Add Resource

```bash
RESOURCE_ID=$(python3 -c "import uuid,time; print(f'<prefix>-{int(time.time())}-{uuid.uuid4().hex}')")
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')

python3 $WS mutate --workspace "$WID" \
  ".<type> += [{\"id\":\"$RESOURCE_ID\",\"title\":\"<title>\",\"notesPath\":\"notes/<type>/$RESOURCE_ID\"}]"

mkdir -p "$WS_ROOT/notes/<type>/$RESOURCE_ID"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"<prefix>_add\",\"targets\":[{\"type\":\"<prefix>\",\"id\":\"$RESOURCE_ID\"}]}"
```

Additional fields per type:

- **Papers**: `authors` (array), `year` (int), `doi`, `arxiv`, `url`, `pdfArtifactId`
- **Datasets**: `name`, `url`, `license`, `artifactIds` (array)
- **Artifacts**: `kind` (required, e.g. `"pdf"`, `"model"`, `"data"`), `displayName`, `sourceUrl`, `sha256`, `sizeBytes`, `materializedPath`

#### List Resources

```bash
python3 $WS read --workspace "$WID" | jq '.<type>[] | {id, title}'
```

#### Remove Resource

```bash
python3 $WS mutate --workspace "$WID" \
  ".<type> = [.<type>[] | select(.id != \"$RESOURCE_ID\")]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"<prefix>_remove\",\"targets\":[{\"type\":\"<prefix>\",\"id\":\"$RESOURCE_ID\"}]}"
```

---

### Resource Links

Links connect any two resources (repo, paper, dataset, artifact, run, index).

#### Add Link

```bash
python3 $WS mutate --workspace "$WID" \
  ".links += [{
    \"from\": {\"type\":\"paper\",\"id\":\"$PAPER_ID\"},
    \"to\": {\"type\":\"repo\",\"id\":\"$REPO_ID\"},
    \"kind\": \"implements\"
  }]"
```

Common `kind` values: `implements`, `uses`, `produces`, `derived_from`, `related_to`.

#### List Links

```bash
python3 $WS read --workspace "$WID" | jq '.links[]'
```

#### Remove Link

```bash
python3 $WS mutate --workspace "$WID" \
  ".links = [.links[] | select(.from.id != \"$FROM_ID\" or .to.id != \"$TO_ID\")]"
```

---

### Snapshots

Snapshots capture the current manifest state for later comparison or rollback.

#### Create Snapshot

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
SNAP_ID=$(python3 -c "import uuid,time; print(f'snap-{int(time.time())}-{uuid.uuid4().hex}')")
SNAP_PATH="notes/workspace/snapshots/$SNAP_ID.json"

# Copy current manifest as snapshot
cp "$WS_ROOT/workspace.json" "$WS_ROOT/$SNAP_PATH"

python3 $WS mutate --workspace "$WID" \
  ".snapshots += [{\"id\":\"$SNAP_ID\",\"path\":\"$SNAP_PATH\"}]"

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"snapshot_create\",\"targets\":[{\"type\":\"snapshot\",\"id\":\"$SNAP_ID\"}]}"
```

#### List Snapshots

```bash
python3 $WS read --workspace "$WID" | jq '.snapshots[] | {id, path}'
```

#### Restore Snapshot (re-apply repo pins)

Restore is **additive** — it never deletes existing repos.

**Conflict strategies:**
- `missing_repo_mode`: `skip` (record and continue), `fail` (abort), `re_add` (re-clone + pin)
- `alias_conflict_mode`: `skip`, `fail` (default)

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
MISSING_MODE="skip"       # skip | fail | re_add
CONFLICT_MODE="fail"      # skip | fail

# Get current repo aliases
CURRENT_ALIASES=$(python3 $WS read --workspace "$WID" | jq -r '.repos[].alias')

# Process each repo from snapshot
jq -c '.repos[]' "$WS_ROOT/$SNAP_PATH" | while read -r REPO; do
  ALIAS=$(echo "$REPO" | jq -r '.alias')
  PIN_SHA=$(echo "$REPO" | jq -r '.pin.pinnedSha // empty')
  REMOTE=$(echo "$REPO" | jq -r '.remoteUrl // empty')

  # Check alias conflict
  if echo "$CURRENT_ALIASES" | grep -qx "$ALIAS"; then
    if [ "$CONFLICT_MODE" = "fail" ]; then
      echo "error: alias '$ALIAS' already exists" >&2; exit 1
    fi
    echo "skip: alias '$ALIAS' already exists" >&2; continue
  fi

  # Check if repo dir exists
  if [ ! -d "$WS_ROOT/repos/$ALIAS" ]; then
    case "$MISSING_MODE" in
      skip) echo "skip: repo '$ALIAS' missing, recording" >&2; continue ;;
      fail) echo "error: repo '$ALIAS' not found" >&2; exit 1 ;;
      re_add)
        # Re-clone and pin (uses repo_add flow above)
        git clone --depth 1 "$REMOTE" "$WS_ROOT/repos/$ALIAS"
        ;;
    esac
  fi

  # Checkout pinned SHA if present
  if [ -n "$PIN_SHA" ]; then
    git -C "$WS_ROOT/repos/$ALIAS" fetch origin "$PIN_SHA" --depth 1 2>/dev/null || true
    git -C "$WS_ROOT/repos/$ALIAS" checkout "$PIN_SHA" 2>/dev/null || \
      echo "warn: could not checkout $PIN_SHA for $ALIAS" >&2
  fi

  # Update manifest pin
  python3 $WS mutate --workspace "$WID" \
    ".repos = [.repos[] | if .alias == \"$ALIAS\" then .pin = $(echo "$REPO" | jq '.pin') else . end]"
done

python3 $WS audit --workspace "$WID" \
  "{\"op\":\"snapshot_restore\",\"targets\":[{\"type\":\"snapshot\",\"id\":\"$SNAP_ID\"}],\"details\":{\"missingMode\":\"$MISSING_MODE\",\"conflictMode\":\"$CONFLICT_MODE\"}}"
```

---

### Import / Export

#### Export Workspace Bundle

Configurable export with mode controls:

| Component | Modes | Default |
|-----------|-------|---------|
| Notes | `workspace-only`, `all`, `none` | `workspace-only` |
| Runs | `metadata+logs`, `metadata-only`, `none` | `none` |
| Artifacts | `metadata-only`, `blobs`, `none` | `metadata-only` |
| Repos | `none`, `bundles` (git bundle) | `none` |
| Format | directory, `.tar.gz` | `.tar.gz` |

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
BUNDLE_PATH="/tmp/$WID-export.tar.gz"
NOTES_MODE="workspace-only"  # workspace-only | all | none
RUNS_MODE="none"             # metadata+logs | metadata-only | none
ARTIFACTS_MODE="metadata-only"  # metadata-only | blobs | none
REPOS_MODE="none"            # none | bundles

# Sanitize URLs in manifest before export (strip tokens)
EXPORT_MANIFEST=$(python3 $WS read --workspace "$WID" | \
  jq '.repos = [.repos[] | .remoteUrl = (.remoteUrl | split("?")[0])]')
echo "$EXPORT_MANIFEST" > "/tmp/$WID-export-manifest.json"

# Build file list
EXPORT_FILES=("/tmp/$WID-export-manifest.json")

case "$NOTES_MODE" in
  workspace-only) EXPORT_FILES+=("notes/workspace/") ;;
  all) EXPORT_FILES+=("notes/") ;;
esac

EXPORT_FILES+=("knowledge-base/")

case "$RUNS_MODE" in
  metadata+logs)
    for RUN_DIR in "$WS_ROOT"/runs/*/; do
      [ -f "$RUN_DIR/run.json" ] && EXPORT_FILES+=("runs/$(basename "$RUN_DIR")/run.json" "runs/$(basename "$RUN_DIR")/logs/")
    done ;;
  metadata-only)
    for RUN_DIR in "$WS_ROOT"/runs/*/; do
      [ -f "$RUN_DIR/run.json" ] && EXPORT_FILES+=("runs/$(basename "$RUN_DIR")/run.json")
    done ;;
esac

case "$ARTIFACTS_MODE" in
  blobs) EXPORT_FILES+=("artifacts/") ;;
  metadata-only) ;; # artifact metadata is in manifest
esac

case "$REPOS_MODE" in
  bundles)
    mkdir -p "/tmp/$WID-bundles"
    for REPO_DIR in "$WS_ROOT"/repos/*/; do
      ALIAS=$(basename "$REPO_DIR")
      git -C "$REPO_DIR" bundle create "/tmp/$WID-bundles/$ALIAS.bundle" --all 2>/dev/null || true
    done ;;
esac

# Create archive
tar -czf "$BUNDLE_PATH" \
  -C "$WS_ROOT" \
  "${EXPORT_FILES[@]}"

echo "Exported to: $BUNDLE_PATH"
```

#### Import Workspace Bundle

```bash
# 1. Create target workspace
NEW_WID=$(python3 $WS init "Imported Project")
NEW_ROOT=$(python3 $WS resolve '@ws' --workspace "$NEW_WID" | sed 's|/$||')

# 2. Extract bundle
tar -xzf "$BUNDLE_PATH" -C "$NEW_ROOT"

# 3. Update manifest ID to match new workspace
python3 $WS mutate --workspace "$NEW_WID" \
  ".id = \"$NEW_WID\""
```

#### Repo Import Plan + Apply

##### Plan Phase

Generate an import plan, detect license files, estimate size, and store for review:

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
PLAN_ID=$(python3 -c "import uuid,time; print(f'plan-{int(time.time())}-{uuid.uuid4().hex}')")
PLAN_DIR="$WS_ROOT/notes/workspace/import-plans"
PLAN_PATH="$PLAN_DIR/$PLAN_ID.json"
mkdir -p "$PLAN_DIR"

# Build plan: list repos with metadata
jq -c '[.repos[] | {
  alias, remoteUrl, pin,
  repoKey: .repoKey
}]' "$IMPORT_MANIFEST" | python3 -c "
import json, sys, time
repos = json.load(sys.stdin)
plan = {
    'planId': '$PLAN_ID',
    'createdAt': int(time.time()),
    'expiresAt': int(time.time()) + 3600,
    'sourceManifest': '$IMPORT_MANIFEST',
    'repos': repos,
    'status': 'pending'
}
json.dump(plan, sys.stdout, indent=2)
" > "$PLAN_PATH"

echo "Import plan written to: $PLAN_PATH (expires in 1 hour)"
```

##### Apply Phase

Validate plan, clone repos, record provenance:

```bash
# 1. Validate plan not expired
EXPIRES=$(jq -r '.expiresAt' "$PLAN_PATH")
NOW=$(date +%s)
if [ "$NOW" -gt "$EXPIRES" ]; then
  echo "error: import plan expired" >&2; exit 1
fi

# 2. Import each repo
jq -c '.repos[]' "$PLAN_PATH" | while read -r REPO; do
  ALIAS=$(echo "$REPO" | jq -r '.alias')
  REMOTE=$(echo "$REPO" | jq -r '.remoteUrl')

  # Skip files matching sensitive patterns
  # Subtree import: git subtree add --prefix="repos/$ALIAS" "$REMOTE" main --squash
  # Vendor copy (exclude secrets):
  #   rsync -a --exclude='.env' --exclude='secrets.*' --exclude='credentials.*' "$SOURCE/" "$WS_ROOT/repos/$ALIAS/"

  # Standard clone
  git clone --depth 1 "$REMOTE" "$WS_ROOT/repos/$ALIAS"

  # Detect license
  LICENSE_FILE=$(find "$WS_ROOT/repos/$ALIAS" -maxdepth 1 -iname 'LICENSE*' -o -iname 'COPYING*' | head -1)
  LICENSE_TYPE=""
  if [ -n "$LICENSE_FILE" ]; then
    LICENSE_TYPE=$(head -1 "$LICENSE_FILE" | grep -oiE 'MIT|Apache|GPL|BSD|ISC|MPL' | head -1)
  fi

  # Register in manifest (use repo_add flow)
  # ... (same as Add Repository above)
done

# 3. Write provenance record (never auto-commit)
cat > "$WS_ROOT/notes/workspace/import-provenance-$PLAN_ID.json" <<PROV
{
  "planId": "$PLAN_ID",
  "importedAt": $NOW,
  "sourceManifest": "$(jq -r '.sourceManifest' "$PLAN_PATH")",
  "repoCount": $(jq '.repos | length' "$PLAN_PATH")
}
PROV

# 4. Mark plan as applied
jq '.status = "applied"' "$PLAN_PATH" > "$PLAN_PATH.tmp" && mv "$PLAN_PATH.tmp" "$PLAN_PATH"
```

**Important:** Never auto-commit imported repos. Only commit if the user explicitly requests it.

---

### Indexes

Indexes support search over workspace content. The schema is scaffolded — no built-in backend; external tools provide the actual indexing.

#### Build Index

```bash
INDEX_ID=$(python3 -c "import uuid,time; print(f'idx-{int(time.time())}-{uuid.uuid4().hex}')")
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
INDEX_PATH="indexes/$INDEX_ID"

mkdir -p "$WS_ROOT/$INDEX_PATH"

NOW=$(date +%s)
python3 $WS mutate --workspace "$WID" \
  ".indexes += [{
    \"id\":\"$INDEX_ID\",
    \"kind\":\"keyword\",
    \"targetType\":\"repo\",
    \"targetId\":\"$REPO_ID\",
    \"createdAt\":$NOW,
    \"status\":\"building\",
    \"path\":\"$INDEX_PATH\"
  }]"

# ... run external indexing tool, then mark ready:
python3 $WS mutate --workspace "$WID" \
  ".indexes = [.indexes[] | if .id == \"$INDEX_ID\" then .status = \"ready\" | .updatedAt = $(date +%s) else . end]"
```

#### List Indexes

```bash
python3 $WS read --workspace "$WID" | jq '.indexes[] | {id, kind, targetType, status}'
```

#### Delete Index

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
rm -rf "$WS_ROOT/indexes/$INDEX_ID"

python3 $WS mutate --workspace "$WID" \
  ".indexes = [.indexes[] | select(.id != \"$INDEX_ID\")]"
```

---

### Audit Trail

All operations append to `notes/workspace/audit.ndjson`. Each line is a JSON object with `schemaVersion`, `ts`, `workspaceId`, `actor`, `op`, `status`, `targets`.

#### Read Audit Log

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
cat "$WS_ROOT/notes/workspace/audit.ndjson" | jq -s '.'
```

#### Filter by Operation

```bash
cat "$WS_ROOT/notes/workspace/audit.ndjson" | jq -s '[.[] | select(.op == "repo_add")]'
```

#### Filter by Target

```bash
cat "$WS_ROOT/notes/workspace/audit.ndjson" | jq -s '[.[] | select(.targets[] | .alias == "my-repo")]'
```

---

### Knowledge Base

KB files live under `<workspace_root>/knowledge-base/`. Use `$kb` skill for card operations. Resolve the KB path:

```bash
python3 $WS resolve '@kb' --workspace "$WID"
```

---

### Shared Caches

Shared caches live under `$CODEX_HOME/caches/` and are workspace-independent:

| Cache | Path | Description |
|-------|------|-------------|
| Repo mirrors | `$CODEX_HOME/caches/repo-mirrors/<repoKeyHash>/` | Bare git mirrors for faster clones |
| Artifact blobs | `$CODEX_HOME/caches/artifacts/<sha256>/blob` | Content-addressed artifact storage |

Use `--reference` with mirrors for faster clones:

```bash
MIRROR="$CODEX_HOME/caches/repo-mirrors/$(echo -n "$REPO_KEY" | sha256sum | cut -c1-16)"
if [ -d "$MIRROR" ]; then
  git clone --reference "$MIRROR" "$REPO_URL" "$WS_ROOT/repos/$ALIAS"
else
  git clone "$REPO_URL" "$WS_ROOT/repos/$ALIAS"
fi
```

---

### Session Context and Project Pin

Workspace resolution order (first match wins):

1. **Explicit `--workspace`**: CLI argument
2. **Project pin**: `.codex/workspace.json` discovered via upward walk from cwd (stops at `.git`)
3. **Session**: `$CODEX_HOME/sessions/<sessionId>/workspace.json`
4. **Global**: `global` workspace (auto-created if missing)

```bash
# Session file location
SESSION_FILE="$CODEX_HOME/sessions/$CODEX_SESSION_ID/workspace.json"

# Project pin location (discovered from cwd)
# Walk up from cwd, stop at .git boundary
PROJECT_PIN=".codex/workspace.json"
```

---

### Locking

ws.py uses workspace-level locking (`locks/workspace.lock`). The design supports finer-grained locks for future use:

| Lock | Path | Purpose |
|------|------|---------|
| Workspace | `locks/workspace.lock` | Manifest mutations (active) |
| Run | `runs/<runId>/run.lock` | Concurrent run execution (future) |
| Index | `indexes/<indexId>/index.lock` | Long index builds (future) |
| KB | `knowledge-base/kb.lock` | KB operations (future) |

---

### Index Operations (Advanced)

#### Query Index (placeholder)

Index query depends on the external indexing backend. Scaffolding:

```bash
# Symbol lookup (backend-specific)
INDEX_PATH=$(python3 $WS resolve "@index/$INDEX_ID" --workspace "$WID")
# ... invoke external search tool against $INDEX_PATH
```

#### Garbage Collect Orphaned Indexes

Remove indexes whose `targetId` no longer references an existing repo or run:

```bash
python3 $WS read --workspace "$WID" | jq -r '
  (.repos[].id) as $repo_ids |
  (.runs[].id) as $run_ids |
  .indexes[] |
  select(
    (.targetId | IN($repo_ids) | not) and
    (.targetId | IN($run_ids) | not)
  ) | .id
' | while read -r ORPHAN_ID; do
  WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
  rm -rf "$WS_ROOT/indexes/$ORPHAN_ID"
  python3 $WS mutate --workspace "$WID" \
    ".indexes = [.indexes[] | select(.id != \"$ORPHAN_ID\")]"
done
```

---

### Artifact Download + Checksum

When adding artifacts from a remote source, download and verify integrity:

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
ART_ID=$(python3 -c "import uuid,time; print(f'artifact-{int(time.time())}-{uuid.uuid4().hex}')")
ARTIFACT_DIR="$WS_ROOT/artifacts/$ART_ID"
mkdir -p "$ARTIFACT_DIR"

# Download
curl -L -o "$ARTIFACT_DIR/blob" "$SOURCE_URL"

# Verify checksum
echo "$EXPECTED_SHA256  $ARTIFACT_DIR/blob" | shasum -a 256 -c

# Register with checksum
ACTUAL_SHA=$(shasum -a 256 "$ARTIFACT_DIR/blob" | cut -d' ' -f1)
SIZE_BYTES=$(stat -f%z "$ARTIFACT_DIR/blob" 2>/dev/null || stat -c%s "$ARTIFACT_DIR/blob")

python3 $WS mutate --workspace "$WID" \
  ".artifacts += [{
    \"id\": \"$ART_ID\",
    \"kind\": \"data\",
    \"sourceUrl\": \"$SOURCE_URL\",
    \"sha256\": \"$ACTUAL_SHA\",
    \"sizeBytes\": $SIZE_BYTES,
    \"materializedPath\": \"artifacts/$ART_ID/blob\"
  }]"
```

---

## Graceful Degradation

- **`jq` not found**: `ws.py mutate` will error. Install jq or use `ws.py read` + manual JSON editing + direct file write.
- **`python3` not found**: Fall back to manual JSON editing with standard file tools. Lock by creating the lock file manually.
- **`git` not found**: Cannot clone repos or create worktree-based runs. Use copy-based runs or manual file placement.
- **Lock contention (30s timeout)**: Another process holds the workspace lock. Wait and retry, or investigate with `lsof <lock_path>`.
- **Partial state**: If `ws.py init` is interrupted, the directory tree may exist without a valid manifest. Delete the directory and re-run init.
- **Missing manifest**: If `workspace.json` is missing but the directory exists, re-initialize: `ws.py init` will skip the existing directory and create a new one with a different ID.
