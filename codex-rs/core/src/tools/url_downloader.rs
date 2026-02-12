use crate::default_client::default_headers;
use crate::default_client::get_codex_user_agent;
use crate::tools::url_validation::UrlValidationError;
use crate::tools::url_validation::UrlValidationOptions;
use crate::tools::url_validation::ValidatedUrl;
use crate::tools::url_validation::validate_parsed_url;
use codex_utils_file::FileMetadata;
use codex_utils_file::MAX_FILE_SIZE;
use codex_utils_file::analyze_file;
use futures::StreamExt;
use http::header::ACCEPT;
use http::header::LOCATION;
use reqwest::Client;
use reqwest::StatusCode;
use sha2::Digest;
use sha2::Sha256;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::task::spawn_blocking;
use tokio::time::timeout;

const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const STALL_TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_TTL: Duration = Duration::from_secs(60 * 60); // 1 hour
pub(crate) const DEFAULT_MAX_DOWNLOAD_CONCURRENCY: usize = 4;

#[derive(Debug, Clone)]
pub(crate) struct UrlDownloadRequest {
    pub(crate) url: ValidatedUrl,
    pub(crate) filename_hint: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct UrlDownloadSuccess {
    pub(crate) url: ValidatedUrl,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UrlDownloadFailure {
    pub(crate) redacted_url: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone)]
pub(crate) enum UrlDownloadOutcome {
    Success(UrlDownloadSuccess),
    Failure(UrlDownloadFailure),
}

#[derive(Debug, Error)]
pub(crate) enum UrlDownloadError {
    #[error(transparent)]
    Validation(#[from] UrlValidationError),
    #[error("request failed: {message}")]
    Request { message: String },
    #[error("HTTP {status} while fetching remote file")]
    HttpStatus { status: StatusCode },
    #[error("redirect response missing Location header")]
    MissingRedirectLocation,
    #[error("redirect target is invalid: {message}")]
    InvalidRedirectLocation { message: String },
    #[error("redirect limit exceeded ({max_redirects})")]
    TooManyRedirects { max_redirects: usize },
    #[error("download exceeded max size of {max_bytes} bytes")]
    TooLarge { max_bytes: u64 },
    #[error("download stalled without progress")]
    StallTimeout,
    #[error("failed to write downloaded file: {message}")]
    WriteError { message: String },
    #[error("downloaded file is not a valid PDF: {message}")]
    InvalidPdf { message: String },
}

pub(crate) async fn download_url_files_to_cache(
    codex_home: &Path,
    requests: Vec<UrlDownloadRequest>,
    max_concurrency: usize,
) -> Vec<UrlDownloadOutcome> {
    download_url_files_to_cache_with_validation_options(
        codex_home,
        requests,
        max_concurrency,
        UrlValidationOptions::default(),
    )
    .await
}

async fn download_url_files_to_cache_with_validation_options(
    codex_home: &Path,
    requests: Vec<UrlDownloadRequest>,
    max_concurrency: usize,
    validation_options: UrlValidationOptions,
) -> Vec<UrlDownloadOutcome> {
    if requests.is_empty() {
        return Vec::new();
    }

    let client = build_downloader_client();
    let concurrency = max_concurrency.max(1);

    let mut results: Vec<(usize, UrlDownloadOutcome)> = futures::stream::iter(requests)
        .enumerate()
        .map(|(idx, request)| {
            let client = client.clone();
            let codex_home = codex_home.to_path_buf();
            async move {
                let redacted_url = request.url.redacted_for_display();
                let outcome = match download_one_url_to_cache(
                    &client,
                    &codex_home,
                    request,
                    validation_options,
                )
                .await
                {
                    Ok(success) => UrlDownloadOutcome::Success(success),
                    Err(error) => UrlDownloadOutcome::Failure(UrlDownloadFailure {
                        redacted_url,
                        reason: error.to_string(),
                    }),
                };
                (idx, outcome)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    results.sort_by_key(|(idx, _)| *idx);
    results.into_iter().map(|(_, outcome)| outcome).collect()
}

async fn download_one_url_to_cache(
    client: &Client,
    codex_home: &Path,
    request: UrlDownloadRequest,
    validation_options: UrlValidationOptions,
) -> Result<UrlDownloadSuccess, UrlDownloadError> {
    let cache_entry_dir = cache_entry_dir(codex_home, request.url.normalized_cache_key());
    let filename = request
        .url
        .derive_pdf_filename(request.filename_hint.as_deref());
    let final_path = cache_entry_dir.join(filename);

    if is_cached_pdf_valid(&final_path).await? {
        return Ok(UrlDownloadSuccess {
            url: request.url,
            path: final_path,
        });
    }

    if fs::try_exists(&cache_entry_dir).await.unwrap_or(false) {
        let mut entries = fs::read_dir(&cache_entry_dir).await.map_err(write_error)?;
        while let Some(entry) = entries.next_entry().await.map_err(write_error)? {
            let candidate = entry.path();
            if candidate == final_path {
                continue;
            }
            if candidate
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".part"))
            {
                continue;
            }
            if is_cached_pdf_valid(&candidate).await? {
                return Ok(UrlDownloadSuccess {
                    url: request.url,
                    path: candidate,
                });
            }
        }
    }

    fs::create_dir_all(&cache_entry_dir)
        .await
        .map_err(|error| UrlDownloadError::WriteError {
            message: error.to_string(),
        })?;

    let mut current_url = request.url.as_url().clone();
    for hop in 0..=MAX_REDIRECTS {
        let response = client
            .get(current_url.clone())
            .header(ACCEPT, "application/pdf")
            .send()
            .await
            .map_err(|error| UrlDownloadError::Request {
                message: sanitize_request_error(&error),
            })?;

        if response.status().is_redirection() {
            if hop >= MAX_REDIRECTS {
                return Err(UrlDownloadError::TooManyRedirects {
                    max_redirects: MAX_REDIRECTS,
                });
            }
            let location = response
                .headers()
                .get(LOCATION)
                .ok_or(UrlDownloadError::MissingRedirectLocation)?
                .to_str()
                .map_err(|error| UrlDownloadError::InvalidRedirectLocation {
                    message: error.to_string(),
                })?;
            let next_url = current_url.join(location).map_err(|error| {
                UrlDownloadError::InvalidRedirectLocation {
                    message: error.to_string(),
                }
            })?;
            let validated = validate_parsed_url(next_url, validation_options).await?;
            current_url = validated.into_url();
            continue;
        }

        if !response.status().is_success() {
            return Err(UrlDownloadError::HttpStatus {
                status: response.status(),
            });
        }

        write_response_to_path(response, &final_path).await?;
        validate_downloaded_pdf(&final_path).await?;
        return Ok(UrlDownloadSuccess {
            url: request.url,
            path: final_path,
        });
    }

    Err(UrlDownloadError::TooManyRedirects {
        max_redirects: MAX_REDIRECTS,
    })
}

fn build_downloader_client() -> Client {
    let user_agent = get_codex_user_agent();

    reqwest::Client::builder()
        .default_headers(default_headers())
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build downloader HTTP client")
}

async fn is_cached_pdf_valid(path: &Path) -> Result<bool, UrlDownloadError> {
    if !fs::try_exists(path).await.unwrap_or(false) {
        return Ok(false);
    }

    if is_cache_entry_stale(path).await {
        let _ = fs::remove_file(path).await;
        remove_dir_if_empty(path.parent()).await;
        return Ok(false);
    }

    match analyze_pdf(path).await {
        Ok(metadata) if metadata.mime_type == "application/pdf" => Ok(true),
        _ => {
            let _ = fs::remove_file(path).await;
            Ok(false)
        }
    }
}

async fn is_cache_entry_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path).await else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified.elapsed().unwrap_or(Duration::ZERO) > CACHE_TTL
}

async fn remove_dir_if_empty(dir: Option<&Path>) {
    if let Some(dir) = dir {
        // fs::remove_dir only succeeds if the directory is empty.
        let _ = fs::remove_dir(dir).await;
    }
}

async fn write_response_to_path(
    response: reqwest::Response,
    final_path: &Path,
) -> Result<(), UrlDownloadError> {
    let temp_path = temp_path_for(final_path);
    if fs::try_exists(&temp_path).await.unwrap_or(false) {
        let _ = fs::remove_file(&temp_path).await;
    }

    let write_result = async {
        let mut file = fs::File::create(&temp_path).await.map_err(write_error)?;
        let mut stream = response.bytes_stream();
        let mut total_written = 0_u64;

        while let Some(chunk_result) = timeout(STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| UrlDownloadError::StallTimeout)?
        {
            let chunk = chunk_result.map_err(|error| UrlDownloadError::Request {
                message: sanitize_request_error(&error),
            })?;
            total_written = total_written.saturating_add(chunk.len() as u64);
            if total_written > MAX_FILE_SIZE {
                return Err(UrlDownloadError::TooLarge {
                    max_bytes: MAX_FILE_SIZE,
                });
            }
            file.write_all(&chunk).await.map_err(write_error)?;
        }

        file.flush().await.map_err(write_error)?;
        file.sync_all().await.map_err(write_error)?;
        drop(file);

        fs::rename(&temp_path, final_path)
            .await
            .map_err(write_error)?;

        Ok(())
    }
    .await;

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path).await;
    }

    write_result
}

