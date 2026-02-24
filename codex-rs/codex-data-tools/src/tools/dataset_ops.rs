use std::path::Path;
use std::time::Instant;

use crate::clients::HuggingFaceClient;
use crate::clients::KaggleClient;
use crate::clients::huggingface::HfSearchRequest;
use crate::clients::kaggle::KaggleSearchRequest;
use crate::config::DataConfig;
use crate::error::DataError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::types::Dataset;
use crate::types::DatasetDetail;
use crate::types::DatasetDownloadParams;
use crate::types::DatasetDownloadResult;
use crate::types::DatasetFilesParams;
use crate::types::DatasetFilesResult;
use crate::types::DatasetGetParams;
use crate::types::DatasetSearchParams;
use crate::types::DatasetSearchResult;
use crate::types::KaggleCompetitionDownloadParams;
use crate::types::KaggleCompetitionDownloadResult;
use crate::types::KaggleCompetitionFilesParams;
use crate::types::KaggleCompetitionFilesResult;
use crate::types::KaggleCompetitionsParams;
use crate::types::KaggleCompetitionsResult;
use crate::types::SourceResult;
use crate::types::SourceStatus;

/// Search for datasets across configured sources
pub async fn dataset_search(
    http: &HttpClient,
    config: &DataConfig,
    params: DatasetSearchParams,
) -> Result<DatasetSearchResult> {
    let mut all_datasets: Vec<Dataset> = Vec::new();
    let mut per_source_status: Vec<SourceStatus> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let source_filter = params.source.as_deref();
    let limit = params.limit.unwrap_or(20);

    // Search HuggingFace
    if source_filter.is_none() || source_filter == Some("huggingface") {
        let hf_client = HuggingFaceClient::new(
            http,
            &config.huggingface_base_url,
            config.huggingface_token.as_deref(),
        );

        let search_request = HfSearchRequest {
            query: &params.query,
            author: params.author.as_deref(),
            limit: Some(limit),
            sort: params.sort_by.as_deref(),
            full: true,
        };

        match hf_client.search(&search_request).await {
            Ok(result) => {
                let count = result.items.len();
                all_datasets.extend(result.items);
                per_source_status.push(SourceStatus {
                    source: "huggingface".to_string(),
                    status: SourceResult::Ok { count },
                });
            }
            Err(e) => {
                per_source_status.push(SourceStatus {
                    source: "huggingface".to_string(),
                    status: SourceResult::Error {
                        message: e.to_string(),
                        retryable: e.is_retryable(),
                    },
                });
            }
        }
    }

    // Search Kaggle (if configured)
    if (source_filter.is_none() || source_filter == Some("kaggle")) && config.is_kaggle_configured()
    {
        let kaggle_client = KaggleClient::new(
            http,
            &config.kaggle_base_url,
            config.kaggle_username.as_deref().unwrap_or(""),
            config.kaggle_key.as_deref().unwrap_or(""),
            config.kaggle_api_token.as_deref(),
        );

        let search_request = KaggleSearchRequest {
            query: &params.query,
            sort_by: params.sort_by.as_deref(),
            page_size: Some(limit.min(20)), // Kaggle max is 20
            user: params.author.as_deref(),
            ..Default::default()
        };

        match kaggle_client.search(&search_request).await {
            Ok(result) => {
                let count = result.items.len();
                all_datasets.extend(result.items);
                per_source_status.push(SourceStatus {
                    source: "kaggle".to_string(),
                    status: SourceResult::Ok { count },
                });
            }
            Err(e) => {
                per_source_status.push(SourceStatus {
                    source: "kaggle".to_string(),
                    status: SourceResult::Error {
                        message: e.to_string(),
                        retryable: e.is_retryable(),
                    },
                });
            }
        }
    } else if source_filter == Some("kaggle") && !config.is_kaggle_configured() {
        warnings.push("Kaggle API credentials not configured. Set KAGGLE_API_TOKEN or KAGGLE_USERNAME and KAGGLE_KEY environment variables.".to_string());
    }

    // Sort combined results by downloads if available
    all_datasets.sort_by(|a, b| b.downloads.cmp(&a.downloads));

    // Apply limit to combined results
    let has_more = all_datasets.len() > limit as usize;
    all_datasets.truncate(limit as usize);

    Ok(DatasetSearchResult {
        datasets: all_datasets,
        per_source_status,
        warnings,
        total_available: None,
        has_more,
    })
}

