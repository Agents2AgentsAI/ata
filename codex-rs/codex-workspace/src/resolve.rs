use crate::error::WorkspaceError;
use crate::paths::workspace_root;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

static RESERVED_ALIASES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["run", "notes", "ws", "artifacts", "kb", "index", "cache"]
        .into_iter()
        .collect()
});

static NOTES_CATEGORIES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "workspace",
        "repos",
        "papers",
        "datasets",
        "runs",
        "artifacts",
        "indexes",
    ]
    .into_iter()
    .collect()
});

/// Check if an alias is reserved.
pub fn is_reserved_alias(alias: &str) -> bool {
    RESERVED_ALIASES.contains(alias)
}

/// Validate a path suffix: must be relative, no `..` components.
pub fn validate_path_suffix(suffix: &str) -> Result<(), WorkspaceError> {
    if suffix.is_empty() {
        return Ok(());
    }
    if Path::new(suffix).is_absolute() {
        return Err(WorkspaceError::AbsolutePathSuffix(suffix.to_string()));
    }
    for component in suffix.replace('\\', "/").split('/') {
        if component == ".." {
            return Err(WorkspaceError::PathTraversal(suffix.to_string()));
        }
    }
    Ok(())
}

/// Join root + suffix safely, verifying result stays under root.
pub fn safe_join(root: &Path, suffix: &str) -> Result<PathBuf, WorkspaceError> {
    validate_path_suffix(suffix)?;
    if suffix.is_empty() {
        return Ok(root.to_path_buf());
    }
    let joined = root.join(suffix);
    // If root exists, verify with realpath
    if root.exists() {
        let real_root = std::fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .to_string_lossy()
            .to_string();
        let real_joined = std::fs::canonicalize(&joined)
            .unwrap_or_else(|_| joined.clone())
            .to_string_lossy()
            .to_string();
        let root_prefix = format!("{real_root}/");
        if real_joined != real_root && !real_joined.starts_with(&root_prefix) {
            return Err(WorkspaceError::PathEscape(suffix.to_string()));
        }
    }
    Ok(joined)
}

/// Resolve an `@`-path spec to an absolute path within a workspace.
pub fn resolve_at_spec(workspace_id: &str, spec: &str) -> Result<PathBuf, WorkspaceError> {
    if !spec.starts_with('@') {
        return Err(WorkspaceError::SpecMissingAt);
    }

    let root = workspace_root(workspace_id);
    let rest = &spec[1..]; // strip leading @

    // @ws/<other_workspace_id>/path...
    if let Some(after_ws) = rest.strip_prefix("ws/") {
        let parts: Vec<&str> = after_ws.splitn(2, '/').collect();
        if parts.is_empty() || parts[0].is_empty() {
            return Err(WorkspaceError::WsIdRequired);
        }
        let target_wid = parts[0];
        let suffix = if parts.len() > 1 { parts[1] } else { "" };
        return safe_join(&workspace_root(target_wid), suffix);
    }

    // @run/<run_id>/path...
    if let Some(after_run) = rest.strip_prefix("run/") {
        let parts: Vec<&str> = after_run.splitn(2, '/').collect();
        if parts.is_empty() || parts[0].is_empty() {
            return Err(WorkspaceError::RunIdRequired);
        }
        let run_id = parts[0];
        let suffix = if parts.len() > 1 { parts[1] } else { "" };
        let run_root = root.join("runs").join(run_id);
        return safe_join(&run_root, suffix);
    }

    // @notes[/path...]
    if rest == "notes" || rest.starts_with("notes/") {
        let suffix = rest
            .strip_prefix("notes")
            .unwrap_or("")
            .trim_start_matches('/');
        if suffix.is_empty() {
            return Ok(root.join("notes").join("workspace"));
        }
        let first = suffix.split('/').next().unwrap_or("");
        if NOTES_CATEGORIES.contains(first) {
            return safe_join(&root.join("notes"), suffix);
        }
        return safe_join(&root.join("notes").join("workspace"), suffix);
    }

    // @kb[/path...]
    if rest == "kb" || rest.starts_with("kb/") {
        let suffix = rest
            .strip_prefix("kb")
            .unwrap_or("")
            .trim_start_matches('/');
        return safe_join(&root.join("knowledge-base"), suffix);
    }

    // @cache[/path...]
    if rest == "cache" || rest.starts_with("cache/") {
        let suffix = rest
            .strip_prefix("cache")
            .unwrap_or("")
            .trim_start_matches('/');
        return safe_join(&root.join("cache"), suffix);
    }

    // @artifacts/<id>/path...
    if let Some(suffix) = rest.strip_prefix("artifacts/") {
        return safe_join(&root.join("artifacts"), suffix);
    }

    // @index/<id>/path...
    if let Some(suffix) = rest.strip_prefix("index/") {
        return safe_join(&root.join("indexes"), suffix);
    }

    // @<alias>/path... — repo alias
    let parts: Vec<&str> = rest.splitn(2, '/').collect();
    let alias = parts[0];
    if RESERVED_ALIASES.contains(alias) {
        return Err(WorkspaceError::ReservedAlias(alias.to_string()));
    }
    let suffix = if parts.len() > 1 { parts[1] } else { "" };
    safe_join(&root.join("repos").join(alias), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_suffix() {
        assert!(validate_path_suffix("").is_ok());
        assert!(validate_path_suffix("foo/bar").is_ok());
        assert!(validate_path_suffix("/absolute").is_err());
        assert!(validate_path_suffix("foo/../bar").is_err());
    }

    #[test]
    fn test_is_reserved_alias() {
        assert!(is_reserved_alias("run"));
        assert!(is_reserved_alias("kb"));
        assert!(!is_reserved_alias("my-repo"));
    }

    #[test]
    fn test_resolve_repo_aliases_that_start_with_notes() {
        let workspace_id = "workspace-1";
        let resolved = resolve_at_spec(workspace_id, "@notebook/docs").unwrap();
        assert_eq!(
            resolved,
            workspace_root(workspace_id)
                .join("repos")
                .join("notebook")
                .join("docs")
        );
    }
}
