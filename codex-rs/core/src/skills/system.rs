pub(crate) use codex_skills::custom_skill_cache_root_dirs;
pub(crate) use codex_skills::install_custom_skills;
pub(crate) use codex_skills::install_system_skills;
pub(crate) use codex_skills::system_cache_root_dir;

use std::path::Path;

pub(crate) fn uninstall_system_skills(codex_home: &Path) {
    let system_skills_dir = system_cache_root_dir(codex_home);
    let _ = std::fs::remove_dir_all(&system_skills_dir);
}
