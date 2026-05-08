use super::*;

/// Read a file from the host filesystem.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsReadFileParams {
    /// Absolute path to read.
    pub path: AbsolutePathBuf,
}

/// Base64-encoded file contents returned by `fs/readFile`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsReadFileResponse {
    /// File contents encoded as base64.
    pub data_base64: String,
}

/// Write a file on the host filesystem.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsWriteFileParams {
    /// Absolute path to write.
    pub path: AbsolutePathBuf,
    /// File contents encoded as base64.
    pub data_base64: String,
}

/// Successful response for `fs/writeFile`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsWriteFileResponse {}

/// Create a directory on the host filesystem.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsCreateDirectoryParams {
    /// Absolute directory path to create.
    pub path: AbsolutePathBuf,
    /// Whether parent directories should also be created. Defaults to `true`.
    #[ts(optional = nullable)]
    pub recursive: Option<bool>,
}

/// Successful response for `fs/createDirectory`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsCreateDirectoryResponse {}

/// Request metadata for an absolute path.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsGetMetadataParams {
    /// Absolute path to inspect.
    pub path: AbsolutePathBuf,
}

/// Metadata returned by `fs/getMetadata`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsGetMetadataResponse {
    /// Whether the path currently resolves to a directory.
    pub is_directory: bool,
    /// Whether the path currently resolves to a regular file.
    pub is_file: bool,
    /// File creation time in Unix milliseconds when available, otherwise `0`.
    #[ts(type = "number")]
    pub created_at_ms: i64,
    /// File modification time in Unix milliseconds when available, otherwise `0`.
    #[ts(type = "number")]
    pub modified_at_ms: i64,
}

/// List direct child names for a directory.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsReadDirectoryParams {
    /// Absolute directory path to read.
    pub path: AbsolutePathBuf,
}

/// A directory entry returned by `fs/readDirectory`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsReadDirectoryEntry {
    /// Direct child entry name only, not an absolute or relative path.
    pub file_name: String,
    /// Whether this entry resolves to a directory.
    pub is_directory: bool,
    /// Whether this entry resolves to a regular file.
    pub is_file: bool,
}

/// Directory entries returned by `fs/readDirectory`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsReadDirectoryResponse {
    /// Direct child entries in the requested directory.
    pub entries: Vec<FsReadDirectoryEntry>,
}

/// Remove a file or directory tree from the host filesystem.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsRemoveParams {
    /// Absolute path to remove.
    pub path: AbsolutePathBuf,
    /// Whether directory removal should recurse. Defaults to `true`.
    #[ts(optional = nullable)]
    pub recursive: Option<bool>,
    /// Whether missing paths should be ignored. Defaults to `true`.
    #[ts(optional = nullable)]
    pub force: Option<bool>,
}

/// Successful response for `fs/remove`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsRemoveResponse {}

/// Copy a file or directory tree on the host filesystem.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsCopyParams {
    /// Absolute source path.
    pub source_path: AbsolutePathBuf,
    /// Absolute destination path.
    pub destination_path: AbsolutePathBuf,
    /// Required for directory copies; ignored for file copies.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recursive: bool,
}

/// Successful response for `fs/copy`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FsCopyResponse {}
