use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use crate::clients::SearchPage;
use crate::error::DataError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::DataApi;
use crate::types::DataSource;
use crate::types::Dataset;
use crate::types::DatasetDetail;
use crate::types::DatasetFile;
use crate::types::DatasetSplit;
use crate::types::Modality;
use crate::types::SourceMeta;

/// HuggingFace Hub API client
#[derive(Debug)]
pub struct HuggingFaceClient<'a> {
    http: &'a HttpClient,
    base_url: &'a str,
    token: Option<&'a str>,
}

impl<'a> HuggingFaceClient<'a> {
    pub fn new(http: &'a HttpClient, base_url: &'a str, token: Option<&'a str>) -> Self {
        Self {
            http,
            base_url,
            token,
        }
    }

    fn add_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = self.token {
            builder.bearer_auth(token)
        } else {
            builder
        }
    }

    /// Search for datasets
    pub async fn search(&self, request: &HfSearchRequest<'_>) -> Result<SearchPage<Dataset>> {
        let mut url = format!("{}/api/datasets", self.base_url);

        let mut params = Vec::new();
        params.push(format!("search={}", urlencoding::encode(request.query)));

        if let Some(author) = request.author {
            params.push(format!("author={}", urlencoding::encode(author)));
        }
        if let Some(limit) = request.limit {
            params.push(format!("limit={limit}"));
        }
        if let Some(sort) = request.sort {
            params.push(format!("sort={sort}"));
        }
        if request.full {
            params.push("full=true".to_string());
        }

        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let response: Vec<HfDatasetResponse> = self
            .http
            .execute_json(DataApi::HuggingFace, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let limit = request.limit.unwrap_or(100) as usize;
        let has_more = response.len() >= limit;

        let datasets: Vec<Dataset> = response
            .into_iter()
            .map(|r| self.convert_dataset(r))
            .collect();

        Ok(SearchPage {
            items: datasets,
            total_available: None, // HF API doesn't provide total count
            has_more,
        })
    }

    /// Get dataset information by ID
    pub async fn get_dataset(&self, repo_id: &str) -> Result<Dataset> {
        let url = format!("{}/api/datasets/{}?full=true", self.base_url, repo_id);

        let response: HfDatasetResponse = self
            .http
            .execute_json(DataApi::HuggingFace, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await
            .map_err(|e| match e {
                DataError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND => {
                    DataError::DatasetNotFound {
                        dataset_id: repo_id.to_string(),
                    }
                }
                other => other,
            })?;

        Ok(self.convert_dataset(response))
    }

    /// Get detailed dataset information including files and splits
    pub async fn get_dataset_detail(&self, repo_id: &str) -> Result<DatasetDetail> {
        let dataset = self.get_dataset(repo_id).await?;

        // Get files
        let files = self.list_files(repo_id, None).await?;

        // Get README if available
        let readme = self.get_readme(repo_id).await.ok();

        // Try to get dataset info for splits
        let splits = self.get_dataset_info(repo_id).await.unwrap_or_default();

        Ok(DatasetDetail {
            dataset,
            files,
            splits,
            readme,
            config: None,
        })
    }

    /// List files in a dataset (recursively explores directories)
    pub async fn list_files(
        &self,
        repo_id: &str,
        revision: Option<&str>,
    ) -> Result<Vec<DatasetFile>> {
        let revision = revision.unwrap_or("main");

        // Use a stack-based approach instead of recursion to avoid async recursion boxing
        let mut all_files = Vec::new();
        let mut dirs_to_process = vec![String::new()]; // Start with root

        while let Some(current_path) = dirs_to_process.pop() {
            // Note: repo_id should NOT be URL-encoded as it contains "/" which is part of the path
            // Only encode the revision and subdirectory paths
            let url = if current_path.is_empty() {
                format!(
                    "{}/api/datasets/{}/tree/{}",
                    self.base_url,
                    repo_id, // Don't encode - the "/" is part of the path
                    urlencoding::encode(revision)
                )
            } else {
                format!(
                    "{}/api/datasets/{}/tree/{}/{}",
                    self.base_url,
                    repo_id, // Don't encode - the "/" is part of the path
                    urlencoding::encode(revision),
                    &current_path
                )
            };

            tracing::debug!("Listing files at: {}", url);

            let response: Vec<HfFileResponse> = match self
                .http
                .execute_json(DataApi::HuggingFace, || {
                    self.add_auth(self.http.client().get(&url))
                })
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to list path '{}': {}", current_path, e);
                    continue;
                }
            };

            for item in response {
                if item.file_type == "file" {
                    tracing::debug!("Found file: {}", item.path);
                    all_files.push(self.convert_file(item));
                } else if item.file_type == "directory" {
                    tracing::debug!("Found directory: {}", item.path);
                    dirs_to_process.push(item.path);
                }
            }
        }

        tracing::info!("Found {} total files in dataset", all_files.len());
        Ok(all_files)
    }

    /// Get README content
    async fn get_readme(&self, repo_id: &str) -> Result<String> {
        let url = format!("{}/datasets/{}/raw/main/README.md", self.base_url, repo_id);

        self.http
            .execute_text(DataApi::HuggingFace, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await
    }

    /// Get dataset info (splits, etc.) from dataset_info.json or similar
    async fn get_dataset_info(&self, repo_id: &str) -> Result<Vec<DatasetSplit>> {
        // Try to get dataset_infos.json first
        let url = format!(
            "{}/datasets/{}/raw/main/dataset_infos.json",
            self.base_url, repo_id
        );

        let response: serde_json::Value = match self
            .http
            .execute_json(DataApi::HuggingFace, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await
        {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };

        // Parse splits from the dataset info
        let mut splits = Vec::new();
        if let Some(obj) = response.as_object() {
            for (_config_name, config) in obj {
                if let Some(split_info) = config.get("splits").and_then(|s| s.as_object()) {
                    for (split_name, info) in split_info {
                        splits.push(DatasetSplit {
                            name: split_name.clone(),
                            num_samples: info
                                .get("num_examples")
                                .and_then(serde_json::Value::as_u64),
                            num_bytes: info.get("num_bytes").and_then(serde_json::Value::as_u64),
                        });
                    }
                }
            }
        }

        Ok(splits)
    }

    /// Get download URL for a file
    pub fn get_file_download_url(&self, repo_id: &str, file_path: &str, revision: &str) -> String {
        format!(
            "{}/datasets/{}/resolve/{}/{}",
            self.base_url,
            repo_id,
            urlencoding::encode(revision),
            file_path
        )
    }

    /// Download a file to a local path
    pub async fn download_file(
        &self,
        repo_id: &str,
        file_path: &str,
        revision: &str,
        output_path: &std::path::Path,
    ) -> Result<u64> {
        let url = self.get_file_download_url(repo_id, file_path, revision);
        tracing::info!("Downloading {} -> {:?}", url, output_path);

        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DataError::Download {
                url: url.clone(),
                message: format!("Failed to create directory: {e}"),
            })?;
        }

        let response = self
            .http
            .execute_response(DataApi::HuggingFace, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let bytes = response.bytes().await.map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to read response: {e}"),
        })?;

        let size = bytes.len() as u64;
        tracing::info!("Downloaded {} bytes for {}", size, file_path);

        std::fs::write(output_path, &bytes).map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to write file: {e}"),
        })?;

        Ok(size)
    }

    fn convert_dataset(&self, response: HfDatasetResponse) -> Dataset {
        let modalities = self.infer_modalities(&response);

        Dataset {
            id: response.id.clone(),
            name: response
                .id
                .split('/')
                .next_back()
                .unwrap_or(&response.id)
                .to_string(),
            source: DataSource::HuggingFace {
                repo_id: response.id.clone(),
            },
            description: response.description,
            size_bytes: response.size_bytes,
            num_samples: response.downloads_all_time,
            num_files: None,
            formats: Vec::new(),
            modalities,
            tags: response.tags.unwrap_or_default(),
            license: response.license,
            author: response.author,
            created_at: response.created_at,
            updated_at: response.last_modified,
            downloads: response.downloads,
            likes: response.likes,
            url: Some(format!("{}/datasets/{}", self.base_url, response.id)),
            source_meta: Some(SourceMeta {
                source: "huggingface".to_string(),
                api_url: format!("{}/api/datasets/{}", self.base_url, response.id),
                fetched_at: Utc::now(),
                canonical_id: Some(response.id),
            }),
        }
    }

    fn convert_file(&self, response: HfFileResponse) -> DatasetFile {
        DatasetFile {
            path: response.path,
            size_bytes: response.size,
            blob_id: response.oid,
            lfs: response.lfs.is_some(),
        }
    }

    fn infer_modalities(&self, response: &HfDatasetResponse) -> Vec<Modality> {
        let mut modalities = Vec::new();

        if let Some(tags) = &response.tags {
            for tag in tags {
                let tag_lower = tag.to_lowercase();
                if tag_lower.contains("image") {
                    modalities.push(Modality::Image);
                }
                if tag_lower.contains("video") {
                    modalities.push(Modality::Video);
                }
                if tag_lower.contains("audio") || tag_lower.contains("speech") {
                    modalities.push(Modality::Audio);
                }
                if tag_lower.contains("text") || tag_lower.contains("nlp") {
                    modalities.push(Modality::Text);
                }
                if tag_lower.contains("tabular") {
                    modalities.push(Modality::Tabular);
                }
            }
        }

        // Deduplicate
        modalities.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        modalities.dedup();

        modalities
    }
}

