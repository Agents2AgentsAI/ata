use crate::audit as audit_mod;
use crate::error::WorkspaceError;
use crate::types::AuditEntry;
use serde_json::Value;

/// Append an audit entry from raw JSON input.
pub fn run(workspace_id: &str, json_str: &str) -> Result<AuditEntry, WorkspaceError> {
    let input: Value =
        serde_json::from_str(json_str).map_err(|e| WorkspaceError::InvalidJson(e.to_string()))?;

    let op = input
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let status_str = input
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("success")
        .to_string();
    let targets: Vec<Value> = input
        .get("targets")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let details = input.get("details").cloned();

    let mut entry = audit_mod::build_audit_entry(workspace_id, &op, targets, details);
    entry.status = status_str;
    audit_mod::append_audit_entry(workspace_id, &entry)?;
    Ok(entry)
}
