use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::with_locked_manifest;
use crate::types::PinMode;
use crate::types::WorkspaceManifest;

/// Pin a repo to a specific SHA.
pub fn run(
    workspace_id: &str,
    alias: &str,
    sha: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    if !git::is_valid_commit_sha(sha) {
        return Err(WorkspaceError::InvalidSha(sha.to_string()));
    }
    let alias = alias.to_string();
    let sha = sha.to_string();

    with_locked_manifest(workspace_id, None, move |m| {
        let repo = m
            .repos
            .iter_mut()
            .find(|r| r.alias == alias)
            .ok_or_else(|| WorkspaceError::EntryNotFound(alias.clone()))?;
        repo.pin.mode = PinMode::Pinned;
        repo.pin.pinned_sha = sha;
        Ok(())
    })
}
