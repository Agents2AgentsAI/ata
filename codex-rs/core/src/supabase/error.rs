use thiserror::Error;

/// Errors returned by Supabase client operations.
#[derive(Debug, Error)]
pub enum SupabaseError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Supabase API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Failed to parse response: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("No active session")]
    NoSession,

    #[error("Session expired")]
    SessionExpired,

    #[error("{0}")]
    Other(String),
}

impl SupabaseError {
    pub fn api(status: u16, message: impl Into<String>) -> Self {
        Self::Api {
            status,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = SupabaseError::api(401, "Unauthorized");
        assert_eq!(format!("{err}"), "Supabase API error (401): Unauthorized");
    }

    #[test]
    fn error_no_session() {
        let err = SupabaseError::NoSession;
        assert_eq!(format!("{err}"), "No active session");
    }

    #[test]
    fn error_session_expired() {
        let err = SupabaseError::SessionExpired;
        assert_eq!(format!("{err}"), "Session expired");
    }

    #[test]
    fn error_other() {
        let err = SupabaseError::Other("custom error".to_string());
        assert_eq!(format!("{err}"), "custom error");
    }
}
