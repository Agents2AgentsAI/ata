use std::path::Path;
use std::path::PathBuf;

pub(crate) const CODEX_KB_PATH_ENV_VAR: &str = "CODEX_KB_PATH";

pub(crate) fn resolve_kb_path(
    codex_home: &Path,
    cwd: &Path,
    kb_path_override: Option<&str>,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> PathBuf {
    if let Some(kb_path_override) = kb_path_override
        && let Some(path) = resolve_override_kb_path(codex_home, kb_path_override)
    {
        return path;
    }

    let workspace_id = resolve_workspace_id(codex_home, cwd, session_id, thread_id);
    codex_workspace::paths::workspace_root_for(codex_home, &workspace_id).join("knowledge-base")
}

fn resolve_override_kb_path(codex_home: &Path, kb_path_override: &str) -> Option<PathBuf> {
    let trimmed = kb_path_override.trim();
    if trimmed.is_empty() {
        return None;
    }

    let expanded = expand_home(trimmed);
    if expanded.is_absolute() {
        Some(expanded)
    } else {
        Some(codex_home.join(expanded))
    }
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(path));
    }

    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty())
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(path)
}

fn resolve_workspace_id(
    codex_home: &Path,
    cwd: &Path,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> String {
    if let Some(workspace_id) =
        codex_workspace::selection::discover_project_pin_for(codex_home, cwd)
    {
        return workspace_id;
    }

    if let Some(workspace_id) =
        codex_workspace::selection::read_session_workspace_for(codex_home, session_id, thread_id)
    {
        return workspace_id;
    }

    "global".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn create_workspace(codex_home: &Path, workspace_id: &str) {
        let manifest_path = codex_home
            .join("workspaces")
            .join(workspace_id)
            .join("workspace.json");
        std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
            .expect("create workspace dirs");
        std::fs::write(&manifest_path, "{}").expect("write workspace manifest");
    }

    fn write_selection(path: &Path, workspace_id: &str) {
        std::fs::create_dir_all(path.parent().expect("selection parent"))
            .expect("create selection parent");
        let payload = format!(r#"{{"schemaVersion":1,"activeWorkspaceId":"{workspace_id}"}}"#);
        std::fs::write(path, payload).expect("write selection");
    }

    #[test]
    fn resolve_kb_path_defaults_to_global_workspace() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let path = resolve_kb_path(&codex_home, &cwd, None, None, None);
        assert_eq!(
            path,
            codex_home
                .join("workspaces")
                .join("global")
                .join("knowledge-base")
        );
    }

    #[test]
    fn resolve_kb_path_uses_session_selection_when_present() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        create_workspace(&codex_home, "workspace-a");
        write_selection(
            &codex_home
                .join("sessions")
                .join("session-1")
                .join("workspace.json"),
            "workspace-a",
        );

        let path = resolve_kb_path(&codex_home, &cwd, None, Some("session-1"), Some("thread-1"));
        assert_eq!(
            path,
            codex_home
                .join("workspaces")
                .join("workspace-a")
                .join("knowledge-base")
        );
    }

    #[test]
    fn resolve_kb_path_prefers_project_pin_over_session() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let repo_root = temp.path().join("repo");
        let cwd = repo_root.join("subdir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create git dir");
        create_workspace(&codex_home, "workspace-a");
        create_workspace(&codex_home, "workspace-b");
        write_selection(
            &repo_root.join(".codex").join("workspace.json"),
            "workspace-a",
        );
        write_selection(
            &codex_home
                .join("sessions")
                .join("session-1")
                .join("workspace.json"),
            "workspace-b",
        );

        let path = resolve_kb_path(&codex_home, &cwd, None, Some("session-1"), Some("thread-1"));
        assert_eq!(
            path,
            codex_home
                .join("workspaces")
                .join("workspace-a")
                .join("knowledge-base")
        );
    }

    #[test]
    fn resolve_kb_path_falls_back_when_selection_points_to_missing_workspace() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        write_selection(
            &codex_home
                .join("sessions")
                .join("session-1")
                .join("workspace.json"),
            "missing-workspace",
        );

        let path = resolve_kb_path(&codex_home, &cwd, None, Some("session-1"), Some("thread-1"));
        assert_eq!(
            path,
            codex_home
                .join("workspaces")
                .join("global")
                .join("knowledge-base")
        );
    }

    #[test]
    fn resolve_kb_path_uses_override_when_configured() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let path = resolve_kb_path(
            &codex_home,
            &cwd,
            Some("custom-kb"),
            Some("session-1"),
            Some("thread-1"),
        );
        assert_eq!(path, codex_home.join("custom-kb"));
    }
}
