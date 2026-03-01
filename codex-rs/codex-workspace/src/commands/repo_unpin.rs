use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;

/// Unpin a repo (switch back to tracking mode).
pub fn run(workspace_id: &str, alias: &str) -> Result<WorkspaceManifest, WorkspaceError> {
    let alias = alias.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let repo = m
            .repos
            .iter_mut()
            .find(|r| r.alias == alias)
            .ok_or_else(|| WorkspaceError::EntryNotFound(alias.clone()))?;
        repo.pin.mode = "tracking".to_string();
        repo.pin.pinned_sha = String::new();
        Ok(())
    })
}