/// Get detailed information about a specific dataset
pub async fn dataset_get(
    http: &HttpClient,
    config: &DataConfig,
    params: DatasetGetParams,
) -> Result<DatasetDetail> {
    let dataset_id = &params.dataset_id;

    // Detect source from ID format
    if dataset_id.starts_with("kaggle:")
        || dataset_id.contains('/') && !dataset_id.contains("huggingface")
    {
        // Try Kaggle format: owner/slug or kaggle:owner/slug
        let id = dataset_id.strip_prefix("kaggle:").unwrap_or(dataset_id);
        let parts: Vec<&str> = id.split('/').collect();

        if parts.len() == 2 && config.is_kaggle_configured() {
            let kaggle_client = KaggleClient::new(
                http,
                &config.kaggle_base_url,
                config.kaggle_username.as_deref().unwrap_or(""),
                config.kaggle_key.as_deref().unwrap_or(""),
                config.kaggle_api_token.as_deref(),
            );

            if let Ok(detail) = kaggle_client.get_dataset_detail(parts[0], parts[1]).await {
                return Ok(detail);
            }
        }
    }

    // Try HuggingFace format: org/dataset or hf:org/dataset
    let hf_id = dataset_id
        .strip_prefix("hf:")
        .or_else(|| dataset_id.strip_prefix("huggingface:"))
        .unwrap_or(dataset_id);

    let hf_client = HuggingFaceClient::new(
        http,
        &config.huggingface_base_url,
        config.huggingface_token.as_deref(),
    );

    hf_client.get_dataset_detail(hf_id).await
}

/// List files in a dataset
pub async fn dataset_list_files(
    http: &HttpClient,
    config: &DataConfig,
    params: DatasetFilesParams,
) -> Result<DatasetFilesResult> {
    let dataset_id = &params.dataset_id;

    // Detect source and get files
    let files = if dataset_id.starts_with("kaggle:") {
        let id = dataset_id.strip_prefix("kaggle:").unwrap_or(dataset_id);
        let parts: Vec<&str> = id.split('/').collect();

        if parts.len() != 2 {
            return Err(DataError::InvalidConfig {
                message: format!(
                    "Invalid Kaggle dataset ID format: {dataset_id}. Expected: owner/slug"
                ),
            });
        }

        if !config.is_kaggle_configured() {
            return Err(DataError::AuthRequired {
                api: crate::rate_limiter::DataApi::Kaggle,
                message: "Kaggle credentials not configured".to_string(),
            });
        }

        let kaggle_client = KaggleClient::new(
            http,
            &config.kaggle_base_url,
            config.kaggle_username.as_deref().unwrap_or(""),
            config.kaggle_key.as_deref().unwrap_or(""),
            config.kaggle_api_token.as_deref(),
        );

        kaggle_client.list_files(parts[0], parts[1]).await?
    } else {
        let hf_id = dataset_id
            .strip_prefix("hf:")
            .or_else(|| dataset_id.strip_prefix("huggingface:"))
            .unwrap_or(dataset_id);

        let hf_client = HuggingFaceClient::new(
            http,
            &config.huggingface_base_url,
            config.huggingface_token.as_deref(),
        );

        hf_client
            .list_files(hf_id, params.revision.as_deref())
            .await?
    };

    let total_size_bytes = files.iter().filter_map(|f| f.size_bytes).sum();

    Ok(DatasetFilesResult {
        files,
        total_size_bytes: Some(total_size_bytes),
    })
}

