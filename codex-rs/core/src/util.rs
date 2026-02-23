use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use codex_protocol::ThreadId;
use codex_secrets::redact_secrets;
use rand::Rng;
use tracing::debug;
use tracing::error;

use crate::parse_command::shlex_join;

const INITIAL_DELAY_MS: u64 = 200;
const BACKOFF_FACTOR: f64 = 2.0;

/// Emit structured feedback metadata as key/value pairs.
///
/// This logs a tracing event with `target: "feedback_tags"`. If
/// `codex_feedback::CodexFeedback::metadata_layer()` is installed, these fields are captured and
/// later attached as tags when feedback is uploaded.
///
/// Values are wrapped with [`tracing::field::DebugValue`], so the expression only needs to
/// implement [`std::fmt::Debug`].
///
/// Example:
///
/// ```rust
/// codex_core::feedback_tags!(model = "gpt-5", cached = true);
/// codex_core::feedback_tags!(provider = provider_id, request_id = request_id);
/// ```
#[macro_export]
macro_rules! feedback_tags {
    ($( $key:ident = $value:expr ),+ $(,)?) => {
        ::tracing::info!(
            target: "feedback_tags",
            $( $key = ::tracing::field::debug(&$value) ),+
        );
    };
}

pub fn backoff(attempt: u64) -> Duration {
    let exp = BACKOFF_FACTOR.powi(attempt.saturating_sub(1) as i32);
    let base = (INITIAL_DELAY_MS as f64 * exp) as u64;
    let jitter = rand::rng().random_range(0.9..1.1);
    Duration::from_millis((base as f64 * jitter) as u64)
}

pub(crate) fn error_or_panic(message: impl std::string::ToString) {
    if cfg!(debug_assertions) {
        panic!("{}", message.to_string());
    } else {
        error!("{}", message.to_string());
    }
}

pub(crate) fn try_parse_error_message(text: &str) -> String {
    debug!("Parsing server error response: {}", text);
    let json = serde_json::from_str::<serde_json::Value>(text).unwrap_or_default();
    if let Some(error) = json.get("error")
        && let Some(message) = error.get("message")
        && let Some(message_str) = message.as_str()
    {
        return message_str.to_string();
    }
    if text.is_empty() {
        return "Unknown error".to_string();
    }
    text.to_string()
}

const REDACTED_SECRET: &str = "[REDACTED_SECRET]";

pub(crate) fn redact_error_body(body: &str) -> String {
    let redacted_json = serde_json::from_str::<serde_json::Value>(body)
        .map(|mut value| {
            redact_sensitive_json_fields(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| body.to_string())
        })
        .unwrap_or_else(|_| body.to_string());

    redact_secrets(redacted_json)
}

fn redact_sensitive_json_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                if is_sensitive_error_key(key) {
                    *nested = serde_json::Value::String(REDACTED_SECRET.to_string());
                } else {
                    redact_sensitive_json_fields(nested);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_sensitive_json_fields(item);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn is_sensitive_error_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "access_token"
            | "refresh_token"
            | "id_token"
            | "token"
            | "authorization"
            | "authorization_code"
            | "code_verifier"
            | "client_secret"
            | "secret"
            | "password"
            | "api_key"
            | "apikey"
    ) || lower.ends_with("_token")
        || lower.ends_with("_api_key")
}

pub fn resolve_path(base: &Path, path: &PathBuf) -> PathBuf {
    if path.is_absolute() {
        path.clone()
    } else {
        base.join(path)
    }
}

/// Trim a thread name and return `None` if it is empty after trimming.
pub fn normalize_thread_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn resume_command(thread_name: Option<&str>, thread_id: Option<ThreadId>) -> Option<String> {
    let resume_target = thread_name
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| thread_id.map(|thread_id| thread_id.to_string()));
    resume_target.map(|target| {
        let needs_double_dash = target.starts_with('-');
        let escaped = shlex_join(&[target]);
        if needs_double_dash {
            format!("ata resume -- {escaped}")
        } else {
            format!("ata resume {escaped}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_try_parse_error_message() {
        let text = r#"{
  "error": {
    "message": "Your refresh token has already been used to generate a new access token. Please try signing in again.",
    "type": "invalid_request_error",
    "param": null,
    "code": "refresh_token_reused"
  }
}"#;
        let message = try_parse_error_message(text);
        assert_eq!(
            message,
            "Your refresh token has already been used to generate a new access token. Please try signing in again."
        );
    }

    #[test]
    fn test_try_parse_error_message_no_error() {
        let text = r#"{"message": "test"}"#;
        let message = try_parse_error_message(text);
        assert_eq!(message, r#"{"message": "test"}"#);
    }

    #[test]
    fn redact_error_body_redacts_sensitive_json_fields() {
        let input = r#"{
            "access_token": "acc-1234567890",
            "refresh_token": "ref-1234567890",
            "nested": {
                "authorization_code": "auth-1234567890",
                "ok": "value"
            }
        }"#;

        let redacted = redact_error_body(input);
        let parsed: serde_json::Value = serde_json::from_str(&redacted).expect("valid json");

        assert_eq!(parsed["access_token"], REDACTED_SECRET);
        assert_eq!(parsed["refresh_token"], REDACTED_SECRET);
        assert_eq!(parsed["nested"]["authorization_code"], REDACTED_SECRET);
        assert_eq!(parsed["nested"]["ok"], "value");
    }

    #[test]
    fn redact_error_body_redacts_bearer_tokens_in_plain_text() {
        let redacted = redact_error_body("Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345");
        assert_eq!(redacted, "Authorization: Bearer [REDACTED_SECRET]");
    }

    #[test]
    fn feedback_tags_macro_compiles() {
        #[derive(Debug)]
        struct OnlyDebug;

        feedback_tags!(model = "gpt-5", cached = true, debug_only = OnlyDebug);
    }

    #[test]
    fn normalize_thread_name_trims_and_rejects_empty() {
        assert_eq!(normalize_thread_name("   "), None);
        assert_eq!(
            normalize_thread_name("  my thread  "),
            Some("my thread".to_string())
        );
    }

    #[test]
    fn resume_command_prefers_name_over_id() {
        let thread_id = ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let command = resume_command(Some("my-thread"), Some(thread_id));
        assert_eq!(command, Some("ata resume my-thread".to_string()));
    }

    #[test]
    fn resume_command_with_only_id() {
        let thread_id = ThreadId::from_string("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let command = resume_command(None, Some(thread_id));
        assert_eq!(
            command,
            Some("ata resume 123e4567-e89b-12d3-a456-426614174000".to_string())
        );
    }

    #[test]
    fn resume_command_with_no_name_or_id() {
        let command = resume_command(None, None);
        assert_eq!(command, None);
    }

    #[test]
    fn resume_command_quotes_thread_name_when_needed() {
        let command = resume_command(Some("-starts-with-dash"), None);
        assert_eq!(command, Some("ata resume -- -starts-with-dash".to_string()));

        let command = resume_command(Some("two words"), None);
        assert_eq!(command, Some("ata resume 'two words'".to_string()));

        let command = resume_command(Some("quote'case"), None);
        assert_eq!(command, Some("ata resume \"quote'case\"".to_string()));
    }
}
