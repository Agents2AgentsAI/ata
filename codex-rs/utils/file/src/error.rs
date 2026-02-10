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

impl Clone for FileProcessingError {
    fn clone(&self) -> Self {
        match self {
            Self::Read { path, source } => Self::Read {
                path: path.clone(),
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Self::UnsupportedType { path } => Self::UnsupportedType { path: path.clone() },
            Self::TooLarge {
                path,
                size_mb,
                max_mb,
            } => Self::TooLarge {
                path: path.clone(),
                size_mb: *size_mb,
                max_mb: *max_mb,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn clone_preserves_read_error_shape() {
        let error = FileProcessingError::Read {
            path: PathBuf::from("/tmp/report.pdf"),
            source: std::io::Error::other("boom"),
        };
        let cloned = error.clone();
        assert_eq!(error.to_string(), cloned.to_string());
    }
}