async fn validate_downloaded_pdf(path: &Path) -> Result<(), UrlDownloadError> {
    let analysis = analyze_pdf(path).await?;
    if analysis.mime_type != "application/pdf" {
        let _ = fs::remove_file(path).await;
        return Err(invalid_pdf_error(format!(
            "detected MIME type `{}`",
            analysis.mime_type
        )));
    }

    Ok(())
}

async fn analyze_pdf(path: &Path) -> Result<FileMetadata, UrlDownloadError> {
    let path_buf = path.to_path_buf();
    spawn_blocking(move || analyze_file(&path_buf))
        .await
        .map_err(invalid_pdf_error)?
        .map_err(invalid_pdf_error)
}

fn write_error(error: std::io::Error) -> UrlDownloadError {
    UrlDownloadError::WriteError {
        message: error.to_string(),
    }
}

fn invalid_pdf_error(error: impl ToString) -> UrlDownloadError {
    UrlDownloadError::InvalidPdf {
        message: error.to_string(),
    }
}

/// Extracts an error description from a `reqwest::Error` without including the URL,
/// which may contain sensitive query parameters or tokens.
fn sanitize_request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "request timed out".to_string();
    }
    if error.is_connect() {
        return "connection failed".to_string();
    }
    if error.is_redirect() {
        return "too many redirects".to_string();
    }
    if error.is_request() {
        return "request error".to_string();
    }
    if error.is_body() {
        return "response body error".to_string();
    }
    if error.is_decode() {
        return "response decode error".to_string();
    }
    if let Some(status) = error.status() {
        return format!("HTTP {status}");
    }
    "unknown request error".to_string()
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file.pdf".to_string());
    path.with_file_name(format!("{file_name}.part"))
}

