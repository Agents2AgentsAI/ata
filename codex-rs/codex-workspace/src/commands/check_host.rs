use crate::error::WorkspaceError;
use crate::manifest::read_manifest;
use crate::url_validation::{check_host_allowlist, validate_repo_url};

/// Validate a URL and check it against the workspace host allowlist.
pub fn run(workspace_id: &str, url: &str) -> Result<(), WorkspaceError> {
    validate_repo_url(url)?;
    let manifest = read_manifest(workspace_id)?;
    check_host_allowlist(url, manifest.policies.repo_hosts_allowlist.as_deref())
}
