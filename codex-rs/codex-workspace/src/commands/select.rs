use crate::error::WorkspaceError;
use crate::paths;
use crate::selection::write_selection;

/// Set the active workspace selection.
pub fn run(workspace_id: &str) -> Result<(), WorkspaceError> {
    if !paths::manifest_path(workspace_id).is_file() {
        return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
    }
    write_selection(workspace_id)
}
