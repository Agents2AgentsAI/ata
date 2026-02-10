use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileProcessingError {
    #[error("failed to read file at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unsupported file type at {path}: expected PDF (magic bytes %PDF-)")]
    UnsupportedType { path: PathBuf },

    #[error("file too large at {path}: {size_mb:.1} MB exceeds {max_mb} MB limit")]
    TooLarge {
        path: PathBuf,
        size_mb: f64,
        max_mb: u64,
    },
}
