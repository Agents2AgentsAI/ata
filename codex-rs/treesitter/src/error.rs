use std::path::PathBuf;

use thiserror::Error;

use crate::file_entry::Language;

#[derive(Debug, Error)]
pub enum TreeSitterError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse source")]
    ParseFailed,

    #[error("invalid regex pattern: {0}")]
    InvalidPattern(String),

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

    #[error("annotation serialization failed: {0}")]
    AnnotationSerde(#[from] serde_json::Error),

    #[error("invalid chunk parameters: chunk_size must be > 0")]
    InvalidChunkSize,

    #[error(
        "invalid chunk parameters: overlap {overlap} must be less than chunk_size {chunk_size}"
    )]
    InvalidChunkOverlap { chunk_size: usize, overlap: usize },
}
