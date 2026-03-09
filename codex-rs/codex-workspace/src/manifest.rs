use crate::error::WorkspaceError;
use crate::lock::FileLock;
use crate::paths;
use crate::types::WorkspaceManifest;
use std::path::Path;

/// Read a workspace manifest from disk.
pub fn read_manifest(workspace_id: &str) -> Result<WorkspaceManifest, WorkspaceError> {
    read_manifest_path(&paths::manifest_path(workspace_id))
}

/// Read a workspace manifest from disk under an explicit Codex home.
pub fn read_manifest_for(
    codex_home: &Path,
    workspace_id: &str,
) -> Result<WorkspaceManifest, WorkspaceError> {
    read_manifest_path(&paths::manifest_path_for(codex_home, workspace_id))
}

/// Write a manifest atomically (write to temp, fsync, rename, fsync dir).
pub fn write_manifest_atomic(
    workspace_id: &str,
    manifest: &WorkspaceManifest,
) -> Result<(), WorkspaceError> {
    write_manifest_atomic_for(&paths::codex_home(), workspace_id, manifest)
}

/// Write a manifest atomically under an explicit Codex home.
pub fn write_manifest_atomic_for(
    codex_home: &Path,
    workspace_id: &str,
    manifest: &WorkspaceManifest,
) -> Result<(), WorkspaceError> {
    let mp = paths::manifest_path_for(codex_home, workspace_id);
    let data = serde_json::to_string_pretty(manifest)?;
    atomic_write(&mp, data.as_bytes())
}

/// Bump manifest version and update timestamp.
pub fn bump_version(manifest: &mut WorkspaceManifest) {
    manifest.manifest_version += 1;
    manifest.updated_at = chrono::Utc::now().timestamp();
}

/// Atomic file write: temp file → fsync → rename → fsync parent.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), WorkspaceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        use std::io::Write;
        tmp.write_all(data)?;
        tmp.as_file().sync_all()?;
    }
    tmp.persist(path)
        .map_err(|e| std::io::Error::other(format!("persist failed: {e}")))?;
    // fsync parent directory
    #[cfg(unix)]
    {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Acquire lock, re-read manifest, apply closure, bump version, write, release lock.
///
/// Returns the updated manifest. If `expect_version` is set, checks for conflict.
pub fn with_locked_manifest<F>(
    workspace_id: &str,
    expect_version: Option<u64>,
    mutate_fn: F,
) -> Result<WorkspaceManifest, WorkspaceError>
where
    F: FnOnce(&mut WorkspaceManifest) -> Result<(), WorkspaceError>,
{
    let lock_file = paths::lock_path(workspace_id);
    let _lock = FileLock::acquire(&lock_file)?;

    let mut manifest = read_manifest(workspace_id)?;

    if let Some(expected) = expect_version
        && manifest.manifest_version != expected
    {
        return Err(WorkspaceError::VersionConflict {
            expected,
            actual: manifest.manifest_version,
        });
    }

    mutate_fn(&mut manifest)?;
    bump_version(&mut manifest);
    write_manifest_atomic(workspace_id, &manifest)?;

    Ok(manifest)
}

fn read_manifest_path(path: &Path) -> Result<WorkspaceManifest, WorkspaceError> {
    if !path.is_file() {
        return Err(WorkspaceError::ManifestNotFound(path.to_path_buf()));
    }
    let data = std::fs::read_to_string(path)?;
    let manifest: WorkspaceManifest = serde_json::from_str(&data)?;
    Ok(manifest)
}
