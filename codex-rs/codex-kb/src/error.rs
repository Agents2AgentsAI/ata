/// Result alias used throughout `codex-kb`.
pub type Result<T> = std::result::Result<T, KbError>;

#[derive(Debug, thiserror::Error)]
pub enum KbError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to persist temp file: {0}")]
    Persist(#[from] tempfile::PersistError),

    #[error("invalid frontmatter: {0}")]
    InvalidFrontmatter(String),

    #[error("invalid card id '{0}': must be kebab-case (lowercase alphanumeric and hyphens)")]
    InvalidCardId(String),

    #[error("card not found: {0}")]
    CardNotFound(String),

    #[error("topic not found: {0}")]
    TopicNotFound(String),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl KbError {
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Io(_) | Self::Persist(_) => true,
            Self::InvalidFrontmatter(_)
            | Self::InvalidCardId(_)
            | Self::CardNotFound(_)
            | Self::TopicNotFound(_)
            | Self::Yaml(_)
            | Self::Json(_)
            | Self::Internal(_) => false,
        }
    }
}
