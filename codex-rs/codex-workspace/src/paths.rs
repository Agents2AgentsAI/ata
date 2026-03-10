use std::path::Component;
use std::path::PathBuf;

const MANIFEST_FILENAME: &str = "workspace.json";
pub const CODEX_SESSION_ID_ENV_VAR: &str = "CODEX_SESSION_ID";
pub const CODEX_THREAD_ID_ENV_VAR: &str = "CODEX_THREAD_ID";

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
    home_dir_from_env(
        std::env::var("HOME").ok().as_deref(),
        std::env::var("USERPROFILE").ok().as_deref(),
        std::env::var("HOMEDRIVE").ok().as_deref(),
        std::env::var("HOMEPATH").ok().as_deref(),
    )
}

fn home_dir_from_env(
    home: Option<&str>,
    userprofile: Option<&str>,
    homedrive: Option<&str>,
    homepath: Option<&str>,
) -> Option<PathBuf> {
    if let Some(value) = home.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    if let Some(value) = userprofile.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    match (
        homedrive.filter(|value| !value.is_empty()),
        homepath.filter(|value| !value.is_empty()),
    ) {
        (Some(drive), Some(path)) => Some(PathBuf::from(format!("{drive}{path}"))),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionContext {
    pub codex_home: PathBuf,
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub cwd: Option<PathBuf>,
}

impl SessionContext {
    pub fn from_env() -> Self {
        Self {
            codex_home: codex_home(),
            session_id: std::env::var(CODEX_SESSION_ID_ENV_VAR)
                .ok()
                .filter(|value| !value.is_empty()),
            thread_id: std::env::var(CODEX_THREAD_ID_ENV_VAR)
                .ok()
                .filter(|value| !value.is_empty()),
            cwd: std::env::current_dir().ok(),
        }
    }

    pub fn scope_id(&self) -> Option<String> {
        workspace_scope_id(self.session_id.as_deref(), self.thread_id.as_deref())
    }
}

/// Root directory for all workspaces.
pub fn workspaces_root() -> PathBuf {
    codex_home().join("workspaces")
}

/// Root directory for all workspaces under a specific Codex home.
pub fn workspaces_root_for(codex_home: &std::path::Path) -> PathBuf {
    codex_home.join("workspaces")
}

/// Ensure the workspace root exists as a directory.
pub fn ensure_workspaces_root() -> std::io::Result<PathBuf> {
    ensure_workspaces_root_for(&codex_home())
}

/// Ensure `codex_home/workspaces` exists as a directory.
pub fn ensure_workspaces_root_for(codex_home: &std::path::Path) -> std::io::Result<PathBuf> {
    let root = workspaces_root_for(codex_home);
    if root.exists() && !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "workspaces root path exists and is not a directory: {}",
                root.display()
            ),
        ));
    }
    if !root.is_dir() {
        std::fs::create_dir_all(&root)?;
    }
    Ok(root)
}

/// Root directory for a specific workspace.
pub fn workspace_root(workspace_id: &str) -> PathBuf {
    workspaces_root().join(workspace_id)
}

/// Root directory for a specific workspace under a specific Codex home.
pub fn workspace_root_for(codex_home: &std::path::Path, workspace_id: &str) -> PathBuf {
    workspaces_root_for(codex_home).join(workspace_id)
}

pub fn repo_checkout_path(workspace_id: &str, alias: &str) -> PathBuf {
    workspace_root(workspace_id).join("repos").join(alias)
}

/// Path to the workspace manifest file.
pub fn manifest_path(workspace_id: &str) -> PathBuf {
    workspace_root(workspace_id).join(MANIFEST_FILENAME)
}

