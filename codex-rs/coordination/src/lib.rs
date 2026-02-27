mod db;
mod model;

pub use db::CoordinationDb;
pub use model::CoordinationMessage;
pub use model::CoordinationSession;

/// Heartbeat interval in seconds.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