// === API Response Types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfDatasetResponse {
    #[serde(rename = "_id")]
    pub mongo_id: Option<String>,
    pub id: String,
    pub author: Option<String>,
    pub sha: Option<String>,
    pub last_modified: Option<chrono::DateTime<Utc>>,
    pub created_at: Option<chrono::DateTime<Utc>>,
    #[serde(rename = "private")]
    pub is_private: Option<bool>,
    pub gated: Option<serde_json::Value>,
    pub disabled: Option<bool>,
    pub downloads: Option<u64>,
    #[serde(rename = "downloads_all_time")]
    pub downloads_all_time: Option<u64>,
    pub likes: Option<u64>,
    pub tags: Option<Vec<String>>,
    pub description: Option<String>,
    pub citation: Option<String>,
    #[serde(rename = "cardData")]
    pub card_data: Option<serde_json::Value>,
    pub license: Option<String>,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfFileResponse {
    #[serde(rename = "type")]
    pub file_type: String,
    pub path: String,
    pub size: Option<u64>,
    pub oid: Option<String>,
    pub lfs: Option<HfLfsInfo>,
    #[serde(rename = "xetHash")]
    pub xet_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfLfsInfo {
    pub size: u64,
    pub oid: String,
    #[serde(rename = "pointerSize")]
    pub pointer_size: Option<u64>,
}

// === Request Types ===

#[derive(Debug, Clone)]
pub struct HfSearchRequest<'a> {
    pub query: &'a str,
    pub author: Option<&'a str>,
    pub limit: Option<u32>,
    pub sort: Option<&'a str>,
    pub full: bool,
}

