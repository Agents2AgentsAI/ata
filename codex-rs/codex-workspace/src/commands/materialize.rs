use crate::audit::write_audit;
use crate::commands::repo_clone;
use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::read_manifest;
use crate::manifest::with_locked_manifest;
use crate::paths;
use crate::spec::WorkspaceSpec;
use crate::spec::read_spec;
use crate::types::GitState;
use crate::types::PinMode;
use crate::types::WorkspaceManifest;
use crate::url_validation::check_host_allowlist;
use crate::url_validation::validate_repo_url;
use serde_json::Value;
use serde_json::json;
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
    Pin {
        current_sha: String,
        target_sha: String,
    },
    /// Repo has a `ref` but no `sha` — needs ref resolution at runtime.
    Ref { ref_name: String },
    /// Repo exists and matches spec — nothing to do.
    Skip,
}

#[derive(Debug, Clone)]
struct RepoManifestUpdate {
    alias: String,
    pin_sha: Option<String>,
    state: Option<GitState>,
    extra: serde_json::Map<String, Value>,
}

/// Compute what materialize would do without executing.
pub fn plan(workspace_id: &str, spec: &WorkspaceSpec) -> Result<Vec<RepoAction>, WorkspaceError> {
    let manifest = read_manifest(workspace_id)?;
    Ok(plan_from_manifest(workspace_id, &manifest, spec))
}

