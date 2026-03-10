use crate::error::WorkspaceError;
use crate::paths;
use crate::selection;
use crate::workspace_id::validate_workspace_id;
use std::path::Path;

/// Delete a workspace directory tree.
pub fn run(workspace_id: &str, force: bool) -> Result<(), WorkspaceError> {
    run_for(&paths::codex_home(), workspace_id, force)?;
    selection::clear_workspace_selection(workspace_id)?;
    Ok(())
}

fn run_for(codex_home: &Path, workspace_id: &str, force: bool) -> Result<(), WorkspaceError> {
    validate_workspace_id(workspace_id)?;
    if workspace_id == "global" {
        return Err(WorkspaceError::DeleteGlobal);
    }
    if !force {
        return Err(WorkspaceError::DeleteRequiresForce);
    }
    let root = paths::workspace_root_for(codex_home, workspace_id);
    if !root.is_dir() || !paths::manifest_path_for(codex_home, workspace_id).is_file() {
        return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
    }
    std::fs::remove_dir_all(&root)?;
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

    #[test]
    fn delete_requires_manifest() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let workspace_root = paths::workspace_root_for(temp.path(), "workspace-1");
        std::fs::create_dir_all(&workspace_root).expect("create workspace dir");

        let err =
            run_for(temp.path(), "workspace-1", true).expect_err("delete should require manifest");
        assert_eq!(
            err.to_string(),
            WorkspaceError::WorkspaceNotFound("workspace-1".to_string()).to_string()
        );
    }
}
