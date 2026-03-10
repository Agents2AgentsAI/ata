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
TMP_MANIFEST=$(mktemp)
jq --arg wid "$NEW_WID" '.id = $wid' "$NEW_ROOT/workspace.json" > "$TMP_MANIFEST"
mv "$TMP_MANIFEST" "$NEW_ROOT/workspace.json"
echo "Imported as: $NEW_WID"
```

**Important:** Never auto-commit imported repos. Only commit if the user explicitly requests it.

## Workspace Spec (Portable Configuration)

A **workspace spec** (`workspace-spec.json`) is a portable, human-readable, Git-friendly file that declares the desired contents of a workspace. Unlike snapshots (which are internal state) and export bundles (which are opaque archives), a spec is designed to be checked into a pipeline/project repo and reviewed in PRs.

### Spec Format

```jsonc
{
  "schemaVersion": 1,
  "name": "3d-recon-pipeline",
  "repos": [
    {
      "url": "https://github.com/colmap/colmap.git",
      "alias": "colmap",
      "sha": "abc123...",           // optional: pin to exact SHA
      "ref": "main",               // optional: branch/tag (used if no sha)
      "full": false,               // optional: override clone policy
      "role": "sfm"                // optional: stored in extra
    }
  ],
  "policies": { ... },             // optional: override default clone policy
  "labels": { ... }                // optional: workspace labels
}
```

### Export Spec

Export the current workspace state as a portable spec:

```bash
# Print to stdout
ata workspace export-spec --workspace "$WID"

# Write to file
ata workspace export-spec --workspace "$WID" --output workspace-spec.json
```

### Diff Spec

Preview what materializing a spec would do:

```bash
ata workspace diff-spec workspace-spec.json --workspace "$WID"
# Output: lists repos to add, pin, or skip
```

### Materialize Spec

Create or update a workspace from a spec file:

```bash
# Into existing workspace
ata workspace materialize workspace-spec.json --workspace "$WID"

# Create new workspace from spec (auto-names from spec.name)
ata workspace materialize workspace-spec.json

# Dry run — show plan without executing
ata workspace materialize workspace-spec.json --workspace "$WID" --dry-run
```

Materialize logic per repo:
- **Not in workspace** → clone + pin (if sha specified) + apply extra fields
- **In workspace, SHA differs** → re-pin + apply extra fields
- **In workspace, matches** → skip (still applies extra field changes)

Also applies spec-level `policies`, `labels`, and records `specSource` provenance.

### Round-Trip

```bash
# Export from workspace A
ata workspace export-spec --workspace "$WID_A" --output spec.json

# Materialize into workspace B
ata workspace materialize spec.json --workspace "$WID_B"

# Verify no drift
ata workspace diff-spec spec.json --workspace "$WID_B"
# → "No changes needed."
```
