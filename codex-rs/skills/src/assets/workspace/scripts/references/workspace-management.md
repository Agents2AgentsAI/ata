# Workspace Management

## Create Workspace

```bash
WID=$(python3 $WS init "My Project")
echo "Created workspace: $WID"
```

Generates ID (`slugified-name-<hash>`), creates full directory tree, writes initial manifest.

## List Workspaces

```bash
python3 $WS list
```

Returns JSON array: `[{"id", "name", "updatedAt", "repoCount"}, ...]`

## Get Workspace Details

```bash
python3 $WS read --workspace "$WID"
```

## Select Active Workspace

```bash
python3 $WS select "$WID"
```

Writes structured JSON to session-scoped selection file. If `$CODEX_SESSION_ID` is set, writes to `$CODEX_HOME/sessions/<sessionId>/workspace.json`; otherwise to `$CODEX_HOME/.workspace_selected`.

## Delete Workspace

```bash
python3 $WS delete "$WID"
```

Removes the entire workspace directory tree. Refuses to delete `global`.

## Resolution Order

Workspace resolution (first match wins):

1. **Explicit `--workspace`**: CLI argument
2. **Project pin**: `.codex/workspace.json` discovered via upward walk from cwd (stops at `.git`)
3. **Session**: `$CODEX_HOME/sessions/<sessionId>/workspace.json`
4. **Global**: `global` workspace (auto-created if missing)

## Manifest Schema

The manifest (`workspace.json`) uses **camelCase** keys:

| Field | Type | Description |
|-------|------|-------------|
| `schemaVersion` | int | Always `2` |
| `id` | string | Workspace ID |
| `name` | string | Human-readable name |
| `createdAt` / `updatedAt` | int | Unix seconds |
| `manifestVersion` | int | Incremented on every mutation |
| `repos` | array | Repository entries |
| `runs` | array | Run summaries |
| `papers` / `datasets` / `artifacts` | array | Research resources |
| `links` | array | Resource links |
| `snapshots` | array | Manifest snapshots |
| `indexes` | array | Search index entries |
| `policies` | object | Clone defaults, host allowlist |
| `knowledgeBase` | object | KB config |
| `labels` | object | User-defined key-value labels |
