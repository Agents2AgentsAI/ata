use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;

/// Update an index entry's status field.
pub fn run(
    workspace_id: &str,
    index_id: &str,
    status: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let index_id = index_id.to_string();
    let status = status.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let entry = m
            .indexes
            .iter_mut()
            .find(|v| v.get("id").and_then(|id| id.as_str()) == Some(&index_id))
            .ok_or_else(|| WorkspaceError::EntryNotFound(index_id.clone()))?;
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("status".to_string(), serde_json::Value::String(status));
            obj.insert(
                "updatedAt".to_string(),
                serde_json::Value::Number(chrono::Utc::now().timestamp().into()),
            );
        }
        Ok(())
    })
}
