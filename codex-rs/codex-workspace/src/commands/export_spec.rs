use crate::error::WorkspaceError;
use crate::manifest::read_manifest;
use crate::spec::RepoSpec;
use crate::spec::WorkspaceSpec;
use serde_json::Map;
use std::path::Path;

/// Export the current workspace state as a workspace spec.
///
/// For each repo, extracts url, alias, current pinned/head SHA, and extra fields.
/// Also copies policies and labels.
pub fn run(workspace_id: &str, output: Option<&Path>) -> Result<String, WorkspaceError> {
    let manifest = read_manifest(workspace_id)?;

    let repos: Vec<RepoSpec> = manifest
        .repos
        .iter()
        .map(|repo| {
            // Use pinned SHA if pinned, otherwise head SHA
            let sha = if !repo.pin.pinned_sha.is_empty() {
                Some(repo.pin.pinned_sha.clone())
            } else if !repo.state.head_sha.is_empty() {
                Some(repo.state.head_sha.clone())
            } else {
                None
            };

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
        policies: Some(manifest.policies.clone()),
        labels: manifest.labels,
        extra: Map::new(), // Don't export runtime extra fields
    };

    let json = serde_json::to_string_pretty(&spec)?;

    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &json)?;
    }

    Ok(json)
}
