use std::path::Path;

use codex_utils_file::FileMetadata;
use codex_utils_file::FileProcessingError;
use codex_utils_file::MAX_FILE_SIZE;
use codex_utils_file::analyze_file;
use codex_utils_file::bytes_to_megabytes;
use thiserror::Error;
use tracing::warn;

use super::capabilities::FileCapabilityConfig;
use super::upload::FileUploadError;
use super::upload::FileUploadService;
use super::upload::UploadedFile;

#[derive(Debug, Error)]
pub enum FileRoutingError {
    #[error("provider does not support PDF attachments")]
    PdfNotSupported,
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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RoutingDecision {
    Inline,
    PreferUpload,
    MustUpload,
}

fn decide_routing(
    path: &Path,
    metadata: &FileMetadata,
    capabilities: &FileCapabilityConfig,
) -> Result<RoutingDecision, FileRoutingError> {
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
        return Ok(RoutingDecision::Inline);
    }

    if metadata.size_bytes <= capabilities.max_inline_file_size {
        return Ok(RoutingDecision::PreferUpload);
    }

    Ok(RoutingDecision::MustUpload)
}

/// Result of `maybe_upload_file`: the upload outcome plus any non-fatal warning.
#[derive(Debug, Clone, PartialEq)]
pub struct MaybeUploadResult {
    pub uploaded: Option<(UploadedFile, FileMetadata)>,
    /// Non-fatal warning to surface to the user (e.g. tier-2 upload fallback).
    pub warning: Option<String>,
}

pub async fn maybe_upload_file(
    path: &Path,
    capabilities: &FileCapabilityConfig,
    upload_service: Option<&dyn FileUploadService>,
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
) -> Result<MaybeUploadResult, FileRoutingError> {
    if !capabilities.supports_pdf {
        return Err(FileRoutingError::PdfNotSupported);
    }

    // Avoid redundant stat + magic-byte probing for always-inline files. The inline encoding path
    // will validate magic bytes, so for tier-1 files we only need a size check here.
    let size_bytes = std::fs::metadata(path)
        .map_err(|source| FileProcessingError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if size_bytes > MAX_FILE_SIZE {
        return Err(FileProcessingError::TooLarge {
            path: path.to_path_buf(),
            size_mb: bytes_to_megabytes(size_bytes),
            max_mb: MAX_FILE_SIZE / (1024 * 1024),
        }
        .into());
    }
    if size_bytes > capabilities.max_upload_file_size {
        return Err(FileRoutingError::UploadTooLarge {
            path: path.display().to_string(),
            size_mb: bytes_to_megabytes(size_bytes),
            max_mb: bytes_to_megabytes(capabilities.max_upload_file_size),
        });
    }

    let no_upload = Ok(MaybeUploadResult {
        uploaded: None,
        warning: None,
    });

    let always_inline_max = capabilities
        .always_inline_max
        .min(capabilities.max_inline_file_size);
    if size_bytes <= always_inline_max {
        return no_upload;
    }

    let metadata = analyze_file(path)?;
    match decide_routing(path, &metadata, capabilities)? {
        RoutingDecision::Inline => no_upload,
        RoutingDecision::PreferUpload => {
            let Some(uploader) = upload_service else {
                return no_upload;
            };

            match uploader
                .upload_file(client, path, &metadata.mime_type, api_key, base_url)
                .await
            {
                Ok(uploaded) => Ok(MaybeUploadResult {
                    uploaded: Some((uploaded, metadata)),
                    warning: None,
                }),
                Err(error) => {
                    warn!(
                        path = %path.display(),
                        %error,
                        "file upload failed for middle-tier file; falling back to inline"
                    );
                    Ok(MaybeUploadResult {
                        uploaded: None,
                        warning: Some(format!(
                            "Upload failed for '{}'; sending inline instead. Error: {error}",
                            path.display()
                        )),
                    })
                }
            }
        }
        RoutingDecision::MustUpload => {
            let Some(uploader) = upload_service else {
                return Err(FileRoutingError::InlineTooLargeWithoutUploader {
                    path: path.display().to_string(),
                    size_mb: bytes_to_megabytes(metadata.size_bytes),
                    max_mb: bytes_to_megabytes(capabilities.max_inline_file_size),
                });
            };

            let uploaded = uploader
                .upload_file(client, path, &metadata.mime_type, api_key, base_url)
                .await?;
            Ok(MaybeUploadResult {
                uploaded: Some((uploaded, metadata)),
                warning: None,
            })
        }
    }
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

        async fn delete_file(
            &self,
            _client: &reqwest::Client,
            _file_id: &str,
            _api_key: &str,
            _base_url: &str,
        ) -> Result<(), FileUploadError> {
            Ok(())
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

    fn unsupported_capabilities() -> FileCapabilityConfig {
        FileCapabilityConfig {
            supports_pdf: false,
            ..capabilities()
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

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_tier1_small_file_skips_upload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "small.pdf", 12);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-1".to_string()),
        };

        let client = reqwest::Client::new();
        let result = maybe_upload_file(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("maybe upload");

        assert_eq!(result.uploaded, None);
        assert_eq!(result.warning, None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_rejects_provider_without_pdf_support() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "small.pdf", 12);

        let client = reqwest::Client::new();
        let error = maybe_upload_file(
            &path,
            &unsupported_capabilities(),
            None,
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect_err("routing should fail");

        assert!(matches!(error, FileRoutingError::PdfNotSupported));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_prefers_upload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "middle.pdf", 32);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-tier2".to_string()),
        };

        let client = reqwest::Client::new();
        let result = maybe_upload_file(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("maybe upload");

        let Some((uploaded, metadata)) = result.uploaded else {
            panic!("expected upload result");
        };
        assert_eq!(uploaded.file_id, "file-tier2");
        assert_eq!(uploaded.source_path, path);
        assert_eq!(metadata.mime_type, "application/pdf");
        assert_eq!(metadata.filename, "middle.pdf");
        assert_eq!(result.warning, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_upload_failure_falls_back_to_inline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "middle.pdf", 32);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Failure,
        };

        let client = reqwest::Client::new();
        let result = maybe_upload_file(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("maybe upload");

        assert_eq!(result.uploaded, None);
        assert!(
            result.warning.is_some(),
            "fallback should produce a warning"
        );
        assert!(result.warning.unwrap().contains("middle.pdf"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_tier3_large_file_requires_upload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Success("file-tier3".to_string()),
        };

        let client = reqwest::Client::new();
        let result = maybe_upload_file(
            &path,
            &capabilities(),
            Some(&uploader),
            &client,
            "api-key",
            "https://example.test",
        )
        .await
        .expect("maybe upload");

        let Some((uploaded, metadata)) = result.uploaded else {
            panic!("expected upload result");
        };
        assert_eq!(uploaded.file_id, "file-tier3");
        assert_eq!(uploaded.source_path, path);
        assert_eq!(metadata.filename, "large.pdf");
        assert_eq!(result.warning, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn maybe_upload_file_tier3_without_uploader_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);

        let client = reqwest::Client::new();
        let error = maybe_upload_file(
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
    async fn maybe_upload_file_tier3_upload_failure_surfaces_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "large.pdf", 96);
        let calls = Arc::new(AtomicUsize::new(0));
        let uploader = MockUploadService {
            calls: Arc::clone(&calls),
            outcome: MockOutcome::Failure,
        };

        let client = reqwest::Client::new();
        let error = maybe_upload_file(
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
    async fn maybe_upload_file_over_upload_limit_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = create_pdf_file(&dir, "too-large.pdf", 129);

        let client = reqwest::Client::new();
        let error = maybe_upload_file(
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
