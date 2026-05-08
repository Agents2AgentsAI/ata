use codex_utils_absolute_path::AbsolutePathBuf;
use include_dir::Dir;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;

const SYSTEM_SKILLS_DIR: Dir = include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets/samples");

// ============================================================================
// Custom skill categories (ours, not upstream)
//
// To add a new skill category:
// 1. Create the directory under src/assets/<name>/SKILL.md
// 2. Add an include_dir!() const below
// 3. Add an entry to CUSTOM_SKILL_CATEGORIES
// 4. Add "src/assets/<name>" to build.rs
// That's it — no other Rust changes needed.
// ============================================================================

const RESEARCH_SKILLS_DIR: Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets/research");

const WORKSPACE_SKILLS_DIR: Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets/workspace");

const ADAPT_ENVIRONMENT_SKILLS_DIR: Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets/adapt-environment");

/// Remote-exec skill is private (ata-plus only) — the directory is stripped
/// on the public release branch, so gating the include_dir avoids a compile
/// error when the directory doesn't exist.
#[cfg(feature = "ata-plus")]
const REMOTE_EXEC_SKILLS_DIR: Dir =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/assets/remote-exec");

/// A custom skill category embedded at compile time.
struct CustomSkillCategory {
    /// The embedded directory contents.
    dir: &'static Dir<'static>,
    /// Subdirectory name under `CODEX_HOME/skills/` (e.g. ".system-adapt-environment").
    cache_dir_name: &'static str,
    /// Salt for fingerprinting (must be unique per category).
    fingerprint_salt: &'static str,
}

/// Registry of all custom (non-upstream) skill categories.
/// Adding a new category only requires adding an entry here
/// (plus the `include_dir!()` const and `build.rs` entry above).
const CUSTOM_SKILL_CATEGORIES_BASE: &[CustomSkillCategory] = &[
    CustomSkillCategory {
        dir: &RESEARCH_SKILLS_DIR,
        cache_dir_name: ".system-research",
        fingerprint_salt: "v1-research",
    },
    CustomSkillCategory {
        dir: &WORKSPACE_SKILLS_DIR,
        cache_dir_name: ".system-workspace",
        fingerprint_salt: "v1-workspace",
    },
    CustomSkillCategory {
        dir: &ADAPT_ENVIRONMENT_SKILLS_DIR,
        cache_dir_name: ".system-adapt-environment",
        fingerprint_salt: "v1-adapt-environment",
    },
];

#[cfg(feature = "ata-plus")]
const CUSTOM_SKILL_CATEGORIES_PLUS: &[CustomSkillCategory] = &[CustomSkillCategory {
    dir: &REMOTE_EXEC_SKILLS_DIR,
    cache_dir_name: ".system-remote-exec",
    fingerprint_salt: "v1-remote-exec",
}];

// ============================================================================
// Upstream skill category (system/samples only)
// ============================================================================

const SYSTEM_SKILLS_DIR_NAME: &str = ".system";
const SKILLS_DIR_NAME: &str = "skills";
const SYSTEM_SKILLS_MARKER_FILENAME: &str = ".codex-system-skills.marker";
const SYSTEM_SKILLS_MARKER_SALT: &str = "v1";

/// Returns the on-disk cache location for embedded system skills from an absolute CODEX_HOME.
pub fn system_cache_root_dir(codex_home: &AbsolutePathBuf) -> AbsolutePathBuf {
    codex_home
        .join(SKILLS_DIR_NAME)
        .join(SYSTEM_SKILLS_DIR_NAME)
}

/// Installs embedded system skills into `CODEX_HOME/skills/.system`.
///
/// Clears any existing system skills directory first and then writes the embedded
/// skills directory into place.
///
/// To avoid doing unnecessary work on every startup, a marker file is written
/// with a fingerprint of the embedded directory. When the marker matches, the
/// install is skipped.
pub fn install_system_skills(codex_home: &AbsolutePathBuf) -> Result<(), SystemSkillsError> {
    let skills_root_dir = codex_home.join(SKILLS_DIR_NAME);
    fs::create_dir_all(skills_root_dir.as_path())
        .map_err(|source| SystemSkillsError::io("create skills root dir", source))?;

    let dest_system = system_cache_root_dir(codex_home);

    let marker_path = dest_system.join(SYSTEM_SKILLS_MARKER_FILENAME);
    let expected_fingerprint = fingerprint_dir(&SYSTEM_SKILLS_DIR, SYSTEM_SKILLS_MARKER_SALT);
    if dest_system.as_path().is_dir()
        && read_marker(&marker_path).is_ok_and(|marker| marker == expected_fingerprint)
    {
        return Ok(());
    }

    if dest_system.as_path().exists() {
        fs::remove_dir_all(dest_system.as_path())
            .map_err(|source| SystemSkillsError::io("remove existing system skills dir", source))?;
    }

    write_embedded_dir(&SYSTEM_SKILLS_DIR, &dest_system)?;
    fs::write(marker_path.as_path(), format!("{expected_fingerprint}\n"))
        .map_err(|source| SystemSkillsError::io("write system skills marker", source))?;
    Ok(())
}

