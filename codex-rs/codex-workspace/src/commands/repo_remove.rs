use crate::audit::write_audit;
use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::paths;
use serde_json::json;

/// Remove a repo: delete directory, remove from manifest, audit.
pub fn run(workspace_id: &str, alias: &str) -> Result<(), WorkspaceError> {
    let alias_owned = alias.to_string();
    let alias_for_closure = alias_owned.clone();
    let mut removed_id = None;
    with_locked_manifest(workspace_id, None, |m| {
        removed_id = m
            .repos
            .iter()
            .find(|r| r.alias == alias_for_closure)
            .map(|r| r.id.clone());
        m.repos.retain(|r| r.alias != alias_for_closure);
        Ok(())
    })?;

    let root = paths::workspace_root(workspace_id);
    let repo_dir = root.join("repos").join(alias);
    if repo_dir.is_dir() {
        std::fs::remove_dir_all(&repo_dir)?;
    }

    // Audit
    let mut target = json!({"type": "repo", "alias": &alias_owned});
    if let Some(id) = removed_id {
        target["id"] = json!(id);
    }
    write_audit(workspace_id, "repo_remove", vec![target], None)?;
    Ok(())
}
