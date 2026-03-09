use crate::audit::write_audit;
use crate::error::WorkspaceError;
use crate::manifest::atomic_write;
use crate::manifest::read_manifest;
use crate::manifest::with_locked_manifest;
use crate::paths;
use crate::types::RunEntry;
use crate::types::RunSource;
use crate::types::RunStatus;
use crate::workspace_id::make_id;
use serde_json::Map;
use serde_json::json;

/// Full run creation: create dirs → materialize code → register → audit.
pub fn run(
    workspace_id: &str,
    run_name: &str,
    source_alias: &str,
    strategy: &str,
) -> Result<serde_json::Value, WorkspaceError> {
    if !["worktree", "copy", "clone"].contains(&strategy) {
        return Err(WorkspaceError::InvalidStrategy(strategy.to_string()));
    }

    // 1. Verify source repo exists
    let manifest = read_manifest(workspace_id)?;
    let _source_repo = manifest
        .repos
        .iter()
        .find(|r| r.alias == source_alias)
        .ok_or_else(|| WorkspaceError::SourceRepoNotFound(source_alias.to_string()))?;

    let version = manifest.manifest_version;
    let root = paths::workspace_root(workspace_id);
    let repo_path = root.join("repos").join(source_alias);

    // 2. Create run structure
    let run_id = make_id("run");
    let run_root = root.join("runs").join(&run_id);
    for sub in ["logs", "outputs", "tmp", "env"] {
        std::fs::create_dir_all(run_root.join(sub))?;
    }

    // 3. Materialize code
    let code_root = run_root.join("root");
    let mut actual_strategy = strategy.to_string();

    match strategy {
        "worktree" => {
            let ok = crate::git::worktree_add(&repo_path, &code_root, "HEAD")?;
            if !ok {
                // Fall back to copy
                copy_dir_recursive(&repo_path, &code_root)?;
                actual_strategy = "copy".to_string();
            }
        }
        "clone" => {
            let exit_code =
                crate::git::clone_repo(repo_path.to_str().unwrap_or("."), &code_root, &[])?;
            if exit_code != 0 {
                let _ = std::fs::remove_dir_all(&run_root);
                return Err(WorkspaceError::GitCloneFailed(exit_code));
            }
        }
        _ => {
            // copy
            copy_dir_recursive(&repo_path, &code_root)?;
        }
    }

    // 4. Read HEAD SHA from materialized code
    let head_sha = std::process::Command::new("git")
        .args(["-C", code_root.to_str().unwrap_or("."), "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // 5. Write run.json
    let now = chrono::Utc::now().timestamp();
    let run_manifest = json!({
        "schemaVersion": 2,
        "id": &run_id,
        "name": run_name,
        "createdAt": now,
        "updatedAt": now,
        "status": RunStatus::Created,
        "source": {"repoAlias": source_alias, "sha": &head_sha},
        "strategy": &actual_strategy,
        "commands": [],
        "env": {},
    });
    let run_json_path = run_root.join("run.json");
    atomic_write(
        &run_json_path,
        serde_json::to_string_pretty(&run_manifest)?.as_bytes(),
    )?;

    // 6. Register in workspace manifest
    let run_entry = RunEntry {
        id: run_id.clone(),
        name: run_name.to_string(),
        created_at: now,
        updated_at: now,
        root_path: format!("runs/{run_id}"),
        status: RunStatus::Created,
        source: RunSource {
            repo_alias: source_alias.to_string(),
            sha: head_sha.clone(),
            extra: Map::new(),
        },
        extra: Map::new(),
    };

    let result = with_locked_manifest(workspace_id, Some(version), move |m| {
        m.runs.push(run_entry);
        Ok(())
    });

    match result {
        Err(e @ WorkspaceError::VersionConflict { .. }) => {
            let _ = std::fs::remove_dir_all(&run_root);
            return Err(e);
        }
        other => other?,
    };

    // 7. Audit
    write_audit(
        workspace_id,
        "run_create",
        vec![json!({"type": "run", "id": &run_id})],
        None,
    )?;

    // 8. Output result
    Ok(json!({
        "runId": &run_id,
        "name": run_name,
        "rootPath": format!("runs/{run_id}"),
        "codePath": format!("runs/{run_id}/root"),
        "strategy": actual_strategy,
        "source": {"repoAlias": source_alias, "sha": &head_sha},
    }))
}

/// Simple recursive directory copy (like Python's shutil.copytree).
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), WorkspaceError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());
        if ty.is_symlink() {
            copy_symlink(&path, &dest_path)?;
        } else if ty.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            std::fs::copy(path, dest_path)?;
        }
    }
    Ok(())
}

fn copy_symlink(src: &std::path::Path, dst: &std::path::Path) -> Result<(), WorkspaceError> {
    let target = std::fs::read_link(src)?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, dst)?;
    }
    #[cfg(windows)]
    {
        if std::fs::metadata(src)
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            std::os::windows::fs::symlink_dir(&target, dst)?;
        } else {
            std::os::windows::fs::symlink_file(&target, dst)?;
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symlink copying is not supported on this platform",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    #[test]
    fn copy_dir_recursive_preserves_symlinks() {
        let src = TempDir::new().expect("create source temp dir");
        let dst = TempDir::new().expect("create destination temp dir");
        std::fs::create_dir_all(src.path().join("dir")).expect("create source dir");
        std::fs::write(src.path().join("dir").join("file.txt"), "hello")
            .expect("write source file");
        std::os::unix::fs::symlink("dir", src.path().join("dir-link"))
            .expect("create source symlink");

        copy_dir_recursive(src.path(), dst.path()).expect("copy directory tree");

        let copied = dst.path().join("dir-link");
        assert!(
            std::fs::symlink_metadata(&copied)
                .expect("read copied metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(copied).expect("read copied symlink"),
            std::path::PathBuf::from("dir")
        );
    }
}
