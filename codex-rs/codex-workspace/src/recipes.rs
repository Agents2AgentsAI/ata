/// Get a recipe by name. Returns `None` if unknown.
pub fn get_recipe(name: &str) -> Option<&'static str> {
    match name {
        "repo_update" => Some(RECIPE_REPO_UPDATE),
        "repo_pin" => Some(RECIPE_REPO_PIN),
        "repo_unpin" => Some(RECIPE_REPO_UNPIN),
        "repo_remove" => Some(RECIPE_REPO_REMOVE),
        "run_exec" => Some(RECIPE_RUN_EXEC),
        "run_delete" => Some(RECIPE_RUN_DELETE),
        "run_gc" => Some(RECIPE_RUN_GC),
        "resource_add" => Some(RECIPE_RESOURCE_ADD),
        "link_add" => Some(RECIPE_LINK_ADD),
        "snapshot_create" => Some(RECIPE_SNAPSHOT_CREATE),
        "snapshot_restore" => Some(RECIPE_SNAPSHOT_RESTORE),
        "export" => Some(RECIPE_EXPORT),
        "import" => Some(RECIPE_IMPORT),
        "index_build" => Some(RECIPE_INDEX_BUILD),
        "materialize" => Some(RECIPE_MATERIALIZE),
        "export_spec" => Some(RECIPE_EXPORT_SPEC),
        _ => None,
    }
}

/// List all available recipe names, sorted.
pub fn list_recipes() -> Vec<&'static str> {
    let mut names = vec![
        "export",
        "export_spec",
        "import",
        "index_build",
        "link_add",
        "materialize",
        "repo_pin",
        "repo_remove",
        "repo_unpin",
        "repo_update",
        "resource_add",
        "run_delete",
        "run_exec",
        "run_gc",
        "snapshot_create",
        "snapshot_restore",
    ];
    names.sort();
    names
}

const RECIPE_REPO_UPDATE: &str = r#"# Update repository to latest upstream
ALIAS="<repo_alias>"
REPO_PATH=$(ata workspace resolve "@$ALIAS" --workspace "$WID")

git -C "$REPO_PATH" fetch --depth 1
DEFAULT=$(git -C "$REPO_PATH" rev-parse --abbrev-ref origin/HEAD | sed 's|origin/||')
git -C "$REPO_PATH" reset --hard "origin/$DEFAULT"

HEAD_SHA=$(git -C "$REPO_PATH" rev-parse HEAD)
ata workspace repo-update-state --alias "$ALIAS" --head-sha "$HEAD_SHA" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"repo_update","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
"#;

const RECIPE_REPO_PIN: &str = r#"# Pin repository to a specific commit
ALIAS="<repo_alias>"
SHA="<commit_sha>"
ata workspace repo-pin --alias "$ALIAS" --sha "$SHA" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"repo_pin","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
"#;

const RECIPE_REPO_UNPIN: &str = r#"# Unpin repository (switch back to tracking mode)
ALIAS="<repo_alias>"
ata workspace repo-unpin --alias "$ALIAS" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"repo_track","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
"#;

const RECIPE_REPO_REMOVE: &str = r#"# Remove repository from workspace
ALIAS="<repo_alias>"
ata workspace repo-remove --alias "$ALIAS" --workspace "$WID"
"#;

const RECIPE_RUN_EXEC: &str = r#"# Execute a command in an existing run
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
"#;

const RECIPE_RUN_DELETE: &str = r#"# Delete a run and clean up worktree
RUN_ID="<run_id>"
ata workspace run-remove --id "$RUN_ID" --workspace "$WID"
"#;

const RECIPE_RUN_GC: &str = r#"# List stale runs (completed/failed, older than 7 days)
ata workspace read --workspace "$WID" | jq -r \
  '.runs[] | select(.status == "completed" or .status == "failed") | select(.updatedAt < (now - 604800)) | .id'
# Then delete each with: ata workspace run-remove --id "$RUN_ID" --workspace "$WID"
"#;

const RECIPE_RESOURCE_ADD: &str = r#"# Add a resource (paper, dataset, or artifact)
# Replace <type> with: papers, datasets, or artifacts
TYPE="<type>"      # papers | datasets | artifacts
PREFIX="<prefix>"  # paper | dataset | artifact

# Generate resource ID
RESOURCE_ID="$PREFIX-$(date +%s)-$(uuidgen | tr -d '-' | tr '[:upper:]' '[:lower:]')"
WS_ROOT=$(ata workspace resolve '@notes' --workspace "$WID" | sed 's|/workspace$||')

ata workspace add-entry --collection "$TYPE" \
  --json "{\"id\":\"$RESOURCE_ID\",\"title\":\"<title>\",\"notesPath\":\"notes/$TYPE/$RESOURCE_ID\"}" \
  --workspace "$WID"
mkdir -p "$WS_ROOT/../notes/$TYPE/$RESOURCE_ID"
ata workspace audit --workspace "$WID" \
  '{"op":"'"${PREFIX}_add"'","targets":[{"type":"'"$PREFIX"'","id":"'"$RESOURCE_ID"'"}]}'

# Additional fields per type:
# Papers: "authors":[], "year":N, "doi":"", "arxiv":"", "url":"", "pdfArtifactId":""
# Datasets: "name":"", "url":"", "license":"", "artifactIds":[]
# Artifacts: "kind":"" (required: pdf|model|data), "sourceUrl":"", "sha256":"", "sizeBytes":N
"#;

