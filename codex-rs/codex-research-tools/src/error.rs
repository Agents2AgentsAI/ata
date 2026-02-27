use crate::rate_limiter::ResearchApi;

/// Result alias used throughout `codex-research-tools`.
pub type Result<T> = std::result::Result<T, ResearchError>;

#[derive(Debug, thiserror::Error)]
pub enum ResearchError {
    #[error("tool '{tool}' is not configured: {reason}")]
    NotConfigured { tool: &'static str, reason: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("rate limiter closed for {api}")]
    RateLimiterClosed { api: ResearchApi },

    #[error("request to {api} timed out after {timeout_ms}ms")]
    Timeout { api: ResearchApi, timeout_ms: u64 },

    #[error("http request to {api} failed: {source}")]
    Http {
        api: ResearchApi,
        #[source]
        source: reqwest::Error,
    },

    #[error("http request to {api} failed: {message}")]
    HttpMessage { api: ResearchApi, message: String },

    #[error("upstream API {api} returned {status}: {message}")]
    Upstream {
        api: ResearchApi,
        status: reqwest::StatusCode,
        message: String,
    },

    #[error("failed to parse response from {api}: {message}")]
    Parse { api: ResearchApi, message: String },

    #[error("internal panic while executing research tool")]
    InternalPanic,

    #[error("internal error: {0}")]
    Internal(String),
}

#[must_use]
pub(crate) fn is_retryable_upstream_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

#[must_use]
pub(crate) fn is_retryable_http_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

impl ResearchError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimiterClosed { .. }
            | Self::Timeout { .. }
            | Self::Http { .. }
            | Self::HttpMessage { .. } => true,
            Self::Upstream { status, .. } => is_retryable_upstream_status(*status),
            Self::NotConfigured { .. }
            | Self::InvalidInput(_)
            | Self::Parse { .. }
            | Self::InternalPanic
            | Self::Internal(_) => false,
        }
    }
}
