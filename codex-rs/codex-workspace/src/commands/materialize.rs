use crate::audit::write_audit;
use crate::commands::{repo_clone, repo_pin, set_field};
use crate::error::WorkspaceError;
use crate::manifest::read_manifest;
use crate::spec::{read_spec, WorkspaceSpec};
use serde_json::{json, Value};
use std::path::Path;

/// Action determined for each repo in the spec.
#[derive(Debug, Clone)]
pub struct RepoAction {
    pub alias: String,
    pub action: ActionKind,
}

#[derive(Debug, Clone)]
pub enum ActionKind {
    /// Repo doesn't exist in workspace — needs cloning.
    Add,
    /// Repo exists but pinned SHA differs — needs re-pin.
    Pin { current_sha: String, target_sha: String },
    /// Repo exists and matches spec — nothing to do.
    Skip,
}

/// Compute what materialize would do without executing.
pub fn plan(
    workspace_id: &str,
    spec: &WorkspaceSpec,
) -> Result<Vec<RepoAction>, WorkspaceError> {
    let manifest = read_manifest(workspace_id)?;
    let mut actions = Vec::new();

    for repo_spec in &spec.repos {
        let existing = manifest.repos.iter().find(|r| r.alias == repo_spec.alias);

        let action = match existing {
            None => ActionKind::Add,
            Some(existing_repo) => {
                if let Some(target_sha) = &repo_spec.sha {
                    let current_sha = if !existing_repo.pin.pinned_sha.is_empty() {
                        &existing_repo.pin.pinned_sha
                    } else {
                        &existing_repo.state.head_sha
                    };
                    if current_sha != target_sha {
                        ActionKind::Pin {
                            current_sha: current_sha.to_string(),
                            target_sha: target_sha.clone(),
                        }
                    } else {
                        ActionKind::Skip
                    }
                } else {
                    ActionKind::Skip
                }
            }
        };

        actions.push(RepoAction {
            alias: repo_spec.alias.clone(),
            action,
        });
    }

    Ok(actions)
}

/// Materialize a workspace spec file into a workspace.
///
/// For each repo in the spec:
/// - If not in workspace → clone it
/// - If in workspace but SHA differs → re-pin
/// - If in workspace and matches → skip
///
/// Also applies policies, labels, and extra fields from the spec.
pub fn run(
    workspace_id: &str,
    spec_path: &Path,
    dry_run: bool,
) -> Result<Value, WorkspaceError> {
    let spec = read_spec(spec_path)?;
    let actions = plan(workspace_id, &spec)?;

    if dry_run {
        return Ok(format_plan(&actions));
    }

    let mut results: Vec<Value> = Vec::new();

    for (i, action) in actions.iter().enumerate() {
        let repo_spec = &spec.repos[i];
        match &action.action {
            ActionKind::Add => {
                // Clone the repo
                let clone_result = repo_clone::run(
                    workspace_id,
                    &repo_spec.url,
                    &repo_spec.alias,
                    repo_spec.full,
                )?;

                // Pin if sha specified
                if let Some(sha) = &repo_spec.sha {
                    repo_pin::run(workspace_id, &repo_spec.alias, sha)?;
                }

                // Apply extra fields (role, metadata, etc.)
                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "added",
                    "details": clone_result,
                }));
            }
            ActionKind::Pin { target_sha, .. } => {
                repo_pin::run(workspace_id, &repo_spec.alias, target_sha)?;

                // Apply extra fields in case they changed
                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "pinned",
                    "sha": target_sha,
                }));
            }
            ActionKind::Skip => {
                // Still apply extra fields in case they changed
                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "skipped",
                }));
            }
        }
    }

    // Apply spec-level policies
    if let Some(policies) = &spec.policies {
        let policies_json = serde_json::to_string(policies)
            .map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;
        set_field::run(workspace_id, "policies", &policies_json)?;
    }

    // Apply labels
    if !spec.labels.is_empty() {
        let labels_json = serde_json::to_string(&spec.labels)
            .map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;
        set_field::run(workspace_id, "labels", &labels_json)?;
    }

    // Record spec source provenance
    let spec_source = spec_path
        .canonicalize()
        .unwrap_or_else(|_| spec_path.to_path_buf())
        .display()
        .to_string();
    set_field::run(
        workspace_id,
        "specSource",
        &format!("\"{}\"", spec_source.replace('"', "\\\"")),
    )?;

    // Audit
    write_audit(
        workspace_id,
        "spec_materialize",
        vec![json!({"type": "spec", "path": spec_path.display().to_string()})],
        Some(json!({"repoCount": spec.repos.len(), "name": spec.name})),
    )?;

    Ok(json!({
        "workspace": workspace_id,
        "spec": spec.name,
        "repos": results,
    }))
}

/// Apply extra fields from a RepoSpec to the corresponding RepoEntry.
fn apply_repo_extra(
    workspace_id: &str,
    alias: &str,
    extra: &serde_json::Map<String, Value>,
) -> Result<(), WorkspaceError> {
    // Find the repo's index in the manifest to build the dotted path
    let manifest = read_manifest(workspace_id)?;
    let idx = manifest
        .repos
        .iter()
        .position(|r| r.alias == alias)
        .ok_or_else(|| WorkspaceError::EntryNotFound(alias.to_string()))?;

    for (key, value) in extra {
        let path = format!("repos.{idx}.{key}");
        let value_str = serde_json::to_string(value)
            .map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;
        set_field::run(workspace_id, &path, &value_str)?;
    }

    Ok(())
}

fn format_plan(actions: &[RepoAction]) -> Value {
    let items: Vec<Value> = actions
        .iter()
        .map(|a| {
            let action_str = match &a.action {
                ActionKind::Add => "add".to_string(),
                ActionKind::Pin {
                    current_sha,
                    target_sha,
                } => format!("pin ({current_sha} → {target_sha})"),
                ActionKind::Skip => "skip".to_string(),
            };
            json!({"alias": a.alias, "action": action_str})
        })
        .collect();
    json!({"dryRun": true, "actions": items})
}
