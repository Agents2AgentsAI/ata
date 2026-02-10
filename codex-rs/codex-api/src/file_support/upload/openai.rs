use std::path::Path;

use super::DEFAULT_UPLOAD_TIMEOUT;
use super::FileUploadError;
use super::FileUploadService;
use super::OpenAiFileUploadResponse;
use super::UploadedFile;
use super::build_file_part;
use super::default_upload_retry_config;
use super::map_transport_error;
use super::read_upload_response;
use super::run_with_upload_retry;
use super::upload_url;
use codex_utils_file::file_name_or_default;

pub struct OpenAiFileUpload;

#[async_trait::async_trait]
impl FileUploadService for OpenAiFileUpload {
    async fn upload_file(
        &self,
        client: &reqwest::Client,
        file_path: &Path,
        mime_type: &str,
        api_key: &str,
        base_url: &str,
    ) -> Result<UploadedFile, FileUploadError> {
        let url = upload_url(base_url, "/v1/files");
        let filename = file_name_or_default(file_path, "file.pdf");
        let retry = default_upload_retry_config();

        let parsed: OpenAiFileUploadResponse = run_with_upload_retry(&retry, |_| {
            let filename = filename.clone();
            let url = url.clone();
            async move {
                let part = build_file_part(file_path, filename, mime_type).await?;
                let form = reqwest::multipart::Form::new()
                    .text("purpose", "user_data")
                    .part("file", part);

                let response = client
                    .post(url)
                    .header("Authorization", format!("Bearer {api_key}"))
                    .timeout(DEFAULT_UPLOAD_TIMEOUT)
                    .multipart(form)
                    .send()
                    .await
                    .map_err(map_transport_error)?;

                read_upload_response(response).await
            }
        })
        .await?;

        Ok(UploadedFile {
            file_id: parsed.id,
            provider: "openai".to_string(),
            expires_at: None,
            source_path: file_path.to_path_buf(),
        })
    }
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

    #[tokio::test]
    async fn upload_file_posts_to_openai_files_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "filename": "report.pdf",
                "purpose": "user_data",
                "bytes": 42
            })))
            .expect(1)
            .mount(&server)
            .await;

        let file = NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), b"%PDF-1.4\npayload").expect("write pdf");

        let uploaded = OpenAiFileUpload
            .upload_file(
                &reqwest::Client::new(),
                file.path(),
                "application/pdf",
                "test-key",
                &server.uri(),
            )
            .await
            .expect("upload should succeed");

        assert_eq!(uploaded.file_id, "file-123");
        assert_eq!(uploaded.provider, "openai");
        assert_eq!(uploaded.source_path, file.path().to_path_buf());
    }
}
