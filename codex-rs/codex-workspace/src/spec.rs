use crate::error::WorkspaceError;
use crate::git;
use crate::types::Policies;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use std::path::Path;

/// Portable workspace specification file format.
///
/// Declares the desired contents of a workspace in a human-readable,
/// Git-friendly JSON file. Lives checked into a pipeline/project repo
/// and can be materialized into a local workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSpec {
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub repos: Vec<RepoSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policies: Option<Policies>,
    #[serde(default)]
    pub labels: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Repository entry in a workspace spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoSpec {
    pub url: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    #[serde(default)]
    pub full: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Read and parse a workspace spec from a file path.
pub fn read_spec(path: &Path) -> Result<WorkspaceSpec, WorkspaceError> {
    if !path.is_file() {
        return Err(WorkspaceError::SpecNotFound(path.to_path_buf()));
    }
    let data = std::fs::read_to_string(path)?;
    let spec: WorkspaceSpec =
        serde_json::from_str(&data).map_err(|e| WorkspaceError::InvalidSpec(e.to_string()))?;
    validate_spec(&spec)?;
    Ok(spec)
}

/// Validate a parsed spec for structural correctness.
pub fn validate_spec(spec: &WorkspaceSpec) -> Result<(), WorkspaceError> {
    if spec.schema_version != 1 {
        return Err(WorkspaceError::InvalidSpec(format!(
            "unsupported schemaVersion: {} (expected 1)",
            spec.schema_version
        )));
    }
    if spec.name.is_empty() {
        return Err(WorkspaceError::InvalidSpec(
            "name must not be empty".to_string(),
        ));
    }

    let mut seen_aliases = std::collections::HashSet::new();
    for repo in &spec.repos {
        if repo.url.is_empty() {
            return Err(WorkspaceError::InvalidSpec(format!(
                "repo '{}': url must not be empty",
                repo.alias
            )));
        }
        if repo.alias.is_empty() {
            return Err(WorkspaceError::InvalidSpec(
                "repo alias must not be empty".to_string(),
            ));
        }
        if !seen_aliases.insert(&repo.alias) {
            return Err(WorkspaceError::InvalidSpec(format!(
                "duplicate alias: '{}'",
                repo.alias
            )));
        }
        if let Some(sha) = &repo.sha
            && !git::is_valid_commit_sha(sha)
        {
            return Err(WorkspaceError::InvalidSpec(format!(
                "repo '{}': invalid sha '{}'",
                repo.alias, sha
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_spec() {
        let json = r#"{
            "schemaVersion": 1,
            "name": "test-pipeline",
            "repos": [
                {"url": "https://github.com/org/repo.git", "alias": "repo"}
            ]
        }"#;
        let spec: WorkspaceSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.schema_version, 1);
        assert_eq!(spec.name, "test-pipeline");
        assert_eq!(spec.repos.len(), 1);
        assert_eq!(spec.repos[0].alias, "repo");
        assert!(spec.repos[0].sha.is_none());
        assert!(spec.repos[0].r#ref.is_none());
        assert!(!spec.repos[0].full);
    }

    #[test]
    fn parse_full_spec() {
        let json = r#"{
            "schemaVersion": 1,
            "name": "3d-recon-pipeline",
            "repos": [
                {
                    "url": "https://github.com/colmap/colmap.git",
                    "alias": "colmap",
                    "sha": "abc123",
                    "ref": "main",
                    "full": true,
                    "role": "sfm"
                }
            ],
            "policies": {
                "defaultClone": {"depth": 1, "singleBranch": true}
            },
            "labels": {"env": "prod"},
            "metadata": {"pipeline_version": "2.0"}
        }"#;
        let spec: WorkspaceSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.repos[0].sha.as_deref(), Some("abc123"));
        assert_eq!(spec.repos[0].r#ref.as_deref(), Some("main"));
        assert!(spec.repos[0].full);
        // "role" lands in extra via flatten
        assert_eq!(
            spec.repos[0].extra.get("role").and_then(|v| v.as_str()),
            Some("sfm")
        );
        assert!(spec.policies.is_some());
        assert_eq!(
            spec.labels.get("env").and_then(|v| v.as_str()),
            Some("prod")
        );
        // "metadata" lands in top-level extra via flatten
        assert!(spec.extra.contains_key("metadata"));
    }

    #[test]
    fn validate_rejects_bad_version() {
        let spec = WorkspaceSpec {
            schema_version: 99,
            name: "test".to_string(),
            repos: vec![],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let spec = WorkspaceSpec {
            schema_version: 1,
            name: String::new(),
            repos: vec![],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_alias() {
        let spec = WorkspaceSpec {
            schema_version: 1,
            name: "test".to_string(),
            repos: vec![
                RepoSpec {
                    url: "https://github.com/a/b.git".to_string(),
                    alias: "dup".to_string(),
                    sha: None,
                    r#ref: None,
                    full: false,
                    extra: Map::new(),
                },
                RepoSpec {
                    url: "https://github.com/c/d.git".to_string(),
                    alias: "dup".to_string(),
                    sha: None,
                    r#ref: None,
                    full: false,
                    extra: Map::new(),
                },
            ],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn validate_accepts_valid_spec() {
        let spec = WorkspaceSpec {
            schema_version: 1,
            name: "test".to_string(),
            repos: vec![RepoSpec {
                url: "https://github.com/a/b.git".to_string(),
                alias: "repo-a".to_string(),
                sha: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                r#ref: None,
                full: false,
                extra: Map::new(),
            }],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };
        assert!(validate_spec(&spec).is_ok());
    }

    #[test]
    fn roundtrip_serialize() {
        let spec = WorkspaceSpec {
            schema_version: 1,
            name: "roundtrip-test".to_string(),
            repos: vec![RepoSpec {
                url: "https://github.com/a/b.git".to_string(),
                alias: "myrepo".to_string(),
                sha: Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()),
                r#ref: Some("main".to_string()),
                full: false,
                extra: Map::new(),
            }],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };
        let json = serde_json::to_string_pretty(&spec).unwrap();
        let parsed: WorkspaceSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, spec.name);
        assert_eq!(parsed.repos.len(), 1);
        assert_eq!(
            parsed.repos[0].sha.as_deref(),
            Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
        );
    }

    #[test]
    fn validate_rejects_invalid_sha() {
        let spec = WorkspaceSpec {
            schema_version: 1,
            name: "test".to_string(),
            repos: vec![RepoSpec {
                url: "https://github.com/a/b.git".to_string(),
                alias: "repo-a".to_string(),
                sha: Some("abc123".to_string()),
                r#ref: None,
                full: false,
                extra: Map::new(),
            }],
            policies: None,
            labels: Map::new(),
            extra: Map::new(),
        };

        assert!(validate_spec(&spec).is_err());
    }
}
