use crate::error::WorkspaceError;
use crate::paths;
use crate::selection;
use crate::types::new_manifest;
use crate::workspace_id::validate_workspace_id;
use std::path::Path;

/// 4-tier workspace resolution: explicit > project pin > session > global.
///
/// If `explicit` is `Some`, returns that immediately.
/// Otherwise walks: project pin → session selection → global fallback.
pub fn resolve_workspace(explicit: Option<&str>) -> Result<String, WorkspaceError> {
    let context = paths::SessionContext::from_env();

    if let Some(wid) = resolve_selected_workspace_for(
        &context.codex_home,
        context.cwd.as_deref(),
        explicit,
        context.session_id.as_deref(),
        context.thread_id.as_deref(),
    )? {
        return Ok(wid);
    }

    // 4. Global fallback — ensure it exists
    ensure_global_workspace_for(&context.codex_home)?;
    Ok("global".to_string())
}

/// Shared 3-tier resolution without creating the global workspace.
///
/// Returns `Ok(None)` when no explicit, project, or session workspace exists,
/// allowing callers to choose their own global fallback behavior.
pub fn resolve_selected_workspace_for(
    codex_home: &Path,
    cwd: Option<&Path>,
    explicit: Option<&str>,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> Result<Option<String>, WorkspaceError> {
    if let Some(wid) = explicit
        && !wid.is_empty()
    {
        validate_workspace_id(wid)?;
        return Ok(Some(wid.to_string()));
    }

    if let Some(cwd) = cwd
        && let Some(wid) = selection::discover_project_pin_for(codex_home, cwd)
    {
        return Ok(Some(wid));
    }

    Ok(selection::read_session_workspace_for(
        codex_home, session_id, thread_id,
    ))
}

fn ensure_global_workspace_for(codex_home: &Path) -> Result<(), WorkspaceError> {
    let wid = "global";
    if paths::manifest_path_for(codex_home, wid).is_file() {
        return Ok(());
    }
    paths::ensure_workspaces_root_for(codex_home)?;
    let root = paths::workspace_root_for(codex_home, wid);
    paths::create_workspace_dirs(&root)?;
    let manifest = new_manifest(wid, "global");
    let manifest_path = paths::manifest_path_for(codex_home, wid);
    let data = serde_json::to_string_pretty(&manifest)?;
    crate::manifest::atomic_write(&manifest_path, data.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_selected_workspace_for_prefers_project_pin_over_session_selection() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let codex_home = temp.path().join(".ata");
        let repo_root = temp.path().join("repo");
        let cwd = repo_root.join("nested");

        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create git dir");
        std::fs::create_dir_all(codex_home.join("workspaces").join("project-ws"))
            .expect("create project workspace dir");
        std::fs::create_dir_all(codex_home.join("workspaces").join("session-ws"))
            .expect("create session workspace dir");
        std::fs::write(
            codex_home
                .join("workspaces")
                .join("project-ws")
                .join("workspace.json"),
            "{}",
        )
        .expect("write project manifest");
        std::fs::write(
            codex_home
                .join("workspaces")
                .join("session-ws")
                .join("workspace.json"),
            "{}",
        )
        .expect("write session manifest");
        std::fs::create_dir_all(repo_root.join(".codex")).expect("create .codex dir");
        std::fs::write(
            repo_root.join(".codex").join("workspace.json"),
            r#"{"schemaVersion":1,"activeWorkspaceId":"project-ws"}"#,
        )
        .expect("write project pin");
        std::fs::create_dir_all(codex_home.join("sessions").join("session-1"))
            .expect("create session dir");
        std::fs::write(
            codex_home
                .join("sessions")
                .join("session-1")
                .join("workspace.json"),
            r#"{"schemaVersion":1,"activeWorkspaceId":"session-ws"}"#,
        )
        .expect("write session selection");

        let resolved = resolve_selected_workspace_for(
            &codex_home,
            Some(cwd.as_path()),
            None,
            Some("session-1"),
            Some("thread-1"),
        )
        .expect("resolve workspace");

        assert_eq!(resolved, Some("project-ws".to_string()));
    }

    #[test]
    fn resolve_selected_workspace_for_returns_none_without_selection() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let codex_home = temp.path().join(".ata");
        let cwd = temp.path().join("repo");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let resolved = resolve_selected_workspace_for(
            &codex_home,
            Some(cwd.as_path()),
            None,
            Some("session-1"),
            Some("thread-1"),
        )
        .expect("resolve workspace");

        assert_eq!(resolved, None);
    }
}
