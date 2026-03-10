use crate::error::WorkspaceError;
use crate::manifest::atomic_write;
use crate::manifest::read_manifest_for;
use crate::spec::RepoSpec;
use crate::spec::WorkspaceSpec;
use serde_json::Map;
use std::path::Path;

/// Export the current workspace state as a workspace spec.
///
/// For each repo, extracts url, alias, current pinned/head SHA, and extra fields.
/// Also copies policies and labels.
pub fn run(workspace_id: &str, output: Option<&Path>) -> Result<String, WorkspaceError> {
    run_for(&crate::paths::codex_home(), workspace_id, output)
}

fn run_for(
    codex_home: &Path,
    workspace_id: &str,
    output: Option<&Path>,
) -> Result<String, WorkspaceError> {
    let manifest = read_manifest_for(codex_home, workspace_id)?;

    let repos: Vec<RepoSpec> = manifest
        .repos
        .iter()
        .map(|repo| {
            let sha = (!repo.effective_sha().is_empty()).then(|| repo.effective_sha().to_string());

            // Use head_ref as the ref if available
            let r#ref = if !repo.state.head_ref.is_empty() {
                Some(repo.state.head_ref.clone())
            } else {
                None
            };

            RepoSpec {
                url: repo.remote_url.clone(),
                alias: repo.alias.clone(),
                sha,
                r#ref,
                full: repo.clone.depth == 0,
                extra: repo.extra.clone(),
            }
        })
        .collect();

    let spec = WorkspaceSpec {
        schema_version: 1,
        name: manifest.name.clone(),
        repos,
        policies: (manifest.policies != crate::types::Policies::default())
            .then_some(manifest.policies.clone()),
        labels: manifest.labels,
        extra: Map::new(), // Don't export runtime extra fields
    };

    let json = serde_json::to_string_pretty(&spec)?;

    if let Some(path) = output {
        atomic_write(path, json.as_bytes())?;
    }

    Ok(json)
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
    use crate::types::SubmodulePolicy;
    use crate::types::new_manifest;
    use pretty_assertions::assert_eq;

    #[test]
    fn run_exports_to_output_file() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut manifest = new_manifest("workspace-1", "Workspace One");
        manifest.repos.push(RepoEntry {
            id: "repo-1".to_string(),
            alias: "repo".to_string(),
            repo_key: "org/repo".to_string(),
            remote_url: "https://github.com/org/repo.git".to_string(),
            checkout_path: "repos/repo".to_string(),
            notes_path: "notes/repos/repo".to_string(),
            clone: CloneRecord {
                depth: 0,
                single_branch: false,
                no_tags: false,
                filter: String::new(),
                submodules: SubmodulePolicy::None,
                lfs: LfsPolicy::Auto,
                extra: Map::new(),
            },
            pin: PinState {
                mode: PinMode::Pinned,
                pinned_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
                extra: Map::new(),
            },
            state: GitState {
                head_sha: "fedcba9876543210fedcba9876543210fedcba98".to_string(),
                head_ref: "main".to_string(),
                default_branch: "main".to_string(),
                shallow: false,
                extra: Map::new(),
            },
            extra: Map::new(),
        });
        write_manifest_atomic_for(temp.path(), "workspace-1", &manifest).expect("write manifest");
        let output = temp.path().join("spec.json");

        let json = run_for(temp.path(), "workspace-1", Some(&output)).expect("export spec");
        let spec: WorkspaceSpec = serde_json::from_str(&json).expect("parse exported spec");

        assert_eq!(spec.name, "Workspace One");
        assert_eq!(spec.repos.len(), 1);
        assert_eq!(spec.repos[0].alias, "repo");
        assert_eq!(
            spec.repos[0].sha.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(
            std::fs::read_to_string(&output).expect("read output file"),
            json
        );
    }
}
