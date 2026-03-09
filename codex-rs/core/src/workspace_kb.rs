use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::Path;
use std::path::PathBuf;

pub(crate) const CODEX_KB_PATH_ENV_VAR: &str = "CODEX_KB_PATH";

pub(crate) struct ResolvedKbEnv {
    pub kb_path: String,
    pub workspace_kb_root: Option<AbsolutePathBuf>,
}

pub(crate) fn resolve_kb_env(
    codex_home: &Path,
    cwd: &Path,
    kb_path_override: Option<&str>,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> ResolvedKbEnv {
    let kb_path = resolve_kb_path(codex_home, cwd, kb_path_override, session_id, thread_id);
    let workspace_kb_root = kb_writable_root(&kb_path);

    ResolvedKbEnv {
        kb_path: kb_path.to_string_lossy().to_string(),
        workspace_kb_root,
    }
}

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

pub(crate) fn kb_writable_root(kb_path: &Path) -> Option<AbsolutePathBuf> {
    if kb_path.as_os_str().is_empty() || !kb_path.is_absolute() {
        tracing::warn!(
            "ignoring non-absolute CODEX_KB_PATH when deriving sandbox writable roots: {}",
            kb_path.display()
        );
        return None;
    }

    if let Err(err) = std::fs::create_dir_all(kb_path) {
        tracing::warn!(
            "failed to create CODEX_KB_PATH directory {} for sandbox writable roots: {err}",
            kb_path.display()
        );
        return None;
    }

    let Ok(kb_root) = AbsolutePathBuf::from_absolute_path(kb_path) else {
        tracing::warn!(
            "ignoring invalid CODEX_KB_PATH when deriving sandbox writable roots: {}",
            kb_path.display()
        );
        return None;
    };

    Some(kb_root)
}

pub(crate) fn with_kb_writable_root(
    policy: &SandboxPolicy,
    kb_root: Option<&AbsolutePathBuf>,
) -> SandboxPolicy {
    let Some(kb_root) = kb_root else {
        return policy.clone();
    };

    match policy {
        SandboxPolicy::WorkspaceWrite {
            writable_roots,
            read_only_access,
            exclude_tmpdir_env_var,
            exclude_slash_tmp,
            network_access,
        } => {
            let mut writable_roots = writable_roots.clone();
            if !writable_roots.iter().any(|root| root == kb_root) {
                writable_roots.push(kb_root.clone());
            }
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access: read_only_access.clone(),
                exclude_tmpdir_env_var: *exclude_tmpdir_env_var,
                exclude_slash_tmp: *exclude_slash_tmp,
                network_access: *network_access,
            }
        }
        SandboxPolicy::DangerFullAccess
        | SandboxPolicy::ExternalSandbox { .. }
        | SandboxPolicy::ReadOnly { .. } => policy.clone(),
    }
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
    match codex_workspace::workspace_resolution::resolve_selected_workspace_for(
        codex_home,
        Some(cwd),
        None,
        session_id,
        thread_id,
    ) {
        Ok(Some(workspace_id)) => workspace_id,
        Ok(None) => "global".to_string(),
        Err(err) => {
            tracing::warn!(
                "failed to resolve workspace for KB path, falling back to global workspace: {err}"
            );
            "global".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ReadOnlyAccess;
    use codex_utils_absolute_path::AbsolutePathBuf;
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

    #[test]
    fn resolve_kb_env_returns_matching_path_and_writable_root() {
        let temp = TempDir::new().expect("temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("project");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let resolved = resolve_kb_env(
            &codex_home,
            &cwd,
            Some("custom-kb"),
            Some("session-1"),
            Some("thread-1"),
        );

        assert_eq!(
            resolved.kb_path,
            codex_home.join("custom-kb").display().to_string()
        );
        assert_eq!(
            resolved.workspace_kb_root,
            Some(
                AbsolutePathBuf::from_absolute_path(codex_home.join("custom-kb").as_path())
                    .expect("absolute kb path")
            )
        );
    }

    #[test]
    fn with_kb_writable_root_adds_resolved_kb_path_for_workspace_write() {
        let temp = TempDir::new().expect("temp dir");
        let kb_path = temp.path().join("kb");
        let kb_root = kb_writable_root(&kb_path).expect("kb writable root");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            read_only_access: ReadOnlyAccess::Restricted {
                include_platform_defaults: false,
                readable_roots: vec![],
            },
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        };

        let updated = with_kb_writable_root(&policy, Some(&kb_root));

        let expected_kb_root =
            AbsolutePathBuf::from_absolute_path(&kb_path).expect("absolute kb path");
        assert_eq!(
            updated,
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![expected_kb_root],
                read_only_access: ReadOnlyAccess::Restricted {
                    include_platform_defaults: false,
                    readable_roots: vec![],
                },
                network_access: false,
                exclude_tmpdir_env_var: false,
                exclude_slash_tmp: false,
            }
        );
    }

    #[test]
    fn with_kb_writable_root_ignores_non_workspace_write_policies() {
        let temp = TempDir::new().expect("temp dir");
        let kb_root = kb_writable_root(temp.path().join("kb").as_path()).expect("kb writable root");
        let policy = SandboxPolicy::ReadOnly {
            access: ReadOnlyAccess::Restricted {
                include_platform_defaults: false,
                readable_roots: vec![],
            },
            network_access: false,
        };

        let updated = with_kb_writable_root(&policy, Some(&kb_root));

        assert_eq!(updated, policy);
    }
}
