use std::path::Path;

use ignore::WalkBuilder;
use ignore::gitignore::Gitignore;

use crate::config::ProjectIndexConfig;
use crate::error::TreeSitterError;
use crate::file_entry::FileEntry;
use crate::file_entry::Language;
use crate::file_tree::FileTree;

const IGNORED_DIR_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "dist",
    "build",
    "target",
    "coverage",
    "vendor",
];

pub fn scan_directory(root: &Path, file_tree: &FileTree) -> Result<usize, TreeSitterError> {
    scan_directory_with_config(root, file_tree, &ProjectIndexConfig::default(), None)
}

pub fn scan_directory_with_limit(
    root: &Path,
    file_tree: &FileTree,
    max_file_size: u64,
) -> Result<usize, TreeSitterError> {
    let config = ProjectIndexConfig {
        max_file_size,
        ..ProjectIndexConfig::default()
    };
    scan_directory_with_config(root, file_tree, &config, None)
}

pub fn scan_directory_with_config(
    root: &Path,
    file_tree: &FileTree,
    config: &ProjectIndexConfig,
    extra_ignores: Option<&Gitignore>,
) -> Result<usize, TreeSitterError> {
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let mut count = 0;

    for entry in walker.flatten() {
        if entry.file_type().is_none_or(|ft| ft.is_dir()) {
            continue;
        }

        let path = entry.path();
        let rel_path = match path.strip_prefix(root) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => continue,
        };

        if should_skip(&rel_path) {
            continue;
        }

        let language = Language::from_path(path);
        if !config.is_language_enabled(language) {
            continue;
        }

        if let Some(ignores) = extra_ignores
            && ignores.matched(path, false).is_ignore()
        {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        let size = metadata.len();
        if size > config.max_file_size {
            continue;
        }

        file_tree.insert(FileEntry::new(rel_path, size));
        count += 1;
    }

    Ok(count)
}

fn should_skip(rel_path: &str) -> bool {
    rel_path
        .split('/')
        .any(|component| IGNORED_DIR_NAMES.contains(&component))
}