const RECIPE_LINK_ADD: &str = r#"# Add a link between two resources
ata workspace add-entry --collection links \
  --json '{"from":{"type":"<from_type>","id":"<from_id>"},"to":{"type":"<to_type>","id":"<to_id>"},"kind":"<kind>"}' \
  --workspace "$WID"
# Common kinds: implements, uses, produces, derived_from, related_to
"#;

const RECIPE_SNAPSHOT_CREATE: &str = r#"# Create a manifest snapshot
SNAP_ID="snap-$(date +%s)-$(uuidgen | tr -d '-' | tr '[:upper:]' '[:lower:]')"
WS_ROOT=$(ata workspace resolve '@notes' --workspace "$WID" | sed 's|notes/workspace$||')
SNAP_PATH="notes/workspace/snapshots/$SNAP_ID.json"
cp "$WS_ROOT/workspace.json" "$WS_ROOT/$SNAP_PATH"
ata workspace add-entry --collection snapshots \
  --json "{\"id\":\"$SNAP_ID\",\"path\":\"$SNAP_PATH\"}" \
  --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"snapshot_create","targets":[{"type":"snapshot","id":"'"$SNAP_ID"'"}]}'
"#;

const RECIPE_SNAPSHOT_RESTORE: &str = r#"# Restore repo pins from a snapshot (additive — never deletes repos)
WS_ROOT=$(ata workspace resolve '@notes' --workspace "$WID" | sed 's|notes/workspace$||')
SNAP_PATH="<path_to_snapshot>"  # e.g. notes/workspace/snapshots/snap-XXX.json

# For each repo in snapshot:
jq -c '.repos[]' "$WS_ROOT/$SNAP_PATH" | while read -r REPO; do
  ALIAS=$(echo "$REPO" | jq -r '.alias')
  PIN_SHA=$(echo "$REPO" | jq -r '.pin.pinnedSha // empty')
  # Skip if alias already exists (or handle per policy)
  # Checkout pinned SHA if present:
  if [ -n "$PIN_SHA" ]; then
    git -C "$WS_ROOT/repos/$ALIAS" fetch origin "$PIN_SHA" --depth 1 2>/dev/null || true
    git -C "$WS_ROOT/repos/$ALIAS" checkout "$PIN_SHA" 2>/dev/null || true
    ata workspace repo-pin --alias "$ALIAS" --sha "$PIN_SHA" --workspace "$WID"
  fi
done
"#;

const RECIPE_EXPORT: &str = r#"# Export workspace bundle
WS_ROOT=$(ata workspace resolve '@notes' --workspace "$WID" | sed 's|notes/workspace$||')
BUNDLE_PATH="/tmp/$WID-export.tar.gz"

# Build file list (customize modes: notes, runs, artifacts, repos)
tar -czf "$BUNDLE_PATH" -C "$WS_ROOT" \
  workspace.json notes/workspace/ knowledge-base/
echo "Exported to: $BUNDLE_PATH"
"#;

const RECIPE_IMPORT: &str = r#"# Import workspace bundle
NEW_WID=$(ata workspace init "Imported Project")
NEW_ROOT=$(ata workspace resolve '@notes' --workspace "$NEW_WID" | sed 's|notes/workspace$||')
tar -xzf "$BUNDLE_PATH" -C "$NEW_ROOT"
ata workspace set-field --path id --value "\"$NEW_WID\"" --workspace "$NEW_WID"
echo "Imported as: $NEW_WID"
"#;

const RECIPE_INDEX_BUILD: &str = r#"# Build a search index (scaffold — uses external indexing tool)
INDEX_ID="idx-$(date +%s)-$(uuidgen | tr -d '-' | tr '[:upper:]' '[:lower:]')"
WS_ROOT=$(ata workspace resolve '@notes' --workspace "$WID" | sed 's|notes/workspace$||')
INDEX_PATH="indexes/$INDEX_ID"
mkdir -p "$WS_ROOT/$INDEX_PATH"
NOW=$(date +%s)
ata workspace add-entry --collection indexes \
  --json "{\"id\":\"$INDEX_ID\",\"kind\":\"keyword\",\"targetType\":\"repo\",\"targetId\":\"$REPO_ID\",\"createdAt\":$NOW,\"status\":\"building\",\"path\":\"$INDEX_PATH\"}" \
  --workspace "$WID"
# ... run external indexing tool, then mark ready:
ata workspace index-update-status --id "$INDEX_ID" --status ready --workspace "$WID"
"#;

const RECIPE_MATERIALIZE: &str = r#"# Materialize workspace from a spec file
SPEC_PATH="<path_to_workspace-spec.json>"

# Preview what would change:
ata workspace diff-spec "$SPEC_PATH" --workspace "$WID"

# Materialize (create/update workspace from spec):
ata workspace materialize "$SPEC_PATH" --workspace "$WID"

# Or dry-run first:
ata workspace materialize "$SPEC_PATH" --workspace "$WID" --dry-run

# Or create a new workspace from the spec (omit --workspace):
ata workspace materialize "$SPEC_PATH"
"#;

const RECIPE_EXPORT_SPEC: &str = r#"# Export workspace to a portable spec file
# Print to stdout:
ata workspace export-spec --workspace "$WID"

# Write to file:
ata workspace export-spec --workspace "$WID" --output workspace-spec.json

# Round-trip: export → materialize into new workspace
ata workspace export-spec --workspace "$WID" --output /tmp/spec.json
NEW_WID=$(ata workspace init "imported-workspace")
ata workspace materialize /tmp/spec.json --workspace "$NEW_WID"
"#;
