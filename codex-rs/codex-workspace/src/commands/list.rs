use crate::error::WorkspaceError;
use crate::paths;
use crate::types::WorkspaceSummary;
use std::path::Path;

/// List all workspaces as summaries.
pub fn run() -> Result<Vec<WorkspaceSummary>, WorkspaceError> {
    run_for(&paths::codex_home())
}

fn run_for(codex_home: &Path) -> Result<Vec<WorkspaceSummary>, WorkspaceError> {
    let root = paths::ensure_workspaces_root_for(codex_home)?;

    let mut results = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&root)?
        .filter_map(std::result::Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let mp = entry.path().join("workspace.json");
        if !mp.is_file() {
            continue;
        }
        let data = match std::fs::read_to_string(&mp) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let manifest: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let entry_name = entry.file_name();
        let fallback = entry_name.to_str().unwrap_or("");
        let id = manifest
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_string();
        let name = manifest
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let updated_at = manifest
            .get("updatedAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let repo_count = manifest
            .get("repos")
            .and_then(|v| v.as_array())
            .map(Vec::len)
            .unwrap_or(0);

        results.push(WorkspaceSummary {
            id,
            name,
            updated_at,
            repo_count,
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::write_manifest_atomic_for;
    use crate::types::new_manifest;
    use pretty_assertions::assert_eq;

    #[test]
    fn run_lists_valid_workspaces_in_sorted_order() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let mut zebra = new_manifest("zebra", "Zebra");
        zebra.updated_at = 20;
        zebra.repos.push(crate::types::RepoEntry {
            id: "repo-1".to_string(),
            alias: "repo".to_string(),
            repo_key: "org/repo".to_string(),
            remote_url: "https://github.com/org/repo.git".to_string(),
            checkout_path: "repos/repo".to_string(),
            notes_path: "notes/repos/repo".to_string(),
            clone: crate::types::CloneRecord {
                depth: 1,
                single_branch: true,
                no_tags: true,
                filter: "blob:limit=1m".to_string(),
                submodules: crate::types::SubmodulePolicy::None,
                lfs: crate::types::LfsPolicy::Auto,
                extra: serde_json::Map::new(),
            },
            pin: crate::types::PinState {
                mode: crate::types::PinMode::Tracking,
                pinned_sha: String::new(),
                extra: serde_json::Map::new(),
            },
            state: crate::types::GitState {
                head_sha: String::new(),
                head_ref: String::new(),
                default_branch: String::new(),
                shallow: false,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
        });
        let alpha = new_manifest("alpha", "Alpha");

        write_manifest_atomic_for(temp.path(), "zebra", &zebra).expect("write zebra manifest");
        write_manifest_atomic_for(temp.path(), "alpha", &alpha).expect("write alpha manifest");
        std::fs::create_dir_all(paths::workspace_root_for(temp.path(), "broken"))
            .expect("create broken dir");
        std::fs::write(paths::manifest_path_for(temp.path(), "broken"), "{not-json")
            .expect("write invalid manifest");

        let summaries = run_for(temp.path()).expect("list workspaces");

        assert_eq!(
            summaries,
            vec![
                WorkspaceSummary {
                    id: "alpha".to_string(),
                    name: "Alpha".to_string(),
                    updated_at: alpha.updated_at,
                    repo_count: 0,
                },
                WorkspaceSummary {
                    id: "zebra".to_string(),
                    name: "Zebra".to_string(),
                    updated_at: 20,
                    repo_count: 1,
                },
            ]
        );
    }
}
