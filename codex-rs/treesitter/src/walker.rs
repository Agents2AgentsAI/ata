use std::path::Path;

use ignore::WalkBuilder;

use crate::error::TreeSitterError;
use crate::file_entry::FileEntry;
use crate::file_tree::FileTree;

const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 1_000_000;

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
    scan_directory_with_limit(root, file_tree, DEFAULT_MAX_FILE_SIZE_BYTES)
}

pub fn scan_directory_with_limit(
    root: &Path,
    file_tree: &FileTree,
    max_file_size: u64,
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

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        let size = metadata.len();
        if size > max_file_size {
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
