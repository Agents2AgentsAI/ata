use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

/// Workspace manifest (schema v2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub manifest_version: u64,
    pub repos: Vec<RepoEntry>,
    pub runs: Vec<RunEntry>,
    #[serde(default)]
    pub papers: Vec<Value>,
    #[serde(default)]
    pub datasets: Vec<Value>,
    #[serde(default)]
    pub artifacts: Vec<Value>,
    #[serde(default)]
    pub links: Vec<Value>,
    #[serde(default)]
    pub snapshots: Vec<Value>,
    #[serde(default)]
    pub indexes: Vec<Value>,
    pub policies: Policies,
    pub knowledge_base: KnowledgeBase,
    #[serde(default)]
    pub labels: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Repository entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    pub id: String,
    pub alias: String,
    pub repo_key: String,
    pub remote_url: String,
    pub checkout_path: String,
    pub notes_path: String,
    pub clone: CloneRecord,
    pub pin: PinState,
    pub state: GitState,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Run entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEntry {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub root_path: String,
    pub status: RunStatus,
    pub source: RunSource,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Source info for a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSource {
    pub repo_alias: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Record of how a repo was cloned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneRecord {
    #[serde(default)]
    pub depth: i64,
    #[serde(default)]
    pub single_branch: bool,
    #[serde(default)]
    pub no_tags: bool,
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub submodules: SubmodulePolicy,
    #[serde(default)]
    pub lfs: LfsPolicy,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Pin state for a repo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinState {
    pub mode: PinMode,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pinned_sha: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Git state of a checked-out repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub head_ref: String,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub shallow: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Workspace policies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policies {
    #[serde(default)]
    pub default_clone: DefaultClonePolicy,
    /// `None` = allow all hosts; empty `[]` = block all hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_hosts_allowlist: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Default clone policy settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultClonePolicy {
    #[serde(default = "default_depth")]
    pub depth: i64,
    #[serde(default = "default_true")]
    pub single_branch: bool,
    #[serde(default = "default_true")]
    pub no_tags: bool,
    #[serde(default = "default_filter")]
    pub filter: String,
    #[serde(default = "default_submodules")]
    pub submodules: SubmodulePolicy,
    #[serde(default = "default_lfs")]
    pub lfs: LfsPolicy,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::str::FromStr for RunStatus {
    type Err = crate::error::WorkspaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "created" => Ok(Self::Created),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::error::WorkspaceError::InvalidRunStatus(
                other.to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinMode {
    #[default]
    Tracking,
    Pinned,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubmodulePolicy {
    #[default]
    None,
    Recursive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LfsPolicy {
    #[default]
    Auto,
    Always,
    Never,
}

fn default_depth() -> i64 {
    1
}
fn default_true() -> bool {
    true
}
fn default_filter() -> String {
    "blob:limit=1m".to_string()
}
fn default_submodules() -> SubmodulePolicy {
    SubmodulePolicy::None
}
fn default_lfs() -> LfsPolicy {
    LfsPolicy::Auto
}

impl Default for DefaultClonePolicy {
    fn default() -> Self {
        Self {
            depth: 1,
            single_branch: true,
            no_tags: true,
            filter: "blob:limit=1m".to_string(),
            submodules: SubmodulePolicy::None,
            lfs: LfsPolicy::Auto,
            extra: Map::new(),
        }
    }
}

impl Default for Policies {
    fn default() -> Self {
        Self {
            default_clone: DefaultClonePolicy::default(),
            repo_hosts_allowlist: None,
            extra: Map::new(),
        }
    }
}

/// Knowledge base config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBase {
    pub path: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Workspace selection file schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSelection {
    pub schema_version: u32,
    pub active_workspace_id: String,
    pub updated_at: i64,
}

/// Audit entry envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub schema_version: u32,
    pub ts: String,
    pub workspace_id: String,
    pub actor: AuditActor,
    pub op: String,
    pub status: String,
    #[serde(default)]
    pub targets: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Audit actor info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditActor {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Summary info for workspace listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    pub updated_at: i64,
    pub repo_count: usize,
}

/// Schema version constant.
pub const SCHEMA_VERSION: u32 = 2;
pub const MANIFEST_COLLECTIONS: &[&str] = &[
    "papers",
    "datasets",
    "artifacts",
    "links",
    "snapshots",
    "indexes",
];

pub fn manifest_collection_mut<'a>(
    manifest: &'a mut WorkspaceManifest,
    collection: &str,
) -> Result<&'a mut Vec<Value>, crate::error::WorkspaceError> {
    if !MANIFEST_COLLECTIONS.contains(&collection) {
        return Err(crate::error::WorkspaceError::UnknownCollection(
            collection.to_string(),
        ));
    }

    match collection {
        "papers" => Ok(&mut manifest.papers),
        "datasets" => Ok(&mut manifest.datasets),
        "artifacts" => Ok(&mut manifest.artifacts),
        "links" => Ok(&mut manifest.links),
        "snapshots" => Ok(&mut manifest.snapshots),
        "indexes" => Ok(&mut manifest.indexes),
        _ => unreachable!("manifest collections list must stay in sync"),
    }
}

/// Create a new default manifest.
pub fn new_manifest(workspace_id: &str, name: &str) -> WorkspaceManifest {
    let now = chrono::Utc::now().timestamp();
    WorkspaceManifest {
        schema_version: SCHEMA_VERSION,
        id: workspace_id.to_string(),
        name: name.to_string(),
        created_at: now,
        updated_at: now,
        manifest_version: 1,
        repos: Vec::new(),
        runs: Vec::new(),
        papers: Vec::new(),
        datasets: Vec::new(),
        artifacts: Vec::new(),
        links: Vec::new(),
        snapshots: Vec::new(),
        indexes: Vec::new(),
        policies: Policies::default(),
        knowledge_base: KnowledgeBase {
            path: "knowledge-base".to_string(),
            extra: Map::new(),
        },
        labels: Map::new(),
        extra: Map::new(),
    }
}