/// Download a dataset to a local directory
pub async fn dataset_download(
    http: &HttpClient,
    config: &DataConfig,
    params: DatasetDownloadParams,
) -> Result<DatasetDownloadResult> {
    let dataset_id = &params.dataset_id;
    let output_path = Path::new(&params.output_path);
    let revision = params.revision.as_deref().unwrap_or("main");
    let start_time = Instant::now();

    tracing::info!(
        "Starting download of dataset '{}' to '{}'",
        dataset_id,
        params.output_path
    );

    // Create output directory
    std::fs::create_dir_all(output_path).map_err(|e| DataError::Download {
        url: params.output_path.clone(),
        message: format!("Failed to create output directory: {e}"),
    })?;

    let mut total_bytes: u64 = 0;
    let mut files_downloaded: u32 = 0;

    // Detect source and download
    if dataset_id.starts_with("kaggle:") {
        let id = dataset_id.strip_prefix("kaggle:").unwrap_or(dataset_id);
        let parts: Vec<&str> = id.split('/').collect();

        if parts.len() != 2 {
            return Err(DataError::InvalidConfig {
                message: format!(
                    "Invalid Kaggle dataset ID format: {dataset_id}. Expected: kaggle:owner/slug"
                ),
            });
        }

        if !config.is_kaggle_configured() {
            return Err(DataError::AuthRequired {
                api: crate::rate_limiter::DataApi::Kaggle,
                message: "Kaggle credentials not configured".to_string(),
            });
        }

        let kaggle_client = KaggleClient::new(
            http,
            &config.kaggle_base_url,
            config.kaggle_username.as_deref().unwrap_or(""),
            config.kaggle_key.as_deref().unwrap_or(""),
            config.kaggle_api_token.as_deref(),
        );

        // Get list of files to download
        let files_to_download = if let Some(ref specific_files) = params.files {
            specific_files.clone()
        } else {
            // Get all files from the dataset
            let file_list = kaggle_client.list_files(parts[0], parts[1]).await?;
            file_list.into_iter().map(|f| f.path).collect()
        };

        // Download each file
        for file_name in &files_to_download {
            let file_output = output_path.join(file_name);
            let bytes = kaggle_client
                .download_file(parts[0], parts[1], file_name, &file_output)
                .await?;
            total_bytes += bytes;
            files_downloaded += 1;
        }
    } else {
        // HuggingFace format
        let hf_id = dataset_id
            .strip_prefix("hf:")
            .or_else(|| dataset_id.strip_prefix("huggingface:"))
            .unwrap_or(dataset_id);

        let hf_client = HuggingFaceClient::new(
            http,
            &config.huggingface_base_url,
            config.huggingface_token.as_deref(),
        );

        // Get list of files to download
        let files_to_download: Vec<String> = if let Some(ref specific_files) = params.files {
            tracing::info!("Using specified files: {:?}", specific_files);
            specific_files.clone()
        } else {
            // Get all files from the dataset (recursive)
            tracing::info!("Listing all files in dataset '{}'...", hf_id);
            let file_list = hf_client.list_files(hf_id, Some(revision)).await?;
            let paths: Vec<String> = file_list.into_iter().map(|f| f.path).collect();
            tracing::info!("Found {} files to download: {:?}", paths.len(), paths);
            paths
        };

        // Download each file
        for file_path in &files_to_download {
            let file_output = output_path.join(file_path);
            tracing::info!("Downloading '{}' -> {:?}", file_path, file_output);
            let bytes = hf_client
                .download_file(hf_id, file_path, revision, &file_output)
                .await?;
            tracing::info!("Downloaded {} bytes", bytes);
            total_bytes += bytes;
            files_downloaded += 1;
        }
    }

    let duration_secs = start_time.elapsed().as_secs_f64();

    Ok(DatasetDownloadResult {
        dataset_id: dataset_id.clone(),
        output_path: params.output_path,
        files_downloaded,
        total_bytes,
        duration_secs,
    })
}

/// List Kaggle competitions
pub async fn kaggle_competitions(
    http: &HttpClient,
    config: &DataConfig,
    params: KaggleCompetitionsParams,
) -> Result<KaggleCompetitionsResult> {
    if !config.is_kaggle_configured() {
        return Err(DataError::AuthRequired {
            api: crate::rate_limiter::DataApi::Kaggle,
            message: "Kaggle credentials not configured".to_string(),
        });
    }

    let kaggle_client = KaggleClient::new(
        http,
        &config.kaggle_base_url,
        config.kaggle_username.as_deref().unwrap_or(""),
        config.kaggle_key.as_deref().unwrap_or(""),
        config.kaggle_api_token.as_deref(),
    );

    let request = crate::clients::kaggle::KaggleCompetitionsRequest {
        search: params.search.as_deref(),
        category: params.category.as_deref(),
        sort_by: params.sort_by.as_deref(),
        page_size: params.limit,
        ..Default::default()
    };

    let mut competitions = kaggle_client.list_competitions(&request).await?;
    if let Some(limit) = params.limit {
        competitions.truncate(limit as usize);
    }
    let total_count = Some(competitions.len() as u64);

    Ok(KaggleCompetitionsResult {
        competitions,
        total_count,
    })
}