fn cache_entry_dir(codex_home: &Path, normalized_cache_key: &str) -> PathBuf {
    codex_home
        .join("cache")
        .join("remote-files")
        .join(url_hash(normalized_cache_key))
}

fn url_hash(normalized_cache_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_cache_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::url_validation::UrlValidationOptions;
    use crate::tools::url_validation::validated_url_for_test;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    fn validated(url: &str) -> ValidatedUrl {
        validated_url_for_test(url)
    }

    async fn download_with_test_options(
        codex_home: &std::path::Path,
        requests: Vec<UrlDownloadRequest>,
    ) -> Vec<UrlDownloadOutcome> {
        download_url_files_to_cache_with_validation_options(
            codex_home,
            requests,
            1,
            UrlValidationOptions {
                allow_http: true,
                resolve_dns: false,
                allow_non_public_hosts: true,
            },
        )
        .await
    }

    #[tokio::test]
    async fn follows_redirects_and_revalidates_targets() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/start.pdf"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/docs/final.pdf"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/docs/final.pdf"))
            .and(header("accept", "application/pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4\nhello".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let request = UrlDownloadRequest {
            url: validated(&format!("{}/start.pdf", server.uri())),
            filename_hint: None,
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], UrlDownloadOutcome::Success(_)));
    }

    #[tokio::test]
    async fn enforces_size_limit_and_cleans_up_part_file() {
        let server = MockServer::start().await;
        let oversized = vec![b'a'; (MAX_FILE_SIZE as usize).saturating_add(1024)];
        Mock::given(method("GET"))
            .and(path("/large.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(oversized))
            .expect(1)
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let request = UrlDownloadRequest {
            url: validated(&format!("{}/large.pdf", server.uri())),
            filename_hint: Some("large.pdf".to_string()),
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Failure(failure) = &outcomes[0] else {
            panic!("expected failure");
        };
        assert!(failure.reason.contains("max size"));

        let cache_root = codex_home.path().join("cache").join("remote-files");
        if cache_root.exists() {
            let mut part_files = Vec::new();
            let mut stack = vec![cache_root];
            while let Some(dir) = stack.pop() {
                let entries = std::fs::read_dir(&dir).expect("read_dir");
                for entry in entries {
                    let entry = entry.expect("entry");
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().ends_with(".part"))
                    {
                        part_files.push(path);
                    }
                }
            }
            assert!(
                part_files.is_empty(),
                "unexpected .part files: {part_files:?}"
            );
        }
    }

    #[tokio::test]
    async fn reuses_cache_for_same_normalized_url() {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_mock = Arc::clone(&hits);
        Mock::given(method("GET"))
            .and(path("/cached.pdf"))
            .respond_with({
                let hits_for_mock = Arc::clone(&hits_for_mock);
                move |_req: &wiremock::Request| {
                    hits_for_mock.fetch_add(1, Ordering::SeqCst);
                    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4\ncached".to_vec())
                }
            })
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let first = UrlDownloadRequest {
            url: validated(&format!("{}/cached.pdf#page=1", server.uri())),
            filename_hint: Some("cached.pdf".to_string()),
        };
        let second = UrlDownloadRequest {
            url: validated(&format!("{}/cached.pdf", server.uri())),
            filename_hint: Some("cached.pdf".to_string()),
        };

        let outcomes1 = download_with_test_options(codex_home.path(), vec![first]).await;
        let outcomes2 = download_with_test_options(codex_home.path(), vec![second]).await;
        assert!(matches!(outcomes1[0], UrlDownloadOutcome::Success(_)));
        assert!(matches!(outcomes2[0], UrlDownloadOutcome::Success(_)));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reuses_cache_when_filename_hint_changes_for_same_url() {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_mock = Arc::clone(&hits);
        Mock::given(method("GET"))
            .and(path("/filename-drift.pdf"))
            .respond_with({
                let hits_for_mock = Arc::clone(&hits_for_mock);
                move |_req: &wiremock::Request| {
                    hits_for_mock.fetch_add(1, Ordering::SeqCst);
                    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4\ncached".to_vec())
                }
            })
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let first = UrlDownloadRequest {
            url: validated(&format!("{}/filename-drift.pdf", server.uri())),
            filename_hint: Some("first-name.pdf".to_string()),
        };
        let second = UrlDownloadRequest {
            url: validated(&format!("{}/filename-drift.pdf", server.uri())),
            filename_hint: Some("second-name.pdf".to_string()),
        };

        let outcomes1 = download_with_test_options(codex_home.path(), vec![first]).await;
        let outcomes2 = download_with_test_options(codex_home.path(), vec![second]).await;

        let UrlDownloadOutcome::Success(first_success) = &outcomes1[0] else {
            panic!("expected first request to succeed");
        };
        let UrlDownloadOutcome::Success(second_success) = &outcomes2[0] else {
            panic!("expected second request to succeed");
        };
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(second_success.path, first_success.path);
    }

    #[tokio::test]
    async fn redirect_without_location_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/no-location.pdf"))
            .respond_with(ResponseTemplate::new(302))
            .expect(1)
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let request = UrlDownloadRequest {
            url: validated(&format!("{}/no-location.pdf", server.uri())),
            filename_hint: None,
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Failure(failure) = &outcomes[0] else {
            panic!("expected failure");
        };
        assert!(failure.reason.contains("missing Location header"));
    }

    #[tokio::test]
    async fn non_pdf_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/not-a-pdf.pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not-a-pdf".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let request = UrlDownloadRequest {
            url: validated(&format!("{}/not-a-pdf.pdf", server.uri())),
            filename_hint: None,
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Failure(failure) = &outcomes[0] else {
            panic!("expected failure");
        };
        assert!(failure.reason.contains("not a valid PDF"));
    }

    #[tokio::test]
    async fn corrupt_cached_file_is_cleaned_up() {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_mock = Arc::clone(&hits);
        Mock::given(method("GET"))
            .and(path("/corrupt-test.pdf"))
            .respond_with({
                let hits_for_mock = Arc::clone(&hits_for_mock);
                move |_req: &wiremock::Request| {
                    hits_for_mock.fetch_add(1, Ordering::SeqCst);
                    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4\nvalid".to_vec())
                }
            })
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let url = validated(&format!("{}/corrupt-test.pdf", server.uri()));

        // Pre-populate cache dir with a corrupt file at the expected path.
        let dir = cache_entry_dir(codex_home.path(), url.normalized_cache_key());
        fs::create_dir_all(&dir).await.expect("create cache dir");
        let filename = url.derive_pdf_filename(Some("corrupt-test.pdf"));
        let corrupt_path = dir.join(&filename);
        fs::write(&corrupt_path, b"corrupt-not-a-pdf")
            .await
            .expect("write corrupt file");

        let request = UrlDownloadRequest {
            url,
            filename_hint: Some("corrupt-test.pdf".to_string()),
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Success(success) = &outcomes[0] else {
            panic!("expected success after re-download");
        };
        // Corrupt file should have been cleaned up and re-downloaded.
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let content = fs::read(&success.path).await.expect("read cached file");
        assert_eq!(content, b"%PDF-1.4\nvalid");
    }

    #[tokio::test]
    async fn http_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/error.pdf"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let request = UrlDownloadRequest {
            url: validated(&format!("{}/error.pdf", server.uri())),
            filename_hint: None,
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Failure(failure) = &outcomes[0] else {
            panic!("expected failure");
        };
        assert!(failure.reason.contains("500"));
    }

    #[tokio::test]
    async fn stale_cache_entry_triggers_redownload() {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_mock = Arc::clone(&hits);
        Mock::given(method("GET"))
            .and(path("/stale-test.pdf"))
            .respond_with({
                let hits_for_mock = Arc::clone(&hits_for_mock);
                move |_req: &wiremock::Request| {
                    hits_for_mock.fetch_add(1, Ordering::SeqCst);
                    ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.4\nfresh".to_vec())
                }
            })
            .mount(&server)
            .await;

        let codex_home = tempfile::tempdir().expect("tempdir");
        let url = validated(&format!("{}/stale-test.pdf", server.uri()));

        // Pre-populate cache with a valid PDF that has an old mtime.
        let dir = cache_entry_dir(codex_home.path(), url.normalized_cache_key());
        fs::create_dir_all(&dir).await.expect("create cache dir");
        let filename = url.derive_pdf_filename(Some("stale-test.pdf"));
        let cached_path = dir.join(&filename);
        fs::write(&cached_path, b"%PDF-1.4\nstale")
            .await
            .expect("write stale file");

        // Backdate the mtime to beyond the TTL.
        let past = std::time::SystemTime::now() - CACHE_TTL - Duration::from_secs(60);
        let file = std::fs::File::options()
            .write(true)
            .open(&cached_path)
            .expect("open for set_times");
        file.set_times(
            std::fs::FileTimes::new()
                .set_accessed(past)
                .set_modified(past),
        )
        .expect("set_times");
        drop(file);

        let request = UrlDownloadRequest {
            url,
            filename_hint: Some("stale-test.pdf".to_string()),
        };

        let outcomes = download_with_test_options(codex_home.path(), vec![request]).await;
        assert_eq!(outcomes.len(), 1);
        let UrlDownloadOutcome::Success(success) = &outcomes[0] else {
            panic!("expected success");
        };
        // Should have re-downloaded (stale entry was evicted).
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let content = fs::read(&success.path).await.expect("read file");
        assert_eq!(content, b"%PDF-1.4\nfresh");
    }
}
