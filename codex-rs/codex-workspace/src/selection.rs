use crate::error::WorkspaceError;
use crate::manifest::atomic_write;
use crate::paths;
use crate::types::WorkspaceSelection;

/// Read a workspace ID from a selection file.
///
/// Handles two formats:
/// - Structured JSON: `{"schemaVersion":1,"activeWorkspaceId":"..."}`
/// - Legacy bare ID string
///
/// Returns `None` if file missing, malformed, or referenced workspace doesn't exist.
pub fn read_workspace_selection_file(path: &std::path::Path) -> Option<String> {
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

    if !wid.is_empty() && paths::manifest_path(&wid).is_file() {
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
    let sid = std::env::var("CODEX_SESSION_ID")
        .unwrap_or_default()
        .trim()
        .to_string();
    if sid.is_empty() {
        return None;
    }
    let path = paths::codex_home()
        .join("sessions")
        .join(&sid)
        .join("workspace.json");
    read_workspace_selection_file(&path)
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
