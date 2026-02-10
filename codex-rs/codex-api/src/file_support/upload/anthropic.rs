use std::path::Path;

use super::AnthropicFileUploadResponse;
use super::DEFAULT_UPLOAD_TIMEOUT;
use super::FileUploadError;
use super::FileUploadService;
use super::UploadedFile;
use super::file_name_or_default;
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
        let mime_type = if mime_type.is_empty() {
            "application/pdf"
        } else {
            mime_type
        };

        let file = tokio::fs::File::open(file_path).await?;
        let stream = tokio_util::io::ReaderStream::new(file);
        let body = reqwest::Body::wrap_stream(stream);
        let part = reqwest::multipart::Part::stream(body)
            .file_name(filename)
            .mime_str(mime_type)
            .map_err(|error| FileUploadError::Request(error.to_string()))?;
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
            .map_err(|error| FileUploadError::Request(error.to_string()))?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| FileUploadError::Request(error.to_string()))?;
        if status >= 400 {
            return Err(FileUploadError::Response { status, body });
        }

        let parsed: AnthropicFileUploadResponse = serde_json::from_str(&body)
            .map_err(|error| FileUploadError::Request(error.to_string()))?;

        Ok(UploadedFile {
            file_id: parsed.id,
            provider: "anthropic".to_string(),
            expires_at: None,
            source_path: file_path.to_path_buf(),
        })
    }
}