/// Path to the workspace manifest file under a specific Codex home.
pub fn manifest_path_for(codex_home: &std::path::Path, workspace_id: &str) -> PathBuf {
    workspace_root_for(codex_home, workspace_id).join(MANIFEST_FILENAME)
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

/// Path to the audit append lock for a workspace.
pub fn audit_lock_path(workspace_id: &str) -> PathBuf {
    workspace_root(workspace_id)
        .join("locks")
        .join("audit.lock")
}

fn normalized_scope_id(value: Option<&str>) -> Option<String> {
    let value = value.map(str::trim).filter(|value| !value.is_empty())?;
    is_safe_single_component(value).then_some(value.to_string())
}

pub(crate) fn is_safe_single_component(value: &str) -> bool {
    if value.contains('\\') {
        return false;
    }

    let mut components = std::path::Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

pub fn workspace_scope_id(session_id: Option<&str>, thread_id: Option<&str>) -> Option<String> {
    normalized_scope_id(session_id).or_else(|| normalized_scope_id(thread_id))
}

pub fn global_selection_path_for(codex_home: &std::path::Path) -> PathBuf {
    codex_home.join(".workspace_selected")
}

pub fn selection_path_for(codex_home: &std::path::Path, scope_id: Option<&str>) -> PathBuf {
    if let Some(scope_id) = scope_id {
        return codex_home
            .join("sessions")
            .join(scope_id)
            .join("workspace.json");
    }

    global_selection_path_for(codex_home)
}

/// Path to the active workspace selection file (session-aware).
pub fn selection_path() -> PathBuf {
    let context = SessionContext::from_env();
    let scope_id = context.scope_id();
    selection_path_for(&context.codex_home, scope_id.as_deref())
}

/// Path to the global legacy workspace selection file.
pub fn global_selection_path() -> PathBuf {
    global_selection_path_for(&codex_home())
}

/// Path to a lock file for a given lock level and optional target ID.
pub fn lock_file_path(workspace_id: &str, level: &str, target_id: Option<&str>) -> PathBuf {
    let root = workspace_root(workspace_id);
    match level {
        "workspace" => root.join("locks").join("workspace.lock"),
        "kb" => root.join("knowledge-base").join("kb.lock"),
        "run" => match target_id {
            Some(target_id) => root.join("runs").join(target_id).join("run.lock"),
            None => root.join("locks").join("run.lock"),
        },
        "index" => match target_id {
            Some(target_id) => root.join("indexes").join(target_id).join("index.lock"),
            None => root.join("locks").join("index.lock"),
        },
        _ => root.join("locks").join(format!("{level}.lock")),
    }
}

/// Mirror cache path for a repo URL.
pub fn mirror_cache_path(url: &str) -> PathBuf {
    let key = normalize_repo_key(url);
    let digest = {
        use sha2::Digest;
        use sha2::Sha256;
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
    let trimmed_len = key.trim_end_matches('/').len();
    key.truncate(trimmed_len);
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

pub fn create_workspace_dirs(root: &std::path::Path) -> std::io::Result<()> {
    for sub in init_dirs() {
        let dir = if sub.is_empty() {
            root.to_path_buf()
        } else {
            root.join(sub)
        };
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

    #[test]
    fn ensure_workspaces_root_for_creates_dir() {
        let temp = TempDir::new().expect("create temp dir");

        let root = ensure_workspaces_root_for(temp.path()).expect("create workspaces root");

        assert_eq!(root, temp.path().join("workspaces"));
        assert!(root.is_dir(), "workspaces root should be a directory");
    }

    #[test]
    fn ensure_workspaces_root_for_errors_when_path_is_file() {
        let temp = TempDir::new().expect("create temp dir");
        let root_file = temp.path().join("workspaces");
        std::fs::write(&root_file, "not a directory").expect("create root file");

        let err =
            ensure_workspaces_root_for(temp.path()).expect_err("expected non-directory error");

        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(
            err.to_string().contains("is not a directory"),
            "error should mention non-directory root path"
        );
    }

    #[test]
    fn workspace_scope_id_prefers_session_over_thread() {
        assert_eq!(
            workspace_scope_id(Some(" session-1 "), Some("thread-1")),
            Some("session-1".to_string())
        );
    }

    #[test]
    fn workspace_scope_id_uses_thread_when_session_missing() {
        assert_eq!(
            workspace_scope_id(None, Some(" thread-1 ")),
            Some("thread-1".to_string())
        );
        assert_eq!(
            workspace_scope_id(Some("   "), Some("thread-2")),
            Some("thread-2".to_string())
        );
        assert_eq!(
            workspace_scope_id(Some("../escape"), Some("thread-3")),
            Some("thread-3".to_string())
        );
    }

    #[test]
    fn workspace_scope_id_rejects_unsafe_values() {
        assert_eq!(workspace_scope_id(Some("../escape"), None), None);
        assert_eq!(workspace_scope_id(Some("nested/path"), None), None);
        assert_eq!(workspace_scope_id(Some(r"nested\\path"), None), None);
        assert_eq!(workspace_scope_id(Some("."), None), None);
        assert_eq!(workspace_scope_id(Some(".."), None), None);
    }

    #[test]
    fn safe_single_component_rejects_nested_or_escaped_paths() {
        assert!(is_safe_single_component("scope-1"));
        assert!(!is_safe_single_component("nested/path"));
        assert!(!is_safe_single_component("../escape"));
        assert!(!is_safe_single_component(r"nested\\path"));
    }

    #[test]
    fn selection_path_for_uses_scoped_session_path_when_scope_present() {
        let codex_home = std::path::Path::new("/tmp/codex-home");
        assert_eq!(
            selection_path_for(codex_home, Some("scope-123")),
            codex_home
                .join("sessions")
                .join("scope-123")
                .join("workspace.json")
        );
    }

    #[test]
    fn selection_path_for_uses_global_path_when_scope_missing() {
        let codex_home = std::path::Path::new("/tmp/codex-home");
        assert_eq!(
            selection_path_for(codex_home, None),
            codex_home.join(".workspace_selected")
        );
    }

    #[test]
    fn lock_file_path_uses_workspace_lock_dir_when_target_missing() {
        assert_eq!(
            lock_file_path("workspace-1", "run", None),
            workspace_root("workspace-1").join("locks").join("run.lock")
        );
        assert_eq!(
            lock_file_path("workspace-1", "index", None),
            workspace_root("workspace-1")
                .join("locks")
                .join("index.lock")
        );
    }

    #[test]
    fn create_workspace_dirs_creates_nested_layout() {
        let temp = TempDir::new().expect("create temp dir");
        let root = temp.path().join("workspace-1");

        create_workspace_dirs(&root).expect("create workspace dirs");

        assert!(root.join("repos").is_dir());
        assert!(root.join("runs").is_dir());
        assert!(root.join("knowledge-base").join("staging").is_dir());
        assert!(root.join("notes").join("workspace").is_dir());
    }

    #[test]
    fn home_dir_from_env_uses_windows_fallbacks() {
        assert_eq!(
            home_dir_from_env(None, Some("C:/Users/test"), None, None),
            Some(PathBuf::from("C:/Users/test"))
        );
        assert_eq!(
            home_dir_from_env(None, None, Some("C:"), Some("\\Users\\test")),
            Some(PathBuf::from("C:\\Users\\test"))
        );
    }
}
