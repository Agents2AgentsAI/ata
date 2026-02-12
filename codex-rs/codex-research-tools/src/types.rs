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
    pub warnings: Vec<String>,
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
    pub parent_item_key: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGrepResult {
    pub candidate_strategy: ZoteroGrepCandidateStrategy,
    pub scanned_items: usize,
    pub returned_matches: usize,
    pub truncated: bool,
    pub warnings: Vec<String>,
    pub hints: Vec<String>,
    pub matches: Vec<ZoteroGrepMatch>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchNoteMatch {
    pub item_key: String,
    pub parent_item: Option<String>,
    pub field: String,
    pub snippet: String,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroSearchNotesResult {
    pub query: String,
    pub notes: Vec<ZoteroSearchNoteMatch>,
    pub total_available: Option<u64>,
    pub has_more: bool,
    pub warnings: Vec<String>,
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
pub struct ZoteroGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ZoteroGroupsResult {
    pub groups: Vec<ZoteroGroup>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoHealth {
    pub license: Option<String>,
    pub last_commit_date: Option<String>,
    pub stars: u32,
    pub open_issues: u32,
    pub releases_count: u32,
    pub ci_passing: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoSummary {
    pub url: String,
    pub commit_sha: Option<String>,
    pub directory_tree: String,
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
    pub cli_args_summary: Option<String>,
    pub config_file: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoIoShape {
    pub input_shapes: Option<String>,
    pub output_shapes: Option<String>,
    pub dtype: Option<String>,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoExportPath {
    pub file_path: String,
    pub target_format: String,
    pub framework_version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ConfigParam {
    pub name: String,
    pub default: Option<String>,
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
pub struct ZoteroSearchParams {
    pub query: String,
    pub library_type: Option<String>,
    pub library_id: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    pub item_type: Option<String>,
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
pub struct LatexCompileParams {
    pub content: String,
    pub output_dir: String,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LatexCompileResult {
    pub success: bool,
    pub pdf_path: Option<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub num_pages: Option<usize>,
    pub compilation_log_snippet: Option<String>,
}
