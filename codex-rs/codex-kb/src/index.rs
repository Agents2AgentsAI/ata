use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use chrono::NaiveDate;
use serde::Deserialize;
use serde::Serialize;
use tempfile::NamedTempFile;

use crate::error::Result;

/// Per-topic staleness tracking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TopicStatus {
    pub last_regen: Option<NaiveDate>,
    pub cards_since_regen: u32,
}

/// Knowledge base index: staleness tracking + tag taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KbIndex {
    #[serde(default)]
    pub topics: BTreeMap<String, TopicStatus>,
    #[serde(default)]
    pub tag_taxonomy: BTreeSet<String>,
}

impl KbIndex {
    /// Read the index from a JSON file; returns default if the file doesn't exist.
    pub fn read_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(serde_json::from_str(&contents)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Write the index atomically via temp file + persist.
    pub fn write_to(&self, path: &Path) -> Result<()> {
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("path {} has no parent directory", path.display()),
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let contents = serde_json::to_string_pretty(self)?;
        let tmp = NamedTempFile::new_in(parent)?;
        std::fs::write(tmp.path(), contents)?;
        tmp.persist(path)?;
        Ok(())
    }

    /// Register tags into the taxonomy.
    pub fn register_tags(&mut self, tags: &[String]) {
        for tag in tags {
            self.tag_taxonomy.insert(tag.clone());
        }
    }

    /// Increment staleness counter for a topic (create entry if missing).
    pub fn increment_staleness(&mut self, topic: &str) {
        let entry = self.topics.entry(topic.to_string()).or_insert(TopicStatus {
            last_regen: None,
            cards_since_regen: 0,
        });
        entry.cards_since_regen += 1;
    }

    /// Reset staleness for a topic after a regeneration.
    pub fn reset_staleness(&mut self, topic: &str, date: NaiveDate) {
        let entry = self.topics.entry(topic.to_string()).or_insert(TopicStatus {
            last_regen: None,
            cards_since_regen: 0,
        });
        entry.last_regen = Some(date);
        entry.cards_since_regen = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_index_is_empty() {
        let idx = KbIndex::default();
        assert!(idx.topics.is_empty());
        assert!(idx.tag_taxonomy.is_empty());
    }

    #[test]
    fn read_missing_file_returns_default() {
        let idx = KbIndex::read_from(Path::new("/tmp/nonexistent-kb-index-xyz.json")).unwrap();
        assert_eq!(idx, KbIndex::default());
    }

    #[test]
    fn roundtrip_write_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");

        let mut idx = KbIndex::default();
        idx.register_tags(&["diffusion".into(), "generative".into()]);
        idx.increment_staleness("diffusion");
        idx.increment_staleness("diffusion");
        idx.reset_staleness(
            "generative",
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap_or_default(),
        );

        idx.write_to(&path).unwrap();
        let loaded = KbIndex::read_from(&path).unwrap();
        assert_eq!(idx, loaded);
    }

    #[test]
    fn increment_staleness_creates_entry() {
        let mut idx = KbIndex::default();
        idx.increment_staleness("new-topic");
        assert_eq!(idx.topics["new-topic"].cards_since_regen, 1);
        assert_eq!(idx.topics["new-topic"].last_regen, None);
    }

    #[test]
    fn reset_staleness_zeroes_counter() {
        let mut idx = KbIndex::default();
        idx.increment_staleness("topic");
        idx.increment_staleness("topic");
        idx.reset_staleness(
            "topic",
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap_or_default(),
        );
        assert_eq!(idx.topics["topic"].cards_since_regen, 0);
        assert!(idx.topics["topic"].last_regen.is_some());
    }
}
