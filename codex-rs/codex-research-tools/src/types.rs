use chrono::DateTime;
use chrono::NaiveDate;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arxiv_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s2_paper_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openalex_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_url: Option<String>,
    #[serde(skip_serializing, default)]
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
    #[serde(skip_serializing, default)]
    pub per_source_status: Vec<SourceStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroLinkedItem {
    pub relation: String,
    pub raw_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItem {
    pub key: String,
    pub title: String,
    pub authors: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_snippet: Option<String>,
    pub tags: Vec<String>,
    pub linked_items: Vec<ZoteroLinkedItem>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItemDetail {
    pub key: String,
    pub title: String,
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication: Option<String>,
    pub item_type: String,
    pub tags: Vec<String>,
    pub linked_items: Vec<ZoteroLinkedItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ZoteroAttachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_resolution: Option<DocumentResolution>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchResult {
    pub items: Vec<ZoteroItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroTagsResult {
    pub tags: Vec<String>,
    pub total_available: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroRecentSortBy {
    DateAdded,
    DateModified,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroAdvancedJoinMode {
    All,
    Any,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroAdvancedSortBy {
    Title,
    Year,
    DateAdded,
    DateModified,
    Relevance,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroSortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroSearchConditionField {
    Title,
    Creator,
    Year,
    Tag,
    ItemType,
    Publication,
    Doi,
    Note,
    Annotation,
    Fulltext,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroSearchConditionOperation {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    Regex,
    BeforeYear,
    AfterYear,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ZoteroSearchCondition {
    pub field: ZoteroSearchConditionField,
    pub operation: ZoteroSearchConditionOperation,
    pub value: Option<String>,
    pub case_sensitive: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroAdvancedCompleteness {
    Exact,
    Approximate,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroAdvancedCandidateStrategy {
    ServerFiltered,
    RecentModifiedFallback,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAdvancedSearchResult {
    pub completeness: ZoteroAdvancedCompleteness,
    pub candidate_strategy: ZoteroAdvancedCandidateStrategy,
    pub scanned_items: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    pub results: ZoteroSearchResult,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroGrepMatchMode {
    Literal,
    Regex,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroGrepField {
    Title,
    Abstract,
    Extra,
    Note,
    Annotation,
    Fulltext,
    Tag,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroGrepCandidateStrategy {
    QueryFiltered,
    RecentModified,
    ParentScoped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGrepMatch {
    pub item_key: String,
    pub field: String,
    pub match_text: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item_key: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGrepResult {
    pub candidate_strategy: ZoteroGrepCandidateStrategy,
    pub scanned_items: usize,
    pub returned_matches: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    pub matches: Vec<ZoteroGrepMatch>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchNotesMatch {
    pub item_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<String>,
    pub field: String,
    pub snippet: String,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchNotesResult {
    pub query: String,
    pub candidate_strategy: ZoteroGrepCandidateStrategy,
    pub scanned_items: usize,
    pub notes: Vec<ZoteroSearchNotesMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSourceKind {
    ArxivPdf,
    AttachmentPdfUrl,
    LocalPdfPath,
    IndexedFulltext,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DocumentResolution {
    pub source_kind: DocumentSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_url: Option<String>,
    pub fallback_urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    pub trace: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroFullTextResult {
    pub item_key: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<DocumentResolution>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroNote {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroNotesResult {
    pub item_key: String,
    pub notes: Vec<ZoteroNote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAnnotation {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<String>,
    pub annotation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_page_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_sort_index: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item_title: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAnnotationsResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_key: Option<String>,
    pub annotations: Vec<ZoteroAnnotation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAttachment {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAttachmentsResult {
    pub item_key: String,
    pub attachments: Vec<ZoteroAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollection {
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_collection: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCollectionsResult {
    pub collections: Vec<ZoteroCollection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGroup {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGroupsResult {
    pub groups: Vec<ZoteroGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroMutationRecord {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_item: Option<String>,
    pub collection_keys: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroMutationResult {
    pub records: Vec<ZoteroMutationRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCreateCollectionResult {
    pub collection: ZoteroCollection,
    pub created: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PaperRecommendationParams {
    pub positive_paper_ids: Vec<String>,
    pub negative_paper_ids: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RecommendationResult {
    pub papers: Vec<Paper>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ZoteroQuickSearchMode {
    TitleCreatorYear,
    TitleCreatorYearNote,
    Everything,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchParams {
    pub query: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub qmode: Option<ZoteroQuickSearchMode>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroTagsParams {
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroRecentParams {
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub sort_by: Option<ZoteroRecentSortBy>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGrepParams {
    pub pattern: String,
    pub match_mode: Option<ZoteroGrepMatchMode>,
    pub case_sensitive: Option<bool>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub parent_item_key: Option<String>,
    pub query_hint: Option<String>,
    pub item_type: Option<String>,
    pub fields: Option<Vec<ZoteroGrepField>>,
    pub limit_items: Option<u32>,
    pub limit_matches: Option<u32>,
    pub max_matches_per_item: Option<u32>,
    pub context_chars: Option<u32>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchNotesParams {
    pub query: String,
    pub match_mode: Option<ZoteroGrepMatchMode>,
    pub case_sensitive: Option<bool>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub parent_item_key: Option<String>,
    pub include_annotations: Option<bool>,
    pub limit: Option<u32>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroCitationFormat {
    Bibtex,
    CslJson,
    Apa,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZoteroCitationGenerator {
    FallbackFormatter,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCitationResult {
    pub item_key: String,
    pub format: ZoteroCitationFormat,
    pub citation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_key: Option<String>,
    pub generator: ZoteroCitationGenerator,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing, default)]
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAdvancedSearchParams {
    pub conditions: Vec<ZoteroSearchCondition>,
    pub join_mode: Option<ZoteroAdvancedJoinMode>,
    pub sort_by: Option<ZoteroAdvancedSortBy>,
    pub sort_direction: Option<ZoteroSortDirection>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItemParams {
    pub item_key: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub max_chars_per_item: Option<u32>,
    pub include_attachments: Option<bool>,
    pub include_fulltext_resolution: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCitationParams {
    pub item_key: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub format: Option<ZoteroCitationFormat>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAnnotationsParams {
    pub item_key: Option<String>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub include_parent_context: Option<bool>,
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
pub struct ZoteroListGroupsParams {
    pub user_id: Option<String>,
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
    pub max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCreateCollectionParams {
    pub name: String,
    pub parent_collection_key: Option<String>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroFindOrCreateCollectionParams {
    pub name: String,
    pub parent_collection_key: Option<String>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCreateItemsParams {
    pub items: Vec<serde_json::Value>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroItemUpdatePayload {
    pub item_key: String,
    pub patch: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroUpdateItemsParams {
    pub items: Vec<ZoteroItemUpdatePayload>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroAddItemsToCollectionParams {
    pub collection_key: String,
    pub item_keys: Vec<String>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCreateAttachmentLinkParams {
    pub parent_item_key: String,
    pub title: String,
    pub url: String,
    pub content_type: Option<String>,
    pub collections: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroCreateAttachmentImportUrlParams {
    pub parent_item_key: String,
    pub title: String,
    pub url: String,
    pub content_type: Option<String>,
    pub filename: Option<String>,
    pub tags: Option<Vec<String>>,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Hacker News types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnItem {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_comments: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub story_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub story_url: Option<String>,
    pub hn_url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnSearchResult {
    pub items: Vec<HnItem>,
    pub total_hits: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnSearchParams {
    pub query: String,
    pub content_type: Option<String>,
    pub sort_by: Option<String>,
    pub min_points: Option<u32>,
    pub min_comments: Option<u32>,
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub author: Option<String>,
    pub story_id: Option<u64>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnComment {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    pub children: Vec<HnComment>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnThread {
    pub story: HnItem,
    pub comments: Vec<HnComment>,
    pub total_comments: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct HnGetThreadParams {
    pub item_id: u64,
    pub max_depth: Option<u32>,
    pub max_comments: Option<u32>,
}

// ---------------------------------------------------------------------------
// Patent types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PatentSearchParams {
    pub query: String,
    pub inventor: Option<String>,
    pub assignee: Option<String>,
    pub cpc_code: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub sort_by: Option<String>,
    pub size: Option<u32>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PatentItem {
    pub patent_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_date: Option<String>,
    pub inventors: Vec<String>,
    pub assignees: Vec<String>,
    pub cpc_codes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u32>,
    pub url: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PatentSearchResult {
    pub patents: Vec<PatentItem>,
    pub total_hits: u64,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PatentGetParams {
    pub patent_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PatentDetail {
    pub patent_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filing_date: Option<String>,
    pub inventors: Vec<String>,
    pub assignees: Vec<String>,
    pub cpc_codes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claims_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cited_by_count: Option<u32>,
    pub url: String,
}

// === Repo analysis (Feature::ResearchRepoAnalysis) ===

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoSummary {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    pub directory_tree: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme_snippet: Option<String>,
    pub key_files: Vec<String>,
    pub total_files: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelDefinition {
    pub file_path: String,
    pub class_name: String,
    pub line_number: usize,
    pub context: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoRequirements {
    pub repo_url: String,
    pub dependencies: Vec<String>,
    pub source_files: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoEntrypoint {
    pub file_path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_args_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_file: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoIoShape {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_shapes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_shapes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dtype: Option<String>,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoExportPath {
    pub file_path: String,
    pub target_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework_version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ConfigParam {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ConfigSchema {
    pub config_file: String,
    pub format: String,
    pub key_params: Vec<ConfigParam>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RequirementsDiff {
    pub repo_url: String,
    pub conflicts: Vec<String>,
    pub missing_locally: Vec<String>,
    pub compatible: Vec<String>,
    pub repo_total_deps: usize,
    pub local_total_deps: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoHealth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit_date: Option<String>,
    pub stars: u32,
    pub open_issues: u32,
    pub releases_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci_passing: Option<bool>,
}
