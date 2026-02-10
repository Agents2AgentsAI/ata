use std::path::Path;

use codex_protocol::models::ContentItem;
use codex_utils_file::FileProcessingError;
use codex_utils_file::analyze_file;
use codex_utils_file::bytes_to_megabytes;
use codex_utils_file::encode_inline_cached;
use thiserror::Error;
use tracing::warn;

use super::capabilities::FileCapabilityConfig;
use super::upload::FileUploadError;
use super::upload::FileUploadService;

#[derive(Debug, Error)]
pub enum FileRoutingError {
    #[error(transparent)]
    Processing(#[from] FileProcessingError),
    #[error(transparent)]
    Upload(#[from] FileUploadError),
    #[error(
        "file `{path}` is {size_mb:.1} MB and exceeds upload limit of {max_mb:.1} MB for this provider"
    )]
    UploadTooLarge {
        path: String,
        size_mb: f64,
        max_mb: f64,
    },
    #[error(
        "file `{path}` is {size_mb:.1} MB and exceeds inline limit of {max_mb:.1} MB while upload is unavailable"
    )]
    InlineTooLargeWithoutUploader {
        path: String,
        size_mb: f64,
        max_mb: f64,
    },
}

fn inline_content_item(path: &Path) -> Result<ContentItem, FileRoutingError> {
    let processed = encode_inline_cached(path)?;
    Ok(ContentItem::inline_file(
        processed.base64,
        processed.mime_type,
        Some(processed.filename),
    ))
}

pub async fn route_file_input(
    path: &Path,
    capabilities: &FileCapabilityConfig,
    upload_service: Option<&dyn FileUploadService>,
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
) -> Result<ContentItem, FileRoutingError> {
    let metadata = analyze_file(path)?;
    let path_str = path.display().to_string();

    if metadata.size_bytes > capabilities.max_upload_file_size {
        return Err(FileRoutingError::UploadTooLarge {
            path: path_str,
            size_mb: bytes_to_megabytes(metadata.size_bytes),
            max_mb: bytes_to_megabytes(capabilities.max_upload_file_size),
        });
    }

    let always_inline_max = capabilities
        .always_inline_max
        .min(capabilities.max_inline_file_size);
    if metadata.size_bytes <= always_inline_max {
        return inline_content_item(path);
    }

    if metadata.size_bytes <= capabilities.max_inline_file_size {
        if let Some(uploader) = upload_service {
            match uploader
                .upload_file(client, path, &metadata.mime_type, api_key, base_url)
                .await
            {
                Ok(uploaded) => {
                    return Ok(ContentItem::file_ref(
                        uploaded.file_id,
                        metadata.mime_type,
                        Some(metadata.filename),
                    ));
                }
                Err(error) => {
                    warn!(
                        path = %path.display(),
                        %error,
                        "file upload failed for middle-tier file; falling back to inline"
                    );
                }
            }
        }

        return inline_content_item(path);
    }

    if let Some(uploader) = upload_service {
        let uploaded = uploader
            .upload_file(client, path, &metadata.mime_type, api_key, base_url)
            .await?;
        return Ok(ContentItem::file_ref(
            uploaded.file_id,
            metadata.mime_type,
            Some(metadata.filename),
        ));
    }

    Err(FileRoutingError::InlineTooLargeWithoutUploader {
        path: path_str,
        size_mb: bytes_to_megabytes(metadata.size_bytes),
        max_mb: bytes_to_megabytes(capabilities.max_inline_file_size),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use async_trait::async_trait;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;
    use crate::file_support::upload::UploadedFile;

    enum MockOutcome {
        Success(String),
        Failure,
    }

    struct MockUploadService {
        calls: Arc<AtomicUsize>,
        outcome: MockOutcome,
    }

    #[async_trait]
    impl FileUploadService for MockUploadService {
        async fn upload_file(
            &self,
            _client: &reqwest::Client,
            file_path: &Path,
            _mime_type: &str,
            _api_key: &str,
            _base_url: &str,
        ) -> Result<UploadedFile, FileUploadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);

            match &self.outcome {
                MockOutcome::Success(file_id) => Ok(UploadedFile {
                    file_id: file_id.clone(),
                    provider: "mock".to_string(),
                    expires_at: None,
                    source_path: file_path.to_path_buf(),
                }),
                MockOutcome::Failure => Err(FileUploadError::Request("upload failed".to_string())),
            }
        }
    }

    fn capabilities() -> FileCapabilityConfig {
        FileCapabilityConfig {
            supports_pdf: true,
            always_inline_max: 16,
            max_inline_file_size: 64,
            max_upload_file_size: 128,
            max_inline_payload_bytes: 256,
        }
    }

    fn create_pdf_file(dir: &TempDir, name: &str, size_bytes: u64) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open pdf")
            .set_len(size_bytes)
            .expect("set size");
        path
    }

    fn assert_inline(item: &ContentItem) {
        match item {
            ContentItem::InputFile {
                file_data, file_id, ..
            } => {
                assert!(file_data.is_some());
                assert_eq!(file_id, &None);
            }
            other => panic!("expected input file, found {other:?}"),
        }
    }

    fn assert_file_ref(item: &ContentItem, expected_file_id: &str) {
        match item {
            ContentItem::InputFile {
                file_data, file_id, ..
            } => {
                assert_eq!(file_data, &None);
                assert_eq!(file_id.as_deref(), Some(expected_file_id));
            }
            other => panic!("expected input file, found {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier1_small_file_always_inlines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "small.pdf", 12);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-1".to_string()),
        };

        let client = reqwest::Client::new();
        let item = route_file_input(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("route file");

        assert_inline(&item);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier2_middle_file_prefers_upload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "middle.pdf", 32);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-tier2".to_string()),
        };

        let client = reqwest::Client::new();
        let item = route_file_input(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("route file");

        assert_file_ref(&item, "file-tier2");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier2_upload_failure_falls_back_to_inline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "middle.pdf", 32);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Failure,
        };

        let client = reqwest::Client::new();
        let item = route_file_input(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("route file");

        assert_inline(&item);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier2_without_uploader_falls_back_to_inline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "middle.pdf", 32);

        let client = reqwest::Client::new();
        let item = route_file_input(
            &path,
            &capabilities(),
            None,
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("route file");

        assert_inline(&item);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier3_large_file_requires_upload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-tier3".to_string()),
        };

        let client = reqwest::Client::new();
        let item = route_file_input(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("route file");

        assert_file_ref(&item, "file-tier3");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier3_without_uploader_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);

        let client = reqwest::Client::new();
        let error = route_file_input(
            &path,
            &capabilities(),
            None,
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect_err("routing should fail");

        assert!(matches!(
            error,
            FileRoutingError::InlineTooLargeWithoutUploader { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tier3_upload_failure_surfaces_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Failure,
        };

        let client = reqwest::Client::new();
        let error = route_file_input(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect_err("routing should fail");

        assert!(matches!(error, FileRoutingError::Upload(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn file_over_upload_limit_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "too-large.pdf", 129);

        let client = reqwest::Client::new();
        let error = route_file_input(
            &path,
            &capabilities(),
            None,
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect_err("routing should fail");

        assert!(matches!(error, FileRoutingError::UploadTooLarge { .. }));
    }
}
