mod db;
mod model;
#[cfg(feature = "relay")]
pub mod relay_client;
#[cfg(feature = "relay")]
mod watcher;

pub use db::CoordinationDb;
pub use model::CoordinationMessage;
pub use model::CoordinationSession;
pub use model::agent_name;
#[cfg(feature = "relay")]
pub use model::compute_project_id;
#[cfg(feature = "relay")]
pub use watcher::CoordinationWatcher;

/// Heartbeat interval in seconds.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
