use crate::error::WorkspaceError;
use crate::paths;
use crate::selection::write_selection_for;
use std::path::Path;

/// Set the active workspace selection.
pub fn run(workspace_id: &str) -> Result<(), WorkspaceError> {
    let context = paths::SessionContext::from_env();
    run_for(
        &context.codex_home,
        workspace_id,
        context.session_id.as_deref(),
        context.thread_id.as_deref(),
    )
}

fn run_for(
    codex_home: &Path,
    workspace_id: &str,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> Result<(), WorkspaceError> {
    if !paths::manifest_path_for(codex_home, workspace_id).is_file() {
        return Err(WorkspaceError::WorkspaceNotFound(workspace_id.to_string()));
    }
    write_selection_for(codex_home, workspace_id, session_id, thread_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::write_manifest_atomic_for;
    use crate::selection::read_workspace_selection_file_for;
    use crate::types::new_manifest;

    #[test]
    fn run_writes_scoped_selection_for_existing_workspace() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "workspace-1",
            &new_manifest("workspace-1", "One"),
        )
        .expect("write manifest");

        run_for(temp.path(), "workspace-1", Some("session-1"), None).expect("select workspace");

        let path = paths::selection_path_for(temp.path(), Some("session-1"));
        assert_eq!(
            read_workspace_selection_file_for(temp.path(), &path),
            Some("workspace-1".to_string())
        );
    }
}
