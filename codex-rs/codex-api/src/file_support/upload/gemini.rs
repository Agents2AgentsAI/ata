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
use super::build_file_part;
use super::default_upload_retry_config;
use super::map_transport_error;
use super::read_upload_response;
use super::run_with_upload_retry;
use super::upload_url;
use codex_utils_file::file_name_or_default;

const GEMINI_API_VERSION: &str = "v1beta";
// Cap processing wait to roughly 10 minutes worst-case
// (1+2+4+8+16+30 + 18*30 = 601 seconds).
const GEMINI_MAX_POLL_ATTEMPTS: usize = 24;
const GEMINI_POLL_BASE_INTERVAL: Duration = Duration::from_secs(1);
const GEMINI_POLL_MAX_INTERVAL: Duration = Duration::from_secs(30);

pub struct GeminiFileUpload;

impl GeminiFileUpload {
    async fn poll_until_active(
        &self,
        client: &reqwest::Client,
        file_name: &str,
        api_key: &str,
        base_url: &str,
        retry: &crate::provider::RetryConfig,
    ) -> Result<GeminiFileMetadata, FileUploadError> {
        let normalized_file_name = file_name.trim_start_matches('/');
        let url = upload_url(
            base_url,
            &format!("/{GEMINI_API_VERSION}/{normalized_file_name}"),
        );

        for attempt in 0..GEMINI_MAX_POLL_ATTEMPTS {
            let parsed: GeminiFileResponse = run_with_upload_retry(retry, |_| async {
                let response = client
                    .get(&url)
                    .header("x-goog-api-key", api_key)
                    .timeout(DEFAULT_UPLOAD_TIMEOUT)
                    .send()
                    .await
                    .map_err(map_transport_error)?;
                read_upload_response(response).await
            })
            .await?;

            match parsed.file.state.as_str() {
                "ACTIVE" => return Ok(parsed.file),
                "PROCESSING" => tokio::time::sleep(poll_delay(attempt)).await,
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
        let url = upload_url(base_url, &format!("/upload/{GEMINI_API_VERSION}/files"));
        let filename = file_name_or_default(file_path, "file.pdf");
        let metadata_json = serde_json::json!({
            "file": {
                "displayName": filename.clone(),
            }
        });
        let retry = default_upload_retry_config();

        let parsed: GeminiFileResponse = run_with_upload_retry(&retry, |_| {
            let filename = filename.clone();
            let metadata_json = metadata_json.clone();
            let url = url.clone();
            async move {
                let part = build_file_part(file_path, filename, mime_type).await?;
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
                    .map_err(map_transport_error)?;

                read_upload_response(response).await
            }
        })
        .await?;
        let mut file_metadata = parsed.file;
        if file_metadata.state == "PROCESSING" {
            file_metadata = self
                .poll_until_active(client, &file_metadata.name, api_key, base_url, &retry)
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
        validate_file_uri(&file_uri)?;

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

fn poll_delay(attempt: usize) -> Duration {
    let factor = 1_u64 << attempt.min(5);
    let seconds = GEMINI_POLL_BASE_INTERVAL
        .as_secs()
        .saturating_mul(factor)
        .min(GEMINI_POLL_MAX_INTERVAL.as_secs());
    Duration::from_secs(seconds)
}

fn validate_file_uri(file_uri: &str) -> Result<(), FileUploadError> {
    let parsed = url::Url::parse(file_uri).map_err(|error| {
        FileUploadError::Request(format!(
            "gemini upload returned invalid file uri `{file_uri}`: {error}"
        ))
    })?;
    if parsed.scheme() != "https" {
        return Err(FileUploadError::Request(format!(
            "gemini upload returned non-https file uri `{file_uri}`"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::NamedTempFile;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

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

    #[test]
    fn poll_delay_uses_exponential_backoff_with_cap() {
        assert_eq!(poll_delay(0), Duration::from_secs(1));
        assert_eq!(poll_delay(1), Duration::from_secs(2));
        assert_eq!(poll_delay(2), Duration::from_secs(4));
        assert_eq!(poll_delay(3), Duration::from_secs(8));
        assert_eq!(poll_delay(4), Duration::from_secs(16));
        assert_eq!(poll_delay(5), Duration::from_secs(30));
        assert_eq!(poll_delay(20), Duration::from_secs(30));
    }

    #[test]
    fn validate_file_uri_accepts_https_uris() {
        let result = validate_file_uri("https://example.com/files/123");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_file_uri_rejects_non_https_uris() {
        let error = validate_file_uri("http://example.com/files/123")
            .expect_err("non-https uri should fail");
        assert!(matches!(error, FileUploadError::Request(_)));
    }

    #[test]
    fn validate_file_uri_rejects_invalid_uris() {
        let error = validate_file_uri("not a uri").expect_err("invalid uri should fail");
        assert!(matches!(error, FileUploadError::Request(_)));
    }

    #[tokio::test]
    async fn upload_file_posts_to_gemini_files_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/upload/v1beta/files"))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "file": {
                    "name": "files/abc123",
                    "uri": "https://generativelanguage.googleapis.com/v1beta/files/abc123",
                    "state": "ACTIVE",
                    "expirationTime": "2026-01-01T00:00:00Z"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let file = NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), b"%PDF-1.4\npayload").expect("write pdf");

        let uploaded = GeminiFileUpload
            .upload_file(
                &reqwest::Client::new(),
                file.path(),
                "application/pdf",
                "test-key",
                &server.uri(),
            )
            .await
            .expect("upload should succeed");

        assert_eq!(
            uploaded.file_id,
            "https://generativelanguage.googleapis.com/v1beta/files/abc123"
        );
        assert_eq!(uploaded.provider, "gemini");
        assert_eq!(uploaded.source_path, file.path().to_path_buf());
        assert!(uploaded.expires_at.is_some());
    }
}