impl<'a> Default for HfSearchRequest<'a> {
    fn default() -> Self {
        Self {
            query: "",
            author: None,
            limit: Some(50),
            sort: Some("downloads"),
            full: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DataConfig;
    use crate::rate_limiter::RateLimiter;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    fn create_test_http_client(config: &DataConfig) -> HttpClient {
        let reqwest_client = reqwest::Client::new();
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limits()));
        HttpClient::new(
            reqwest_client,
            rate_limiter,
            config.retry,
            config.request_timeout,
            config.tool_timeout,
        )
    }

    #[tokio::test]
    async fn search_returns_datasets() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/api/datasets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": "squad",
                    "author": "rajpurkar",
                    "downloads": 1000000,
                    "likes": 500,
                    "tags": ["text", "question-answering"],
                    "description": "Stanford Question Answering Dataset"
                },
                {
                    "id": "imagenet-1k",
                    "author": "imagenet",
                    "downloads": 500000,
                    "tags": ["image", "classification"]
                }
            ])))
            .mount(&server)
            .await;

        let config = DataConfig::default();
        let http_client = create_test_http_client(&config);
        let client = HuggingFaceClient::new(&http_client, &base_url, None);

        let result = client
            .search(&HfSearchRequest {
                query: "squad",
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].id, "squad");
        assert_eq!(result.items[0].downloads, Some(1000000));
    }

    #[tokio::test]
    async fn get_dataset_returns_detail() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/api/datasets/squad"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "squad",
                "author": "rajpurkar",
                "downloads": 1000000,
                "likes": 500,
                "tags": ["text", "question-answering"],
                "description": "Stanford Question Answering Dataset",
                "license": "cc-by-4.0"
            })))
            .mount(&server)
            .await;

        let config = DataConfig::default();
        let http_client = create_test_http_client(&config);
        let client = HuggingFaceClient::new(&http_client, &base_url, None);

        let result = client.get_dataset("squad").await.unwrap();

        assert_eq!(result.id, "squad");
        assert_eq!(result.license, Some("cc-by-4.0".to_string()));
    }

    #[tokio::test]
    async fn get_dataset_not_found_returns_error() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/api/datasets/nonexistent"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let config = DataConfig::default();
        let http_client = create_test_http_client(&config);
        let client = HuggingFaceClient::new(&http_client, &base_url, None);

        let result = client.get_dataset("nonexistent").await;

        assert!(matches!(result, Err(DataError::DatasetNotFound { .. })));
    }

    #[tokio::test]
    async fn list_files_returns_file_list() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/api/datasets/squad/tree/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "type": "file",
                    "path": "train.json",
                    "size": 1024000,
                    "oid": "abc123"
                },
                {
                    "type": "file",
                    "path": "validation.json",
                    "size": 512000,
                    "oid": "def456",
                    "lfs": {
                        "size": 512000,
                        "sha256": "abc..."
                    }
                }
            ])))
            .mount(&server)
            .await;

        let config = DataConfig::default();
        let http_client = create_test_http_client(&config);
        let client = HuggingFaceClient::new(&http_client, &base_url, None);

        let result = client.list_files("squad", None).await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].path, "train.json");
        assert!(!result[0].lfs);
        assert!(result[1].lfs);
    }
}
