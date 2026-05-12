use crate::error::WorkspaceError;
use crate::error::WorkspaceSelectorCandidate;
use crate::paths;
use crate::selection;
use crate::types::new_manifest;
use crate::workspace_id::slugify_name;
use crate::workspace_id::validate_workspace_id;
use std::path::Path;

/// 4-tier workspace resolution: explicit > project pin > session > global.
///
/// If `explicit` is `Some`, returns that immediately.
/// Otherwise walks: project pin → session selection → global fallback.
pub fn resolve_workspace(explicit: Option<&str>) -> Result<String, WorkspaceError> {
    let context = paths::SessionContext::from_env();
    resolve_workspace_for(
        &context.codex_home,
        context.cwd.as_deref(),
        explicit,
        context.session_id.as_deref(),
        context.thread_id.as_deref(),
    )
}

/// 4-tier workspace resolution using an explicit session context.
pub fn resolve_workspace_for(
    codex_home: &Path,
    cwd: Option<&Path>,
    explicit: Option<&str>,
    session_id: Option<&str>,
    thread_id: Option<&str>,
) -> Result<String, WorkspaceError> {
    if let Some(wid) =
        resolve_selected_workspace_for(codex_home, cwd, explicit, session_id, thread_id)?
    {
        return Ok(wid);
    }

    // 4. Global fallback — ensure it exists
    ensure_global_workspace_for(codex_home)?;
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

/// Resolve a user-facing selector to a canonical workspace ID.
///
/// Matching precedence is:
/// 1. exact workspace ID
/// 2. exact manifest name
/// 3. slugified manifest name
pub fn resolve_workspace_selector_for(
    codex_home: &Path,
    selector: &str,
) -> Result<String, WorkspaceError> {
    let selector = selector.trim();

    if validate_workspace_id(selector).is_ok()
        && paths::manifest_path_for(codex_home, selector).is_file()
    {
        return Ok(selector.to_string());
    }

    let summaries = crate::commands::list::run_for(codex_home)?;

    let exact_name_matches = summaries
        .iter()
        .filter(|summary| summary.name == selector)
        .collect::<Vec<_>>();
    match exact_name_matches.as_slice() {
        [summary] => return Ok(summary.id.clone()),
        [] => {}
        many => {
            return Err(WorkspaceError::WorkspaceSelectorAmbiguous {
                selector: selector.to_string(),
                candidates: many
                    .iter()
                    .map(|summary| selector_candidate(summary))
                    .collect(),
            });
        }
    }

    let selector_slug = slugify_name(selector);
    let slug_matches = summaries
        .iter()
        .filter(|summary| slugify_name(&summary.name) == selector_slug)
        .collect::<Vec<_>>();
    match slug_matches.as_slice() {
        [summary] => Ok(summary.id.clone()),
        [] => Err(WorkspaceError::WorkspaceSelectorNotFound {
            selector: selector.to_string(),
            candidates: selector_suggestions(selector, &selector_slug, &summaries),
        }),
        many => Err(WorkspaceError::WorkspaceSelectorAmbiguous {
            selector: selector.to_string(),
            candidates: many
                .iter()
                .map(|summary| selector_candidate(summary))
                .collect(),
        }),
    }
}

fn selector_candidate(summary: &crate::types::WorkspaceSummary) -> WorkspaceSelectorCandidate {
    WorkspaceSelectorCandidate {
        id: summary.id.clone(),
        name: summary.name.clone(),
    }
}

fn selector_suggestions(
    selector: &str,
    selector_slug: &str,
    summaries: &[crate::types::WorkspaceSummary],
) -> Vec<WorkspaceSelectorCandidate> {
    let selector_lower = selector.to_ascii_lowercase();
    let mut suggestions = summaries
        .iter()
        .filter(|summary| {
            summary.id.contains(selector)
                || summary.id.starts_with(selector_slug)
                || summary.name.to_ascii_lowercase().contains(&selector_lower)
                || slugify_name(&summary.name).contains(selector_slug)
        })
        .map(selector_candidate)
        .collect::<Vec<_>>();
    suggestions.sort_by(|left, right| left.id.cmp(&right.id));
    suggestions.truncate(5);
    suggestions
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
    use crate::manifest::write_manifest_atomic_for;
    use crate::types::new_manifest;

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

    #[test]
    fn resolve_workspace_selector_prefers_exact_id() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "alpha-1234abcd",
            &new_manifest("alpha-1234abcd", "Alpha"),
        )
        .expect("write manifest");

        let resolved = resolve_workspace_selector_for(temp.path(), "alpha-1234abcd")
            .expect("resolve selector");

        assert_eq!(resolved, "alpha-1234abcd");
    }

    #[test]
    fn resolve_workspace_selector_matches_exact_name() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "alpha-1234abcd",
            &new_manifest("alpha-1234abcd", "Alpha Workspace"),
        )
        .expect("write manifest");

        let resolved = resolve_workspace_selector_for(temp.path(), "Alpha Workspace")
            .expect("resolve selector");

        assert_eq!(resolved, "alpha-1234abcd");
    }

    #[test]
    fn resolve_workspace_selector_matches_slugified_name() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "alpha-1234abcd",
            &new_manifest("alpha-1234abcd", "Alpha Workspace"),
        )
        .expect("write manifest");

        let resolved = resolve_workspace_selector_for(temp.path(), "alpha workspace")
            .expect("resolve selector");

        assert_eq!(resolved, "alpha-1234abcd");
    }

    #[test]
    fn resolve_workspace_selector_reports_ambiguity() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "alpha-1234abcd",
            &new_manifest("alpha-1234abcd", "Alpha Workspace"),
        )
        .expect("write first manifest");
        write_manifest_atomic_for(
            temp.path(),
            "alpha-5678efgh",
            &new_manifest("alpha-5678efgh", "Alpha Workspace"),
        )
        .expect("write second manifest");

        let err = resolve_workspace_selector_for(temp.path(), "Alpha Workspace")
            .expect_err("selector should be ambiguous");

        match err {
            WorkspaceError::WorkspaceSelectorAmbiguous {
                selector,
                candidates,
            } => {
                assert_eq!(selector, "Alpha Workspace");
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected ambiguous selector error, got {other:?}"),
        }
    }

    #[test]
    fn resolve_workspace_selector_reports_suggestions() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        write_manifest_atomic_for(
            temp.path(),
            "multi-repos-34c9219f",
            &new_manifest("multi-repos-34c9219f", "Multi Repos"),
        )
        .expect("write manifest");

        let err = resolve_workspace_selector_for(temp.path(), "multi-repo")
            .expect_err("selector should not resolve");

        match err {
            WorkspaceError::WorkspaceSelectorNotFound {
                selector,
                candidates,
            } => {
                assert_eq!(selector, "multi-repo");
                assert_eq!(candidates.len(), 1);
                assert_eq!(candidates[0].id, "multi-repos-34c9219f");
            }
            other => panic!("expected selector not found error, got {other:?}"),
        }
    }
}