/// List files available for a Kaggle competition
pub async fn kaggle_competition_list_files(
    http: &HttpClient,
    config: &DataConfig,
    params: KaggleCompetitionFilesParams,
) -> Result<KaggleCompetitionFilesResult> {
    if !config.is_kaggle_configured() {
        return Err(DataError::AuthRequired {
            api: crate::rate_limiter::DataApi::Kaggle,
            message: "Kaggle credentials not configured".to_string(),
        });
    }

    let kaggle_client = KaggleClient::new(
        http,
        &config.kaggle_base_url,
        config.kaggle_username.as_deref().unwrap_or(""),
        config.kaggle_key.as_deref().unwrap_or(""),
        config.kaggle_api_token.as_deref(),
    );

    let files = kaggle_client
        .list_competition_files(&params.competition)
        .await?;
    let total_size_bytes = files.iter().filter_map(|f| f.size_bytes).sum();

    Ok(KaggleCompetitionFilesResult {
        competition: params.competition,
        files,
        total_size_bytes: Some(total_size_bytes),
    })
}

/// Download files for a Kaggle competition
pub async fn kaggle_competition_download(
    http: &HttpClient,
    config: &DataConfig,
    params: KaggleCompetitionDownloadParams,
) -> Result<KaggleCompetitionDownloadResult> {
    if !config.is_kaggle_configured() {
        return Err(DataError::AuthRequired {
            api: crate::rate_limiter::DataApi::Kaggle,
            message: "Kaggle credentials not configured".to_string(),
        });
    }

    let output_path = Path::new(&params.output_path);
    std::fs::create_dir_all(output_path).map_err(|e| DataError::Download {
        url: params.output_path.clone(),
        message: format!("Failed to create output directory: {e}"),
    })?;

    let kaggle_client = KaggleClient::new(
        http,
        &config.kaggle_base_url,
        config.kaggle_username.as_deref().unwrap_or(""),
        config.kaggle_key.as_deref().unwrap_or(""),
        config.kaggle_api_token.as_deref(),
    );

    let start_time = Instant::now();
    let files_to_download = if let Some(files) = &params.files {
        files.clone()
    } else {
        kaggle_client
            .list_competition_files(&params.competition)
            .await?
            .into_iter()
            .map(|f| f.path)
            .collect()
    };

    let mut total_bytes = 0_u64;
    let mut files_downloaded = 0_u32;
    for file_name in &files_to_download {
        let destination = output_path.join(file_name);
        let bytes = kaggle_client
            .download_competition_file(&params.competition, file_name, &destination)
            .await?;
        total_bytes += bytes;
        files_downloaded += 1;
    }

    Ok(KaggleCompetitionDownloadResult {
        competition: params.competition,
        output_path: params.output_path,
        files_downloaded,
        total_bytes,
        duration_secs: start_time.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate_limiter::RateLimiter;
    use crate::types::DatasetFile;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    async fn create_test_setup(mock_server: &MockServer) -> (HttpClient, DataConfig) {
        let config = DataConfig {
            huggingface_base_url: mock_server.uri(),
            kaggle_base_url: mock_server.uri(),
            kaggle_username: Some("test_user".to_string()),
            kaggle_key: Some("test_key".to_string()),
            ..Default::default()
        };

        let reqwest_client = reqwest::Client::new();
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limits()));
        let http_client = HttpClient::new(
            reqwest_client,
            rate_limiter,
            config.retry,
            config.request_timeout,
            config.tool_timeout,
        );

        (http_client, config)
    }

    #[tokio::test]
    async fn dataset_search_aggregates_sources() {
        let server = MockServer::start().await;

        // HuggingFace response
        Mock::given(method("GET"))
            .and(path("/api/datasets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "squad",
                    "author": "rajpurkar",
                    "downloads": 1000000
                }
            ])))
            .mount(&server)
            .await;

        // Kaggle response
        Mock::given(method("GET"))
            .and(path("/datasets/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "ref": "openai/test-dataset",
                    "ownerRef": "openai",
                    "title": "Test Dataset",
                    "downloadCount": 500
                }
            ])))
            .mount(&server)
            .await;

        let (http_client, config) = create_test_setup(&server).await;

        let result = dataset_search(
            &http_client,
            &config,
            DatasetSearchParams {
                query: "test".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Should have results from both sources
        assert_eq!(result.datasets.len(), 2);
        assert_eq!(result.per_source_status.len(), 2);

        // Should be sorted by downloads (HF first with 1M)
        assert_eq!(result.datasets[0].id, "squad");
    }

    #[tokio::test]
    async fn dataset_search_filters_by_source() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/datasets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "squad", "author": "rajpurkar"}
            ])))
            .mount(&server)
            .await;

        let (http_client, config) = create_test_setup(&server).await;

        let result = dataset_search(
            &http_client,
            &config,
            DatasetSearchParams {
                query: "test".to_string(),
                source: Some("huggingface".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Should only have HuggingFace results
        assert_eq!(result.per_source_status.len(), 1);
        assert_eq!(result.per_source_status[0].source, "huggingface");
    }

    #[tokio::test]
    async fn kaggle_competitions_respects_limit() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/competitions/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "ref": "https://www.kaggle.com/competitions/comp-1",
                    "title": "Comp 1",
                    "url": "https://www.kaggle.com/competitions/comp-1"
                },
                {
                    "ref": "https://www.kaggle.com/competitions/comp-2",
                    "title": "Comp 2",
                    "url": "https://www.kaggle.com/competitions/comp-2"
                },
                {
                    "ref": "https://www.kaggle.com/competitions/comp-3",
                    "title": "Comp 3",
                    "url": "https://www.kaggle.com/competitions/comp-3"
                }
            ])))
            .mount(&server)
            .await;

        let (http_client, config) = create_test_setup(&server).await;
        let result = kaggle_competitions(
            &http_client,
            &config,
            KaggleCompetitionsParams {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(result.competitions.len(), 2);
        assert_eq!(result.total_count, Some(2));
        assert_eq!(result.competitions[0].title, "Comp 1");
        assert_eq!(result.competitions[1].title, "Comp 2");
    }

    #[tokio::test]
    async fn kaggle_competition_list_files_returns_file_metadata() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/competitions/data/list/titanic"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "files": [
                    {
                        "name": "train.csv",
                        "totalBytes": 1024
                    },
                    {
                        "name": "test.csv",
                        "totalBytes": 2048
                    }
                ]
            })))
            .mount(&server)
            .await;

        let (http_client, config) = create_test_setup(&server).await;
        let result = kaggle_competition_list_files(
            &http_client,
            &config,
            KaggleCompetitionFilesParams {
                competition: "titanic".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            result,
            KaggleCompetitionFilesResult {
                competition: "titanic".to_string(),
                files: vec![
                    DatasetFile {
                        path: "train.csv".to_string(),
                        size_bytes: Some(1024),
                        blob_id: None,
                        lfs: false,
                    },
                    DatasetFile {
                        path: "test.csv".to_string(),
                        size_bytes: Some(2048),
                        blob_id: None,
                        lfs: false,
                    },
                ],
                total_size_bytes: Some(3072),
            }
        );
    }

    #[tokio::test]
    async fn kaggle_competition_download_downloads_requested_files() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/competitions/data/download/titanic/train.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("train,data\n1,2\n"))
            .mount(&server)
            .await;

        let (http_client, config) = create_test_setup(&server).await;

        let output_dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("failed to create tempdir: {err}"),
        };

        let params = KaggleCompetitionDownloadParams {
            competition: "titanic".to_string(),
            output_path: output_dir.path().to_string_lossy().to_string(),
            files: Some(vec!["train.csv".to_string()]),
        };

        let result = kaggle_competition_download(&http_client, &config, params)
            .await
            .unwrap();

        assert_eq!(result.competition, "titanic");
        assert_eq!(result.files_downloaded, 1);
        assert_eq!(result.total_bytes, 15);

        let downloaded_file = output_dir.path().join("train.csv");
        let content = match std::fs::read_to_string(downloaded_file) {
            Ok(content) => content,
            Err(err) => panic!("failed to read downloaded competition file: {err}"),
        };
        assert_eq!(content, "train,data\n1,2\n");
    }
}
