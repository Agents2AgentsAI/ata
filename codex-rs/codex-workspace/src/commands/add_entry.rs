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
    validate_entry(&entry)?;
    let collection = collection.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let vec = manifest_collection_mut(m, &collection)?;
        vec.push(entry);
        Ok(())
    })
}

fn validate_entry(entry: &Value) -> Result<(), WorkspaceError> {
    let has_id = entry
        .as_object()
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if has_id {
        Ok(())
    } else {
        Err(WorkspaceError::InvalidCollectionEntryId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_entry_accepts_object_with_non_empty_id() {
        assert!(validate_entry(&json!({"id": "entry-1"})).is_ok());
    }

    #[test]
    fn validate_entry_rejects_missing_or_empty_id() {
        assert!(matches!(
            validate_entry(&json!({"name": "entry"})),
            Err(WorkspaceError::InvalidCollectionEntryId)
        ));
        assert!(matches!(
            validate_entry(&json!({"id": ""})),
            Err(WorkspaceError::InvalidCollectionEntryId)
        ));
    }
}
