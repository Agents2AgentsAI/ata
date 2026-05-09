use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
use crate::types::remove_manifest_collection_entry;

/// Remove an entry by ID from a named collection in the manifest.
pub fn run(
    workspace_id: &str,
    collection: &str,
    entry_id: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let collection = collection.to_string();
    let entry_id = entry_id.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        if !remove_manifest_collection_entry(m, &collection, &entry_id)? {
            return Err(WorkspaceError::EntryNotFound(entry_id));
        }
        Ok(())
    })
}
