//! Error types for the LSP client.

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("server process exited unexpectedly")]
    ServerExited,

    #[error("initialize handshake timed out")]
    InitializeTimeout,

    #[error("shutdown timed out")]
    ShutdownTimeout,

    #[error("request timed out (id={0})")]
    RequestTimeout(i64),

    #[error("server returned error: code={code}, message={message}")]
    ServerError { code: i64, message: String },

    #[error("invalid Content-Length header")]
    InvalidHeader,

    #[error("server binary not found: {0}")]
    BinaryNotFound(String),

    #[error("install failed: {0}")]
    InstallFailed(String),

    #[error("spawn failed: {0}")]
    SpawnFailed(String),

    #[error("server process exited immediately (status={status}): {stderr}")]
    ProcessExitedImmediately { status: String, stderr: String },
}
