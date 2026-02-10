use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;

use chrono::Utc;

use super::DEFAULT_UPLOAD_TIMEOUT;
use super::FileUploadError;
use super::FileUploadService;
use super::GeminiFileMetadata;
use super::GeminiFileResponse;
use super::UploadedFile;
use super::file_name_or_default;
use super::upload_url;

const GEMINI_POLL_INTERVAL: Duration = Duration::from_secs(1);
const GEMINI_MAX_POLL_ATTEMPTS: usize = 60;

pub struct GeminiFileUpload;

impl GeminiFileUpload {
    async fn poll_until_active(
        &self,
        client: &reqwest::Client,
        file_name: &str,
        api_key: &str,
        base_url: &str,
    ) -> Result<GeminiFileMetadata, FileUploadError> {
        let normalized_file_name = file_name.trim_start_matches('/');
        let url = upload_url(base_url, &format!("/v1beta/{normalized_file_name}"));

        for _ in 0..GEMINI_MAX_POLL_ATTEMPTS {
            let response = client
                .get(&url)
                .header("x-goog-api-key", api_key)
                .timeout(DEFAULT_UPLOAD_TIMEOUT)
                .send()
                .await
                .map_err(|error| FileUploadError::Request(error.to_string()))?;

            let status = response.status().as_u16();
            let body = response
                .text()
                .await
                .map_err(|error| FileUploadError::Request(error.to_string()))?;
            if status >= 400 {
                return Err(FileUploadError::Response { status, body });
            }

            let parsed: GeminiFileResponse = serde_json::from_str(&body)
                .map_err(|error| FileUploadError::Request(error.to_string()))?;

            match parsed.file.state.as_str() {
                "ACTIVE" => return Ok(parsed.file),
                "PROCESSING" => tokio::time::sleep(GEMINI_POLL_INTERVAL).await,
                state => {
                    return Err(FileUploadError::Request(format!(
                        "gemini file processing failed with state `{state}`"
                    )));
                }
            }
        }

        Err(FileUploadError::ProcessingTimeout)
    }
}

#[async_trait::async_trait]
impl FileUploadService for GeminiFileUpload {
    async fn upload_file(
        &self,
        client: &reqwest::Client,
        file_path: &Path,
        mime_type: &str,
        api_key: &str,
        base_url: &str,
    ) -> Result<UploadedFile, FileUploadError> {
        let url = upload_url(base_url, "/upload/v1beta/files");
        let filename = file_name_or_default(file_path, "file.pdf");
        let mime_type = if mime_type.is_empty() {
            "application/pdf"
        } else {
            mime_type
        };

        let file = tokio::fs::File::open(file_path).await?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);
        let metadata_json = serde_json::json!({
            "file": {
                "display_name": filename.clone(),
            }
        });

        let part = reqwest::multipart::Part::stream(body)
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|error| FileUploadError::Request(error.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("metadata", metadata_json.to_string())
            .part("file", part);

        let response = client
            .post(url)
            .header("x-goog-api-key", api_key)
            .timeout(DEFAULT_UPLOAD_TIMEOUT)
            .multipart(form)
            .send()
            .await
            .map_err(|error| FileUploadError::Request(error.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| FileUploadError::Request(error.to_string()))?;
        if status >= 400 {
            return Err(FileUploadError::Response { status, body });
        }

        let parsed: GeminiFileResponse = serde_json::from_str(&body)
            .map_err(|error| FileUploadError::Request(error.to_string()))?;
        let mut file_metadata = parsed.file;
        if file_metadata.state == "PROCESSING" {
            file_metadata = self
                .poll_until_active(client, &file_metadata.name, api_key, base_url)
                .await?;
        }
        if file_metadata.state != "ACTIVE" {
            return Err(FileUploadError::Request(format!(
                "gemini upload returned unexpected state `{}`",
                file_metadata.state
            )));
        }

        let file_uri = file_metadata
            .uri
            .ok_or_else(|| FileUploadError::Request("missing file uri".to_string()))?;

        Ok(UploadedFile {
            file_id: file_uri,
            provider: "gemini".to_string(),
            expires_at: parse_expiration_time(file_metadata.expiration_time.as_deref()),
            source_path: file_path.to_path_buf(),
        })
    }
}

fn parse_expiration_time(expiration_time: Option<&str>) -> Option<SystemTime> {
    expiration_time
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| SystemTime::from(dt.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_rfc3339_expiration_time() {
        let parsed = parse_expiration_time(Some("2026-01-01T00:00:00Z"));
        assert!(parsed.is_some());
    }

    #[test]
    fn invalid_expiration_time_returns_none() {
        let parsed = parse_expiration_time(Some("not-a-timestamp"));
        assert_eq!(parsed, None);
    }
}