// ============================================================================
// Backward-compatible public functions for research/workspace
// These delegate to the custom category system but preserve the existing API
// so that upstream callers (system.rs, manager.rs, loader.rs) don't break.
// ============================================================================

/// Returns the on-disk cache location for embedded research skills.
pub fn research_cache_root_dir(codex_home: &Path) -> PathBuf {
    custom_category_cache_root_dir(codex_home, ".system-research")
}

/// Installs embedded research skills.
pub fn install_research_skills(codex_home: &Path) -> Result<(), SystemSkillsError> {
    install_skill_category(codex_home, &CUSTOM_SKILL_CATEGORIES_BASE[0])
}

/// Returns the on-disk cache location for embedded workspace skills.
pub fn workspace_cache_root_dir(codex_home: &Path) -> PathBuf {
    custom_category_cache_root_dir(codex_home, ".system-workspace")
}

/// Installs embedded workspace skills.
pub fn install_workspace_skills(codex_home: &Path) -> Result<(), SystemSkillsError> {
    install_skill_category(codex_home, &CUSTOM_SKILL_CATEGORIES_BASE[1])
}

// ============================================================================
// Custom skill categories — generic install and discovery
// ============================================================================

fn all_custom_categories() -> impl Iterator<Item = &'static CustomSkillCategory> {
    let base = CUSTOM_SKILL_CATEGORIES_BASE.iter();
    #[cfg(feature = "ata-plus")]
    let base = base.chain(CUSTOM_SKILL_CATEGORIES_PLUS.iter());
    base
}

/// Installs all custom (non-upstream) skill categories.
/// Called once at startup alongside the upstream install_system_skills().
pub fn install_custom_skills(codex_home: &Path) -> Result<(), SystemSkillsError> {
    for cat in all_custom_categories() {
        install_skill_category(codex_home, cat)?;
    }
    Ok(())
}

/// Returns cache root directories for all custom skill categories.
/// The loader adds these as skill roots so they get discovered automatically.
pub fn custom_skill_cache_root_dirs(codex_home: &Path) -> Vec<PathBuf> {
    all_custom_categories()
        .map(|cat| custom_category_cache_root_dir(codex_home, cat.cache_dir_name))
        .collect()
}

fn custom_category_cache_root_dir(codex_home: &Path, cache_dir_name: &str) -> PathBuf {
    AbsolutePathBuf::try_from(codex_home)
        .map(|ch| {
            ch.join(SKILLS_DIR_NAME)
                .join(cache_dir_name)
                .into_path_buf()
        })
        .unwrap_or_else(|_| codex_home.join(SKILLS_DIR_NAME).join(cache_dir_name))
}

fn install_skill_category(
    codex_home: &Path,
    cat: &CustomSkillCategory,
) -> Result<(), SystemSkillsError> {
    let codex_home = AbsolutePathBuf::try_from(codex_home)
        .map_err(|source| SystemSkillsError::io("normalize codex home dir", source))?;
    let skills_root_dir = codex_home.join(SKILLS_DIR_NAME);
    fs::create_dir_all(skills_root_dir.as_path())
        .map_err(|source| SystemSkillsError::io("create skills root dir", source))?;

    let dest = codex_home.join(SKILLS_DIR_NAME).join(cat.cache_dir_name);

    let marker_filename = format!(".codex-{}.marker", cat.cache_dir_name);
    let marker_path = dest.join(&marker_filename);
    let expected_fingerprint = fingerprint_dir(cat.dir, cat.fingerprint_salt);
    if dest.as_path().is_dir()
        && read_marker(&marker_path).is_ok_and(|marker| marker == expected_fingerprint)
    {
        return Ok(());
    }

    if dest.as_path().exists() {
        fs::remove_dir_all(dest.as_path())
            .map_err(|source| SystemSkillsError::io("remove existing custom skills dir", source))?;
    }

    write_embedded_dir(cat.dir, &dest)?;
    fs::write(marker_path.as_path(), format!("{expected_fingerprint}\n"))
        .map_err(|source| SystemSkillsError::io("write custom skills marker", source))?;
    Ok(())
}

// ============================================================================
// Shared internals
// ============================================================================

fn read_marker(path: &AbsolutePathBuf) -> Result<String, SystemSkillsError> {
    Ok(fs::read_to_string(path.as_path())
        .map_err(|source| SystemSkillsError::io("read system skills marker", source))?
        .trim()
        .to_string())
}

