use crate::audit::write_audit;
use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::{read_manifest, with_locked_manifest};
use crate::paths;
use crate::resolve::is_reserved_alias;
use crate::types::{CloneRecord, PinState, RepoEntry};
use crate::url_validation::{check_host_allowlist, validate_repo_url};
use crate::workspace_id::make_id;
use regex::Regex;
use serde_json::{Map, json};
use std::sync::LazyLock;

// SAFETY: regex pattern is a compile-time string literal and is known valid.
#[allow(clippy::expect_used)]
static ALIAS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").expect("valid regex"));

/// Full repo_add flow: validate URL → clone → register → audit.
/// Returns JSON-serializable result.
pub fn run(
    workspace_id: &str,
    url: &str,
    alias: &str,
    full: bool,
) -> Result<serde_json::Value, WorkspaceError> {
    // Validate alias
    if is_reserved_alias(alias) {
        return Err(WorkspaceError::ReservedAlias(alias.to_string()));
    }
    if !ALIAS_RE.is_match(alias) {
        return Err(WorkspaceError::InvalidAlias {
            alias: alias.to_string(),
        });
    }

    // 1. Validate URL
    validate_repo_url(url)?;
    let manifest = read_manifest(workspace_id)?;
    check_host_allowlist(url, manifest.policies.repo_hosts_allowlist.as_deref())?;

    // 2. Check alias not already used
    for repo in &manifest.repos {
        if repo.alias == alias {
            return Err(WorkspaceError::AliasExists(alias.to_string()));
        }
    }

    let version = manifest.manifest_version;
    let clone_policy = &manifest.policies.default_clone;

    // 3. Build clone args
    let clone_args = git::build_clone_args(clone_policy, full);

    // 4. Clone
    let root = paths::workspace_root(workspace_id);
    let clone_dest = root.join("repos").join(alias);
    if clone_dest.exists() {
        return Err(WorkspaceError::DirectoryExists(clone_dest));
    }

    let exit_code = git::clone_repo(url, &clone_dest, &clone_args)?;
    if exit_code != 0 {
        return Err(WorkspaceError::GitCloneFailed(exit_code));
    }

    // 4b. LFS pull if policy says so
    let lfs_policy = &clone_policy.lfs;
    if lfs_policy == "always" || lfs_policy == "auto" {
        git::lfs_pull(&clone_dest);
    }

    // 5. Read git state
    let mut git_state = git::read_git_state(&clone_dest);
    let depth = if full { 0 } else { clone_policy.depth };
    git_state.shallow = depth > 0;

    // 6. Build repo entry
    let repo_id = make_id("repo");
    let repo_key = git::derive_repo_key(url);

    let clone_record = if full {
        CloneRecord {
            depth: 0,
            single_branch: false,
            no_tags: false,
            filter: String::new(),
            submodules: clone_policy.submodules.clone(),
            lfs: clone_policy.lfs.clone(),
            extra: Map::new(),
        }
    } else {
        CloneRecord {
            depth: clone_policy.depth,
            single_branch: clone_policy.single_branch,
            no_tags: clone_policy.no_tags,
            filter: clone_policy.filter.clone(),
            submodules: clone_policy.submodules.clone(),
            lfs: clone_policy.lfs.clone(),
            extra: Map::new(),
        }
    };

    let repo_entry = RepoEntry {
        id: repo_id.clone(),
        alias: alias.to_string(),
        repo_key,
        remote_url: url.to_string(),
        checkout_path: format!("repos/{alias}"),
        notes_path: format!("notes/repos/{alias}"),
        clone: clone_record,
        pin: PinState {
            mode: "tracking".to_string(),
            pinned_sha: String::new(),
            extra: Map::new(),
        },
        state: git_state.clone(),
        extra: Map::new(),
    };

    // 7. Mutate manifest with optimistic concurrency
    let alias_owned = alias.to_string();
    let result = with_locked_manifest(workspace_id, Some(version), move |m| {
        m.repos.push(repo_entry);
        Ok(())
    });

    match result {
        Err(e @ WorkspaceError::VersionConflict { .. }) => {
            // Clean up the clone since we can't register it
            let _ = std::fs::remove_dir_all(&clone_dest);
            return Err(e);
        }
        other => other?,
    };

    // 8. Create notes dir
    let notes_dir = root.join("notes").join("repos").join(&alias_owned);
    std::fs::create_dir_all(notes_dir)?;

    // 9. Audit
    write_audit(
        workspace_id,
        "repo_add",
        vec![json!({"type": "repo", "id": &repo_id, "alias": &alias_owned})],
        None,
    )?;

    // 10. Output result
    Ok(json!({
        "repoId": repo_id,
        "alias": alias_owned,
        "checkoutPath": format!("repos/{alias_owned}"),
        "state": {
            "headSha": git_state.head_sha,
            "headRef": git_state.head_ref,
            "defaultBranch": git_state.default_branch,
            "shallow": git_state.shallow,
        }
    }))
}
