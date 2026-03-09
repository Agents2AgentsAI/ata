use crate::audit::write_audit;
use crate::error::WorkspaceError;
use crate::git;
use crate::lock::FileLock;
use crate::manifest::bump_version;
use crate::manifest::read_manifest;
use crate::manifest::write_manifest_atomic;
use crate::paths;
use crate::resolve::is_reserved_alias;
use crate::types::CloneRecord;
use crate::types::LfsPolicy;
use crate::types::PinMode;
use crate::types::PinState;
use crate::types::RepoEntry;
use crate::url_validation::check_host_allowlist;
use crate::url_validation::validate_repo_url;
use crate::workspace_id::make_id;
use regex::Regex;
use serde_json::Map;
use serde_json::json;
use std::sync::LazyLock;

// SAFETY: regex pattern is a compile-time string literal and is known valid.
#[allow(clippy::expect_used)]
static ALIAS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").expect("valid regex"));

/// Full repo_add flow: validate URL -> clone -> register -> audit.
/// Returns JSON-serializable result.
pub fn run(
    workspace_id: &str,
    url: &str,
    alias: &str,
    full: bool,
) -> Result<serde_json::Value, WorkspaceError> {
    if is_reserved_alias(alias) {
        return Err(WorkspaceError::ReservedAlias(alias.to_string()));
    }
    if !ALIAS_RE.is_match(alias) {
        return Err(WorkspaceError::InvalidAlias {
            alias: alias.to_string(),
        });
    }

    validate_repo_url(url)?;

    let root = paths::workspace_root(workspace_id);
    let clone_dest = root.join("repos").join(alias);
    // Keep clone side effects, manifest mutation, notes creation, and audit ordering
    // under one workspace lock. `with_locked_manifest` is not a good fit here because
    // the external `git clone` must be ordered against the manifest snapshot we validate.
    let lock = FileLock::acquire(&paths::lock_path(workspace_id))?;

    let mut manifest = read_manifest(workspace_id)?;
    check_host_allowlist(url, manifest.policies.repo_hosts_allowlist.as_deref())?;

    if manifest.repos.iter().any(|repo| repo.alias == alias) {
        return Err(WorkspaceError::AliasExists(alias.to_string()));
    }
    if clone_dest.exists() {
        return Err(WorkspaceError::DirectoryExists(clone_dest));
    }

    let clone_policy = manifest.policies.default_clone.clone();
    let mut clone_args = git::build_clone_args(&clone_policy, full);
    let mirror_path = paths::mirror_cache_path(url);
    if git::prepare_reference_mirror(url, &mirror_path) {
        clone_args.push("--reference-if-able".to_string());
        clone_args.push(mirror_path.display().to_string());
    }

    let exit_code = git::clone_repo(url, &clone_dest, &clone_args)?;
    if exit_code != 0 {
        return Err(WorkspaceError::GitCloneFailed(exit_code));
    }

    if matches!(clone_policy.lfs, LfsPolicy::Always | LfsPolicy::Auto) {
        git::lfs_pull(&clone_dest);
    }

    let git_state = git::read_git_state(&clone_dest);
    let repo_id = make_id("repo");
    let repo_key = git::derive_repo_key(url);
    let alias_owned = alias.to_string();
    let notes_dir = root.join("notes").join("repos").join(&alias_owned);
    std::fs::create_dir_all(&notes_dir)?;

    let clone_record = if full {
        CloneRecord {
            depth: 0,
            single_branch: false,
            no_tags: false,
            filter: String::new(),
            submodules: clone_policy.submodules,
            lfs: clone_policy.lfs,
            extra: Map::new(),
        }
    } else {
        CloneRecord {
            depth: clone_policy.depth,
            single_branch: clone_policy.single_branch,
            no_tags: clone_policy.no_tags,
            filter: clone_policy.filter.clone(),
            submodules: clone_policy.submodules,
            lfs: clone_policy.lfs,
            extra: Map::new(),
        }
    };

    manifest.repos.push(RepoEntry {
        id: repo_id.clone(),
        alias: alias.to_string(),
        repo_key,
        remote_url: url.to_string(),
        checkout_path: format!("repos/{alias}"),
        notes_path: format!("notes/repos/{alias}"),
        clone: clone_record,
        pin: PinState {
            mode: PinMode::Tracking,
            pinned_sha: String::new(),
            extra: Map::new(),
        },
        state: git_state.clone(),
        extra: Map::new(),
    });
    bump_version(&mut manifest);
    if let Err(err) = write_manifest_atomic(workspace_id, &manifest) {
        let _ = std::fs::remove_dir_all(&clone_dest);
        return Err(err);
    }

    write_audit(
        workspace_id,
        "repo_add",
        vec![json!({"type": "repo", "id": &repo_id, "alias": &alias_owned})],
        None,
    )?;
    drop(lock);

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
