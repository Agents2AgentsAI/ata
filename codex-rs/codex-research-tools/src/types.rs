use chrono::DateTime;
use chrono::Utc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SourceMeta {
    pub source: String,
    pub api_url: String,
    pub fetched_at: DateTime<Utc>,
    pub canonical_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Paper {
    pub title: String,
    pub authors: String,
    pub year: Option<u32>,
    pub venue: Option<String>,
    pub citation_count: Option<u32>,
    pub abstract_text: Option<String>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub s2_paper_id: Option<String>,
    pub openalex_id: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub code_url: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceResult {
    Ok { count: usize },
    Error { message: String, retryable: bool },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SourceStatus {
    pub source: String,
    pub status: SourceResult,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SearchResult {
    pub papers: Vec<Paper>,
    pub per_source_status: Vec<SourceStatus>,
    pub warnings: Vec<String>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PaperDetail {
    pub paper: Paper,
    pub references: Vec<Paper>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CitationResult {
    pub papers: Vec<Paper>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SotaEntry {
    pub task: String,
    pub dataset: String,
    pub metric: String,
    pub best_method: String,
    pub best_score: Option<f64>,
    pub paper_title: Option<String>,
    pub paper_url: Option<String>,
    pub code_url: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SotaResult {
    pub entries: Vec<SotaEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CodeRepo {
    pub url: String,
    pub framework: Option<String>,
    pub stars: Option<u32>,
    pub is_official: Option<bool>,
    pub description: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoListResult {
    pub repos: Vec<CodeRepo>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItem {
    pub key: String,
    pub title: String,
    pub authors: String,
    pub year: Option<String>,
    pub item_type: String,
    pub doi: Option<String>,
    pub abstract_snippet: Option<String>,
    pub tags: Vec<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItemDetail {
    pub key: String,
    pub title: String,
    pub authors: Vec<String>,
    pub abstract_text: Option<String>,
    pub date: Option<String>,
    pub doi: Option<String>,
    pub url: Option<String>,
    pub publication: Option<String>,
    pub item_type: String,
    pub tags: Vec<String>,
    pub extra: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchResult {
    pub items: Vec<ZoteroItem>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroFullTextResult {
    pub item_key: String,
    pub content: String,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroNote {
    pub key: String,
    pub title: Option<String>,
    pub note: Option<String>,
    pub parent_item: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroNotesResult {
    pub item_key: String,
    pub notes: Vec<ZoteroNote>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAttachment {
    pub key: String,
    pub title: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub link_mode: Option<String>,
    pub url: Option<String>,
    pub parent_item: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAttachmentsResult {
    pub item_key: String,
    pub attachments: Vec<ZoteroAttachment>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollection {
    pub key: String,
    pub name: String,
    pub parent_collection: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollectionsResult {
    pub collections: Vec<ZoteroCollection>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PaperSearchParams {
    pub query: String,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub fields_of_study: Option<Vec<String>>,
    pub source: Option<String>,
    pub sort_by: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub include_abstract: Option<bool>,
    pub fields: Option<Vec<String>>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PaginationParams {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SotaSearchParams {
    pub task: String,
    pub dataset: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchParams {
    pub query: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub fields: Option<Vec<String>>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItemParams {
    pub item_key: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroTagSearchParams {
    pub tags: Vec<String>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollectionsParams {
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollectionItemsParams {
    pub collection_key: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub fields: Option<Vec<String>>,
    pub max_chars_per_item: Option<u32>,
}
