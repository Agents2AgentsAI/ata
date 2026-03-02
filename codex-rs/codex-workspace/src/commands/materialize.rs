use crate::audit::write_audit;
use crate::commands::repo_clone;
use crate::commands::repo_pin;
use crate::commands::repo_update_state;
use crate::commands::set_field;
use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::read_manifest;
use crate::manifest::with_locked_manifest;
use crate::paths;
use crate::spec::WorkspaceSpec;
use crate::spec::read_spec;
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

/// Compute what materialize would do without executing.
pub fn plan(workspace_id: &str, spec: &WorkspaceSpec) -> Result<Vec<RepoAction>, WorkspaceError> {
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
                } else if let Some(ref_name) = &repo_spec.r#ref {
                    // Ref without sha — resolve at runtime, but try to detect
                    // if checkout already matches by resolving locally.
                    let checkout = paths::workspace_root(workspace_id)
                        .join("repos")
                        .join(&repo_spec.alias);
                    if let Some(resolved) = git::resolve_ref(&checkout, ref_name) {
                        let current_sha = if !existing_repo.pin.pinned_sha.is_empty() {
                            &existing_repo.pin.pinned_sha
                        } else {
                            &existing_repo.state.head_sha
                        };
                        if current_sha != &resolved {
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
pub fn run(workspace_id: &str, spec_path: &Path, dry_run: bool) -> Result<Value, WorkspaceError> {
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

                let checkout_path = paths::workspace_root(workspace_id)
                    .join("repos")
                    .join(&repo_spec.alias);

                // Determine target SHA: explicit sha, or resolve ref in fresh checkout
                let target_sha = repo_spec.sha.clone().or_else(|| {
                    repo_spec
                        .r#ref
                        .as_deref()
                        .and_then(|r| git::resolve_ref(&checkout_path, r))
                });

                // Pin and checkout if we have a target SHA
                if let Some(sha) = &target_sha {
                    repo_pin::run(workspace_id, &repo_spec.alias, sha)?;
                    pin_checkout(workspace_id, &repo_spec.alias, &checkout_path, sha);
                }

                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "added",
                    "details": clone_result,
                }));
            }
            ActionKind::Pin { target_sha, .. } => {
                repo_pin::run(workspace_id, &repo_spec.alias, target_sha)?;

                let checkout_path = paths::workspace_root(workspace_id)
                    .join("repos")
                    .join(&repo_spec.alias);
                pin_checkout(workspace_id, &repo_spec.alias, &checkout_path, target_sha);

                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;

                results.push(json!({
                    "alias": repo_spec.alias,
                    "action": "pinned",
                    "sha": target_sha,
                }));
            }
            ActionKind::Ref { ref_name } => {
                let checkout_path = paths::workspace_root(workspace_id)
                    .join("repos")
                    .join(&repo_spec.alias);

                if let Some(resolved) = git::resolve_ref(&checkout_path, ref_name) {
                    repo_pin::run(workspace_id, &repo_spec.alias, &resolved)?;
                    pin_checkout(workspace_id, &repo_spec.alias, &checkout_path, &resolved);

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

                apply_repo_extra(workspace_id, &repo_spec.alias, &repo_spec.extra)?;
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

    // Merge spec-level policies and labels (additive, not overwrite)
    if spec.policies.is_some() || !spec.labels.is_empty() {
        let spec_policies = spec.policies.clone();
        let spec_labels = spec.labels.clone();
        with_locked_manifest(workspace_id, None, move |m| {
            if let Some(sp) = spec_policies {
                let mut existing =
                    serde_json::to_value(&m.policies).map_err(WorkspaceError::Json)?;
                let incoming = serde_json::to_value(&sp).map_err(WorkspaceError::Json)?;
                if let (Some(base), Some(patch)) = (existing.as_object_mut(), incoming.as_object())
                {
                    for (k, v) in patch {
                        base.insert(k.clone(), v.clone());
                    }
                }
                m.policies = serde_json::from_value(existing).map_err(WorkspaceError::Json)?;
            }
            for (k, v) in spec_labels {
                m.labels.insert(k, v);
            }
            Ok(())
        })?;
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

/// After pinning, checkout the SHA and update the repo's git state in the manifest.
fn pin_checkout(workspace_id: &str, alias: &str, checkout_path: &Path, sha: &str) {
    if let Err(e) = git::fetch_and_checkout(checkout_path, sha) {
        eprintln!("warning: git checkout failed for {alias}: {e}");
        return;
    }
    let git_state = git::read_git_state(checkout_path);
    if let Err(e) = repo_update_state::run(
        workspace_id,
        alias,
        &git_state.head_sha,
        Some(git_state.head_ref.as_str()).filter(|s| !s.is_empty()),
    ) {
        eprintln!("warning: failed to update git state for {alias}: {e}");
    }
}

/// Apply extra fields from a RepoSpec to the corresponding RepoEntry.
fn apply_repo_extra(
    workspace_id: &str,
    alias: &str,
    extra: &serde_json::Map<String, Value>,
) -> Result<(), WorkspaceError> {
    if extra.is_empty() {
        return Ok(());
    }
    let alias = alias.to_string();
    let extra = extra.clone();
    with_locked_manifest(workspace_id, None, move |m| {
        let repo = m
            .repos
            .iter_mut()
            .find(|r| r.alias == alias)
            .ok_or_else(|| WorkspaceError::EntryNotFound(alias.clone()))?;
        for (key, value) in extra {
            repo.extra.insert(key, value);
        }
        Ok(())
    })?;
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
