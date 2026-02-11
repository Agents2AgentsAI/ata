use crate::default_client::get_codex_user_agent;
use crate::default_client::originator;
use crate::tools::url_validation::UrlValidationError;
use crate::tools::url_validation::UrlValidationOptions;
use crate::tools::url_validation::ValidatedUrl;
use crate::tools::url_validation::validate_parsed_url;
use codex_utils_file::MAX_FILE_SIZE;
use codex_utils_file::analyze_file;
use futures::StreamExt;
use http::header::ACCEPT;
use http::header::LOCATION;
use reqwest::Client;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
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
                message: error.to_string(),
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
    let mut headers = HeaderMap::new();
    headers.insert("originator", originator().header_value);
    let user_agent = get_codex_user_agent();

    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn is_cached_pdf_valid(path: &Path) -> Result<bool, UrlDownloadError> {
    if !path.exists() {
        return Ok(false);
    }

    let path_buf = path.to_path_buf();
    let analysis = spawn_blocking(move || analyze_file(&path_buf))
        .await
        .map_err(|error| UrlDownloadError::InvalidPdf {
            message: error.to_string(),
        })?;

    match analysis {
        Ok(metadata) if metadata.mime_type == "application/pdf" => Ok(true),
        _ => {
            let _ = fs::remove_file(path).await;
            Ok(false)
        }
    }
}

async fn write_response_to_path(
    response: reqwest::Response,
    final_path: &Path,
) -> Result<(), UrlDownloadError> {
    let temp_path = temp_path_for(final_path);
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path).await;
    }

    let write_result = async {
        let mut file =
            fs::File::create(&temp_path)
                .await
                .map_err(|error| UrlDownloadError::WriteError {
                    message: error.to_string(),
                })?;
        let mut stream = response.bytes_stream();
        let mut total_written = 0_u64;

        while let Some(chunk_result) = timeout(STALL_TIMEOUT, stream.next())
            .await
            .map_err(|_| UrlDownloadError::StallTimeout)?
        {
            let chunk = chunk_result.map_err(|error| UrlDownloadError::Request {
                message: error.to_string(),
            })?;
            total_written = total_written.saturating_add(chunk.len() as u64);
            if total_written > MAX_FILE_SIZE {
                return Err(UrlDownloadError::TooLarge {
                    max_bytes: MAX_FILE_SIZE,
                });
            }
            file.write_all(&chunk)
                .await
                .map_err(|error| UrlDownloadError::WriteError {
                    message: error.to_string(),
                })?;
        }

        file.flush()
            .await
            .map_err(|error| UrlDownloadError::WriteError {
                message: error.to_string(),
            })?;
        file.sync_all()
            .await
            .map_err(|error| UrlDownloadError::WriteError {
                message: error.to_string(),
            })?;
        drop(file);

        fs::rename(&temp_path, final_path)
            .await
            .map_err(|error| UrlDownloadError::WriteError {
                message: error.to_string(),
            })?;

        Ok(())
    }
    .await;

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path).await;
    }

    write_result
}

async fn validate_downloaded_pdf(path: &Path) -> Result<(), UrlDownloadError> {
    let path_buf = path.to_path_buf();
    let analysis = spawn_blocking(move || analyze_file(&path_buf))
        .await
        .map_err(|error| UrlDownloadError::InvalidPdf {
            message: error.to_string(),
        })?
        .map_err(|error| UrlDownloadError::InvalidPdf {
            message: error.to_string(),
        })?;

    if analysis.mime_type != "application/pdf" {
        let _ = fs::remove_file(path).await;
        return Err(UrlDownloadError::InvalidPdf {
            message: format!("detected MIME type `{}`", analysis.mime_type),
        });
    }

    Ok(())
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
}
