use anyhow::Context;
use sqlx::SqlitePool;

/// Get a state value for a job.
pub async fn get_state(
    pool: &SqlitePool,
    job_id: &str,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM job_state WHERE job_id = ?1 AND key = ?2")
            .bind(job_id)
            .bind(key)
            .fetch_optional(pool)
            .await
            .context("failed to get job state")?;
    Ok(row.map(|(v,)| v))
}

/// Set a state value for a job (upsert).
pub async fn set_state(
    pool: &SqlitePool,
    job_id: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    sqlx::query(
        r#"
        INSERT INTO job_state (job_id, key, value, updated_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(job_id, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(job_id)
    .bind(key)
    .bind(value)
    .bind(now)
    .execute(pool)
    .await
    .context("failed to set job state")?;
    Ok(())
}

/// Delete a state key for a job.
pub async fn delete_state(pool: &SqlitePool, job_id: &str, key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM job_state WHERE job_id = ?1 AND key = ?2")
        .bind(job_id)
        .bind(key)
        .execute(pool)
        .await
        .context("failed to delete job state")?;
    Ok(())
}
