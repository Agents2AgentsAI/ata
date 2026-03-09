use std::path::PathBuf;

/// Errors that can occur during workspace operations.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace id must not be empty")]
    EmptyId,

    #[error("workspace id must not be '.' or '..'")]
    DotId,

    #[error("workspace id exceeds {max} characters")]
    IdTooLong { max: usize },

    #[error("workspace id '{id}' must not start or end with '-' or '_'")]
    IdBadEdge { id: String },

    #[error("workspace id '{id}' contains invalid characters; allowed: [a-z0-9_-]")]
    IdBadChars { id: String },

    #[error("workspace manifest not found: {0}")]
    ManifestNotFound(PathBuf),

    #[error("workspace '{0}' not found")]
    WorkspaceNotFound(String),

    #[error("cannot delete the global workspace")]
    DeleteGlobal,

    #[error("workspace deletion requires --force")]
    DeleteRequiresForce,

    #[error("path spec must start with '@'")]
    SpecMissingAt,

    #[error("'{0}' is a reserved alias")]
    ReservedAlias(String),

    #[error("invalid alias '{alias}': allowed chars [a-zA-Z0-9_-]")]
    InvalidAlias { alias: String },

    #[error("alias '{0}' already exists in workspace")]
    AliasExists(String),

    #[error("path suffix must be relative, got: {0}")]
    AbsolutePathSuffix(String),

    #[error("path suffix must not contain '..': {0}")]
    PathTraversal(String),

    #[error("resolved path escapes workspace root: {0}")]
    PathEscape(String),

    #[error("only https:// URLs allowed, got: {0}")]
    NotHttps(String),

    #[error("embedded credentials not allowed in URL")]
    EmbeddedCredentials,

    #[error("token query parameters not allowed in URL")]
    TokenQueryParams,

    #[error("GitHub PAT/token patterns not allowed in URL")]
    GitHubPatDetected,

    #[error("host '{host}' not in allowlist: {allowlist:?}")]
    HostNotAllowed {
        host: String,
        allowlist: Vec<String>,
    },

    #[error("timeout acquiring lock after {timeout_secs}s: {path}")]
    LockTimeout { path: PathBuf, timeout_secs: u64 },

    #[error("version conflict: expected {expected}, got {actual}")]
    VersionConflict { expected: u64, actual: u64 },

    #[error("source repo alias '{0}' not found in workspace")]
    SourceRepoNotFound(String),

    #[error("invalid strategy '{0}': must be worktree, copy, or clone")]
    InvalidStrategy(String),

    #[error("git clone failed (exit {0})")]
    GitCloneFailed(i32),

    #[error("git worktree add failed: {0}")]
    GitWorktreeFailed(String),

    #[error("no command specified for run-locked")]
    NoCommand,

    #[error("--target-id is required for lock level '{0}'")]
    TargetIdRequired(String),

    #[error("unknown lock level: {0}")]
    UnknownLockLevel(String),

    #[error("unknown recipe '{0}'")]
    UnknownRecipe(String),

    #[error("spec file not found: {0}")]
    SpecNotFound(PathBuf),

    #[error("invalid spec: {0}")]
    InvalidSpec(String),

    #[error("invalid JSON: {0}")]
    InvalidJson(String),

    #[error("invalid commit SHA '{0}': expected 40 hexadecimal characters")]
    InvalidSha(String),

    #[error("invalid run status '{0}': expected created, running, completed, failed, or cancelled")]
    InvalidRunStatus(String),

    #[error("field path '{0}' is managed by the system and cannot be set")]
    ProtectedFieldPath(String),

    #[error("exhausted workspace id retries for '{name}' after {max_attempts} attempts")]
    InitRetryLimitExceeded { name: String, max_attempts: u32 },

    #[error("collection '{0}' not found in manifest")]
    UnknownCollection(String),

    #[error("entry with id '{0}' not found")]
    EntryNotFound(String),

    #[error("field path '{0}' is invalid or unsupported")]
    InvalidFieldPath(String),

    #[error("directory already exists: {0}")]
    DirectoryExists(PathBuf),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Json(#[from] serde_json::Error),

    #[error("@run requires a run id: @run/<id>[/path]")]
    RunIdRequired,

    #[error("@ws requires a workspace id: @ws/<id>[/path]")]
    WsIdRequired,
}

impl WorkspaceError {
    /// Returns exit code 2 for version conflict, 1 for everything else.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::VersionConflict { .. } => 2,
            _ => 1,
        }
    }
}
