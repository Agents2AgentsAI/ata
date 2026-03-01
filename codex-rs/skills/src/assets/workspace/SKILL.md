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

#### Add Repository

```bash
# 1. Determine workspace root
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
ALIAS="my-repo"

# 2. Clone with shallow/partial defaults
git clone --depth 1 --single-branch --no-tags \
  --filter=blob:limit=1m \
  "https://github.com/org/repo.git" \
  "$WS_ROOT/repos/$ALIAS"

# 3. Read git state
HEAD_SHA=$(git -C "$WS_ROOT/repos/$ALIAS" rev-parse HEAD)
HEAD_REF=$(git -C "$WS_ROOT/repos/$ALIAS" symbolic-ref --short HEAD 2>/dev/null || echo "")
DEFAULT_BRANCH=$(git -C "$WS_ROOT/repos/$ALIAS" rev-parse --abbrev-ref origin/HEAD 2>/dev/null | sed 's|origin/||' || echo "main")
REPO_ID=$(python3 -c "import uuid; print(f'repo-{int(__import__(\"time\").time())}-{uuid.uuid4().hex}')")

# 4. Register in manifest
python3 $WS mutate --workspace "$WID" \
  ".repos += [{
    \"id\": \"$REPO_ID\",
    \"alias\": \"$ALIAS\",
    \"repoKey\": \"org/repo\",
    \"remoteUrl\": \"https://github.com/org/repo.git\",
    \"checkoutPath\": \"repos/$ALIAS\",
    \"notesPath\": \"notes/repos/$ALIAS\",
    \"clone\": {\"depth\":1,\"singleBranch\":true,\"noTags\":true,\"filter\":\"blob:limit=1m\"},
    \"pin\": {\"mode\":\"tracking\"},
    \"state\": {\"headSha\":\"$HEAD_SHA\",\"headRef\":\"$HEAD_REF\",\"defaultBranch\":\"$DEFAULT_BRANCH\",\"shallow\":true}
  }]"

# 5. Create notes dir and audit
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

#### Create Run (from repo worktree)

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
RUN_ID=$(python3 -c "import uuid,time; print(f'run-{int(time.time())}-{uuid.uuid4().hex}')")
REPO_ALIAS="my-repo"
REPO_PATH="$WS_ROOT/repos/$REPO_ALIAS"
RUN_ROOT="$WS_ROOT/runs/$RUN_ID"

mkdir -p "$RUN_ROOT/logs"

# Create worktree (preferred) or clone
git -C "$REPO_PATH" worktree add "$RUN_ROOT/root" HEAD

NOW=$(date +%s)
python3 $WS mutate --workspace "$WID" \
  ".runs += [{
    \"id\": \"$RUN_ID\",
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
cd "$WS_ROOT/runs/$RUN_ID/root"

# Run the command, capture output
<command> 2>&1 | tee "$WS_ROOT/runs/$RUN_ID/logs/$(date +%s).txt"

# Update status
python3 $WS mutate --workspace "$WID" \
  ".runs = [.runs[] | if .id == \"$RUN_ID\" then .status = \"completed\" | .updatedAt = $(date +%s) else . end]"
```

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

```bash
# Read snapshot manifest and extract repo pins
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
jq '.repos[] | {alias, pin}' "$WS_ROOT/$SNAP_PATH"

# For each pinned repo, checkout the pinned SHA
# Then update the live manifest with the restored pins
```

---

### Import / Export

#### Export Workspace Bundle

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
BUNDLE_PATH="/tmp/$WID-export.tar.gz"

# Export manifest + notes (not repo checkouts or run data)
tar -czf "$BUNDLE_PATH" \
  -C "$WS_ROOT" \
  workspace.json notes/ knowledge-base/

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

To import repos from a snapshot or external manifest:

```bash
# 1. Generate import plan: list repos to clone
jq '.repos[] | {alias, remoteUrl, pin}' "$IMPORT_MANIFEST" > /tmp/import-plan.json

# 2. Review plan, then clone each repo using the repo_add flow above
```

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

## Graceful Degradation

- **`jq` not found**: `ws.py mutate` will error. Install jq or use `ws.py read` + manual JSON editing + direct file write.
- **`python3` not found**: Fall back to manual JSON editing with standard file tools. Lock by creating the lock file manually.
- **`git` not found**: Cannot clone repos or create worktree-based runs. Use copy-based runs or manual file placement.
- **Lock contention (30s timeout)**: Another process holds the workspace lock. Wait and retry, or investigate with `lsof <lock_path>`.
- **Partial state**: If `ws.py init` is interrupted, the directory tree may exist without a valid manifest. Delete the directory and re-run init.
- **Missing manifest**: If `workspace.json` is missing but the directory exists, re-initialize: `ws.py init` will skip the existing directory and create a new one with a different ID.
