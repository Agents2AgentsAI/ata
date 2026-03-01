use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
use serde_json::Value;

/// Remove an entry by ID from a named collection in the manifest.
pub fn run(
    workspace_id: &str,
    collection: &str,
    entry_id: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let collection = collection.to_string();
    let entry_id = entry_id.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let vec = get_collection_mut(m, &collection)?;
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

fn get_collection_mut<'a>(
    m: &'a mut WorkspaceManifest,
    collection: &str,
) -> Result<&'a mut Vec<Value>, WorkspaceError> {
    match collection {
        "papers" => Ok(&mut m.papers),
        "datasets" => Ok(&mut m.datasets),
        "artifacts" => Ok(&mut m.artifacts),
        "links" => Ok(&mut m.links),
        "snapshots" => Ok(&mut m.snapshots),
        "indexes" => Ok(&mut m.indexes),
        _ => Err(WorkspaceError::UnknownCollection(collection.to_string())),
    }
}
