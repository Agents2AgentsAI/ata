use serde::Deserialize;
use serde::Serialize;

/// A registered coordination session.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CoordinationSession {
    pub session_id: String,
    pub repo_path: String,
    pub branch: Option<String>,
    pub description: Option<String>,
    pub started_at: i64,
    pub last_heartbeat: i64,
}

/// A message posted to the coordination channel.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CoordinationMessage {
    pub id: i64,
    pub session_id: String,
    pub repo_path: String,
    pub message: String,
    pub message_type: String,
    pub created_at: i64,
}
