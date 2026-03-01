# Snapshots & Export/Import

## Create Snapshot

Captures the current manifest state for later comparison or rollback.

```bash
WS_ROOT=$(ata workspace resolve '@ws' --workspace "$WID" | sed 's|/$||')
SNAP_ID=$(python3 -c "import uuid,time; print(f'snap-{int(time.time())}-{uuid.uuid4().hex}')")
SNAP_PATH="notes/workspace/snapshots/$SNAP_ID.json"
cp "$WS_ROOT/workspace.json" "$WS_ROOT/$SNAP_PATH"
ata workspace add-entry --collection snapshots --json "{\"id\":\"$SNAP_ID\",\"path\":\"$SNAP_PATH\"}" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"snapshot_create","targets":[{"type":"snapshot","id":"'"$SNAP_ID"'"}]}'
```

## List Snapshots

```bash
ata workspace read --workspace "$WID" | jq '.snapshots[] | {id, path}'
```

## Restore Snapshot

Restore is **additive** — it never deletes existing repos.

**Conflict strategies:**
- `missing_repo_mode`: `skip` (record and continue), `fail` (abort), `re_add` (re-clone + pin)
- `alias_conflict_mode`: `skip`, `fail` (default)

```bash
WS_ROOT=$(ata workspace resolve '@ws' --workspace "$WID" | sed 's|/$||')
SNAP_PATH="<snapshot_path>"  # e.g. notes/workspace/snapshots/snap-XXX.json

jq -c '.repos[]' "$WS_ROOT/$SNAP_PATH" | while read -r REPO; do
  ALIAS=$(echo "$REPO" | jq -r '.alias')
  PIN_SHA=$(echo "$REPO" | jq -r '.pin.pinnedSha // empty')

  # Skip if alias already exists
  if ata workspace read --workspace "$WID" | jq -e ".repos[] | select(.alias == \"$ALIAS\")" >/dev/null 2>&1; then
    echo "skip: alias '$ALIAS' already exists" >&2; continue
  fi

  # Checkout pinned SHA if present
  if [ -n "$PIN_SHA" ] && [ -d "$WS_ROOT/repos/$ALIAS" ]; then
    git -C "$WS_ROOT/repos/$ALIAS" fetch origin "$PIN_SHA" --depth 1 2>/dev/null || true
    git -C "$WS_ROOT/repos/$ALIAS" checkout "$PIN_SHA" 2>/dev/null || true
  fi

  ata workspace repo-pin --alias "$ALIAS" --sha "$PIN_SHA" --workspace "$WID"
done

ata workspace audit --workspace "$WID" \
  '{"op":"snapshot_restore","targets":[{"type":"snapshot","id":"'"$SNAP_ID"'"}]}'
```

## Export Workspace Bundle

Configurable export with mode controls:

| Component | Modes | Default |
|-----------|-------|---------|
| Notes | `workspace-only`, `all`, `none` | `workspace-only` |
| Runs | `metadata+logs`, `metadata-only`, `none` | `none` |
| Artifacts | `metadata-only`, `blobs`, `none` | `metadata-only` |
| Repos | `none`, `bundles` | `none` |

```bash
WS_ROOT=$(ata workspace resolve '@ws' --workspace "$WID" | sed 's|/$||')
BUNDLE_PATH="/tmp/$WID-export.tar.gz"

# Sanitize URLs before export
ata workspace read --workspace "$WID" | \
  jq '.repos = [.repos[] | .remoteUrl = (.remoteUrl | split("?")[0])]' \
  > "/tmp/$WID-export-manifest.json"

tar -czf "$BUNDLE_PATH" -C "$WS_ROOT" \
  workspace.json notes/workspace/ knowledge-base/
echo "Exported to: $BUNDLE_PATH"
```

## Import Workspace Bundle

```bash
NEW_WID=$(ata workspace init "Imported Project")
NEW_ROOT=$(ata workspace resolve '@ws' --workspace "$NEW_WID" | sed 's|/$||')
tar -xzf "$BUNDLE_PATH" -C "$NEW_ROOT"
ata workspace set-field --path id --value '"$NEW_WID"' --workspace "$NEW_WID"
echo "Imported as: $NEW_WID"
```

**Important:** Never auto-commit imported repos. Only commit if the user explicitly requests it.
