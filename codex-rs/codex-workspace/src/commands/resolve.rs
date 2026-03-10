use crate::error::WorkspaceError;
use crate::resolve::resolve_at_spec;
use std::path::PathBuf;

/// Resolve an @-path spec to an absolute path within a workspace.
pub fn run(workspace_id: &str, spec: &str) -> Result<PathBuf, WorkspaceError> {
    resolve_at_spec(workspace_id, spec)
}
