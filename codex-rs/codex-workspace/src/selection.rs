use crate::error::WorkspaceError;
use crate::manifest::atomic_write;
use crate::paths;
use crate::types::WorkspaceSelection;
use std::path::Path;

/// Read a workspace ID from a selection file.
///
/// Handles two formats:
/// - Structured JSON: `{"schemaVersion":1,"activeWorkspaceId":"..."}`
/// - Legacy bare ID string
///
/// Returns `None` if file missing, malformed, or referenced workspace doesn't exist.
pub fn read_workspace_selection_file(path: &std::path::Path) -> Option<String> {
    read_workspace_selection_file_with(path, &|wid| paths::manifest_path(wid).is_file())
}

fn read_workspace_selection_file_with(
    path: &std::path::Path,
    workspace_exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // Try structured JSON first
    let wid = if let Ok(data) = serde_json::from_str::<serde_json::Value>(raw) {
        data.as_object()
            .and_then(|obj| obj.get("activeWorkspaceId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        // Legacy: bare workspace ID string
        raw.to_string()
    };

    if !wid.is_empty() && workspace_exists(&wid) {
        Some(wid)
    } else {
        None
    }
}

/// Write the active workspace selection (session-aware).
pub fn write_selection(workspace_id: &str) -> Result<(), WorkspaceError> {
    let sp = paths::selection_path();
    if let Some(parent) = sp.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let selection = WorkspaceSelection {
        schema_version: 1,
        active_workspace_id: workspace_id.to_string(),
        updated_at: chrono::Utc::now().timestamp(),
    };
    let data = serde_json::to_string_pretty(&selection)?;
    atomic_write(&sp, data.as_bytes())
}

/// Read the session-scoped workspace selection.
pub fn read_session_workspace() -> Option<String> {
    let scoped_path = paths::selection_path();
    let global_path = paths::global_selection_path();
    read_session_workspace_from_paths_with(&scoped_path, &global_path, &|wid| {
        paths::manifest_path(wid).is_file()
    })
}

fn read_session_workspace_from_paths_with(
    scoped_path: &Path,
    global_path: &Path,
    workspace_exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    if let Some(wid) = read_workspace_selection_file_with(scoped_path, workspace_exists) {
        return Some(wid);
    }

    if scoped_path != global_path {
        return read_workspace_selection_file_with(global_path, workspace_exists);
    }

    None
}

/// Discover project-pinned workspace by walking ancestors from cwd.
///
/// Stops at `.git` boundary or filesystem root.
pub fn discover_project_pin(cwd: &std::path::Path) -> Option<String> {
    let mut current = match std::fs::canonicalize(cwd) {
        Ok(p) => p,
        Err(_) => cwd.to_path_buf(),
    };
    loop {
        let candidate = current.join(".codex").join("workspace.json");
        if let Some(wid) = read_workspace_selection_file(&candidate) {
            return Some(wid);
        }
        // Stop at .git boundary
        if current.join(".git").is_dir() {
            break;
        }
        match current.parent() {
            Some(parent) if parent != current => {
                current = parent.to_path_buf();
            }
            _ => break,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_selection_file(path: &Path, workspace_id: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create selection parent");
        }
        std::fs::write(
            path,
            format!("{{\"schemaVersion\":1,\"activeWorkspaceId\":\"{workspace_id}\"}}"),
        )
        .expect("write selection file");
    }

    #[test]
    fn read_session_workspace_prefers_scoped_selection() {
        let temp = TempDir::new().expect("create temp dir");
        let scoped_path = temp
            .path()
            .join("sessions")
            .join("thread-1")
            .join("workspace.json");
        let global_path = temp.path().join(".workspace_selected");
        write_selection_file(&scoped_path, "scoped-ws");
        write_selection_file(&global_path, "global-ws");

        let result = read_session_workspace_from_paths_with(&scoped_path, &global_path, &|wid| {
            matches!(wid, "scoped-ws" | "global-ws")
        });
        assert_eq!(result, Some("scoped-ws".to_string()));
    }

    #[test]
    fn read_session_workspace_falls_back_to_global_selection() {
        let temp = TempDir::new().expect("create temp dir");
        let scoped_path = temp
            .path()
            .join("sessions")
            .join("thread-1")
            .join("workspace.json");
        let global_path = temp.path().join(".workspace_selected");
        write_selection_file(&global_path, "global-ws");

        let result = read_session_workspace_from_paths_with(&scoped_path, &global_path, &|wid| {
            wid == "global-ws"
        });
        assert_eq!(result, Some("global-ws".to_string()));
    }

    #[test]
    fn read_session_workspace_ignores_global_fallback_for_same_path() {
        let temp = TempDir::new().expect("create temp dir");
        let scoped_path = temp.path().join(".workspace_selected");
        write_selection_file(&scoped_path, "global-ws");

        let result = read_session_workspace_from_paths_with(&scoped_path, &scoped_path, &|wid| {
            wid == "global-ws"
        });
        assert_eq!(result, Some("global-ws".to_string()));
    }

    #[test]
    fn read_session_workspace_returns_none_for_invalid_or_missing_workspace() {
        let temp = TempDir::new().expect("create temp dir");
        let scoped_path = temp
            .path()
            .join("sessions")
            .join("thread-1")
            .join("workspace.json");
        let global_path = temp.path().join(".workspace_selected");
        write_selection_file(&global_path, "missing-ws");

        let result = read_session_workspace_from_paths_with(&scoped_path, &global_path, &|_| false);
        assert_eq!(result, None);
    }
}
