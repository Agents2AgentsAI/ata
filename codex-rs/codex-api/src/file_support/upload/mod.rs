use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

pub mod anthropic;
pub mod gemini;
pub mod openai;

pub use anthropic::AnthropicFileUpload;
pub use gemini::GeminiFileUpload;
pub use openai::OpenAiFileUpload;

pub const DEFAULT_UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
pub enum FileUploadError {
    #[error("upload request failed: {0}")]
    Request(String),
    #[error("upload response error ({status}): {body}")]
    Response { status: u16, body: String },
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
