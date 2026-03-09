use crate::error::WorkspaceError;
use crate::manifest::with_locked_manifest;
use crate::types::RunStatus;
use crate::types::WorkspaceManifest;

/// Update a run's status field.
pub fn run(
    workspace_id: &str,
    run_id: &str,
    status: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    let run_id = run_id.to_string();
    let status = status
        .parse::<RunStatus>()
        .map_err(WorkspaceError::InvalidRunStatus)?;

    with_locked_manifest(workspace_id, None, move |m| {
        let entry = m
            .runs
            .iter_mut()
            .find(|r| r.id == run_id)
            .ok_or_else(|| WorkspaceError::EntryNotFound(run_id.clone()))?;
        entry.status = status;
        entry.updated_at = chrono::Utc::now().timestamp();
        Ok(())
    })
}
