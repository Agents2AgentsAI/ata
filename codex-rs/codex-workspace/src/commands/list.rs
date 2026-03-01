use crate::error::WorkspaceError;
use crate::paths;
use crate::types::WorkspaceSummary;

/// List all workspaces as summaries.
pub fn run() -> Result<Vec<WorkspaceSummary>, WorkspaceError> {
    let root = paths::workspaces_root();
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&root)?
        .filter_map(std::result::Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let mp = entry.path().join("workspace.json");
        if !mp.is_file() {
            continue;
        }
        let data = match std::fs::read_to_string(&mp) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let manifest: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let entry_name = entry.file_name();
        let fallback = entry_name.to_str().unwrap_or("");
        let id = manifest
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_string();
        let name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let updated_at = manifest
            .get("updatedAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let repo_count = manifest
            .get("repos")
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);

        results.push(WorkspaceSummary {
            id,
            name,
            updated_at,
            repo_count,
        });
    }

    Ok(results)
}
