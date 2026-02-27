use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use tracing::warn;

use crate::model::CoordinationMessage;
use crate::model::CoordinationSession;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Maximum number of recent messages returned by [`CoordinationDb::recent_messages`].
const MAX_RECENT_MESSAGES: i32 = 20;

/// Sessions with no heartbeat for longer than this are considered stale.
const SESSION_STALE_SECS: i64 = 300;

/// Handle to the shared coordination SQLite database.
#[derive(Clone)]
pub struct CoordinationDb {
    pool: SqlitePool,
}

impl CoordinationDb {
    /// Open (or create) the coordination database at `<codex_home>/coordination.sqlite`.
    pub async fn open(codex_home: &Path) -> anyhow::Result<Self> {
        let db_path = codex_home.join("coordination.sqlite");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .with_context(|| format!("failed to open coordination db at {}", db_path.display()))?;
        MIGRATOR
            .run(&pool)
            .await
            .context("failed to run coordination migrations")?;
        Ok(Self { pool })
    }

    /// Register a new session.
    pub async fn register_session(
        &self,
        session_id: &str,
        repo_path: &str,
        branch: Option<&str>,
        description: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO sessions (session_id, repo_path, branch, description, started_at, last_heartbeat)
             VALUES (?, ?, ?, ?, unixepoch(), unixepoch())",
        )
        .bind(session_id)
        .bind(repo_path)
        .bind(branch)
        .bind(description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update the session description (e.g. from the first user prompt).
    pub async fn update_description(
        &self,
        session_id: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE sessions SET description = ? WHERE session_id = ?")
            .bind(description)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update the heartbeat timestamp for a session.
    pub async fn heartbeat(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE sessions SET last_heartbeat = unixepoch() WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove a session and its messages on shutdown.
    pub async fn deregister_session(&self, session_id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM messages WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Post a message to the coordination channel.
    pub async fn post_message(
        &self,
        session_id: &str,
        repo_path: &str,
        message: &str,
        message_type: Option<&str>,
    ) -> anyhow::Result<()> {
        let msg_type = message_type.unwrap_or("info");
        sqlx::query(
            "INSERT INTO messages (session_id, repo_path, message, message_type, created_at)
             VALUES (?, ?, ?, ?, unixepoch())",
        )
        .bind(session_id)
        .bind(repo_path)
        .bind(message)
        .bind(msg_type)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Fetch recent messages for a repo, newest last.
    pub async fn recent_messages(
        &self,
        repo_path: &str,
    ) -> anyhow::Result<Vec<CoordinationMessage>> {
        let mut rows: Vec<CoordinationMessage> = sqlx::query_as(
            "SELECT id, session_id, repo_path, message, message_type, created_at
             FROM messages
             WHERE repo_path = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(repo_path)
        .bind(MAX_RECENT_MESSAGES)
        .fetch_all(&self.pool)
        .await?;
        // Return in chronological order (oldest first).
        rows.reverse();
        Ok(rows)
    }

    /// Return all active (non-stale) sessions for a given repo.
    pub async fn active_sessions(
        &self,
        repo_path: &str,
    ) -> anyhow::Result<Vec<CoordinationSession>> {
        let rows: Vec<CoordinationSession> = sqlx::query_as(
            "SELECT session_id, repo_path, branch, description, started_at, last_heartbeat
             FROM sessions
             WHERE repo_path = ?
               AND last_heartbeat > unixepoch() - ?",
        )
        .bind(repo_path)
        .bind(SESSION_STALE_SECS)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Prune stale sessions and old messages (> 7 days).
    pub async fn prune_old(&self) {
        if let Err(e) = sqlx::query("DELETE FROM sessions WHERE last_heartbeat < unixepoch() - ?")
            .bind(SESSION_STALE_SECS)
            .execute(&self.pool)
            .await
        {
            warn!("failed to prune stale sessions: {e}");
        }
        // Delete messages older than 7 days.
        let seven_days: i64 = 7 * 24 * 3600;
        if let Err(e) = sqlx::query("DELETE FROM messages WHERE created_at < unixepoch() - ?")
            .bind(seven_days)
            .execute(&self.pool)
            .await
        {
            warn!("failed to prune old messages: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    async fn temp_db() -> (CoordinationDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = CoordinationDb::open(dir.path())
            .await
            .expect("open coordination db");
        (db, dir)
    }

    #[tokio::test]
    async fn register_and_list_sessions() {
        let (db, _dir) = temp_db().await;
        db.register_session("s1", "/repo", Some("main"), None)
            .await
            .expect("register");
        db.register_session("s2", "/repo", Some("feature"), None)
            .await
            .expect("register");
        db.register_session("s3", "/other", None, None)
            .await
            .expect("register");

        let sessions = db.active_sessions("/repo").await.expect("active");
        assert_eq!(sessions.len(), 2);
    }

    #[tokio::test]
    async fn post_and_fetch_messages() {
        let (db, _dir) = temp_db().await;
        db.register_session("s1", "/repo", None, None)
            .await
            .expect("register");
        db.post_message("s1", "/repo", "hello world", None)
            .await
            .expect("post");
        db.post_message("s1", "/repo", "working on auth", Some("intent"))
            .await
            .expect("post");

        let msgs = db.recent_messages("/repo").await.expect("recent");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].message, "hello world");
        assert_eq!(msgs[1].message_type, "intent");
    }

    #[tokio::test]
    async fn deregister_removes_session() {
        let (db, _dir) = temp_db().await;
        db.register_session("s1", "/repo", None, None)
            .await
            .expect("register");
        db.deregister_session("s1").await.expect("deregister");

        let sessions = db.active_sessions("/repo").await.expect("active");
        assert_eq!(sessions.len(), 0);
    }
}
