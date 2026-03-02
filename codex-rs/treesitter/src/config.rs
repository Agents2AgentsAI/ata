use std::collections::HashSet;

use crate::file_entry::Language;

pub const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone)]
pub struct ProjectIndexConfig {
    pub max_file_size: u64,
    pub ignore_patterns: Vec<String>,
    pub disabled_languages: HashSet<Language>,
    pub watch: bool,
    pub persist_annotations: bool,
}

impl Default for ProjectIndexConfig {
    fn default() -> Self {
        Self {
            max_file_size: DEFAULT_MAX_FILE_SIZE_BYTES,
            ignore_patterns: Vec::new(),
            disabled_languages: HashSet::new(),
            watch: true,
            persist_annotations: true,
        }
    }
}

impl ProjectIndexConfig {
    pub fn with_disabled_languages<I>(mut self, languages: I) -> Self
    where
        I: IntoIterator<Item = Language>,
    {
        self.disabled_languages = languages.into_iter().collect();
        self
    }

    pub fn is_language_enabled(&self, language: Language) -> bool {
        !self.disabled_languages.contains(&language)
    }
}
