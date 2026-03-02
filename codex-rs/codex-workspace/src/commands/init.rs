use crate::error::WorkspaceError;
use crate::manifest::write_manifest_atomic;
use crate::paths;
use crate::types::new_manifest;
use crate::workspace_id::validate_workspace_id;
use crate::workspace_id::workspace_id_from_name;

/// Create a new workspace, returns the workspace ID.
pub fn run(name: &str) -> Result<String, WorkspaceError> {
    let mut attempt = 0u32;
    let workspace_id = loop {
        let wid = workspace_id_from_name(name, attempt);
        validate_workspace_id(&wid)?;
        let root = paths::workspace_root(&wid);
        if !root.exists() {
            break wid;
        }
        attempt += 1;
    };

    let root = paths::workspace_root(&workspace_id);
    for sub in paths::init_dirs() {
        let dir = if sub.is_empty() {
            root.clone()
        } else {
            root.join(sub)
        };
        std::fs::create_dir_all(dir)?;
    }

    write_manifest_atomic(&workspace_id, &new_manifest(&workspace_id, name))?;
    Ok(workspace_id)
}
