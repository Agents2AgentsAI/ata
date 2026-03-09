use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
use crate::types::manifest_collection_mut;

/// Remove an entry by ID from a named collection in the manifest.
pub fn run(
    workspace_id: &str,
    collection: &str,
    entry_id: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let collection = collection.to_string();
    let entry_id = entry_id.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let vec = manifest_collection_mut(m, &collection)?;
        let before_len = vec.len();
        vec.retain(|v| {
            v.get("id")
                .and_then(|id| id.as_str())
                .map(|id| id != entry_id)
                .unwrap_or(true)
        });
        if vec.len() == before_len {
            return Err(WorkspaceError::EntryNotFound(entry_id));
        }
        Ok(())
    })
}
