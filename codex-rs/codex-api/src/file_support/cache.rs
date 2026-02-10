use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::SystemTime;

use super::upload::UploadedFile;

const DEFAULT_REFRESH_SKEW: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Default)]
pub struct FileReferenceCache {
    entries: HashMap<String, UploadedFile>,
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

    pub fn remove(&mut self, file_id: &str) {
        let _ = self.entries.remove(file_id);
    }

    pub fn contains(&self, file_id: &str) -> bool {
        self.entries.contains_key(file_id)
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
}
