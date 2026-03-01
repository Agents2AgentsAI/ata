use crate::error::WorkspaceError;
use crate::paths;

/// Delete a workspace directory tree.
pub fn run(workspace_id: &str) -> Result<(), WorkspaceError> {
    if workspace_id == "global" {
        return Err(WorkspaceError::DeleteGlobal);
    }
    let root = paths::workspace_root(workspace_id);
    if !root.is_dir() {
        return Err(WorkspaceError::WorkspaceNotFound(
            workspace_id.to_string(),
        ));
    }
    std::fs::remove_dir_all(&root)?;
    Ok(())
}
