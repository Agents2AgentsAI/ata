use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

/// Metadata about the source of data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceMeta {
    pub source: String,
    pub api_url: String,
    pub fetched_at: DateTime<Utc>,
    pub canonical_id: Option<String>,
}

/// A dataset from any source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub source: DataSource,
    pub description: Option<String>,
    pub size_bytes: Option<u64>,
    pub num_samples: Option<u64>,
    pub num_files: Option<u32>,
    pub formats: Vec<String>,
    pub modalities: Vec<Modality>,
    pub tags: Vec<String>,
    pub license: Option<String>,
    pub author: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    pub url: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

/// Where the dataset comes from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DataSource {
    HuggingFace { repo_id: String },
    Kaggle { owner: String, slug: String },
    IsaacSim { scene_id: String },
    Cosmos { generation_id: String },
    S3 { bucket: String, prefix: String },
    Gcs { bucket: String, prefix: String },
    Local { path: String },
    Url { url: String },
}

/// Data modality
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Image,
    Video,
    Audio,
    Text,
    Tabular,
    PointCloud,
    Mesh,
    Depth,
    Segmentation,
    BoundingBox,
    Keypoints,
    Other(String),
}

/// A file within a dataset
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetFile {
    pub path: String,
    pub size_bytes: Option<u64>,
    pub blob_id: Option<String>,
    pub lfs: bool,
}

/// Dataset split information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetSplit {
    pub name: String,
    pub num_samples: Option<u64>,
    pub num_bytes: Option<u64>,
}

/// Detailed dataset information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetDetail {
    pub dataset: Dataset,
    pub files: Vec<DatasetFile>,
    pub splits: Vec<DatasetSplit>,
    pub readme: Option<String>,
    pub config: Option<serde_json::Value>,
}

// === Search Parameters ===

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatasetSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatasetGetParams {
    pub dataset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_files: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_readme: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatasetFilesParams {
    pub dataset_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetDownloadParams {
    pub dataset_id: String,
    pub output_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

// === Results ===

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceResult {
    Ok { count: usize },
    Error { message: String, retryable: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceStatus {
    pub source: String,
    pub status: SourceResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetSearchResult {
    pub datasets: Vec<Dataset>,
    pub per_source_status: Vec<SourceStatus>,
    pub warnings: Vec<String>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetFilesResult {
    pub files: Vec<DatasetFile>,
    pub total_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatasetDownloadResult {
    pub dataset_id: String,
    pub output_path: String,
    pub files_downloaded: u32,
    pub total_bytes: u64,
    pub duration_secs: f64,
}

// === Competitions ===

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KaggleCompetitionsParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Competition {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub description: Option<String>,
    pub organization: Option<String>,
    pub category: Option<String>,
    pub reward: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
    pub team_count: Option<u64>,
    pub evaluation_metric: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KaggleCompetitionsResult {
    pub competitions: Vec<Competition>,
    pub total_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KaggleCompetitionFilesParams {
    pub competition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KaggleCompetitionFilesResult {
    pub competition: String,
    pub files: Vec<DatasetFile>,
    pub total_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaggleCompetitionDownloadParams {
    pub competition: String,
    pub output_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KaggleCompetitionDownloadResult {
    pub competition: String,
    pub output_path: String,
    pub files_downloaded: u32,
    pub total_bytes: u64,
    pub duration_secs: f64,
}

// === Pagination ===

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PaginationParams {
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

impl PaginationParams {
    #[must_use]
    pub fn offset_or(&self, default: u32) -> u32 {
        self.offset.unwrap_or(default)
    }

    #[must_use]
    pub fn limit_or(&self, default: u32) -> u32 {
        self.limit.unwrap_or(default)
    }
}
