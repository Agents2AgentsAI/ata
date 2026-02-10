use std::path::Path;

use super::AnthropicFileUploadResponse;
use super::DEFAULT_UPLOAD_TIMEOUT;
use super::FileUploadError;
use super::FileUploadService;
use super::UploadedFile;
use super::build_file_part;
use super::default_upload_retry_config;
use super::file_name_or_default;
use super::map_transport_error;
use super::read_upload_response;
use super::run_with_upload_retry;
use super::upload_url;

pub struct AnthropicFileUpload;

#[async_trait::async_trait]
impl FileUploadService for AnthropicFileUpload {
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

        let parsed: AnthropicFileUploadResponse = run_with_upload_retry(&retry, |_| {
            let filename = filename.clone();
            let url = url.clone();
            async move {
                let part = build_file_part(file_path, filename, mime_type).await?;
                let form = reqwest::multipart::Form::new().part("file", part);

                let response = client
                    .post(url)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("anthropic-beta", "files-api-2025-04-14")
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
            provider: "anthropic".to_string(),
            expires_at: None,
            source_path: file_path.to_path_buf(),
        })
    }
}
