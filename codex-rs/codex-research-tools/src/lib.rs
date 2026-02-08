pub mod cache;
pub mod config;
pub mod error;
pub mod http_client;
pub mod paper_id;
pub mod rate_limiter;
pub mod tool_specs;
pub mod types;

mod clients;
mod tools;

use std::sync::Arc;

use cache::ResponseCache;
use config::ResearchConfig;
use error::ResearchError;
use error::Result;
use http_client::HttpClient;
use paper_id::PaperIdResolver;
use rate_limiter::RateLimiter;
use types::CitationResult;
use types::PaginationParams;
use types::PaperDetail;
use types::PaperSearchParams;
use types::RepoListResult;
use types::SearchResult;
use types::SotaResult;
use types::SotaSearchParams;
use types::ZoteroItemDetail;
use types::ZoteroSearchParams;
use types::ZoteroSearchResult;

#[derive(Debug)]
pub struct ResearchToolkit {
    http: HttpClient,
    cache: ResponseCache,
    paper_ids: PaperIdResolver,
    config: ResearchConfig,
}

impl ResearchToolkit {
    #[must_use]
    pub fn new(http_client: reqwest::Client, config: ResearchConfig) -> Self {
        let rate_limiter = Arc::new(RateLimiter::new(config.rate_limits()));
        let http = HttpClient::new(http_client, rate_limiter, config.retry);

        Self {
            http,
            cache: ResponseCache::new(config.cache_max_entries),
            paper_ids: PaperIdResolver,
            config,
        }
    }

    #[must_use]
    pub fn config(&self) -> &ResearchConfig {
        &self.config
    }

    #[must_use]
    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    #[must_use]
    pub fn cache(&self) -> &ResponseCache {
        &self.cache
    }

    #[must_use]
    pub fn paper_ids(&self) -> &PaperIdResolver {
        &self.paper_ids
    }

    #[must_use]
    pub fn is_tool_configured(&self, tool_id: &str) -> bool {
        if tool_id.starts_with("zotero_") {
            let has_library_id =
                self.config.zotero_user_id.is_some() || self.config.zotero_group_id.is_some();
            return self.config.zotero_api_key.is_some() && has_library_id;
        }

        true
    }

    pub async fn paper_search(&self, _params: PaperSearchParams) -> Result<SearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_search",
        })
    }

    pub async fn paper_get(&self, _id: &str) -> Result<PaperDetail> {
        Err(ResearchError::NotImplemented { tool: "paper_get" })
    }

    pub async fn paper_citations(
        &self,
        _id: &str,
        _params: PaginationParams,
    ) -> Result<CitationResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_citations",
        })
    }

    pub async fn paper_references(
        &self,
        _id: &str,
        _params: PaginationParams,
    ) -> Result<CitationResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_references",
        })
    }

    pub async fn paper_search_sota(&self, _params: SotaSearchParams) -> Result<SotaResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_search_sota",
        })
    }

    pub async fn paper_find_repos(&self, _paper_id: &str) -> Result<RepoListResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_find_repos",
        })
    }

    pub async fn zotero_search(&self, _params: ZoteroSearchParams) -> Result<ZoteroSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_search",
        })
    }

    pub async fn zotero_get_item(&self, _item_key: &str) -> Result<ZoteroItemDetail> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_item",
        })
    }
}
