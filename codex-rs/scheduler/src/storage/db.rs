use std::path::PathBuf;

use anyhow::Context;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;

const MIGRATION_SQL: &str = include_str!("../../migrations/001_init.sql");

/// Wrapper around a SQLite connection pool for scheduler data.
#[derive(Clone)]
pub struct SchedulerDb {
    pool: SqlitePool,
}

impl SchedulerDb {
    /// Open (or create) the scheduler database at the given path.
    pub async fn open(path: &PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .with_context(|| format!("failed to open scheduler DB at {}", path.display()))?;

        // Run migrations.
        sqlx::raw_sql(MIGRATION_SQL)
            .execute(&pool)
            .await
            .context("failed to run scheduler DB migrations")?;

        Ok(Self { pool })
    }

    /// Open the default scheduler database at `~/.ata/scheduler.sqlite`.
    pub async fn open_default() -> anyhow::Result<Self> {
        let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
        let path = home.join("scheduler.sqlite");
        Self::open(&path).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the database pool gracefully.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}