fn fingerprint_dir(dir: &Dir<'_>, salt: &str) -> String {
    let mut items = Vec::new();
    collect_fingerprint_items(dir, &mut items);
    items.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    let mut hasher = DefaultHasher::new();
    salt.hash(&mut hasher);
    for (path, contents_hash) in items {
        path.hash(&mut hasher);
        contents_hash.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn collect_fingerprint_items(dir: &Dir<'_>, items: &mut Vec<(String, Option<u64>)>) {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(subdir) => {
                items.push((subdir.path().to_string_lossy().to_string(), None));
                collect_fingerprint_items(subdir, items);
            }
            include_dir::DirEntry::File(file) => {
                let mut file_hasher = DefaultHasher::new();
                file.contents().hash(&mut file_hasher);
                items.push((
                    file.path().to_string_lossy().to_string(),
                    Some(file_hasher.finish()),
                ));
            }
        }
    }
}

/// Writes the embedded `include_dir::Dir` to disk under `dest`.
///
/// Preserves the embedded directory structure.
fn write_embedded_dir(dir: &Dir<'_>, dest: &AbsolutePathBuf) -> Result<(), SystemSkillsError> {
    fs::create_dir_all(dest.as_path())
        .map_err(|source| SystemSkillsError::io("create system skills dir", source))?;

    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(subdir) => {
                let subdir_dest = dest.join(subdir.path());
                fs::create_dir_all(subdir_dest.as_path()).map_err(|source| {
                    SystemSkillsError::io("create system skills subdir", source)
                })?;
                write_embedded_dir(subdir, dest)?;
            }
            include_dir::DirEntry::File(file) => {
                let path = dest.join(file.path());
                if let Some(parent) = path.as_path().parent() {
                    fs::create_dir_all(parent).map_err(|source| {
                        SystemSkillsError::io("create system skills file parent", source)
                    })?;
                }
                fs::write(path.as_path(), file.contents())
                    .map_err(|source| SystemSkillsError::io("write system skill file", source))?;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Error)]
pub enum SystemSkillsError {
    #[error("io error while {action}: {source}")]
    Io {
        action: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl SystemSkillsError {
    fn io(action: &'static str, source: std::io::Error) -> Self {
        Self::Io { action, source }
    }
}

#[cfg(test)]
mod tests {
    use super::SYSTEM_SKILLS_DIR;
    use super::collect_fingerprint_items;

    #[test]
    fn fingerprint_traverses_nested_entries() {
        let mut items = Vec::new();
        collect_fingerprint_items(&SYSTEM_SKILLS_DIR, &mut items);
        let mut paths: Vec<String> = items.into_iter().map(|(path, _)| path).collect();
        paths.sort_unstable();

        assert!(
            paths
                .binary_search_by(|probe| probe.as_str().cmp("skill-creator/SKILL.md"))
                .is_ok()
        );
        assert!(
            paths
                .binary_search_by(|probe| probe.as_str().cmp("skill-creator/scripts/init_skill.py"))
                .is_ok()
        );
    }

    #[test]
    fn workspace_fingerprint_traverses_nested_entries() {
        use super::WORKSPACE_SKILLS_DIR;

        let mut items = Vec::new();
        collect_fingerprint_items(&WORKSPACE_SKILLS_DIR, &mut items);
        let mut paths: Vec<String> = items.into_iter().map(|(path, _)| path).collect();
        paths.sort_unstable();

        assert!(
            paths
                .binary_search_by(|probe| probe.as_str().cmp("SKILL.md"))
                .is_ok()
        );
        assert!(
            paths
                .binary_search_by(|probe| {
                    probe
                        .as_str()
                        .cmp("scripts/references/workspace-management.md")
                })
                .is_ok()
        );
    }

    #[test]
    fn research_fingerprint_traverses_nested_entries() {
        use super::RESEARCH_SKILLS_DIR;

        let mut items = Vec::new();
        collect_fingerprint_items(&RESEARCH_SKILLS_DIR, &mut items);
        let mut paths: Vec<String> = items.into_iter().map(|(path, _)| path).collect();
        paths.sort_unstable();

        assert!(
            paths
                .binary_search_by(|probe| probe.as_str().cmp("cross-paper-report/SKILL.md"))
                .is_ok()
        );
    }

    #[test]
    fn custom_skill_categories_install_and_discover() {
        let codex_home = tempfile::tempdir().expect("tempdir");
        super::install_custom_skills(codex_home.path()).expect("install custom skills");

        let dirs = super::custom_skill_cache_root_dirs(codex_home.path());
        assert!(
            dirs.len() >= 3,
            "should have at least 3 custom categories (research, workspace, adapt-environment)"
        );
        for dir in &dirs {
            assert!(dir.is_dir(), "custom skill cache dir should exist: {dir:?}");
        }
    }
}
