use crate::error::WorkspaceError;
use crate::manifest::read_manifest_for;
use crate::paths;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub workspace_id: String,
    pub ok: bool,
    pub missing_repos: Vec<String>,
    pub missing_runs: Vec<String>,
    pub orphan_repo_dirs: Vec<String>,
    pub orphan_run_dirs: Vec<String>,
}

pub fn run(workspace_id: &str) -> Result<ValidationReport, WorkspaceError> {
    run_for(&paths::codex_home(), workspace_id)
}

fn run_for(codex_home: &Path, workspace_id: &str) -> Result<ValidationReport, WorkspaceError> {
    let manifest = read_manifest_for(codex_home, workspace_id)?;
    let workspace_root = paths::workspace_root_for(codex_home, workspace_id);

    let mut missing_repos: Vec<String> = manifest
        .repos
        .iter()
        .filter(|repo| !workspace_root.join(&repo.checkout_path).is_dir())
        .map(|repo| repo.alias.clone())
        .collect();
    missing_repos.sort();

    let mut missing_runs: Vec<String> = manifest
        .runs
        .iter()
        .filter(|run| !workspace_root.join(&run.root_path).is_dir())
        .map(|run| run.id.clone())
        .collect();
    missing_runs.sort();

    let known_repo_dirs: HashSet<String> = manifest
        .repos
        .iter()
        .filter_map(|repo| direct_child_name("repos", &repo.checkout_path))
        .collect();
    let known_run_dirs: HashSet<String> = manifest
        .runs
        .iter()
        .filter_map(|run| direct_child_name("runs", &run.root_path))
        .collect();

    let orphan_repo_dirs = orphan_dir_names(&workspace_root.join("repos"), &known_repo_dirs)?;
    let orphan_run_dirs = orphan_dir_names(&workspace_root.join("runs"), &known_run_dirs)?;

    Ok(ValidationReport {
        workspace_id: workspace_id.to_string(),
        ok: missing_repos.is_empty()
            && missing_runs.is_empty()
            && orphan_repo_dirs.is_empty()
            && orphan_run_dirs.is_empty(),
        missing_repos,
        missing_runs,
        orphan_repo_dirs,
        orphan_run_dirs,
    })
}

fn direct_child_name(parent: &str, relative_path: &str) -> Option<String> {
    let path = Path::new(relative_path);
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (
            Some(std::path::Component::Normal(root)),
            Some(std::path::Component::Normal(child)),
            None,
        ) if root == parent => Some(child.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn orphan_dir_names(
    root: &Path,
    known_dirs: &HashSet<String>,
) -> Result<Vec<String>, WorkspaceError> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !known_dirs.contains(&name) {
                orphans.push(name);
            }
        }
    }
    orphans.sort();
    Ok(orphans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::write_manifest_atomic_for;
    use crate::types::CloneRecord;
    use crate::types::GitState;
    use crate::types::LfsPolicy;
    use crate::types::PinMode;
    use crate::types::PinState;
    use crate::types::RepoEntry;
    use crate::types::RunEntry;
    use crate::types::RunSource;
    use crate::types::RunStatus;
    use crate::types::SubmodulePolicy;
    use crate::types::new_manifest;
    use pretty_assertions::assert_eq;
    use serde_json::Map;

    #[test]
    fn run_reports_missing_and_orphaned_paths() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut manifest = new_manifest("workspace-1", "Workspace One");
        manifest.repos.push(RepoEntry {
            id: "repo-1".to_string(),
            alias: "tracked".to_string(),
            repo_key: "org/repo".to_string(),
            remote_url: "https://github.com/org/repo.git".to_string(),
            checkout_path: "repos/tracked".to_string(),
            notes_path: "notes/repos/tracked".to_string(),
            clone: CloneRecord {
                depth: 1,
                single_branch: true,
                no_tags: true,
                filter: "blob:limit=1m".to_string(),
                submodules: SubmodulePolicy::None,
                lfs: LfsPolicy::Auto,
                extra: Map::new(),
            },
            pin: PinState {
                mode: PinMode::Tracking,
                pinned_sha: String::new(),
                extra: Map::new(),
            },
            state: GitState {
                head_sha: String::new(),
                head_ref: String::new(),
                default_branch: String::new(),
                shallow: false,
                extra: Map::new(),
            },
            extra: Map::new(),
        });
        manifest.runs.push(RunEntry {
            id: "run-1".to_string(),
            name: "Run".to_string(),
            created_at: 1,
            updated_at: 1,
            root_path: "runs/run-1".to_string(),
            status: RunStatus::Created,
            source: RunSource {
                repo_alias: "tracked".to_string(),
                sha: String::new(),
                extra: Map::new(),
            },
            extra: Map::new(),
        });
        write_manifest_atomic_for(temp.path(), "workspace-1", &manifest).expect("write manifest");

        let workspace_root = paths::workspace_root_for(temp.path(), "workspace-1");
        std::fs::create_dir_all(workspace_root.join("repos").join("orphan-repo"))
            .expect("create orphan repo dir");
        std::fs::create_dir_all(workspace_root.join("runs").join("orphan-run"))
            .expect("create orphan run dir");

        let report = run_for(temp.path(), "workspace-1").expect("validate workspace");

        assert_eq!(
            report,
            ValidationReport {
                workspace_id: "workspace-1".to_string(),
                ok: false,
                missing_repos: vec!["tracked".to_string()],
                missing_runs: vec!["run-1".to_string()],
                orphan_repo_dirs: vec!["orphan-repo".to_string()],
                orphan_run_dirs: vec!["orphan-run".to_string()],
            }
        );
    }
}
