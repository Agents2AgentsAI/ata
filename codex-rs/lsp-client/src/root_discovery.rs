//! Discover the nearest project root for a given file by walking up directories
//! and looking for marker files (e.g. `Cargo.toml`, `go.mod`).

use std::path::Path;
use std::path::PathBuf;

use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;

/// Walk up from `file`'s parent directory looking for any of the given marker
/// files/patterns. Returns the first directory containing a match, or
/// `workspace_root` as a fallback.
pub fn nearest_root(file: &Path, workspace_root: &Path, markers: &[String]) -> PathBuf {
    if markers.is_empty() {
        return workspace_root.to_path_buf();
    }

    let glob_set = build_glob_set(markers);

    let start = if file.is_file() {
        file.parent().unwrap_or(workspace_root)
    } else {
        file
    };

    let mut dir = start;
    loop {
        if has_marker(dir, &glob_set, markers) {
            return dir.to_path_buf();
        }
        // Stop at the workspace root — don't walk above it.
        if dir == workspace_root {
            break;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }

    workspace_root.to_path_buf()
}

/// Returns true if `dir` contains any marker from `markers`.
///
/// Marker entries may be literal file names (like `Cargo.toml`) or glob patterns
/// (like `*.xcodeproj`).
pub fn dir_has_any_marker(dir: &Path, markers: &[String]) -> bool {
    if markers.is_empty() {
        return true;
    }
    let glob_set = build_glob_set(markers);
    has_marker(dir, &glob_set, markers)
}

/// Build a `GlobSet` from marker patterns (only those containing glob chars).
fn build_glob_set(markers: &[String]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut has_globs = false;
    for m in markers {
        if m.contains('*') || m.contains('?') || m.contains('[') {
            if let Ok(glob) = Glob::new(m) {
                builder.add(glob);
                has_globs = true;
            }
        }
    }
    if has_globs {
        builder.build().ok()
    } else {
        None
    }
}

/// Check whether `dir` contains any of the marker files/patterns.
fn has_marker(dir: &Path, glob_set: &Option<GlobSet>, markers: &[String]) -> bool {
    // Fast path: check literal markers by simple existence.
    for m in markers {
        if !(m.contains('*') || m.contains('?') || m.contains('[')) {
            if dir.join(m).exists() {
                return true;
            }
        }
    }

    // Slow path: check glob patterns via directory listing.
    if let Some(gs) = glob_set {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if gs.is_match(name) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn finds_marker_in_parent() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        let sub = root.join("src/deep");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(root.join("Cargo.toml"), "").expect("write marker");

        let result = nearest_root(&sub.join("main.rs"), root, &["Cargo.toml".to_string()]);
        assert_eq!(result, root);
    }

    #[test]
    fn falls_back_to_workspace_root() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        let file = root.join("src/main.rs");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");

        let result = nearest_root(&file, root, &["nonexistent.marker".to_string()]);
        assert_eq!(result, root);
    }

    #[test]
    fn glob_marker() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        let sub = root.join("pkg");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(sub.join("my-project.cabal"), "").expect("write cabal");

        let result = nearest_root(&sub.join("Main.hs"), root, &["*.cabal".to_string()]);
        assert_eq!(result, sub);
    }

    #[test]
    fn empty_markers_returns_workspace_root() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        let result = nearest_root(&root.join("file.rs"), root, &[]);
        assert_eq!(result, root);
    }

    #[test]
    fn dir_has_any_marker_literal() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        std::fs::write(root.join("Cargo.toml"), "").expect("write marker");
        assert!(dir_has_any_marker(root, &["Cargo.toml".to_string()]));
        assert!(!dir_has_any_marker(root, &["go.mod".to_string()]));
    }

    #[test]
    fn dir_has_any_marker_glob() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("Foo.xcodeproj")).expect("mkdir");
        assert!(dir_has_any_marker(root, &["*.xcodeproj".to_string()]));
    }
}
