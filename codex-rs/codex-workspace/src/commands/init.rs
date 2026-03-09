use crate::error::WorkspaceError;
use crate::manifest::write_manifest_atomic;
use crate::paths;
use crate::types::new_manifest;
use crate::workspace_id::short_hash;
use crate::workspace_id::validate_workspace_id;
use crate::workspace_id::workspace_id_from_name_with_hash;

/// Create a new workspace, returns the workspace ID.
pub fn run(name: &str) -> Result<String, WorkspaceError> {
    let workspaces_root = paths::ensure_workspaces_root()?;
    let hash = short_hash(name);
    let mut attempt = 0u32;
    let workspace_id = loop {
        let wid = workspace_id_from_name_with_hash(name, &hash, attempt);
        validate_workspace_id(&wid)?;
        match std::fs::create_dir(workspaces_root.join(&wid)) {
            Ok(()) => break wid,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                attempt += 1;
            }
            Err(err) => return Err(err.into()),
        }
    };

    let root = paths::workspace_root(&workspace_id);
    for sub in paths::init_dirs().into_iter().filter(|sub| !sub.is_empty()) {
        std::fs::create_dir_all(root.join(sub))?;
    }

    write_manifest_atomic(&workspace_id, &new_manifest(&workspace_id, name))?;
    Ok(workspace_id)
}
