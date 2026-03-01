use crate::types::{DefaultClonePolicy, GitState};
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
    if policy.submodules == "recursive" {
        args.push("--recurse-submodules".to_string());
    }
    args
}

/// Read HEAD sha, ref, and default branch from a cloned repo.
pub fn read_git_state(repo_dir: &Path) -> GitState {
    let git = |args: &[&str]| -> String {
        let mut cmd_args = vec!["-C", repo_dir.to_str().unwrap_or(".")];
        cmd_args.extend_from_slice(args);
        Command::new("git")
            .args(&cmd_args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };

    let head_sha = git(&["rev-parse", "HEAD"]);
    let head_ref = git(&["symbolic-ref", "--short", "HEAD"]);
    let mut default_branch = git(&["rev-parse", "--abbrev-ref", "origin/HEAD"]);
    if let Some(stripped) = default_branch.strip_prefix("origin/") {
        default_branch = stripped.to_string();
    }
    if default_branch.is_empty() {
        default_branch = "main".to_string();
    }

    GitState {
        head_sha,
        head_ref,
        default_branch,
        shallow: false,
        extra: serde_json::Map::new(),
    }
}

/// Run `git clone` with the given arguments.
pub fn clone_repo(url: &str, dest: &Path, extra_args: &[String]) -> Result<i32, std::io::Error> {
    let mut cmd = Command::new("git");
    cmd.arg("clone");
    cmd.args(extra_args);
    cmd.arg(url);
    cmd.arg(dest);
    let status = cmd.status()?;
    Ok(status.code().unwrap_or(1))
}

/// Run `git lfs pull` in a directory.
pub fn lfs_pull(repo_dir: &Path) {
    let _ = Command::new("git")
        .args(["-C", repo_dir.to_str().unwrap_or("."), "lfs", "pull"])
        .output();
}

/// Add a git worktree.
pub fn worktree_add(repo_dir: &Path, dest: &Path, rev: &str) -> Result<bool, std::io::Error> {
    let output = Command::new("git")
        .args([
            "-C",
            repo_dir.to_str().unwrap_or("."),
            "worktree",
            "add",
            dest.to_str().unwrap_or("."),
            rev,
        ])
        .output()?;
    Ok(output.status.success())
}

/// Remove a git worktree.
pub fn worktree_remove(repo_dir: &Path, worktree_path: &Path) -> bool {
    Command::new("git")
        .args([
            "-C",
            repo_dir.to_str().unwrap_or("."),
            "worktree",
            "remove",
            worktree_path.to_str().unwrap_or("."),
            "--force",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the common git dir (for worktree cleanup).
pub fn git_common_dir(path: &Path) -> Option<std::path::PathBuf> {
    Command::new("git")
        .args([
            "-C",
            path.to_str().unwrap_or("."),
            "rev-parse",
            "--git-common-dir",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let p = std::path::PathBuf::from(&s);
            // --git-common-dir returns relative to the worktree, resolve it
            if p.is_absolute() {
                p.parent().unwrap_or(&p).to_path_buf()
            } else {
                path.join(&p)
                    .parent()
                    .unwrap_or(path)
                    .to_path_buf()
            }
        })
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
        assert_eq!(
            derive_repo_key("https://github.com/org/repo/"),
            "org/repo"
        );
    }
}
