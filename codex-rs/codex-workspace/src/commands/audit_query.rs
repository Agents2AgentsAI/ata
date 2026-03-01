use crate::audit;
use crate::error::WorkspaceError;
use serde_json::Value;

/// Query audit log entries with optional filters.
pub fn run(
    workspace_id: &str,
    since: Option<i64>,
    until: Option<i64>,
    ops: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, WorkspaceError> {
    audit::query_audit(workspace_id, since, until, ops, limit)
}
