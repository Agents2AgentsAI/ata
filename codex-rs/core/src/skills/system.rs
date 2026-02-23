pub(crate) use codex_skills::install_research_skills;
pub(crate) use codex_skills::install_system_skills;
pub(crate) use codex_skills::research_cache_root_dir;
pub(crate) use codex_skills::system_cache_root_dir;

use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::Path;
use std::path::PathBuf;

const VISUAL_REPORTS_SKILLS_DIR_NAME: &str = ".system-visual-reports";
const SKILLS_DIR_NAME: &str = "skills";

/// Returns the on-disk cache location for embedded visual-reports skills.
///
/// This is typically located at `CODEX_HOME/skills/.system-visual-reports`.
/// Always available so that the loader can add the path to skill roots
/// unconditionally (non-existent directories are skipped at discovery time).
pub(crate) fn visual_reports_cache_root_dir(codex_home: &Path) -> PathBuf {
    AbsolutePathBuf::try_from(codex_home)
        .and_then(|codex_home| visual_reports_cache_root_dir_abs(&codex_home))
        .map(AbsolutePathBuf::into_path_buf)
        .unwrap_or_else(|_| {
            codex_home
                .join(SKILLS_DIR_NAME)
                .join(VISUAL_REPORTS_SKILLS_DIR_NAME)
        })
}

fn visual_reports_cache_root_dir_abs(
    codex_home: &AbsolutePathBuf,
) -> std::io::Result<AbsolutePathBuf> {
    codex_home
        .join(SKILLS_DIR_NAME)?
        .join(VISUAL_REPORTS_SKILLS_DIR_NAME)
}

#[cfg(feature = "visual-reports")]
mod visual_reports_impl {
    use super::*;
    use codex_skills::SystemSkillsError;
    use include_dir::Dir;
    use std::collections::hash_map::DefaultHasher;
    use std::fs;
    use std::hash::Hash;
    use std::hash::Hasher;

    const VISUAL_REPORTS_SKILLS_DIR: Dir =
        include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/skills/assets/visual-reports");
    const VISUAL_REPORTS_SKILLS_MARKER_FILENAME: &str = ".codex-visual-reports-skills.marker";
    const VISUAL_REPORTS_SKILLS_MARKER_SALT: &str = "v1";

    fn skills_io_error(action: &'static str, source: std::io::Error) -> SystemSkillsError {
        SystemSkillsError::Io { action, source }
    }

    /// Installs embedded visual-reports skills into `CODEX_HOME/skills/.system-visual-reports`.
    ///
    /// Same caching strategy as [`install_system_skills`] but for visual-reports-feature-gated skills.
    pub(crate) fn install_visual_reports_skills(
        codex_home: &Path,
    ) -> Result<(), SystemSkillsError> {
        let codex_home = AbsolutePathBuf::try_from(codex_home)
            .map_err(|source| skills_io_error("normalize codex home dir", source))?;
        let skills_root_dir = codex_home
            .join(SKILLS_DIR_NAME)
            .map_err(|source| skills_io_error("resolve skills root dir", source))?;
        fs::create_dir_all(skills_root_dir.as_path())
            .map_err(|source| skills_io_error("create skills root dir", source))?;

        let dest_visual_reports =
            visual_reports_cache_root_dir_abs(&codex_home).map_err(|source| {
                skills_io_error("resolve visual-reports skills cache root dir", source)
            })?;

        let marker_path = dest_visual_reports
            .join(VISUAL_REPORTS_SKILLS_MARKER_FILENAME)
            .map_err(|source| {
                skills_io_error("resolve visual-reports skills marker path", source)
            })?;
        let expected_fingerprint = embedded_visual_reports_skills_fingerprint();
        if dest_visual_reports.as_path().is_dir()
            && read_marker(&marker_path).is_ok_and(|marker| marker == expected_fingerprint)
        {
            return Ok(());
        }

        if dest_visual_reports.as_path().exists() {
            fs::remove_dir_all(dest_visual_reports.as_path()).map_err(|source| {
                skills_io_error("remove existing visual-reports skills dir", source)
            })?;
        }

        write_embedded_dir(&VISUAL_REPORTS_SKILLS_DIR, &dest_visual_reports)?;
        fs::write(marker_path.as_path(), format!("{expected_fingerprint}\n"))
            .map_err(|source| skills_io_error("write visual-reports skills marker", source))?;
        Ok(())
    }

    fn read_marker(path: &AbsolutePathBuf) -> Result<String, SystemSkillsError> {
        Ok(fs::read_to_string(path.as_path())
            .map_err(|source| skills_io_error("read system skills marker", source))?
            .trim()
            .to_string())
    }

    fn embedded_visual_reports_skills_fingerprint() -> String {
        let mut items = Vec::new();
        collect_fingerprint_items(&VISUAL_REPORTS_SKILLS_DIR, &mut items);
        items.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        let mut hasher = DefaultHasher::new();
        VISUAL_REPORTS_SKILLS_MARKER_SALT.hash(&mut hasher);
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
    fn write_embedded_dir(
        dir: &Dir<'_>,
        dest: &AbsolutePathBuf,
    ) -> Result<(), SystemSkillsError> {
        fs::create_dir_all(dest.as_path())
            .map_err(|source| skills_io_error("create system skills dir", source))?;

        for entry in dir.entries() {
            match entry {
                include_dir::DirEntry::Dir(subdir) => {
                    let subdir_dest = dest.join(subdir.path()).map_err(|source| {
                        skills_io_error("resolve system skills subdir", source)
                    })?;
                    fs::create_dir_all(subdir_dest.as_path()).map_err(|source| {
                        skills_io_error("create system skills subdir", source)
                    })?;
                    write_embedded_dir(subdir, dest)?;
                }
                include_dir::DirEntry::File(file) => {
                    let path = dest.join(file.path()).map_err(|source| {
                        skills_io_error("resolve system skills file", source)
                    })?;
                    if let Some(parent) = path.as_path().parent() {
                        fs::create_dir_all(parent).map_err(|source| {
                            skills_io_error("create system skills file parent", source)
                        })?;
                    }
                    fs::write(path.as_path(), file.contents())
                        .map_err(|source| skills_io_error("write system skill file", source))?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(feature = "visual-reports")]
pub(crate) use visual_reports_impl::install_visual_reports_skills;
