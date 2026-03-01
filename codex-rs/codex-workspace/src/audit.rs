use crate::error::WorkspaceError;
use crate::paths;
use crate::types::{AuditActor, AuditEntry};
use serde_json::Value;

/// Build the audit actor from environment variables.
pub fn build_actor() -> AuditActor {
    let session_id = std::env::var("CODEX_SESSION_ID").ok().filter(|s| !s.is_empty());
    let thread_id = std::env::var("CODEX_THREAD_ID").ok().filter(|s| !s.is_empty());
    AuditActor {
        kind: "agent".to_string(),
        session_id,
        thread_id,
    }
}

/// Build a full audit entry with envelope fields.
pub fn build_audit_entry(
    workspace_id: &str,
    op: &str,
    targets: Vec<Value>,
    details: Option<Value>,
) -> AuditEntry {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    AuditEntry {
        schema_version: 1,
        ts,
        workspace_id: workspace_id.to_string(),
        actor: build_actor(),
        op: op.to_string(),
        status: "success".to_string(),
        targets,
        details,
    }
}

/// Append an audit entry to the workspace audit log (NDJSON).
pub fn append_audit_entry(
    workspace_id: &str,
    entry: &AuditEntry,
) -> Result<(), WorkspaceError> {
    let ap = paths::audit_path(workspace_id);
    if let Some(parent) = ap.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry)? + "\n";
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ap)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// Write an audit entry: build envelope, append to log, return the entry.
pub fn write_audit(
    workspace_id: &str,
    op: &str,
    targets: Vec<Value>,
    details: Option<Value>,
) -> Result<AuditEntry, WorkspaceError> {
    let entry = build_audit_entry(workspace_id, op, targets, details);
    append_audit_entry(workspace_id, &entry)?;
    Ok(entry)
}

/// Query audit log entries with optional filters.
pub fn query_audit(
    workspace_id: &str,
    since: Option<i64>,
    until: Option<i64>,
    ops: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, WorkspaceError> {
    let ap = paths::audit_path(workspace_id);
    if !ap.is_file() {
        return Ok(Vec::new());
    }

    let ops_set: Option<std::collections::HashSet<String>> = ops.map(|o| {
        o.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let mut results = Vec::new();
    let data = std::fs::read_to_string(&ap)?;

    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Time filtering: parse ISO timestamp
        if (since.is_some() || until.is_some())
            && let Some(ts_str) = entry.get("ts").and_then(|v| v.as_str())
            && let Ok(dt) =
                chrono::NaiveDateTime::parse_from_str(ts_str, "%Y-%m-%dT%H:%M:%SZ")
        {
            let ts_unix = dt.and_utc().timestamp();
            if let Some(s) = since
                && ts_unix < s
            {
                continue;
            }
            if let Some(u) = until
                && ts_unix > u
            {
                continue;
            }
        }

        // Op filtering
        if let Some(ref ops_set) = ops_set {
            let op = entry
                .get("op")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !ops_set.contains(op) {
                continue;
            }
        }

        results.push(entry);
        if results.len() >= limit {
            break;
        }
    }

    Ok(results)
}
