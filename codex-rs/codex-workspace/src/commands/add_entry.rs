use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
use crate::types::manifest_collection_mut;
use serde_json::Value;

/// Append a JSON object to a named collection in the manifest.
pub fn run(
    workspace_id: &str,
    collection: &str,
    json_str: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let entry: Value =
        serde_json::from_str(json_str).map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;
    let collection = collection.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let vec = manifest_collection_mut(m, &collection)?;
        vec.push(entry);
        Ok(())
    })
}
