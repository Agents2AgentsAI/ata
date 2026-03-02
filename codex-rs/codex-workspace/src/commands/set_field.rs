use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;
use serde_json::Value;

/// Set a field at a dotted path in the manifest.
///
/// Supports paths like `id`, `policies.repoHostsAllowlist`, `labels.foo`.
/// The value is parsed as JSON.
pub fn run(
    workspace_id: &str,
    path: &str,
    value_str: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let value: Value =
        serde_json::from_str(value_str).map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;
    let path = path.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        // Convert manifest to Value, set path, convert back
        let mut v = serde_json::to_value(&*m).map_err(WorkspaceError::Json)?;
        set_nested_value(&mut v, &path, value.clone())?;
        *m = serde_json::from_value(v).map_err(WorkspaceError::Json)?;
        Ok(())
    })
}

/// Set a value at a dotted path within a JSON Value.
fn set_nested_value(root: &mut Value, path: &str, value: Value) -> Result<(), WorkspaceError> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return Err(WorkspaceError::InvalidFieldPath(path.to_string()));
    }

    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Set the value
            if let Some(obj) = current.as_object_mut() {
                obj.insert((*part).to_string(), value);
                return Ok(());
            }
            return Err(WorkspaceError::InvalidFieldPath(path.to_string()));
        }
        // Navigate deeper
        if let Some(obj) = current.as_object_mut() {
            current = obj
                .entry((*part).to_string())
                .or_insert(Value::Object(serde_json::Map::new()));
        } else {
            return Err(WorkspaceError::InvalidFieldPath(path.to_string()));
        }
    }
    Err(WorkspaceError::InvalidFieldPath(path.to_string()))
}
