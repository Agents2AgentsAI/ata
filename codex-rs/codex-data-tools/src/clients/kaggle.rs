use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use crate::clients::SearchPage;
use crate::error::DataError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::DataApi;
use crate::types::Competition;
use crate::types::DataSource;
use crate::types::Dataset;
use crate::types::DatasetDetail;
use crate::types::DatasetFile;
use crate::types::Modality;
use crate::types::SourceMeta;

/// Kaggle API client
#[derive(Debug)]
pub struct KaggleClient<'a> {
    http: &'a HttpClient,
    base_url: &'a str,
    username: &'a str,
    key: &'a str,
    api_token: Option<&'a str>,
}

impl<'a> KaggleClient<'a> {
    pub fn new(
        http: &'a HttpClient,
        base_url: &'a str,
        username: &'a str,
        key: &'a str,
        api_token: Option<&'a str>,
    ) -> Self {
        Self {
            http,
            base_url,
            username,
            key,
            api_token,
        }
    }

    fn add_auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = self.resolve_api_token() {
            builder.bearer_auth(token)
        } else {
            builder.basic_auth(self.username, Some(self.key))
        }
    }

    fn resolve_api_token(&self) -> Option<&str> {
        self.api_token
            .filter(|token| !token.is_empty())
            .or_else(|| {
                if self.key.starts_with("KGAT_") {
                    Some(self.key)
                } else {
                    None
                }
            })
    }

    /// Search for datasets
    pub async fn search(&self, request: &KaggleSearchRequest<'_>) -> Result<SearchPage<Dataset>> {
        let mut url = format!("{}/datasets/list", self.base_url);

        let mut params = Vec::new();
        if !request.query.is_empty() {
            params.push(format!("search={}", urlencoding::encode(request.query)));
        }
        if let Some(sort_by) = request.sort_by {
            params.push(format!("sortBy={sort_by}"));
        }
        if let Some(file_type) = request.file_type {
            params.push(format!("filetype={file_type}"));
        }
        if let Some(license) = request.license {
            params.push(format!("license={license}"));
        }
        if let Some(tag_ids) = &request.tag_ids {
            params.push(format!("tagIds={}", tag_ids.join(",")));
        }
        if let Some(page) = request.page {
            params.push(format!("page={page}"));
        }
        if let Some(page_size) = request.page_size {
            params.push(format!("pageSize={page_size}"));
        }
        if let Some(user) = request.user {
            params.push(format!("user={}", urlencoding::encode(user)));
        }

        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let response: Vec<KaggleDatasetResponse> = self
            .http
            .execute_json(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let page_size = request.page_size.unwrap_or(20) as usize;
        let has_more = response.len() >= page_size;

        let datasets: Vec<Dataset> = response
            .into_iter()
            .map(|r| self.convert_dataset(r))
            .collect();

        Ok(SearchPage {
            items: datasets,
            total_available: None, // Kaggle API doesn't return total
            has_more,
        })
    }

    /// Get dataset information by ref (owner/dataset-slug)
    pub async fn get_dataset(&self, owner: &str, slug: &str) -> Result<Dataset> {
        let url = format!(
            "{}/datasets/view/{}/{}",
            self.base_url,
            urlencoding::encode(owner),
            urlencoding::encode(slug)
        );

        let response: KaggleDatasetResponse = self
            .http
            .execute_json(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await
            .map_err(|e| match e {
                DataError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND => {
                    DataError::DatasetNotFound {
                        dataset_id: format!("{owner}/{slug}"),
                    }
                }
                other => other,
            })?;

        Ok(self.convert_dataset(response))
    }

    /// Get detailed dataset information including files
    pub async fn get_dataset_detail(&self, owner: &str, slug: &str) -> Result<DatasetDetail> {
        let dataset = self.get_dataset(owner, slug).await?;
        let files = self.list_files(owner, slug).await?;

        Ok(DatasetDetail {
            dataset,
            files,
            splits: Vec::new(), // Kaggle doesn't have built-in splits concept
            readme: None,       // Would need to download and parse
            config: None,
        })
    }

    /// List files in a dataset
    pub async fn list_files(&self, owner: &str, slug: &str) -> Result<Vec<DatasetFile>> {
        let url = format!(
            "{}/datasets/list/{}/{}",
            self.base_url,
            urlencoding::encode(owner),
            urlencoding::encode(slug)
        );

        let response: KaggleFilesResponse = self
            .http
            .execute_json(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        Ok(response
            .dataset_files
            .into_iter()
            .map(|f| self.convert_file(f))
            .collect())
    }

    /// Get download URL for a dataset
    /// Note: Kaggle downloads are ZIP files of the entire dataset
    #[allow(dead_code)]
    pub fn get_download_url(&self, owner: &str, slug: &str) -> String {
        format!(
            "{}/datasets/download/{}/{}",
            self.base_url,
            urlencoding::encode(owner),
            urlencoding::encode(slug)
        )
    }

    /// Get download URL for a specific file
    pub fn get_file_download_url(&self, owner: &str, slug: &str, file_name: &str) -> String {
        format!(
            "{}/datasets/download/{}/{}/{}",
            self.base_url,
            urlencoding::encode(owner),
            urlencoding::encode(slug),
            urlencoding::encode(file_name)
        )
    }

    /// Download a file to a local path
    pub async fn download_file(
        &self,
        owner: &str,
        slug: &str,
        file_name: &str,
        output_path: &std::path::Path,
    ) -> Result<u64> {
        let url = self.get_file_download_url(owner, slug, file_name);

        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DataError::Download {
                url: url.clone(),
                message: format!("Failed to create directory: {e}"),
            })?;
        }

        let response = self
            .http
            .execute_response(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let bytes = response.bytes().await.map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to read response: {e}"),
        })?;

        std::fs::write(output_path, &bytes).map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to write file: {e}"),
        })?;

        Ok(bytes.len() as u64)
    }

    /// List competitions
    pub async fn list_competitions(
        &self,
        request: &KaggleCompetitionsRequest<'_>,
    ) -> Result<Vec<Competition>> {
        let mut url = format!("{}/competitions/list", self.base_url);

        let mut params = Vec::new();
        if let Some(search) = request.search
            && !search.is_empty()
        {
            params.push(format!("search={}", urlencoding::encode(search)));
        }
        if let Some(category) = request.category {
            params.push(format!("category={category}"));
        }
        if let Some(sort_by) = request.sort_by {
            params.push(format!("sortBy={sort_by}"));
        }
        if let Some(page) = request.page {
            params.push(format!("page={page}"));
        }
        if let Some(page_size) = request.page_size {
            params.push(format!("pageSize={page_size}"));
        }

        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let response: Vec<KaggleCompetitionResponse> = self
            .http
            .execute_json(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        Ok(response
            .into_iter()
            .map(|c| self.convert_competition(c))
            .collect())
    }

    /// List files in a competition
    pub async fn list_competition_files(&self, competition: &str) -> Result<Vec<DatasetFile>> {
        let url = format!(
            "{}/competitions/data/list/{}",
            self.base_url,
            urlencoding::encode(competition)
        );

        let response: KaggleCompetitionFilesResponse = self
            .http
            .execute_json(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        Ok(response
            .into_files()
            .into_iter()
            .map(|f| self.convert_file(f))
            .collect())
    }

    /// Get download URL for a specific competition file
    pub fn get_competition_file_download_url(&self, competition: &str, file_name: &str) -> String {
        format!(
            "{}/competitions/data/download/{}/{}",
            self.base_url,
            urlencoding::encode(competition),
            urlencoding::encode(file_name)
        )
    }

    /// Download a competition file to a local path
    pub async fn download_competition_file(
        &self,
        competition: &str,
        file_name: &str,
        output_path: &std::path::Path,
    ) -> Result<u64> {
        let url = self.get_competition_file_download_url(competition, file_name);

        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DataError::Download {
                url: url.clone(),
                message: format!("Failed to create directory: {e}"),
            })?;
        }

        let response = self
            .http
            .execute_response(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let bytes = response.bytes().await.map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to read response: {e}"),
        })?;

        std::fs::write(output_path, &bytes).map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to write file: {e}"),
        })?;

        Ok(bytes.len() as u64)
    }

    /// Download entire dataset as a ZIP file
    #[allow(dead_code)]
    pub async fn download_dataset(
        &self,
        owner: &str,
        slug: &str,
        output_path: &std::path::Path,
    ) -> Result<u64> {
        let url = self.get_download_url(owner, slug);

        // Ensure parent directory exists
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DataError::Download {
                url: url.clone(),
                message: format!("Failed to create directory: {e}"),
            })?;
        }

        let response = self
            .http
            .execute_response(DataApi::Kaggle, || {
                self.add_auth(self.http.client().get(&url))
            })
            .await?;

        let bytes = response.bytes().await.map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to read response: {e}"),
        })?;

        std::fs::write(output_path, &bytes).map_err(|e| DataError::Download {
            url: url.clone(),
            message: format!("Failed to write file: {e}"),
        })?;

        Ok(bytes.len() as u64)
    }

    fn convert_dataset(&self, response: KaggleDatasetResponse) -> Dataset {
        // Extract owner and slug from ref (format: "owner/slug") or owner_ref
        let (owner, slug) = if let Some(ref_str) = &response.dataset_ref {
            let parts: Vec<&str> = ref_str.splitn(2, '/').collect();
            if parts.len() == 2 {
                (parts[0].to_string(), parts[1].to_string())
            } else {
                (
                    response.owner_ref.clone().unwrap_or_default(),
                    ref_str.clone(),
                )
            }
        } else {
            (
                response.owner_ref.clone().unwrap_or_default(),
                String::new(),
            )
        };

        let id = format!("{owner}/{slug}");
        let modalities = self.infer_modalities(&response);

        // Extract tag names from the tag objects
        let tags: Vec<String> = response
            .tags
            .as_ref()
            .map(|tags| tags.iter().filter_map(|t| t.name.clone()).collect())
            .unwrap_or_default();

        Dataset {
            id: id.clone(),
            name: response.title.unwrap_or_else(|| slug.clone()),
            source: DataSource::Kaggle {
                owner: owner.clone(),
                slug: slug,
            },
            description: response.description,
            size_bytes: response.total_bytes,
            num_samples: None,
            num_files: None,
            formats: Vec::new(),
            modalities,
            tags,
            license: response.license_name,
            author: Some(owner),
            created_at: None,
            updated_at: response.last_updated,
            downloads: response.download_count.map(|c| c as u64),
            likes: response.vote_count.map(|c| c as u64),
            url: Some(format!("https://www.kaggle.com/datasets/{id}")),
            source_meta: Some(SourceMeta {
                source: "kaggle".to_string(),
                api_url: format!("{}/datasets/view/{id}", self.base_url),
                fetched_at: Utc::now(),
                canonical_id: Some(id),
            }),
        }
    }

    fn convert_file(&self, response: KaggleFileResponse) -> DatasetFile {
        DatasetFile {
            path: response.name,
            size_bytes: Some(response.total_bytes as u64),
            blob_id: None,
            lfs: false,
        }
    }

    fn convert_competition(&self, response: KaggleCompetitionResponse) -> Competition {
        Competition {
            id: response.competition_ref.clone().unwrap_or_default(),
            title: response.title.unwrap_or_default(),
            url: response.url.or_else(|| {
                response
                    .competition_ref
                    .as_ref()
                    .map(|r| format!("https://www.kaggle.com/competitions/{r}"))
            }),
            description: response.description,
            organization: response.organization_name,
            category: response.category,
            reward: response.reward,
            deadline: response.deadline,
            team_count: response.team_count.map(|c| c as u64),
            evaluation_metric: response.evaluation_metric,
        }
    }

    fn infer_modalities(&self, response: &KaggleDatasetResponse) -> Vec<Modality> {
        let mut modalities = Vec::new();

        // Check tags
        if let Some(tags) = &response.tags {
            for tag in tags {
                if let Some(name) = &tag.name {
                    let tag_lower = name.to_lowercase();
                    if tag_lower.contains("image") || tag_lower.contains("computer vision") {
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
                    if tag_lower.contains("tabular") || tag_lower.contains("csv") {
                        modalities.push(Modality::Tabular);
                    }
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
pub struct KaggleDatasetResponse {
    pub id: Option<i64>,
    #[serde(rename = "ref")]
    pub dataset_ref: Option<String>,
    pub owner_ref: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub creator_name: Option<String>,
    pub creator_url: Option<String>,
    pub total_bytes: Option<u64>,
    pub url: Option<String>,
    pub last_updated: Option<chrono::DateTime<Utc>>,
    pub download_count: Option<i64>,
    pub vote_count: Option<i64>,
    pub usability_rating: Option<f64>,
    pub license_name: Option<String>,
    pub tags: Option<Vec<KaggleTag>>,
    pub is_private: Option<bool>,
    pub is_featured: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaggleTag {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaggleFilesResponse {
    pub dataset_files: Vec<KaggleFileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum KaggleCompetitionFilesResponse {
    Files {
        files: Vec<KaggleFileResponse>,
    },
    DatasetFiles {
        dataset_files: Vec<KaggleFileResponse>,
    },
    Bare(Vec<KaggleFileResponse>),
}

impl KaggleCompetitionFilesResponse {
    fn into_files(self) -> Vec<KaggleFileResponse> {
        match self {
            Self::Files { files }
            | Self::DatasetFiles {
                dataset_files: files,
            } => files,
            Self::Bare(files) => files,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaggleFileResponse {
    pub name: String,
    pub total_bytes: i64,
    pub creation_date: Option<chrono::DateTime<Utc>>,
    pub description: Option<String>,
    pub columns: Option<Vec<KaggleColumnInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaggleColumnInfo {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub column_type: Option<String>,
}

// === Competition Response Types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KaggleCompetitionResponse {
    pub id: Option<i64>,
    #[serde(rename = "ref")]
    pub competition_ref: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub organization_name: Option<String>,
    pub organization_ref: Option<String>,
    pub category: Option<String>,
    pub reward: Option<String>,
    pub deadline: Option<chrono::DateTime<Utc>>,
    pub kernel_count: Option<i64>,
    pub team_count: Option<i64>,
    pub user_has_entered: Option<bool>,
    pub user_rank: Option<i64>,
    pub merger_deadline: Option<chrono::DateTime<Utc>>,
    pub new_entrant_deadline: Option<chrono::DateTime<Utc>>,
    pub enabled_date: Option<chrono::DateTime<Utc>>,
    pub max_daily_submissions: Option<i64>,
    pub max_team_size: Option<i64>,
    pub evaluation_metric: Option<String>,
    pub awards_points: Option<bool>,
    pub is_kernels_submissions_only: Option<bool>,
    pub submittions_disabled: Option<bool>,
}

// === Request Types ===

#[derive(Debug, Clone, Default)]
pub struct KaggleSearchRequest<'a> {
    pub query: &'a str,
    pub sort_by: Option<&'a str>, // "hottest", "votes", "updated", "active", "published"
    pub file_type: Option<&'a str>, // "csv", "json", "sqlite", "bigQuery"
    pub license: Option<&'a str>,
    pub tag_ids: Option<Vec<String>>,
    pub user: Option<&'a str>,
    pub page: Option<u32>,
    pub page_size: Option<u32>, // max 20
}

#[derive(Debug, Clone, Default)]
pub struct KaggleCompetitionsRequest<'a> {
    pub search: Option<&'a str>,
    pub category: Option<&'a str>, // "featured", "research", "getting-started", "masters", "playground"
    pub sort_by: Option<&'a str>,  // "deadline", "prize", "numberOfTeams"
    pub page: Option<u32>,
    pub page_size: Option<u32>,
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
    use wiremock::matchers::header;
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
            .and(path("/datasets/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "ref": "openai/gpt-4-dataset",
                    "ownerRef": "openai",
                    "title": "GPT-4 Training Data",
                    "description": "A sample dataset",
                    "totalBytes": 1000000,
                    "downloadCount": 5000,
                    "voteCount": 100,
                    "tags": [{"name": "nlp"}, {"name": "text"}]
                },
                {
                    "ref": "google/image-classification",
                    "ownerRef": "google",
                    "title": "Image Classification Dataset",
                    "totalBytes": 5000000,
                    "tags": [{"name": "image"}, {"name": "computer vision"}]
                }
            ])))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let result = client
            .search(&KaggleSearchRequest {
                query: "dataset",
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].id, "openai/gpt-4-dataset");
        assert_eq!(result.items[0].downloads, Some(5000));
    }

    #[tokio::test]
    async fn get_dataset_returns_detail() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/datasets/view/openai/gpt-4-dataset"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ref": "openai/gpt-4-dataset",
                "ownerRef": "openai",
                "title": "GPT-4 Training Data",
                "description": "A sample dataset",
                "totalBytes": 1000000,
                "licenseName": "MIT",
                "downloadCount": 5000,
                "voteCount": 100
            })))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let result = client.get_dataset("openai", "gpt-4-dataset").await.unwrap();

        assert_eq!(result.id, "openai/gpt-4-dataset");
        assert_eq!(result.license, Some("MIT".to_string()));
    }

    #[tokio::test]
    async fn list_files_returns_file_list() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/datasets/data/openai/gpt-4-dataset"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "datasetFiles": [
                    {
                        "name": "train.csv",
                        "totalBytes": 1024000
                    },
                    {
                        "name": "test.csv",
                        "totalBytes": 512000
                    }
                ]
            })))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let result = client.list_files("openai", "gpt-4-dataset").await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].path, "train.csv");
        assert_eq!(result[0].size_bytes, Some(1024000));
    }

    #[tokio::test]
    async fn list_competitions_uses_bearer_auth_from_api_token() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/competitions/list"))
            .and(header("authorization", "Bearer test_api_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_api_token = Some("test_api_token".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(
            &http_client,
            &base_url,
            "unused_user",
            "unused_key",
            Some("test_api_token"),
        );

        let competitions = client
            .list_competitions(&KaggleCompetitionsRequest::default())
            .await
            .unwrap();

        assert_eq!(competitions, Vec::<Competition>::new());
    }

    #[tokio::test]
    async fn list_competitions_uses_bearer_auth_for_token_shaped_key() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/competitions/list"))
            .and(header("authorization", "Bearer KGAT_token_from_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("KGAT_token_from_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(
            &http_client,
            &base_url,
            "test_user",
            "KGAT_token_from_key",
            None,
        );

        let competitions = client
            .list_competitions(&KaggleCompetitionsRequest::default())
            .await
            .unwrap();

        assert_eq!(competitions, Vec::<Competition>::new());
    }

    #[tokio::test]
    async fn list_competition_files_returns_files() {
        let server = MockServer::start().await;
        let base_url = server.uri();

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

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let files = client.list_competition_files("titanic").await.unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "train.csv");
        assert_eq!(files[0].size_bytes, Some(1024));
        assert_eq!(files[1].path, "test.csv");
        assert_eq!(files[1].size_bytes, Some(2048));
    }

    #[tokio::test]
    async fn download_competition_file_writes_output() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/competitions/data/download/titanic/train.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes("row1,row2\n1,2\n"))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("train.csv");
        let bytes = client
            .download_competition_file("titanic", "train.csv", &output_path)
            .await
            .unwrap();

        assert_eq!(bytes, 14);
        assert_eq!(
            std::fs::read_to_string(output_path).unwrap(),
            "row1,row2\n1,2\n"
        );
    }

    #[tokio::test]
    async fn get_dataset_not_found_returns_error() {
        let server = MockServer::start().await;
        let base_url = server.uri();

        Mock::given(method("GET"))
            .and(path("/datasets/view/nonexistent/dataset"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let mut config = DataConfig::default();
        config.kaggle_username = Some("test_user".to_string());
        config.kaggle_key = Some("test_key".to_string());
        let http_client = create_test_http_client(&config);
        let client = KaggleClient::new(&http_client, &base_url, "test_user", "test_key", None);

        let result = client.get_dataset("nonexistent", "dataset").await;

        assert!(matches!(result, Err(DataError::DatasetNotFound { .. })));
    }
}
