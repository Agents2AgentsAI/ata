use crate::types::DefaultClonePolicy;
use crate::types::GitState;
use crate::types::SubmodulePolicy;
use std::path::Path;
use std::process::Command;

/// Build git clone arguments from a clone policy.
pub fn build_clone_args(policy: &DefaultClonePolicy, full: bool) -> Vec<String> {
    if full {
        return Vec::new();
    }
    let mut args = Vec::new();
    if policy.depth > 0 {
        args.push("--depth".to_string());
        args.push(policy.depth.to_string());
    }
    if policy.single_branch {
        args.push("--single-branch".to_string());
    }
    if policy.no_tags {
        args.push("--no-tags".to_string());
    }
    if !policy.filter.is_empty() && policy.filter != "null" {
        args.push(format!("--filter={}", policy.filter));
    }
    if policy.submodules == SubmodulePolicy::Recursive {
        args.push("--recurse-submodules".to_string());
    }
    args
}

pub fn is_valid_commit_sha(sha: &str) -> bool {
    sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(repo_dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Read HEAD sha, ref, and default branch from a cloned repo.
pub fn read_git_state(repo_dir: &Path) -> GitState {
    let head_sha = git_output(repo_dir, &["rev-parse", "HEAD"]).unwrap_or_default();
    let head_ref = git_output(repo_dir, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_default();
    let mut default_branch =
        git_output(repo_dir, &["rev-parse", "--abbrev-ref", "origin/HEAD"]).unwrap_or_default();
    if let Some(stripped) = default_branch.strip_prefix("origin/") {
        default_branch = stripped.to_string();
    }
    if default_branch.is_empty() {
        default_branch = "main".to_string();
    }
    let shallow =
        git_output(repo_dir, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true");

    GitState {
        head_sha,
        head_ref,
        default_branch,
        shallow,
        extra: serde_json::Map::new(),
    }
}

/// Run `git clone` with the given arguments.
pub fn clone_repo<S: AsRef<std::ffi::OsStr>>(
    source: S,
    dest: &Path,
    extra_args: &[String],
) -> Result<i32, std::io::Error> {
    let mut cmd = Command::new("git");
    cmd.arg("clone");
    cmd.args(extra_args);
    cmd.arg(source);
    cmd.arg(dest);
    let status = cmd.status()?;
    Ok(status.code().unwrap_or(1))
}

/// Run `git lfs pull` in a directory.
pub fn lfs_pull(repo_dir: &Path) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["lfs", "pull"])
        .output();
}

pub fn prepare_reference_mirror(url: &str, mirror_path: &Path) -> bool {
    if let Some(parent) = mirror_path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }

    let status = if mirror_path.is_dir() {
        Command::new("git")
            .arg("-C")
            .arg(mirror_path)
            .args(["remote", "update", "--prune"])
            .status()
    } else {
        Command::new("git")
            .args(["clone", "--mirror", url])
            .arg(mirror_path)
            .status()
    };

    status.map(|value| value.success()).unwrap_or(false)
}

/// Add a git worktree.
pub fn worktree_add(repo_dir: &Path, dest: &Path, rev: &str) -> Result<bool, std::io::Error> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["worktree", "add"])
        .arg(dest)
        .arg(rev)
        .output()?;
    Ok(output.status.success())
}

/// Remove a git worktree from the worktree itself.
pub fn worktree_remove(worktree_path: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["worktree", "remove"])
        .arg(worktree_path)
        .arg("--force")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve a git ref (branch/tag) to a commit SHA in a local checkout.
pub fn resolve_ref(repo_dir: &Path, ref_name: &str) -> Option<String> {
    git_output(repo_dir, &["rev-parse", ref_name]).filter(|value| !value.is_empty())
}

/// Fetch a specific SHA (if needed) and checkout to it.
pub fn fetch_and_checkout(repo_dir: &Path, sha: &str) -> Result<bool, std::io::Error> {
    // Fetch is opportunistic here: it may fail even when the target object is
    // already available locally, and checkout success is the authoritative
    // signal for whether the requested SHA can be used.
    let _ = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["fetch", "origin", sha, "--depth", "1"])
        .output();
    // Checkout
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["checkout", sha])
        .output()?;
    Ok(output.status.success())
}

/// Derive repo_key from URL (strip scheme, trailing .git, keep last 2 path segments).
pub fn derive_repo_key(url: &str) -> String {
    let mut key = url.trim_end_matches('/').to_string();
    if key.ends_with(".git") {
        key.truncate(key.len() - 4);
    }
    if let Some((_scheme, rest)) = key.split_once("://") {
        key = rest.to_string();
    }
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_repo_key() {
        assert_eq!(
            derive_repo_key("https://github.com/rust-lang/rust.git"),
            "rust-lang/rust"
        );
        assert_eq!(derive_repo_key("https://github.com/org/repo/"), "org/repo");
    }

    #[test]
    fn test_is_valid_commit_sha() {
        assert!(is_valid_commit_sha(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_valid_commit_sha("0123456"));
        assert!(!is_valid_commit_sha(
            "zz23456789abcdef0123456789abcdef01234567"
        ));
    }
}
