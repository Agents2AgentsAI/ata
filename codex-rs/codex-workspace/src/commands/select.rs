use crate::error::WorkspaceError;
use crate::paths;
use crate::selection::write_selection_for;
use crate::workspace_resolution;
use std::path::Path;

/// Resolve a workspace selector and set it as the active selection.
///
/// Returns the resolved workspace ID.
pub fn run(selector: &str) -> Result<String, WorkspaceError> {
    let context = paths::SessionContext::from_env();
    run_for(
        &context.codex_home,
        selector,
        context.session_id.as_deref(),
        context.thread_id.as_deref(),
    )
}

fn run_for(
    codex_home: &Path,
    selector: &str,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> Result<String, WorkspaceError> {
    let workspace_id = workspace_resolution::resolve_workspace_selector_for(codex_home, selector)?;
    write_selection_for(codex_home, &workspace_id, session_id, thread_id)?;
    Ok(workspace_id)
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

        let resolved =
            run_for(temp.path(), "workspace-1", Some("session-1"), None).expect("select workspace");
        assert_eq!(resolved, "workspace-1");

        let path = paths::selection_path_for(temp.path(), Some("session-1"));
        assert_eq!(
            read_workspace_selection_file_for(temp.path(), &path),
            Some("workspace-1".to_string())
        );
    }
}
