use crate::error::WorkspaceError;
use sha2::Digest;
use sha2::Sha256;

const MAX_WORKSPACE_ID_LEN: usize = 96;

/// Slugify a workspace name: lowercase alphanumeric, hyphens only.
pub fn slugify_name(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let segments: Vec<&str> = raw.split('-').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        "workspace".to_string()
    } else {
        segments.join("-")
    }
}

/// Compute a short hash from name + current timestamp.
pub fn short_hash(name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let ts = chrono::Utc::now().timestamp().to_string();
    hasher.update(ts.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..4]) // 8 hex chars
}

/// Generate a workspace ID from a human-readable name.
pub fn workspace_id_from_name(name: &str, attempt: u32) -> String {
    let hash_val = short_hash(name);
    let suffix = if attempt == 0 {
        format!("-{hash_val}")
    } else {
        format!("-{hash_val}-{attempt}")
    };
    let max_slug = MAX_WORKSPACE_ID_LEN - suffix.len();
    let mut slug = slugify_name(name);
    if slug.len() > max_slug {
        slug.truncate(max_slug);
        slug = slug.trim_end_matches('-').to_string();
    }
    if slug.is_empty() {
        slug = "workspace".to_string();
    }
    format!("{slug}{suffix}")
}

/// Validate a workspace ID string.
pub fn validate_workspace_id(workspace_id: &str) -> Result<(), WorkspaceError> {
    if workspace_id.is_empty() {
        return Err(WorkspaceError::EmptyId);
    }
    if workspace_id == "." || workspace_id == ".." {
        return Err(WorkspaceError::DotId);
    }
    if workspace_id.len() > MAX_WORKSPACE_ID_LEN {
        return Err(WorkspaceError::IdTooLong {
            max: MAX_WORKSPACE_ID_LEN,
        });
    }
    let first = workspace_id.as_bytes()[0];
    let last = workspace_id.as_bytes()[workspace_id.len() - 1];
    if first == b'-' || first == b'_' || last == b'-' || last == b'_' {
        return Err(WorkspaceError::IdBadEdge {
            id: workspace_id.to_string(),
        });
    }
    if !workspace_id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(WorkspaceError::IdBadChars {
            id: workspace_id.to_string(),
        });
    }
    Ok(())
}

/// Hex-encode helper (avoids adding the `hex` crate).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

/// Generate a prefixed ID: `{prefix}-{unix_ts}-{uuid_hex}`.
pub fn make_id(prefix: &str) -> String {
    let ts = chrono::Utc::now().timestamp();
    let uid = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{ts}-{uid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify_name("Hello World!"), "hello-world");
        assert_eq!(slugify_name("---"), "workspace");
        assert_eq!(slugify_name("My Project 2024"), "my-project-2024");
    }

    #[test]
    fn test_validate_workspace_id() {
        assert!(validate_workspace_id("foo-bar-123").is_ok());
        assert!(validate_workspace_id("").is_err());
        assert!(validate_workspace_id(".").is_err());
        assert!(validate_workspace_id("-foo").is_err());
        assert!(validate_workspace_id("foo-").is_err());
        assert!(validate_workspace_id("Foo").is_err());
    }

    #[test]
    fn test_make_id() {
        let id = make_id("repo");
        assert!(id.starts_with("repo-"));
        // prefix-timestamp-uuid = at least 40 chars
        assert!(id.len() > 40);
    }
}
