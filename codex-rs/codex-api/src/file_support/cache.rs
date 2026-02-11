use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use super::upload::UploadedFile;

const DEFAULT_REFRESH_SKEW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
struct PathCacheEntry {
    file_id: String,
    provider: String,
    mime_type: String,
    filename: String,
    source_mtime: SystemTime,
    expires_at: Option<SystemTime>,
}

/// Result of a successful path-based cache lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFileHit {
    pub file_id: String,
    pub mime_type: String,
    pub filename: String,
}

#[derive(Debug, Clone, Default)]
pub struct FileReferenceCache {
    entries: HashMap<String, UploadedFile>,
    path_index: HashMap<PathBuf, PathCacheEntry>,
}

impl FileReferenceCache {
    pub fn record(&mut self, uploaded: UploadedFile) {
        self.entries.insert(uploaded.file_id.clone(), uploaded);
    }

    pub fn record_all<I>(&mut self, uploaded_files: I)
    where
        I: IntoIterator<Item = UploadedFile>,
    {
        for uploaded in uploaded_files {
            self.record(uploaded);
        }
    }

    pub fn get(&self, file_id: &str) -> Option<&UploadedFile> {
        self.entries.get(file_id)
    }

    pub fn remove(&mut self, file_id: &str) {
        let _ = self.entries.remove(file_id);
        self.path_index.retain(|_, entry| entry.file_id != file_id);
    }

    pub fn contains(&self, file_id: &str) -> bool {
        self.entries.contains_key(file_id)
    }

    /// Look up a previously uploaded file by its canonical source path.
    ///
    /// Returns `None` when the mtime differs (file changed), the provider
    /// changed, the upload is about to expire, or the `file_id` was removed
    /// from the primary entries (e.g. after a refresh).
    pub fn lookup_by_path(
        &self,
        canonical_path: &Path,
        source_mtime: SystemTime,
        current_provider: &str,
        now: SystemTime,
    ) -> Option<CachedFileHit> {
        let entry = self.path_index.get(canonical_path)?;
        if entry.source_mtime != source_mtime
            || entry.provider != current_provider
            || expires_soon(entry.expires_at, now)
            || !self.entries.contains_key(&entry.file_id)
        {
            return None;
        }
        Some(CachedFileHit {
            file_id: entry.file_id.clone(),
            mime_type: entry.mime_type.clone(),
            filename: entry.filename.clone(),
        })
    }

    /// Record a mapping from a canonical source path to its uploaded file metadata.
    #[allow(clippy::too_many_arguments)]
    pub fn record_path(
        &mut self,
        canonical_path: PathBuf,
        file_id: &str,
        provider: &str,
        mime_type: String,
        filename: String,
        source_mtime: SystemTime,
        expires_at: Option<SystemTime>,
    ) {
        self.path_index.insert(
            canonical_path,
            PathCacheEntry {
                file_id: file_id.to_string(),
                provider: provider.to_string(),
                mime_type,
                filename,
                source_mtime,
                expires_at,
            },
        );
    }

    pub fn refresh_candidates<'a>(
        &self,
        referenced_file_ids: impl IntoIterator<Item = &'a str>,
        current_provider: &str,
        now: SystemTime,
    ) -> Vec<UploadedFile> {
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for file_id in referenced_file_ids {
            if !seen.insert(file_id) {
                continue;
            }

            let Some(entry) = self.entries.get(file_id) else {
                continue;
            };

            if entry.provider != current_provider || expires_soon(entry.expires_at, now) {
                candidates.push(entry.clone());
            }
        }
        candidates
    }
}

