use std::path::PathBuf;

const MANIFEST_FILENAME: &str = "workspace.json";

/// Get the codex home directory (`$CODEX_HOME` or `~/.ata`).
///
/// Unlike `codex_utils_home_dir::find_codex_home`, this does not validate
/// that the directory exists, since workspace operations may create it.
pub fn codex_home() -> PathBuf {
    if let Ok(val) = std::env::var("CODEX_HOME")
        && !val.is_empty()
    {
        return PathBuf::from(val);
    }
    // No CODEX_HOME set — use ~/.ata
    if let Some(mut home) = home_dir() {
        home.push(".ata");
        return home;
    }
    PathBuf::from(".ata")
}

/// Minimal home dir lookup without pulling in `dirs` as a direct dep.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Root directory for all workspaces.
pub fn workspaces_root() -> PathBuf {
    codex_home().join("workspaces")
}

/// Root directory for a specific workspace.
pub fn workspace_root(workspace_id: &str) -> PathBuf {
    workspaces_root().join(workspace_id)
}

/// Path to the workspace manifest file.
pub fn manifest_path(workspace_id: &str) -> PathBuf {
    workspace_root(workspace_id).join(MANIFEST_FILENAME)
}

/// Path to the workspace lock file.
pub fn lock_path(workspace_id: &str) -> PathBuf {
    workspace_root(workspace_id)
        .join("locks")
        .join("workspace.lock")
}

/// Path to the workspace audit log (NDJSON).
pub fn audit_path(workspace_id: &str) -> PathBuf {
    workspace_root(workspace_id)
        .join("notes")
        .join("workspace")
        .join("audit.ndjson")
}

/// Path to the active workspace selection file (session-aware).
pub fn selection_path() -> PathBuf {
    let sid = std::env::var("CODEX_SESSION_ID")
        .unwrap_or_default()
        .trim()
        .to_string();
    if !sid.is_empty() {
        return codex_home().join("sessions").join(&sid).join("workspace.json");
    }
    codex_home().join(".workspace_selected")
}

/// Path to a lock file for a given lock level and optional target ID.
pub fn lock_file_path(workspace_id: &str, level: &str, target_id: Option<&str>) -> PathBuf {
    let root = workspace_root(workspace_id);
    match level {
        "workspace" => root.join("locks").join("workspace.lock"),
        "kb" => root.join("knowledge-base").join("kb.lock"),
        "run" => root
            .join("runs")
            .join(target_id.unwrap_or(""))
            .join("run.lock"),
        "index" => root
            .join("indexes")
            .join(target_id.unwrap_or(""))
            .join("index.lock"),
        _ => root.join("locks").join(format!("{level}.lock")),
    }
}

/// Mirror cache path for a repo URL.
pub fn mirror_cache_path(url: &str) -> PathBuf {
    let key = normalize_repo_key(url);
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let result = hasher.finalize();
        // First 8 bytes = 16 hex chars
        let mut s = String::with_capacity(16);
        for b in &result[..8] {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    };
    codex_home()
        .join("caches")
        .join("repo-mirrors")
        .join(digest)
}

/// Normalize a repo URL to a stable cache key.
fn normalize_repo_key(url: &str) -> String {
    let mut key = if let Some((_scheme, rest)) = url.split_once("://") {
        rest.to_string()
    } else {
        url.to_string()
    };
    key = key.to_lowercase();
    if key.ends_with(".git") {
        key.truncate(key.len() - 4);
    }
    key = key.trim_end_matches('/').to_string();
    key
}

/// Directories created when initializing a new workspace.
pub fn init_dirs() -> Vec<&'static str> {
    vec![
        "",
        "repos",
        "runs",
        "artifacts",
        "indexes",
        "cache",
        "locks",
        "notes/workspace",
        "notes/workspace/snapshots",
        "notes/repos",
        "notes/papers",
        "notes/datasets",
        "notes/artifacts",
        "notes/runs",
        "notes/indexes",
        "knowledge-base",
        "knowledge-base/cards",
        "knowledge-base/topics",
        "knowledge-base/briefings",
        "knowledge-base/explanations",
        "knowledge-base/assets",
        "knowledge-base/staging",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_repo_key() {
        assert_eq!(
            normalize_repo_key("https://github.com/Org/Repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(
            normalize_repo_key("https://github.com/org/repo/"),
            "github.com/org/repo"
        );
    }
}
