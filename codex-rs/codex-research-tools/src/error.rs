use crate::rate_limiter::ResearchApi;

/// Result alias used throughout `codex-research-tools`.
pub type Result<T> = std::result::Result<T, ResearchError>;

#[derive(Debug, thiserror::Error)]
pub enum ResearchError {
    #[error("tool '{tool}' is not configured: {reason}")]
    NotConfigured { tool: &'static str, reason: String },

    #[error("tool '{tool}' is not implemented yet")]
    NotImplemented { tool: &'static str },

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
