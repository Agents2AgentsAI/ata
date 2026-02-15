pub mod cache;
pub mod config;
pub mod error;
pub mod http_client;
pub mod paper_id;
pub mod rate_limiter;
pub mod tool_specs;
pub mod types;

mod clients;
mod text_utils;
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
use types::ConfigSchema;
use types::HnGetThreadParams;
use types::HnSearchParams;
use types::HnSearchResult;
use types::HnThread;
use types::LatexCompileParams;
use types::LatexCompileResult;
use types::ModelDefinition;
use types::PaginationParams;
use types::PaperDetail;
use types::PaperRecommendationParams;
use types::PaperSearchParams;
use types::PatentDetail;
use types::PatentGetParams;
use types::PatentSearchParams;
use types::PatentSearchResult;
use types::PdfExtractFiguresParams;
use types::PdfExtractFiguresResult;
use types::RecommendationResult;
use types::RepoEntrypoint;
use types::RepoExportPath;
use types::RepoHealth;
use types::RepoIoShape;
use types::RepoRequirements;
use types::RepoSummary;
use types::RequirementsDiff;
use types::SearchResult;
use types::ZoteroAdvancedSearchParams;
use types::ZoteroAdvancedSearchResult;
use types::ZoteroAnnotationsParams;
use types::ZoteroAnnotationsResult;
use types::ZoteroAttachmentsResult;
use types::ZoteroCitationParams;
use types::ZoteroCitationResult;
use types::ZoteroCollectionItemsParams;
use types::ZoteroCollectionsParams;
use types::ZoteroCollectionsResult;
use types::ZoteroFullTextResult;
use types::ZoteroGrepParams;
use types::ZoteroGrepResult;
use types::ZoteroGroupsResult;
use types::ZoteroItemDetail;
use types::ZoteroItemParams;
use types::ZoteroListGroupsParams;
use types::ZoteroNotesResult;
use types::ZoteroRecentParams;
use types::ZoteroSearchNotesParams;
use types::ZoteroSearchNotesResult;
use types::ZoteroSearchParams;
use types::ZoteroSearchResult;
use types::ZoteroTagSearchParams;
use types::ZoteroTagsParams;
use types::ZoteroTagsResult;

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
            return self.config.zotero_api_key.is_some() || self.config.uses_local_zotero_api();
        }

        if tool_id == "repo_get_health" {
            // Unauthenticated GitHub access is still viable; a token only raises
            // the rate limit tier for better throughput.
            return true;
        }

        // Paper + repo analysis tools intentionally remain available without API keys.
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

    #[cfg(feature = "paper_search")]
    pub async fn paper_recommendations(
        &self,
        params: PaperRecommendationParams,
    ) -> Result<RecommendationResult> {
        tools::paper_search::paper_recommendations(self, params).await
    }

    #[cfg(not(feature = "paper_search"))]
    pub async fn paper_recommendations(
        &self,
        _params: PaperRecommendationParams,
    ) -> Result<RecommendationResult> {
        Err(ResearchError::NotImplemented {
            tool: "paper_recommendations",
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
    pub async fn zotero_get_tags(&self, params: ZoteroTagsParams) -> Result<ZoteroTagsResult> {
        tools::zotero::zotero_get_tags(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_tags(&self, _params: ZoteroTagsParams) -> Result<ZoteroTagsResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_tags",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_get_recent(
        &self,
        params: ZoteroRecentParams,
    ) -> Result<ZoteroSearchResult> {
        tools::zotero::zotero_get_recent(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_recent(
        &self,
        _params: ZoteroRecentParams,
    ) -> Result<ZoteroSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_recent",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_advanced_search(
        &self,
        params: ZoteroAdvancedSearchParams,
    ) -> Result<ZoteroAdvancedSearchResult> {
        tools::zotero::zotero_advanced_search(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_advanced_search(
        &self,
        _params: ZoteroAdvancedSearchParams,
    ) -> Result<ZoteroAdvancedSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_advanced_search",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_grep_text(&self, params: ZoteroGrepParams) -> Result<ZoteroGrepResult> {
        tools::zotero::zotero_grep_text(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_grep_text(&self, _params: ZoteroGrepParams) -> Result<ZoteroGrepResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_grep_text",
        })
    }

    #[cfg(feature = "zotero")]
    pub async fn zotero_search_notes(
        &self,
        params: ZoteroSearchNotesParams,
    ) -> Result<ZoteroSearchNotesResult> {
        tools::zotero::zotero_search_notes(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_search_notes(
        &self,
        _params: ZoteroSearchNotesParams,
    ) -> Result<ZoteroSearchNotesResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_search_notes",
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
    pub async fn zotero_get_item_citation(
        &self,
        params: ZoteroCitationParams,
    ) -> Result<ZoteroCitationResult> {
        tools::zotero::zotero_get_item_citation(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_item_citation(
        &self,
        _params: ZoteroCitationParams,
    ) -> Result<ZoteroCitationResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_item_citation",
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
    pub async fn zotero_get_annotations(
        &self,
        params: ZoteroAnnotationsParams,
    ) -> Result<ZoteroAnnotationsResult> {
        tools::zotero::zotero_get_annotations(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_get_annotations(
        &self,
        _params: ZoteroAnnotationsParams,
    ) -> Result<ZoteroAnnotationsResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_get_annotations",
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
    pub async fn zotero_list_groups(
        &self,
        params: ZoteroListGroupsParams,
    ) -> Result<ZoteroGroupsResult> {
        tools::zotero::zotero_list_groups(self, params).await
    }

    #[cfg(not(feature = "zotero"))]
    pub async fn zotero_list_groups(
        &self,
        _params: ZoteroListGroupsParams,
    ) -> Result<ZoteroGroupsResult> {
        Err(ResearchError::NotImplemented {
            tool: "zotero_list_groups",
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

    #[cfg(feature = "latex")]
    pub async fn latex_compile(&self, params: LatexCompileParams) -> Result<LatexCompileResult> {
        tools::latex::latex_compile(params).await
    }

    #[cfg(not(feature = "latex"))]
    pub async fn latex_compile(&self, _params: LatexCompileParams) -> Result<LatexCompileResult> {
        Err(ResearchError::NotImplemented {
            tool: "latex_compile",
        })
    }

    #[cfg(feature = "pdf_images")]
    pub async fn pdf_extract_figures(
        &self,
        params: PdfExtractFiguresParams,
    ) -> Result<PdfExtractFiguresResult> {
        tools::pdf_images::pdf_extract_figures(params).await
    }

    #[cfg(not(feature = "pdf_images"))]
    pub async fn pdf_extract_figures(
        &self,
        _params: PdfExtractFiguresParams,
    ) -> Result<PdfExtractFiguresResult> {
        Err(ResearchError::NotImplemented {
            tool: "pdf_extract_figures",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_clone_and_summarize(
        &self,
        repo_url: &str,
        branch: Option<&str>,
    ) -> Result<RepoSummary> {
        tools::repo_analysis::repo_clone_and_summarize(self, repo_url, branch).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_clone_and_summarize(
        &self,
        _repo_url: &str,
        _branch: Option<&str>,
    ) -> Result<RepoSummary> {
        Err(ResearchError::NotImplemented {
            tool: "repo_clone_and_summarize",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_find_models(
        &self,
        repo_url: &str,
        framework: Option<&str>,
    ) -> Result<Vec<ModelDefinition>> {
        tools::repo_analysis::repo_find_models(self, repo_url, framework).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_find_models(
        &self,
        _repo_url: &str,
        _framework: Option<&str>,
    ) -> Result<Vec<ModelDefinition>> {
        Err(ResearchError::NotImplemented {
            tool: "repo_find_models",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_extract_requirements(&self, repo_url: &str) -> Result<RepoRequirements> {
        tools::repo_analysis::repo_extract_requirements(self, repo_url).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_extract_requirements(&self, _repo_url: &str) -> Result<RepoRequirements> {
        Err(ResearchError::NotImplemented {
            tool: "repo_extract_requirements",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_find_entrypoints(
        &self,
        repo_url: &str,
        task_hint: Option<&str>,
    ) -> Result<Vec<RepoEntrypoint>> {
        tools::repo_analysis::repo_find_entrypoints(self, repo_url, task_hint).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_find_entrypoints(
        &self,
        _repo_url: &str,
        _task_hint: Option<&str>,
    ) -> Result<Vec<RepoEntrypoint>> {
        Err(ResearchError::NotImplemented {
            tool: "repo_find_entrypoints",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_extract_io_shapes(
        &self,
        repo_url: &str,
        model_class: Option<&str>,
    ) -> Result<Vec<RepoIoShape>> {
        tools::repo_analysis::repo_extract_io_shapes(self, repo_url, model_class).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_extract_io_shapes(
        &self,
        _repo_url: &str,
        _model_class: Option<&str>,
    ) -> Result<Vec<RepoIoShape>> {
        Err(ResearchError::NotImplemented {
            tool: "repo_extract_io_shapes",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_get_health(&self, repo_url: &str) -> Result<RepoHealth> {
        tools::repo_analysis::repo_get_health(self, repo_url).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_get_health(&self, _repo_url: &str) -> Result<RepoHealth> {
        Err(ResearchError::NotImplemented {
            tool: "repo_get_health",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_find_export_paths(&self, repo_url: &str) -> Result<Vec<RepoExportPath>> {
        tools::repo_analysis::repo_find_export_paths(self, repo_url).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_find_export_paths(&self, _repo_url: &str) -> Result<Vec<RepoExportPath>> {
        Err(ResearchError::NotImplemented {
            tool: "repo_find_export_paths",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_extract_config_schema(&self, repo_url: &str) -> Result<Vec<ConfigSchema>> {
        tools::repo_analysis::repo_extract_config_schema(self, repo_url).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_extract_config_schema(&self, _repo_url: &str) -> Result<Vec<ConfigSchema>> {
        Err(ResearchError::NotImplemented {
            tool: "repo_extract_config_schema",
        })
    }

    #[cfg(feature = "repo_analysis")]
    pub async fn repo_diff_requirements(
        &self,
        repo_url: &str,
        local_requirements_path: &str,
    ) -> Result<RequirementsDiff> {
        tools::repo_analysis::repo_diff_requirements(self, repo_url, local_requirements_path).await
    }

    #[cfg(not(feature = "repo_analysis"))]
    pub async fn repo_diff_requirements(
        &self,
        _repo_url: &str,
        _local_requirements_path: &str,
    ) -> Result<RequirementsDiff> {
        Err(ResearchError::NotImplemented {
            tool: "repo_diff_requirements",
        })
    }

    #[cfg(feature = "hackernews")]
    pub async fn hn_search(&self, params: HnSearchParams) -> Result<HnSearchResult> {
        tools::hackernews::hn_search(self, params).await
    }

    #[cfg(not(feature = "hackernews"))]
    pub async fn hn_search(&self, _params: HnSearchParams) -> Result<HnSearchResult> {
        Err(ResearchError::NotImplemented { tool: "hn_search" })
    }

    #[cfg(feature = "hackernews")]
    pub async fn hn_get_thread(&self, params: HnGetThreadParams) -> Result<HnThread> {
        tools::hackernews::hn_get_thread(self, params).await
    }

    #[cfg(not(feature = "hackernews"))]
    pub async fn hn_get_thread(&self, _params: HnGetThreadParams) -> Result<HnThread> {
        Err(ResearchError::NotImplemented {
            tool: "hn_get_thread",
        })
    }

    #[cfg(feature = "patents")]
    pub async fn patent_search(&self, params: PatentSearchParams) -> Result<PatentSearchResult> {
        tools::patents::patent_search(self, params).await
    }

    #[cfg(not(feature = "patents"))]
    pub async fn patent_search(&self, _params: PatentSearchParams) -> Result<PatentSearchResult> {
        Err(ResearchError::NotImplemented {
            tool: "patent_search",
        })
    }

    #[cfg(feature = "patents")]
    pub async fn patent_get(&self, params: PatentGetParams) -> Result<PatentDetail> {
        tools::patents::patent_get(self, params).await
    }

    #[cfg(not(feature = "patents"))]
    pub async fn patent_get(&self, _params: PatentGetParams) -> Result<PatentDetail> {
        Err(ResearchError::NotImplemented { tool: "patent_get" })
    }
}
