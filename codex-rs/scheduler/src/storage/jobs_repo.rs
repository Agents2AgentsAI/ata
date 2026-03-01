use anyhow::Context;
use sqlx::FromRow;
use sqlx::SqlitePool;

/// A row in the `jobs` table.
#[derive(Debug, Clone, FromRow)]
pub struct JobRecord {
    pub id: String,
    pub definition_hash: String,
    pub enabled: bool,
    pub paused: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub run_count: i64,
    pub consecutive_failures: i64,
}

/// Insert or update a job record, keyed by ID.
pub async fn upsert_job(pool: &SqlitePool, job: &JobRecord) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO jobs (id, definition_hash, enabled, paused, created_at, updated_at, last_run_at, next_run_at, run_count, consecutive_failures)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(id) DO UPDATE SET
            definition_hash = excluded.definition_hash,
            enabled = excluded.enabled,
            updated_at = excluded.updated_at,
            next_run_at = excluded.next_run_at
        "#,
    )
    .bind(&job.id)
    .bind(&job.definition_hash)
    .bind(job.enabled)
    .bind(job.paused)
    .bind(job.created_at)
    .bind(job.updated_at)
    .bind(job.last_run_at)
    .bind(job.next_run_at)
    .bind(job.run_count)
    .bind(job.consecutive_failures)
    .execute(pool)
    .await
    .context("failed to upsert job")?;
    Ok(())
}

/// Get a single job record by ID.
pub async fn get_job(pool: &SqlitePool, id: &str) -> anyhow::Result<Option<JobRecord>> {
    let row = sqlx::query_as::<_, JobRecord>("SELECT * FROM jobs WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("failed to fetch job")?;
    Ok(row)
}

/// List all job records.
pub async fn list_jobs(pool: &SqlitePool) -> anyhow::Result<Vec<JobRecord>> {
    let rows = sqlx::query_as::<_, JobRecord>("SELECT * FROM jobs ORDER BY id")
        .fetch_all(pool)
        .await
        .context("failed to list jobs")?;
    Ok(rows)
}

/// Delete a job record and its associated runs/state (cascading).
pub async fn delete_job(pool: &SqlitePool, id: &str) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM jobs WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await
        .context("failed to delete job")?;
    Ok(result.rows_affected() > 0)
}

/// Set the `paused` flag on a job.
pub async fn set_paused(pool: &SqlitePool, id: &str, paused: bool) -> anyhow::Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result = sqlx::query("UPDATE jobs SET paused = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(paused)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await
        .context("failed to set paused")?;
    Ok(result.rows_affected() > 0)
}

/// Update `last_run_at`, `next_run_at`, `run_count`, and `consecutive_failures`.
pub async fn update_after_run(
    pool: &SqlitePool,
    id: &str,
    last_run_at: i64,
    next_run_at: Option<i64>,
    success: bool,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    if success {
        sqlx::query(
            "UPDATE jobs SET last_run_at = ?1, next_run_at = ?2, run_count = run_count + 1, consecutive_failures = 0, updated_at = ?3 WHERE id = ?4",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE jobs SET last_run_at = ?1, next_run_at = ?2, run_count = run_count + 1, consecutive_failures = consecutive_failures + 1, updated_at = ?3 WHERE id = ?4",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Fetch jobs that are due to run (enabled, not paused, `next_run_at <= now`).
pub async fn fetch_due_jobs(pool: &SqlitePool, now: i64) -> anyhow::Result<Vec<JobRecord>> {
    let rows = sqlx::query_as::<_, JobRecord>(
        "SELECT * FROM jobs WHERE enabled = 1 AND paused = 0 AND next_run_at IS NOT NULL AND next_run_at <= ?1 ORDER BY next_run_at",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .context("failed to fetch due jobs")?;
    Ok(rows)
}
