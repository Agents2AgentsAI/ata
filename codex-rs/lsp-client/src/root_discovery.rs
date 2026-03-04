//! Discover the nearest project root for a given file by walking up directories
//! and looking for marker files (e.g. `Cargo.toml`, `go.mod`).

use std::path::Path;
use std::path::PathBuf;

use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;

use crate::server_config::RootStrategy;

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
        if (m.contains('*') || m.contains('?') || m.contains('['))
            && let Ok(glob) = Glob::new(m)
        {
            builder.add(glob);
            has_globs = true;
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
        if !(m.contains('*') || m.contains('?') || m.contains('[')) && dir.join(m).exists() {
            return true;
        }
    }

    // Slow path: check glob patterns via directory listing.
    if let Some(gs) = glob_set
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && gs.is_match(name)
            {
                return true;
            }
        }
    }

    false
}

/// Refine an initial root based on the server's [`RootStrategy`].
///
/// `initial` is the root returned by [`nearest_root`]. `workspace_root` is the
/// upper bound — we never walk above it.
pub fn refine_root(initial: &Path, workspace_root: &Path, strategy: &RootStrategy) -> PathBuf {
    match strategy {
        RootStrategy::NearestMarker => initial.to_path_buf(),
        RootStrategy::WalkUpForContent {
            marker_file,
            content_pattern,
        } => walk_up_for_content(initial, workspace_root, marker_file, content_pattern),
        RootStrategy::PreferAncestorMarker { preferred_marker } => {
            walk_up_for_marker(initial, workspace_root, preferred_marker)
        }
    }
}

/// Walk up from `start` (inclusive) looking for a directory containing
/// `marker_file` whose contents include `content_pattern`. Returns the
/// NEAREST (first) match. Falls back to `start`.
fn walk_up_for_content(
    start: &Path,
    bound: &Path,
    marker_file: &str,
    content_pattern: &str,
) -> PathBuf {
    let mut dir = start;
    loop {
        let candidate = dir.join(marker_file);
        if candidate.is_file()
            && let Ok(contents) = std::fs::read_to_string(&candidate)
            && contents.contains(content_pattern)
        {
            return dir.to_path_buf();
        }
        if dir == bound {
            break;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    start.to_path_buf()
}

/// Walk up from `start` (exclusive — starts at parent) looking for a directory
/// containing `preferred_marker`. Returns the FIRST match. Falls back to `start`.
fn walk_up_for_marker(start: &Path, bound: &Path, preferred_marker: &str) -> PathBuf {
    let mut dir = match start.parent() {
        Some(parent) if parent != start => parent,
        _ => return start.to_path_buf(),
    };
    loop {
        if dir.join(preferred_marker).exists() {
            return dir.to_path_buf();
        }
        if dir == bound {
            break;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => break,
        }
    }
    start.to_path_buf()
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

    // -- refine_root tests --

    #[test]
    fn refine_root_nearest_marker_is_identity() {
        let tmp = TempDir::new().expect("tmp");
        let initial = tmp.path().join("crates/foo");
        std::fs::create_dir_all(&initial).expect("mkdir");
        let result = refine_root(&initial, tmp.path(), &RootStrategy::NearestMarker);
        assert_eq!(result, initial);
    }

    #[test]
    fn refine_root_walk_up_finds_workspace_cargo_toml() {
        // workspace_root/
        //   Cargo.toml  (contains [workspace])
        //   crates/foo/
        //     Cargo.toml  (plain package)
        let tmp = TempDir::new().expect("tmp");
        let ws = tmp.path();
        let crate_dir = ws.join("crates/foo");
        std::fs::create_dir_all(&crate_dir).expect("mkdir");
        std::fs::write(
            ws.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]",
        )
        .expect("write ws");
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname = \"foo\"")
            .expect("write pkg");

        let strategy = RootStrategy::WalkUpForContent {
            marker_file: "Cargo.toml",
            content_pattern: "[workspace]",
        };
        // initial root is crate_dir (where nearest_root would land)
        let result = refine_root(&crate_dir, ws, &strategy);
        assert_eq!(result, ws);
    }

    #[test]
    fn refine_root_walk_up_no_workspace_falls_back() {
        let tmp = TempDir::new().expect("tmp");
        let ws = tmp.path();
        let crate_dir = ws.join("crates/foo");
        std::fs::create_dir_all(&crate_dir).expect("mkdir");
        // No workspace Cargo.toml at root
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname = \"foo\"")
            .expect("write pkg");

        let strategy = RootStrategy::WalkUpForContent {
            marker_file: "Cargo.toml",
            content_pattern: "[workspace]",
        };
        let result = refine_root(&crate_dir, ws, &strategy);
        assert_eq!(result, crate_dir, "should fall back to initial root");
    }

    #[test]
    fn refine_root_walk_up_respects_workspace_bound() {
        // Simulate a workspace Cargo.toml ABOVE the workspace_root bound.
        // It should NOT be found.
        let tmp = TempDir::new().expect("tmp");
        let outer = tmp.path();
        let ws = outer.join("repo");
        let crate_dir = ws.join("crates/foo");
        std::fs::create_dir_all(&crate_dir).expect("mkdir");
        // Workspace toml above bound
        std::fs::write(
            outer.join("Cargo.toml"),
            "[workspace]\nmembers = [\"repo/*\"]",
        )
        .expect("write");
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname = \"foo\"").expect("write");

        let strategy = RootStrategy::WalkUpForContent {
            marker_file: "Cargo.toml",
            content_pattern: "[workspace]",
        };
        let result = refine_root(&crate_dir, &ws, &strategy);
        assert_eq!(result, crate_dir, "should not walk above workspace_root");
    }

    #[test]
    fn refine_root_prefer_ancestor_marker_finds_go_work() {
        // workspace_root/
        //   go.work
        //   services/api/
        //     go.mod
        let tmp = TempDir::new().expect("tmp");
        let ws = tmp.path();
        let svc = ws.join("services/api");
        std::fs::create_dir_all(&svc).expect("mkdir");
        std::fs::write(ws.join("go.work"), "go 1.21\nuse ./services/api").expect("write");
        std::fs::write(svc.join("go.mod"), "module api").expect("write");

        let strategy = RootStrategy::PreferAncestorMarker {
            preferred_marker: "go.work",
        };
        let result = refine_root(&svc, ws, &strategy);
        assert_eq!(result, ws);
    }

    #[test]
    fn refine_root_prefer_ancestor_marker_no_go_work_falls_back() {
        let tmp = TempDir::new().expect("tmp");
        let ws = tmp.path();
        let svc = ws.join("services/api");
        std::fs::create_dir_all(&svc).expect("mkdir");
        std::fs::write(svc.join("go.mod"), "module api").expect("write");

        let strategy = RootStrategy::PreferAncestorMarker {
            preferred_marker: "go.work",
        };
        let result = refine_root(&svc, ws, &strategy);
        assert_eq!(result, svc, "should fall back when go.work not found");
    }
}