fn expires_soon(expires_at: Option<SystemTime>, now: SystemTime) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };

    match now.checked_add(DEFAULT_REFRESH_SKEW) {
        Some(refresh_by) => expires_at <= refresh_by,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;

    fn uploaded_file(
        file_id: &str,
        provider: &str,
        expires_at: Option<SystemTime>,
    ) -> UploadedFile {
        UploadedFile {
            file_id: file_id.to_string(),
            provider: provider.to_string(),
            expires_at,
            source_path: PathBuf::from("report.pdf"),
        }
    }

    #[test]
    fn record_and_contains_track_entries() {
        let mut cache = FileReferenceCache::default();
        assert!(!cache.contains("file-1"));

        cache.record(uploaded_file("file-1", "openai", None));
        assert!(cache.contains("file-1"));

        cache.remove("file-1");
        assert!(!cache.contains("file-1"));
    }

    #[test]
    fn refresh_candidates_selects_provider_mismatches() {
        let mut cache = FileReferenceCache::default();
        cache.record(uploaded_file("file-1", "openai", None));

        let now = SystemTime::UNIX_EPOCH;
        let candidates = cache.refresh_candidates(["file-1"], "gemini", now);

        assert_eq!(candidates, vec![uploaded_file("file-1", "openai", None)]);
    }

    #[test]
    fn refresh_candidates_selects_near_expiry_entries() {
        let mut cache = FileReferenceCache::default();

        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let soon = now + Duration::from_secs(60);
        let later = now + Duration::from_secs(60 * 60);

        cache.record(uploaded_file("file-soon", "gemini", Some(soon)));
        cache.record(uploaded_file("file-later", "gemini", Some(later)));

        let candidates = cache.refresh_candidates(["file-soon", "file-later"], "gemini", now);

        assert_eq!(
            candidates,
            vec![uploaded_file("file-soon", "gemini", Some(soon))]
        );
    }

    #[test]
    fn refresh_candidates_dedupes_references() {
        let mut cache = FileReferenceCache::default();
        cache.record(uploaded_file("file-1", "openai", None));

        let now = SystemTime::UNIX_EPOCH;
        let candidates = cache.refresh_candidates(["file-1", "file-1"], "gemini", now);

        assert_eq!(candidates, vec![uploaded_file("file-1", "openai", None)]);
    }

    // --- Path index tests ---

    fn setup_cache_with_path_entry(
        mtime: SystemTime,
        expires_at: Option<SystemTime>,
    ) -> (FileReferenceCache, PathBuf) {
        let mut cache = FileReferenceCache::default();
        cache.record(uploaded_file("file-1", "openai", expires_at));
        let path = PathBuf::from("/tmp/report.pdf");
        cache.record_path(
            path.clone(),
            "file-1",
            "openai",
            "application/pdf".to_string(),
            "report.pdf".to_string(),
            mtime,
            expires_at,
        );
        (cache, path)
    }

    #[test]
    fn lookup_by_path_returns_hit_for_matching_entry() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (cache, path) = setup_cache_with_path_entry(mtime, None);

        let hit = cache
            .lookup_by_path(&path, mtime, "openai", SystemTime::UNIX_EPOCH)
            .expect("should return a cache hit");

        assert_eq!(
            hit,
            CachedFileHit {
                file_id: "file-1".to_string(),
                mime_type: "application/pdf".to_string(),
                filename: "report.pdf".to_string(),
            }
        );
    }

    #[test]
    fn lookup_by_path_returns_none_when_mtime_differs() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (cache, path) = setup_cache_with_path_entry(mtime, None);

        let different_mtime = mtime + Duration::from_secs(1);
        assert!(
            cache
                .lookup_by_path(&path, different_mtime, "openai", SystemTime::UNIX_EPOCH)
                .is_none()
        );
    }

    #[test]
    fn lookup_by_path_returns_none_when_provider_differs() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (cache, path) = setup_cache_with_path_entry(mtime, None);

        assert!(
            cache
                .lookup_by_path(&path, mtime, "anthropic", SystemTime::UNIX_EPOCH)
                .is_none()
        );
    }

    #[test]
    fn lookup_by_path_returns_none_when_expired() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        // Expires very soon relative to `now`.
        let expires_at = Some(now + Duration::from_secs(60));
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (cache, path) = setup_cache_with_path_entry(mtime, expires_at);

        assert!(cache.lookup_by_path(&path, mtime, "openai", now).is_none());
    }

    #[test]
    fn lookup_by_path_returns_none_when_file_id_removed() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (mut cache, path) = setup_cache_with_path_entry(mtime, None);

        cache.remove("file-1");
        assert!(
            cache
                .lookup_by_path(&path, mtime, "openai", SystemTime::UNIX_EPOCH)
                .is_none()
        );
    }

    #[test]
    fn remove_cleans_path_index() {
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let (mut cache, path) = setup_cache_with_path_entry(mtime, None);

        // Before removal, path index has the entry.
        assert!(
            cache
                .lookup_by_path(&path, mtime, "openai", SystemTime::UNIX_EPOCH)
                .is_some()
        );

        cache.remove("file-1");

        // After removal, path index is also cleaned.
        assert!(cache.path_index.is_empty());
    }
}
