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
#[allow(unused_imports)]
use error::ResearchError;
use error::Result;
use http_client::HttpClient;
use paper_id::PaperIdResolver;
use rate_limiter::RateLimiter;
use types::CitationResult;
use types::PaginationParams;
use types::PaperDetail;
use types::PaperSearchParams;
use types::SearchResult;
use types::ZoteroAttachmentsResult;
use types::ZoteroCollectionItemsParams;
use types::ZoteroCollectionsParams;
use types::ZoteroCollectionsResult;
use types::ZoteroFullTextResult;
use types::ZoteroItemDetail;
use types::ZoteroItemParams;
use types::ZoteroNotesResult;
use types::ZoteroSearchParams;
use types::ZoteroSearchResult;
use types::ZoteroTagSearchParams;

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

    #[cfg(feature = "paper_search")]
    pub async fn paper_search(&self, params: PaperSearchParams) -> Result<SearchResult> {
        tools::paper_search::paper_search(self, params).await
    }

    #[cfg(not(feature = "paper_search"))]
    pub async fn paper_search(&self, _params: PaperSearchParams) -> Result<SearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_search",
        })
    }

    #[cfg(feature = "paper_search")]
    pub async fn paper_get(&self, id: &str) -> Result<PaperDetail> {
        tools::paper_search::paper_get(self, id).await
    }

    #[cfg(not(feature = "paper_search"))]
    pub async fn paper_get(&self, _id: &str) -> Result<PaperDetail> {
        Err(ResearchError::NotImplemented { tool: "paper_get" })
    }

    #[cfg(feature = "paper_search")]
    pub async fn paper_citations(
        &self,
        id: &str,
        params: PaginationParams,
    ) -> Result<CitationResult> {
        tools::paper_search::paper_citations(self, id, params).await
    }

    #[cfg(not(feature = "paper_search"))]
    pub async fn paper_citations(
        &self,
        _id: &str,
        _params: PaginationParams,
    ) -> Result<CitationResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_citations",
        })
    }

    #[cfg(feature = "paper_search")]
    pub async fn paper_references(
        &self,
        id: &str,
        params: PaginationParams,
    ) -> Result<CitationResult> {
        tools::paper_search::paper_references(self, id, params).await
    }

    #[cfg(not(feature = "paper_search"))]
    pub async fn paper_references(
        &self,
        _id: &str,
        _params: PaginationParams,
    ) -> Result<CitationResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_references",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_search(&self, params: ZoteroSearchParams) -> Result<ZoteroSearchResult> {
        tools::zotero::zotero_search(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_search(&self, _params: ZoteroSearchParams) -> Result<ZoteroSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_search",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_item(&self, params: ZoteroItemParams) -> Result<ZoteroItemDetail> {
        tools::zotero::zotero_get_item(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_item(&self, _params: ZoteroItemParams) -> Result<ZoteroItemDetail> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_item",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_fulltext(
        &self,
        params: ZoteroItemParams,
    ) -> Result<ZoteroFullTextResult> {
        tools::zotero::zotero_get_fulltext(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_fulltext(
        &self,
        _params: ZoteroItemParams,
    ) -> Result<ZoteroFullTextResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_fulltext",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_notes(&self, params: ZoteroItemParams) -> Result<ZoteroNotesResult> {
        tools::zotero::zotero_get_notes(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_notes(&self, _params: ZoteroItemParams) -> Result<ZoteroNotesResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_notes",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_attachments(
        &self,
        params: ZoteroItemParams,
    ) -> Result<ZoteroAttachmentsResult> {
        tools::zotero::zotero_get_attachments(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_attachments(
        &self,
        _params: ZoteroItemParams,
    ) -> Result<ZoteroAttachmentsResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_attachments",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_search_by_tag(
        &self,
        params: ZoteroTagSearchParams,
    ) -> Result<ZoteroSearchResult> {
        tools::zotero::zotero_search_by_tag(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_search_by_tag(
        &self,
        _params: ZoteroTagSearchParams,
    ) -> Result<ZoteroSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_search_by_tag",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_collections(
        &self,
        params: ZoteroCollectionsParams,
    ) -> Result<ZoteroCollectionsResult> {
        tools::zotero::zotero_get_collections(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_collections(
        &self,
        _params: ZoteroCollectionsParams,
    ) -> Result<ZoteroCollectionsResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_collections",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_collection_items(
        &self,
        params: ZoteroCollectionItemsParams,
    ) -> Result<ZoteroSearchResult> {
        tools::zotero::zotero_get_collection_items(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_collection_items(
        &self,
        _params: ZoteroCollectionItemsParams,
    ) -> Result<ZoteroSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_collection_items",
        })
    }
}
