use std::path::PathBuf;

use thiserror::Error;

use crate::file_entry::Language;

#[derive(Debug, Error)]
pub enum TreeSitterError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse source")]
    ParseFailed,

    #[error("tree-sitter language error: {0}")]
    Language(#[from] tree_sitter::LanguageError),

    #[error("tree-sitter query compile failed: {0}")]
    Query(#[from] tree_sitter::QueryError),

    #[error("unsupported language: {0:?}")]
    UnsupportedLanguage(Language),

    #[error("path is outside project root: {path}")]
    PathOutsideRoot { path: PathBuf },

    #[error("invalid line range: start {start} > end {end}")]
    InvalidRange { start: usize, end: usize },

    #[error("invalid ignore pattern: {0}")]
    InvalidIgnorePattern(String),
}