fn plan_from_manifest(
    workspace_id: &str,
    manifest: &WorkspaceManifest,
    spec: &WorkspaceSpec,
) -> Vec<RepoAction> {
    let mut actions = Vec::new();

    for repo_spec in &spec.repos {
        let existing = manifest.repo_by_alias(&repo_spec.alias).ok();

        let action = match existing {
            None => ActionKind::Add,
            Some(existing_repo) => {
                if let Some(target_sha) = &repo_spec.sha {
                    let current_sha = existing_repo.effective_sha();
                    if current_sha != target_sha {
                        ActionKind::Pin {
                            current_sha: current_sha.to_string(),
                            target_sha: target_sha.clone(),
                        }
                    } else {
                        ActionKind::Skip
                    }
                } else if let Some(ref_name) = &repo_spec.r#ref {
                    // Ref without sha — resolve at runtime, but try to detect
                    // if checkout already matches by resolving locally.
                    let checkout = paths::repo_checkout_path(workspace_id, &repo_spec.alias);
                    if let Some(resolved) = git::resolve_ref(&checkout, ref_name) {
                        let current_sha = existing_repo.effective_sha();
                        if current_sha != resolved {
                            ActionKind::Pin {
                                current_sha: current_sha.to_string(),
                                target_sha: resolved,
                            }
                        } else {
                            ActionKind::Skip
                        }
                    } else {
                        ActionKind::Ref {
                            ref_name: ref_name.clone(),
                        }
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

    actions
}

/// Materialize a workspace spec file into a workspace.
///
/// For each repo in the spec:
/// - If not in workspace → clone it
/// - If in workspace but SHA differs → re-pin
/// - If in workspace and matches → skip
///
/// Also applies policies, labels, and extra fields from the spec.
pub fn run(workspace_id: &str, spec_path: &Path, dry_run: bool) -> Result<Value, WorkspaceError> {
    let spec = read_spec(spec_path)?;
    let manifest = read_manifest(workspace_id)?;
    let actions = plan_from_manifest(workspace_id, &manifest, &spec);

    if dry_run {
        return Ok(format_plan(&actions));
    }

    validate_materialize_preflight(
        &spec,
        &actions,
        manifest.policies.repo_hosts_allowlist.as_deref(),
    )?;

    let mut results: Vec<Value> = Vec::new();
    let mut repo_updates: Vec<RepoManifestUpdate> = Vec::new();

    for (i, action) in actions.iter().enumerate() {
        let repo_spec = &spec.repos[i];
        let mut repo_update = RepoManifestUpdate {
            alias: repo_spec.alias.clone(),
            pin_sha: None,
            state: None,
            extra: repo_spec.extra.clone(),
        };
        match &action.action {
            ActionKind::Add => {
                // Clone the repo
                let clone_result = repo_clone::run(
                    workspace_id,
                    &repo_spec.url,
                    &repo_spec.alias,
                    repo_spec.full,
                )?;

                let checkout_path = paths::repo_checkout_path(workspace_id, &repo_spec.alias);

                // Determine target SHA: explicit sha, or resolve ref in fresh checkout
                let target_sha = repo_spec.sha.clone().or_else(|| {
                    repo_spec
                        .r#ref
                        .as_deref()
                        .and_then(|r| git::resolve_ref(&checkout_path, r))
                });

                // Pin and checkout if we have a target SHA
                if let Some(sha) = &target_sha {
                    repo_update.state = Some(pin_checkout(&repo_spec.alias, &checkout_path, sha)?);
                    repo_update.pin_sha = Some(sha.clone());
                }

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "added",
                    "details": clone_result,
                }));
            }
            ActionKind::Pin { target_sha, .. } => {
                let checkout_path = paths::repo_checkout_path(workspace_id, &repo_spec.alias);
                repo_update.state =
                    Some(pin_checkout(&repo_spec.alias, &checkout_path, target_sha)?);
                repo_update.pin_sha = Some(target_sha.clone());

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "pinned",
                    "sha": target_sha,
                }));
            }
            ActionKind::Ref { ref_name } => {
                let checkout_path = paths::repo_checkout_path(workspace_id, &repo_spec.alias);

                if let Some(resolved) = git::resolve_ref(&checkout_path, ref_name) {
                    repo_update.state =
                        Some(pin_checkout(&repo_spec.alias, &checkout_path, &resolved)?);
                    repo_update.pin_sha = Some(resolved.clone());

                    results.push(json!({
                        "alias": repo_spec.alias,
                        "action": "pinned",
                        "ref": ref_name,
                        "sha": resolved,
                    }));
                } else {
                    eprintln!(
                        "warning: could not resolve ref '{}' for {}",
                        ref_name, repo_spec.alias
                    );
                    results.push(json!({
                        "alias": repo_spec.alias,
                        "action": "skipped",
                        "reason": format!("could not resolve ref '{ref_name}'"),
                    }));
                }
            }
            ActionKind::Skip => {
                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "skipped",
                }));
            }
        }
        repo_updates.push(repo_update);
    }

    let spec_source = spec_path
        .canonicalize()
        .unwrap_or_else(|_| spec_path.to_path_buf())
        .display()
        .to_string();
    let spec_policies = spec.policies.clone();
    let spec_labels = spec.labels.clone();
    with_locked_manifest(workspace_id, None, move |m| {
        for repo_update in repo_updates {
            let repo = m.repo_by_alias_mut(&repo_update.alias)?;
            if let Some(pin_sha) = repo_update.pin_sha {
                repo.pin.mode = PinMode::Pinned;
                repo.pin.pinned_sha = pin_sha;
            }
            if let Some(git_state) = repo_update.state {
                repo.state.head_sha = git_state.head_sha;
                if !git_state.head_ref.is_empty() {
                    repo.state.head_ref = git_state.head_ref;
                }
                if !git_state.default_branch.is_empty() {
                    repo.state.default_branch = git_state.default_branch;
                }
                repo.state.shallow = git_state.shallow;
            }
            for (key, value) in repo_update.extra {
                repo.extra.insert(key, value);
            }
        }

        if let Some(sp) = spec_policies {
            let mut existing = serde_json::to_value(&m.policies).map_err(WorkspaceError::Json)?;
            let incoming = serde_json::to_value(&sp).map_err(WorkspaceError::Json)?;
            if let (Some(base), Some(patch)) = (existing.as_object_mut(), incoming.as_object()) {
                for (key, value) in patch {
                    base.insert(key.clone(), value.clone());
                }
            }
            m.policies = serde_json::from_value(existing).map_err(WorkspaceError::Json)?;
        }
        for (key, value) in spec_labels {
            m.labels.insert(key, value);
        }
        m.extra
            .insert("specSource".to_string(), Value::String(spec_source));
        Ok(())
    })?;

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

/// After pinning, checkout the SHA and return the resulting git state.
fn pin_checkout(alias: &str, checkout_path: &Path, sha: &str) -> Result<GitState, WorkspaceError> {
    let checked_out = git::fetch_and_checkout(checkout_path, sha)?;
    if !checked_out {
        return Err(WorkspaceError::GitCheckoutFailed {
            alias: alias.to_string(),
            sha: sha.to_string(),
        });
    }
    Ok(git::read_git_state(checkout_path))
}

fn validate_materialize_preflight(
    spec: &WorkspaceSpec,
    actions: &[RepoAction],
    allowlist: Option<&[String]>,
) -> Result<(), WorkspaceError> {
    for (repo_spec, action) in spec.repos.iter().zip(actions) {
        if matches!(action.action, ActionKind::Add) {
            validate_repo_url(&repo_spec.url)?;
            check_host_allowlist(&repo_spec.url, allowlist)?;
        }
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
                ActionKind::Ref { ref_name } => format!("ref ({ref_name})"),
                ActionKind::Skip => "skip".to_string(),
            };
            json!({"alias": a.alias, "action": action_str})
        })
        .collect();
    json!({"dryRun": true, "actions": items})
}
