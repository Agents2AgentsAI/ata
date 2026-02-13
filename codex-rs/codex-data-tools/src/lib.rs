//! Data tools for Codex - dataset discovery, download, and management
//!
//! This crate provides tools for working with datasets from various sources:
//! - HuggingFace Datasets
//! - Kaggle Datasets
//! - (Future) Isaac Sim synthetic data
//! - (Future) NVIDIA Cosmos generated data

pub mod cache;
pub mod config;
pub mod error;
pub mod http_client;
pub mod rate_limiter;
pub mod tool_specs;
pub mod types;

mod clients;
mod tools;

use std::sync::Arc;

use cache::ResponseCache;
use config::DataConfig;

// Re-export commonly used items
pub use config::load_dotenv;
use error::Result;
use http_client::HttpClient;
use rate_limiter::RateLimiter;
use types::DatasetDetail;
use types::DatasetDownloadParams;
use types::DatasetDownloadResult;
use types::DatasetFilesParams;
use types::DatasetFilesResult;
use types::DatasetGetParams;
use types::DatasetSearchParams;
use types::DatasetSearchResult;
use types::KaggleCompetitionDownloadParams;
use types::KaggleCompetitionDownloadResult;
use types::KaggleCompetitionFilesParams;
use types::KaggleCompetitionFilesResult;
use types::KaggleCompetitionsParams;
use types::KaggleCompetitionsResult;

/// Main toolkit for data operations
#[derive(Debug)]
pub struct DataToolkit {
    http: HttpClient,
    cache: ResponseCache,
    config: DataConfig,
}

impl DataToolkit {
    /// Create a new DataToolkit with default HTTP client
    #[must_use]
    pub fn new(http_client: reqwest::Client, config: DataConfig) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limits()));
        let http = HttpClient::new(
            http_client,
            rate_limiter,
            config.retry,
            config.request_timeout,
            config.tool_timeout,
        );

        Self {
            http,
            cache: ResponseCache::new(config.cache_max_entries),
            config,
        }
    }

    /// Get the configuration
    #[must_use]
    pub fn config(&self) -> &DataConfig {
        &self.config
    }

    /// Get the HTTP client
    #[must_use]
    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    /// Get the response cache
    #[must_use]
    pub fn cache(&self) -> &ResponseCache {
        &self.cache
    }

    /// Check if a tool is properly configured
    #[must_use]
    pub fn is_tool_configured(&self, tool_id: &str) -> bool {
        match tool_id {
            "dataset_search" | "dataset_get" | "dataset_list_files" | "dataset_download" => {
                // These work with at least HuggingFace (no auth required for public datasets)
                true
            }
            "hf_dataset_info" => true, // HuggingFace works without auth
            "kaggle_dataset_info"
            | "kaggle_competitions"
            | "kaggle_competition_list_files"
            | "kaggle_competition_download" => self.config.is_kaggle_configured(),
            _ => false,
        }
    }

    // === Dataset Operations ===

    /// Search for datasets across configured sources
    pub async fn dataset_search(&self, params: DatasetSearchParams) -> Result<DatasetSearchResult> {
        tools::dataset_search(&self.http, &self.config, params).await
    }

    /// Get detailed information about a dataset
    pub async fn dataset_get(&self, params: DatasetGetParams) -> Result<DatasetDetail> {
        tools::dataset_get(&self.http, &self.config, params).await
    }

    /// List files in a dataset
    pub async fn dataset_list_files(
        &self,
        params: DatasetFilesParams,
    ) -> Result<DatasetFilesResult> {
        tools::dataset_list_files(&self.http, &self.config, params).await
    }

    /// Download a dataset to a local directory
    pub async fn dataset_download(
        &self,
        params: DatasetDownloadParams,
    ) -> Result<DatasetDownloadResult> {
        tools::dataset_download(&self.http, &self.config, params).await
    }

    /// List Kaggle competitions
    pub async fn kaggle_competitions(
        &self,
        params: KaggleCompetitionsParams,
    ) -> Result<KaggleCompetitionsResult> {
        tools::kaggle_competitions(&self.http, &self.config, params).await
    }

    /// List files available in a Kaggle competition
    pub async fn kaggle_competition_list_files(
        &self,
        params: KaggleCompetitionFilesParams,
    ) -> Result<KaggleCompetitionFilesResult> {
        tools::kaggle_competition_list_files(&self.http, &self.config, params).await
    }

    /// Download files from a Kaggle competition to a local directory
    pub async fn kaggle_competition_download(
        &self,
        params: KaggleCompetitionDownloadParams,
    ) -> Result<KaggleCompetitionDownloadResult> {
        tools::kaggle_competition_download(&self.http, &self.config, params).await
    }
}

/// Type alias for shared toolkit reference
pub type SharedDataToolkit = DataToolkit;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolkit_creation_works() {
        let config = DataConfig::from_env();
        let client = reqwest::Client::new();
        let toolkit = DataToolkit::new(client, config);

        assert!(toolkit.is_tool_configured("dataset_search"));
    }

    #[test]
    fn kaggle_tools_require_auth() {
        let config = DataConfig::default(); // No Kaggle credentials
        let client = reqwest::Client::new();
        let toolkit = DataToolkit::new(client, config);

        assert!(!toolkit.is_tool_configured("kaggle_dataset_info"));
        assert!(!toolkit.is_tool_configured("kaggle_competitions"));
        assert!(!toolkit.is_tool_configured("kaggle_competition_list_files"));
        assert!(!toolkit.is_tool_configured("kaggle_competition_download"));
    }
}
