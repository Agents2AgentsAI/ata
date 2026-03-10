use crate::error::WorkspaceError;
use crate::manifest::read_manifest;
use crate::types::WorkspaceManifest;

/// Read and return the workspace manifest.
pub fn run(workspace_id: &str) -> Result<WorkspaceManifest, WorkspaceError> {
    read_manifest(workspace_id)
}
