use std::fmt;

use anyhow::Context;
use sqlx::FromRow;
use sqlx::SqlitePool;

/// Status of a job run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Pending,
    Running,
    Success,
    Failed,
    Timeout,
    Skipped,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Success => write!(f, "success"),
            Self::Failed => write!(f, "failed"),
            Self::Timeout => write!(f, "timeout"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "success" => Self::Success,
            "failed" => Self::Failed,
            "timeout" => Self::Timeout,
            "skipped" => Self::Skipped,
            _ => Self::Failed,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Timeout | Self::Skipped
        )
    }
}

/// A row in the `runs` table.
#[derive(Debug, Clone, FromRow)]
pub struct RunRecord {
    pub id: String,
    pub job_id: String,
    pub status: String,
    pub attempt: i64,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub output_path: Option<String>,
    pub output_preview: Option<String>,
    pub error_message: Option<String>,
    pub trigger_data: Option<String>,
    pub delivery_results: Option<String>,
}

/// Insert a new run record.
pub async fn insert_run(pool: &SqlitePool, run: &RunRecord) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO runs (id, job_id, status, attempt, started_at, finished_at, duration_ms, output_path, output_preview, error_message, trigger_data, delivery_results)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
    )
    .bind(&run.id)
    .bind(&run.job_id)
    .bind(&run.status)
    .bind(run.attempt)
    .bind(run.started_at)
    .bind(run.finished_at)
    .bind(run.duration_ms)
    .bind(&run.output_path)
    .bind(&run.output_preview)
    .bind(&run.error_message)
    .bind(&run.trigger_data)
    .bind(&run.delivery_results)
    .execute(pool)
    .await
    .context("failed to insert run")?;
    Ok(())
}

/// Update a run's status and completion fields.
pub async fn finish_run(
    pool: &SqlitePool,
    run_id: &str,
    status: RunStatus,
    finished_at: i64,
    duration_ms: i64,
    output_preview: Option<&str>,
    error_message: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE runs SET status = ?1, finished_at = ?2, duration_ms = ?3, output_preview = ?4, error_message = ?5 WHERE id = ?6",
    )
    .bind(status.as_str())
    .bind(finished_at)
    .bind(duration_ms)
    .bind(output_preview)
    .bind(error_message)
    .bind(run_id)
    .execute(pool)
    .await
    .context("failed to finish run")?;
    Ok(())
}

/// Get runs for a job, ordered by most recent first.
pub async fn list_runs(
    pool: &SqlitePool,
    job_id: &str,
    limit: i64,
) -> anyhow::Result<Vec<RunRecord>> {
    let rows = sqlx::query_as::<_, RunRecord>(
        "SELECT * FROM runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT ?2",
    )
    .bind(job_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("failed to list runs")?;
    Ok(rows)
}

/// Get a specific run by exact ID or unique prefix.
pub async fn get_run(pool: &SqlitePool, run_id: &str) -> anyhow::Result<Option<RunRecord>> {
    // Try exact match first.
    let row = sqlx::query_as::<_, RunRecord>("SELECT * FROM runs WHERE id = ?1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
        .context("failed to fetch run")?;
    if row.is_some() {
        return Ok(row);
    }

    // Fall back to prefix match (users often copy the short 8-char ID from history).
    let pattern = format!("{run_id}%");
    let rows = sqlx::query_as::<_, RunRecord>("SELECT * FROM runs WHERE id LIKE ?1 LIMIT 2")
        .bind(&pattern)
        .fetch_all(pool)
        .await
        .context("failed to fetch run by prefix")?;

    match rows.len() {
        1 => Ok(rows.into_iter().next()),
        0 => Ok(None),
        _ => anyhow::bail!(
            "ambiguous run ID prefix '{run_id}' — matches multiple runs. Use a longer prefix."
        ),
    }
}

/// Check if a job has any currently-running runs.
pub async fn has_running_run(pool: &SqlitePool, job_id: &str) -> anyhow::Result<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM runs WHERE job_id = ?1 AND status = 'running'")
            .bind(job_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.is_some_and(|(count,)| count > 0))
}

/// Fetch runs with status='pending' for the daemon to pick up.
/// These are runs submitted by sandboxed CLI sessions via `ata jobs run`.
pub async fn get_pending_triggers(pool: &SqlitePool) -> anyhow::Result<Vec<RunRecord>> {
    let rows = sqlx::query_as::<_, RunRecord>(
        "SELECT * FROM runs WHERE status = 'pending' ORDER BY started_at LIMIT 10",
    )
    .fetch_all(pool)
    .await
    .context("failed to fetch pending triggers")?;
    Ok(rows)
}

/// Atomically transition a run from one status to another.
/// Returns true if the row was updated (i.e. the run was in the expected state).
pub async fn set_run_status(
    pool: &SqlitePool,
    run_id: &str,
    from: RunStatus,
    to: RunStatus,
) -> anyhow::Result<bool> {
    let result = sqlx::query("UPDATE runs SET status = ?1 WHERE id = ?2 AND status = ?3")
        .bind(to.as_str())
        .bind(run_id)
        .bind(from.as_str())
        .execute(pool)
        .await
        .context("failed to set run status")?;
    Ok(result.rows_affected() > 0)
}

/// Prune old runs older than `before_timestamp`.
pub async fn prune_runs(pool: &SqlitePool, before_timestamp: i64) -> anyhow::Result<u64> {
    let result = sqlx::query("DELETE FROM runs WHERE finished_at IS NOT NULL AND finished_at < ?1")
        .bind(before_timestamp)
        .execute(pool)
        .await
        .context("failed to prune runs")?;
    Ok(result.rows_affected())
}
