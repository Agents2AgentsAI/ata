use crate::paths;
use std::path::PathBuf;

/// Return the shared mirror cache path for a repo URL.
pub fn run(url: &str) -> PathBuf {
    paths::mirror_cache_path(url)
}
