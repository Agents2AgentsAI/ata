use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::WorkspaceManifest;

/// Update a repo's git state (headSha, optionally headRef).
pub fn run(
    workspace_id: &str,
    alias: &str,
    head_sha: &str,
    head_ref: Option<&str>,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let alias = alias.to_string();
    let head_sha = head_sha.to_string();
    let head_ref = head_ref.map(str::to_string);

    with_locked_manifest(workspace_id, None, move |m| {
        let repo = m
            .repos
            .iter_mut()
            .find(|r| r.alias == alias)
            .ok_or_else(|| WorkspaceError::EntryNotFound(alias.clone()))?;
        repo.state.head_sha = head_sha;
        if let Some(ref hr) = head_ref {
            repo.state.head_ref = hr.clone();
        }
        Ok(())
    })
}
