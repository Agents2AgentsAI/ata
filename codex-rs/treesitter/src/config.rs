use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use crate::file_entry::Language;

pub const DEFAULT_MAX_FILE_SIZE_BYTES: u64 = 1_048_576;
const DEFAULT_IGNORED_EXTENSIONS: &[&str] = &[
    "min.js", "min.css", "pyc", "pyo", "class", "o", "so", "dylib", "dll", "exe", "a", "lib",
    "jar", "war", "ear", "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "png", "jpg", "jpeg", "gif",
    "bmp", "ico", "svg", "webp", "mp3", "mp4", "avi", "mov", "wmv", "flv", "woff", "woff2", "ttf",
    "eot", "otf", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "db", "sqlite", "sqlite3",
    "lock", "map",
];

#[derive(Debug, Clone)]
pub struct ProjectIndexConfig {
    pub max_file_size: u64,
    pub ignore_patterns: Vec<String>,
    pub ignore_extensions: HashSet<String>,
    pub disabled_languages: HashSet<Language>,
    pub annotation_store_path: Option<PathBuf>,
    pub watch: bool,
    pub persist_annotations: bool,
}

impl Default for ProjectIndexConfig {
    fn default() -> Self {
        Self {
            max_file_size: DEFAULT_MAX_FILE_SIZE_BYTES,
            ignore_patterns: Vec::new(),
            ignore_extensions: DEFAULT_IGNORED_EXTENSIONS
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            disabled_languages: HashSet::new(),
            annotation_store_path: None,
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

    pub fn with_ignore_extensions<I>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        for extension in extensions {
            if let Some(normalized) = normalize_ignore_extension(&extension) {
                self.ignore_extensions.insert(normalized);
            }
        }
        self
    }

    pub fn is_language_enabled(&self, language: Language) -> bool {
        !self.disabled_languages.contains(&language)
    }

    pub fn is_extension_ignored(&self, rel_path: &str) -> bool {
        let Some(file_name) = Path::new(rel_path)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return false;
        };

        let lower_name = file_name.to_ascii_lowercase();
        if let Some(extension) = Path::new(file_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            && self.ignore_extensions.contains(&extension)
        {
            return true;
        }

        self.ignore_extensions
            .iter()
            .filter(|value| value.contains('.'))
            .any(|suffix| lower_name.ends_with(&format!(".{suffix}")))
    }
}

fn normalize_ignore_extension(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}
