use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
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
        let vec = get_collection_mut(m, &collection)?;
        vec.push(entry);
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
