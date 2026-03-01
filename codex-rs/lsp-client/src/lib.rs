//! Standalone LSP client library for codex-rs.
//!
//! Provides JSON-RPC transport, server lifecycle management, and query dispatch
//! for Language Server Protocol integration. Zero codex dependencies.

pub use lsp_types;

pub mod builtin_servers;
pub mod client;
pub mod config_merge;
pub mod error;
pub mod jsonrpc;
pub mod language;
pub mod root_discovery;
pub mod server_config;
pub mod server_registry;

pub use client::LspClient;
pub use config_merge::{UserServerFull, UserServerOverride, merge_configs};
pub use error::LspError;
pub use server_config::{InstallConfig, InstallMethod, LspServerConfig};
pub use server_registry::ServerRegistry;
