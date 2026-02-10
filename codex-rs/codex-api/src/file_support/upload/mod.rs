use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use async_trait::async_trait;
use codex_client::backoff;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::time::sleep;

use crate::provider::RetryConfig;

pub mod anthropic;
pub mod gemini;
pub mod openai;

pub use anthropic::AnthropicFileUpload;
pub use gemini::GeminiFileUpload;
pub use openai::OpenAiFileUpload;

pub const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);
pub const DEFAULT_UPLOAD_RETRY: RetryConfig = RetryConfig {
    max_attempts: 4,
    base_delay: Duration::from_millis(200),
    retry_429: false,
    retry_5xx: true,
    retry_transport: true,
};

#[derive(Debug, Error)]
pub enum FileUploadError {
    #[error("upload request failed: {0}")]
    Request(String),
    #[error("upload transport error: {0}")]
    Transport(String),
    #[error("upload response error ({status}): {body}")]
    Response { status: u16, body: String },
    #[error("failed to parse upload response: {0}")]
    Parse(String),
    #[error("failed to read file for upload: {0}")]
    Io(#[from] std::io::Error),
    #[error("upload processing timeout")]
    ProcessingTimeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub file_id: String,
    pub provider: String,
    pub expires_at: Option<SystemTime>,
    pub source_path: PathBuf,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct OpenAiFileUploadResponse {
    pub id: String,
    pub object: String,
    pub filename: String,
    pub purpose: String,
    pub bytes: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct AnthropicFileUploadResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub filename: String,
    pub size: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct GeminiFileResponse {
    pub file: GeminiFileMetadata,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct GeminiFileMetadata {
    pub name: String,
    pub uri: Option<String>,
    pub state: String,
    #[serde(rename = "expirationTime")]
    pub expiration_time: Option<String>,
}

#[async_trait]
pub trait FileUploadService: Send + Sync {
    async fn upload_file(
        &self,
        client: &reqwest::Client,
        file_path: &Path,
        mime_type: &str,
        api_key: &str,
        base_url: &str,
    ) -> Result<UploadedFile, FileUploadError>;
}

pub(crate) fn upload_url(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) fn file_name_or_default(file_path: &Path, default_name: &str) -> String {
    file_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| default_name.to_string())
}

pub(crate) fn mime_type_or_default(mime_type: &str) -> &str {
    if mime_type.is_empty() {
        "application/pdf"
    } else {
        mime_type
    }
}

pub(crate) fn default_upload_retry_config() -> RetryConfig {
    DEFAULT_UPLOAD_RETRY.clone()
}

pub(crate) fn map_transport_error(error: reqwest::Error) -> FileUploadError {
    FileUploadError::Transport(error.to_string())
}

fn should_retry_upload_error(retry: &RetryConfig, error: &FileUploadError, attempt: u64) -> bool {
    if attempt >= retry.max_attempts {
        return false;
    }

    match error {
        FileUploadError::Response { status, .. } => {
            (*status == 429 && retry.retry_429) || ((*status >= 500) && retry.retry_5xx)
        }
        FileUploadError::Transport(_) => retry.retry_transport,
        FileUploadError::Request(_)
        | FileUploadError::Parse(_)
        | FileUploadError::Io(_)
        | FileUploadError::ProcessingTimeout => false,
    }
}

pub(crate) async fn run_with_upload_retry<T, F, Fut>(
    retry: &RetryConfig,
    mut op: F,
) -> Result<T, FileUploadError>
where
    F: FnMut(u64) -> Fut,
    Fut: std::future::Future<Output = Result<T, FileUploadError>>,
{
    // `retry.max_attempts` is treated as max retries after the initial try.
    // So max total executions = 1 + max_attempts.
    for attempt in 0..=retry.max_attempts {
        match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) if should_retry_upload_error(retry, &error, attempt) => {
                sleep(backoff(retry.base_delay, attempt + 1)).await;
            }
            Err(error) => return Err(error),
        }
    }

    Err(FileUploadError::Request(
        "upload retry exhausted without final error".to_string(),
    ))
}

pub(crate) async fn build_file_part(
    file_path: &Path,
    filename: String,
    mime_type: &str,
) -> Result<reqwest::multipart::Part, FileUploadError> {
    let file = tokio::fs::File::open(file_path).await?;
    let file_size = file.metadata().await?.len();
    let stream = tokio_util::io::ReaderStream::new(file);
    let body = reqwest::Body::wrap_stream(stream);
    reqwest::multipart::Part::stream_with_length(body, file_size)
        .file_name(filename)
        .mime_str(mime_type_or_default(mime_type))
        .map_err(|error| FileUploadError::Request(error.to_string()))
}

pub(crate) async fn read_upload_response<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, FileUploadError> {
    let status = response.status().as_u16();
    let body = response.text().await.map_err(map_transport_error)?;
    if status >= 400 {
        return Err(FileUploadError::Response { status, body });
    }

    serde_json::from_str(&body).map_err(|error| FileUploadError::Parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn joins_upload_urls_without_duplicate_slashes() {
        let url = upload_url("https://example.com/v1/", "/files");
        assert_eq!(url, "https://example.com/v1/files");
    }

    #[test]
    fn resolves_filename_or_default() {
        let resolved = file_name_or_default(Path::new("/tmp/report.pdf"), "fallback.pdf");
        assert_eq!(resolved, "report.pdf");

        let fallback = file_name_or_default(Path::new("/"), "fallback.pdf");
        assert_eq!(fallback, "fallback.pdf");
    }

    #[test]
    fn resolves_mime_type_or_default() {
        assert_eq!(mime_type_or_default(""), "application/pdf");
        assert_eq!(mime_type_or_default("application/json"), "application/json");
    }

    #[tokio::test]
    async fn retries_transport_error_until_success() {
        let retry = RetryConfig {
            max_attempts: 2,
            base_delay: Duration::from_millis(1),
            retry_429: false,
            retry_5xx: false,
            retry_transport: true,
        };
        let mut attempts = 0_u64;
        let result = run_with_upload_retry(&retry, |_| {
            attempts += 1;
            async move {
                if attempts < 3 {
                    Err(FileUploadError::Transport("network".to_string()))
                } else {
                    Ok("ok")
                }
            }
        })
        .await
        .expect("retry should succeed");

        assert_eq!(result, "ok");
        assert_eq!(attempts, 3);
    }

    #[tokio::test]
    async fn does_not_retry_parse_error() {
        let retry = RetryConfig {
            max_attempts: 5,
            base_delay: Duration::from_millis(1),
            retry_429: true,
            retry_5xx: true,
            retry_transport: true,
        };
        let mut attempts = 0_u64;
        let error = run_with_upload_retry::<(), _, _>(&retry, |_| {
            attempts += 1;
            async { Err(FileUploadError::Parse("bad json".to_string())) }
        })
        .await
        .expect_err("parse errors should fail immediately");

        assert!(matches!(error, FileUploadError::Parse(_)));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn parses_openai_upload_response() {
        let body = r#"{
            "id": "file-123",
            "object": "file",
            "filename": "report.pdf",
            "purpose": "user_data",
            "bytes": 1024
        }"#;
        let parsed: OpenAiFileUploadResponse =
            serde_json::from_str(body).expect("openai upload response");
        assert_eq!(
            parsed,
            OpenAiFileUploadResponse {
                id: "file-123".to_string(),
                object: "file".to_string(),
                filename: "report.pdf".to_string(),
                purpose: "user_data".to_string(),
                bytes: 1024,
            }
        );
    }

    #[test]
    fn parses_anthropic_upload_response() {
        let body = r#"{
            "id": "file_123",
            "type": "file",
            "filename": "report.pdf",
            "size": 1024
        }"#;
        let parsed: AnthropicFileUploadResponse =
            serde_json::from_str(body).expect("anthropic upload response");
        assert_eq!(
            parsed,
            AnthropicFileUploadResponse {
                id: "file_123".to_string(),
                object_type: "file".to_string(),
                filename: "report.pdf".to_string(),
                size: 1024,
            }
        );
    }

    #[test]
    fn parses_gemini_upload_response() {
        let body = r#"{
            "file": {
                "name": "files/abc",
                "uri": "https://example.com/files/abc",
                "state": "ACTIVE",
                "expirationTime": "2026-01-01T00:00:00Z"
            }
        }"#;
        let parsed: GeminiFileResponse = serde_json::from_str(body).expect("gemini upload");
        assert_eq!(
            parsed,
            GeminiFileResponse {
                file: GeminiFileMetadata {
                    name: "files/abc".to_string(),
                    uri: Some("https://example.com/files/abc".to_string()),
                    state: "ACTIVE".to_string(),
                    expiration_time: Some("2026-01-01T00:00:00Z".to_string()),
                },
            }
        );
    }
}
