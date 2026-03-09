use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::with_locked_manifest;
use crate::paths;
use crate::types::WorkspaceManifest;

/// Update a repo's git state (headSha, optionally headRef).
pub fn run(
    workspace_id: &str,
    alias: &str,
    head_sha: &str,
    head_ref: Option<&str>,
) -> Result<WorkspaceManifest, WorkspaceError> {
    if !git::is_valid_commit_sha(head_sha) {
        return Err(WorkspaceError::InvalidSha(head_sha.to_string()));
    }
    let alias = alias.to_string();
    let head_sha = head_sha.to_string();
    let head_ref = head_ref.map(str::to_string);
    let workspace_root = paths::workspace_root(workspace_id);

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
        let git_state = git::read_git_state(&workspace_root.join(&repo.checkout_path));
        if !git_state.default_branch.is_empty() {
            repo.state.default_branch = git_state.default_branch;
        }
        repo.state.shallow = git_state.shallow;
        Ok(())
    })
}
