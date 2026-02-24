use std::fmt;

use crate::rate_limiter::DataApi;

pub type Result<T> = std::result::Result<T, DataError>;

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("HTTP error for {api}: {source}")]
    Http {
        api: DataApi,
        source: reqwest::Error,
    },

    #[error("Upstream error from {api} (status {status}): {message}")]
    Upstream {
        api: DataApi,
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("Parse error from {api}: {message}")]
    Parse { api: DataApi, message: String },

    #[error("Timeout for {api} after {timeout_ms}ms")]
    Timeout { api: DataApi, timeout_ms: u64 },

    #[error("Rate limit exceeded for {api}")]
    RateLimitExceeded { api: DataApi },

    #[error("Authentication required for {api}: {message}")]
    AuthRequired { api: DataApi, message: String },

    #[error("Dataset not found: {dataset_id}")]
    DatasetNotFound { dataset_id: String },

    #[error("Download failed: {message}")]
    DownloadFailed { message: String },

    #[error("Download failed for {url}: {message}")]
    Download { url: String, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("Tool not implemented: {tool}")]
    NotImplemented { tool: &'static str },

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Internal panic in data tool")]
    InternalPanic,
}

impl DataError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            DataError::Http { source, .. } if source.is_timeout() || source.is_connect()
        ) || matches!(
            self,
            DataError::Upstream { status, .. } if status.is_server_error() || *status == reqwest::StatusCode::TOO_MANY_REQUESTS
        ) || matches!(self, DataError::Timeout { .. })
    }
}

impl fmt::Display for DataApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataApi::HuggingFace => write!(f, "HuggingFace"),
            DataApi::Kaggle => write!(f, "Kaggle"),
            DataApi::IsaacSim => write!(f, "Isaac Sim"),
            DataApi::Cosmos => write!(f, "Cosmos"),
        }
    }
}
