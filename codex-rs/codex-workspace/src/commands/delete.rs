use crate::error::WorkspaceError;
use crate::paths;
use crate::selection;

/// Delete a workspace directory tree.
pub fn run(workspace_id: &str, force: bool) -> Result<(), WorkspaceError> {
    if workspace_id == "global" {
        return Err(WorkspaceError::DeleteGlobal);
    }
    if !force {
        return Err(WorkspaceError::DeleteRequiresForce);
    }
    let root = paths::workspace_root(workspace_id);
    if !root.is_dir() {
        return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
    }
    std::fs::remove_dir_all(&root)?;
    selection::clear_workspace_selection(workspace_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn delete_requires_force() {
        let err = run("workspace-1", false).expect_err("delete should require force");
        assert_eq!(
            err.to_string(),
            WorkspaceError::DeleteRequiresForce.to_string()
        );
    }
}
