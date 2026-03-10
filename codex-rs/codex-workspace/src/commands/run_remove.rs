use crate::audit::write_audit;
use crate::error::WorkspaceError;
use crate::git;
use crate::manifest::with_locked_manifest;
use crate::paths;
use serde_json::json;

/// Remove a run: clean worktree, remove directory, remove from manifest, audit.
pub fn run(workspace_id: &str, run_id: &str) -> Result<(), WorkspaceError> {
    let root = paths::workspace_root(workspace_id);
    let run_root = root.join("runs").join(run_id);
    let code_root = run_root.join("root");
    let run_id_owned = run_id.to_string();
    let run_id_for_closure = run_id_owned.clone();

    with_locked_manifest(workspace_id, None, |m| {
        let before_len = m.runs.len();
        m.runs.retain(|r| r.id != run_id_for_closure);
        if m.runs.len() == before_len {
            return Err(WorkspaceError::EntryNotFound(run_id_owned.clone()));
        }
        Ok(())
    })?;

    // Clean up worktree if applicable
    if code_root.is_dir() && !git::worktree_remove(&code_root) {
        eprintln!(
            "warning: failed to remove git worktree cleanly for run {run_id_owned}: {}",
            code_root.display()
        );
    }

    // Remove directory
    if run_root.is_dir() {
        std::fs::remove_dir_all(&run_root)?;
    }

    // Audit
    write_audit(
        workspace_id,
        "run_delete",
        vec![json!({"type": "run", "id": &run_id_owned})],
        None,
    )?;
    Ok(())
}
