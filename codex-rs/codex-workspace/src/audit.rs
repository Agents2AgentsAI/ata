use crate::error::WorkspaceError;
use crate::lock::FileLock;
use crate::paths;
use crate::types::AuditActor;
use crate::types::AuditEntry;
use serde_json::Value;

/// Build the audit actor from environment variables.
pub fn build_actor() -> AuditActor {
    let context = paths::SessionContext::from_env();
    AuditActor {
        kind: "agent".to_string(),
        session_id: context.session_id,
        thread_id: context.thread_id,
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
///
/// This only serializes concurrent audit appends. Current callers invoke it
/// after workspace mutations complete, but future code that holds
/// `workspace.lock` while auditing must acquire that lock before `audit.lock`.
pub fn append_audit_entry(workspace_id: &str, entry: &AuditEntry) -> Result<(), WorkspaceError> {
    let ap = paths::audit_path(workspace_id);
    if let Some(parent) = ap.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = FileLock::acquire(&paths::audit_lock_path(workspace_id))?;
    let line = serde_json::to_string(entry)? + "\n";
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ap)?;
    file.write_all(line.as_bytes())?;
    file.sync_all()?;
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
    let file = std::fs::File::open(&ap)?;
    let reader = std::io::BufReader::new(file);

    for line in std::io::BufRead::lines(reader) {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if !entry_matches_filters(&entry, since, until, ops_set.as_ref()) {
            continue;
        }

        results.push(entry);
        if results.len() >= limit {
            break;
        }
    }

    Ok(results)
}

fn entry_matches_filters(
    entry: &Value,
    since: Option<i64>,
    until: Option<i64>,
    ops_set: Option<&std::collections::HashSet<String>>,
) -> bool {
    if since.is_some() || until.is_some() {
        let ts_unix = entry
            .get("ts")
            .and_then(|value| value.as_str())
            .and_then(parse_audit_timestamp);
        let Some(ts_unix) = ts_unix else {
            return false;
        };

        if let Some(start) = since
            && ts_unix < start
        {
            return false;
        }
        if let Some(end) = until
            && ts_unix > end
        {
            return false;
        }
    }

    if let Some(ops_set) = ops_set {
        let op = entry
            .get("op")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !ops_set.contains(op) {
            return false;
        }
    }

    true
}

fn parse_audit_timestamp(ts: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%SZ")
        .ok()
        .map(|value| value.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn malformed_timestamp_is_excluded_when_time_filter_is_active() {
        let entry = json!({"ts": "not-a-time", "op": "repo_add"});
        assert!(!entry_matches_filters(&entry, Some(1), None, None));
    }

    #[test]
    fn malformed_timestamp_is_allowed_without_time_filter() {
        let entry = json!({"ts": "not-a-time", "op": "repo_add"});
        assert!(entry_matches_filters(&entry, None, None, None));
    }

    #[test]
    fn op_filter_still_applies_after_timestamp_check() {
        let entry = json!({"ts": "2026-03-09T00:00:00Z", "op": "repo_add"});
        let mut ops = HashSet::new();
        ops.insert("repo_remove".to_string());
        assert_eq!(
            entry_matches_filters(&entry, Some(0), None, Some(&ops)),
            false
        );
    }
}
