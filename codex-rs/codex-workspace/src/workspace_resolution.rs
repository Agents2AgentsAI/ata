use crate::paths;
use crate::selection;
use crate::types::new_manifest;

/// 4-tier workspace resolution: explicit > project pin > session > global.
///
/// If `explicit` is `Some`, returns that immediately.
/// Otherwise walks: project pin → session selection → global fallback.
pub fn resolve_workspace(explicit: Option<&str>) -> String {
    // 1. Explicit --workspace flag
    if let Some(wid) = explicit
        && !wid.is_empty()
    {
        return wid.to_string();
    }

    // 2. Project pin (.codex/workspace.json from cwd upward)
    if let Ok(cwd) = std::env::current_dir()
        && let Some(wid) = selection::discover_project_pin(&cwd)
    {
        return wid;
    }

    // 3. Session selection
    if let Some(wid) = selection::read_session_workspace() {
        return wid;
    }

    // 4. Global fallback — ensure it exists
    ensure_global_workspace();
    "global".to_string()
}

/// Create the global workspace if it doesn't exist yet.
fn ensure_global_workspace() {
    let wid = "global";
    if paths::manifest_path(wid).is_file() {
        return;
    }
    let root = paths::workspace_root(wid);
    for sub in paths::init_dirs() {
        let dir = if sub.is_empty() {
            root.clone()
        } else {
            root.join(sub)
        };
        let _ = std::fs::create_dir_all(dir);
    }
    let manifest = new_manifest(wid, "global");
    let _ = crate::manifest::write_manifest_atomic(wid, &manifest);
}
