use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::Duration;
use tokio::time::interval;

use crate::job::JobDefinition;
use crate::job::loader;
use crate::storage::RunStatus;
use crate::storage::db::SchedulerDb;
use crate::storage::jobs_repo;
use crate::storage::runs_repo;

use super::concurrency::ConcurrencyGuard;
use super::runner;

/// Compute the next run time from a cron expression (normalizes 5-field to 6-field).
fn next_cron_time(cron_expr: &str) -> Option<i64> {
    let normalized = crate::job::normalize_cron(cron_expr);
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    schedule.upcoming(Utc).next().map(|dt| dt.timestamp())
}

/// Compute the next run time from an interval.
fn next_interval_time(interval_minutes: u64) -> i64 {
    Utc::now().timestamp() + (interval_minutes as i64 * 60)
}

/// Compute the next run time for a job based on its schedule.
fn compute_next_run(def: &JobDefinition) -> Option<i64> {
    if let Some(cron_expr) = &def.schedule.cron {
        return next_cron_time(cron_expr);
    }
    if let Some(mins) = def.schedule.interval_minutes {
        return Some(next_interval_time(mins));
    }
    // Trigger-based jobs don't have scheduled next-run times.
    None
}

/// Hash a job definition for change detection.
fn hash_definition(def: &JobDefinition) -> String {
    let serialized = toml::to_string(def).unwrap_or_default();
    use std::hash::Hash;
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Sync TOML files on disk to the database. Adds new jobs, updates changed ones.
async fn sync_jobs(db: &SchedulerDb, defs: &HashMap<String, JobDefinition>) -> anyhow::Result<()> {
    let now = Utc::now().timestamp();

    for (id, def) in defs {
        let hash = hash_definition(def);
        let existing = jobs_repo::get_job(db.pool(), id).await?;

        match existing {
            Some(record) if record.definition_hash == hash => {
                // No changes, skip.
            }
            Some(_) => {
                // Definition changed — update hash and recompute next run.
                let next = compute_next_run(def);
                let record = jobs_repo::JobRecord {
                    id: id.clone(),
                    definition_hash: hash,
                    enabled: def.enabled,
                    paused: false,
                    created_at: now,
                    updated_at: now,
                    last_run_at: None,
                    next_run_at: next,
                    run_count: 0,
                    consecutive_failures: 0,
                };
                jobs_repo::upsert_job(db.pool(), &record).await?;
                tracing::info!("updated job '{id}' (definition changed)");
            }
            None => {
                // New job.
                let next = compute_next_run(def);
                let record = jobs_repo::JobRecord {
                    id: id.clone(),
                    definition_hash: hash,
                    enabled: def.enabled,
                    paused: false,
                    created_at: now,
                    updated_at: now,
                    last_run_at: None,
                    next_run_at: next,
                    run_count: 0,
                    consecutive_failures: 0,
                };
                jobs_repo::upsert_job(db.pool(), &record).await?;
                tracing::info!("registered new job '{id}'");
            }
        }
    }

    Ok(())
}

/// Execute a single due job: create run record, execute, update status.
async fn fire_job(
    db: &SchedulerDb,
    guard: &ConcurrencyGuard,
    job_id: &str,
    def: &JobDefinition,
    runs_dir: &PathBuf,
) {
    // Overlap check.
    let job_guard = match guard
        .try_acquire(job_id, def.execution.skip_if_running)
        .await
    {
        Some(g) => g,
        None => {
            tracing::info!("skipping job '{job_id}': already running");
            if let Err(e) = runner::record_skipped_run(db, job_id).await {
                tracing::error!("failed to record skipped run for '{job_id}': {e:#}");
            }
            return;
        }
    };

    let run_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().timestamp();

    // Insert pending run.
    let run_record = runs_repo::RunRecord {
        id: run_id.clone(),
        job_id: job_id.to_string(),
        status: RunStatus::Running.as_str().to_string(),
        attempt: 1,
        started_at: now,
        finished_at: None,
        duration_ms: None,
        output_path: Some(runs_dir.join(format!("{run_id}.md")).display().to_string()),
        output_preview: None,
        error_message: None,
        trigger_data: None,
        delivery_results: None,
    };
    if let Err(e) = runs_repo::insert_run(db.pool(), &run_record).await {
        tracing::error!("failed to insert run for '{job_id}': {e:#}");
        return;
    }

    // Execute.
    let result = runner::execute_job(db, &run_id, job_id, def, runs_dir).await;

    let finished_at = Utc::now().timestamp();

    match result {
        Ok(run_result) => {
            if let Err(e) = runs_repo::finish_run(
                db.pool(),
                &run_id,
                run_result.status,
                finished_at,
                run_result.duration_ms,
                run_result.output_preview.as_deref(),
                run_result.error_message.as_deref(),
            )
            .await
            {
                tracing::error!("failed to finish run '{run_id}': {e:#}");
            }

            let success = run_result.status == RunStatus::Success;
            let next = compute_next_run(def);
            if let Err(e) =
                jobs_repo::update_after_run(db.pool(), job_id, finished_at, next, success).await
            {
                tracing::error!("failed to update job '{job_id}' after run: {e:#}");
            }

            if success {
                tracing::info!(
                    "job '{job_id}' completed successfully in {}ms",
                    run_result.duration_ms
                );
            } else {
                tracing::warn!(
                    "job '{job_id}' failed: {}",
                    run_result
                        .error_message
                        .as_deref()
                        .unwrap_or("unknown error")
                );
            }
        }
        Err(e) => {
            tracing::error!("job '{job_id}' execution error: {e:#}");
            let _ = runs_repo::finish_run(
                db.pool(),
                &run_id,
                RunStatus::Failed,
                finished_at,
                0,
                None,
                Some(&format!("{e:#}")),
            )
            .await;
            let next = compute_next_run(def);
            let _ = jobs_repo::update_after_run(db.pool(), job_id, finished_at, next, false).await;
        }
    }

    // Keep the guard alive until execution completes.
    drop(job_guard);
}

/// Execute a triggered job (submitted via CLI `ata jobs run` while daemon is running).
/// Unlike `fire_job`, this reuses an existing run record instead of creating a new one.
async fn fire_triggered_job(
    db: &SchedulerDb,
    guard: &ConcurrencyGuard,
    run: &runs_repo::RunRecord,
    def: &JobDefinition,
    runs_dir: &PathBuf,
) {
    let job_id = &run.job_id;

    // Overlap check.
    let job_guard = match guard
        .try_acquire(job_id, def.execution.skip_if_running)
        .await
    {
        Some(g) => g,
        None => {
            tracing::info!(
                "skipping triggered run '{}': job '{job_id}' already running",
                &run.id[..8]
            );
            let now = Utc::now().timestamp();
            let _ = runs_repo::finish_run(
                db.pool(),
                &run.id,
                RunStatus::Skipped,
                now,
                0,
                None,
                Some("skipped: job already running"),
            )
            .await;
            return;
        }
    };

    // Atomically claim: pending → running.
    match runs_repo::set_run_status(db.pool(), &run.id, RunStatus::Pending, RunStatus::Running)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            // Another tick already claimed this run.
            return;
        }
        Err(e) => {
            tracing::error!("failed to claim triggered run '{}': {e:#}", &run.id[..8]);
            return;
        }
    }

    tracing::info!(
        "executing triggered run '{}' for job '{job_id}'",
        &run.id[..8]
    );

    // Execute.
    let result = runner::execute_job(db, &run.id, job_id, def, runs_dir).await;

    let finished_at = Utc::now().timestamp();

    match result {
        Ok(run_result) => {
            if let Err(e) = runs_repo::finish_run(
                db.pool(),
                &run.id,
                run_result.status,
                finished_at,
                run_result.duration_ms,
                run_result.output_preview.as_deref(),
                run_result.error_message.as_deref(),
            )
            .await
            {
                tracing::error!("failed to finish triggered run '{}': {e:#}", &run.id[..8]);
            }

            let success = run_result.status == RunStatus::Success;
            let next = compute_next_run(def);
            let _ =
                jobs_repo::update_after_run(db.pool(), job_id, finished_at, next, success).await;
        }
        Err(e) => {
            tracing::error!("triggered run '{}' execution error: {e:#}", &run.id[..8]);
            let _ = runs_repo::finish_run(
                db.pool(),
                &run.id,
                RunStatus::Failed,
                finished_at,
                0,
                None,
                Some(&format!("{e:#}")),
            )
            .await;
            let next = compute_next_run(def);
            let _ = jobs_repo::update_after_run(db.pool(), job_id, finished_at, next, false).await;
        }
    }

    drop(job_guard);
}

/// Main scheduler loop. Ticks every second, checks for due jobs, fires them.
pub async fn run_scheduler(
    db: SchedulerDb,
    mut shutdown: watch::Receiver<bool>,
    max_concurrent: usize,
) -> anyhow::Result<()> {
    let guard = Arc::new(ConcurrencyGuard::new(max_concurrent));
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    let runs_dir = home.join("scheduler").join("runs");

    // Initial sync.
    let mut defs = load_all_defs()?;
    sync_jobs(&db, &defs).await?;

    let mut tick = interval(Duration::from_secs(1));
    let mut rescan_tick = interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let now = Utc::now().timestamp();
                let due_jobs = match jobs_repo::fetch_due_jobs(db.pool(), now).await {
                    Ok(jobs) => jobs,
                    Err(e) => {
                        tracing::error!("failed to fetch due jobs: {e:#}");
                        continue;
                    }
                };

                for job_record in due_jobs {
                    if let Some(def) = defs.get(&job_record.id) {
                        let db_clone = db.clone();
                        let guard_clone = Arc::clone(&guard);
                        let def = def.clone();
                        let runs_dir = runs_dir.clone();
                        let job_id = job_record.id.clone();
                        tokio::spawn(async move {
                            fire_job(&db_clone, &guard_clone, &job_id, &def, &runs_dir).await;
                        });
                    }
                }

                // Pick up pending runs submitted by sandboxed CLI sessions.
                let pending = match runs_repo::get_pending_triggers(db.pool()).await {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!("failed to fetch pending triggers: {e:#}");
                        Vec::new()
                    }
                };
                if !pending.is_empty() {
                    // Reload defs on-demand so newly created jobs are found
                    // immediately (the user may have just created the TOML).
                    if let Ok(fresh) = load_all_defs()
                        && fresh.len() != defs.len()
                    {
                        defs = fresh;
                        let _ = sync_jobs(&db, &defs).await;
                    }
                }
                for run in pending {
                    if let Some(def) = defs.get(&run.job_id) {
                        let db_clone = db.clone();
                        let guard_clone = Arc::clone(&guard);
                        let def = def.clone();
                        let runs_dir = runs_dir.clone();
                        let run = run.clone();
                        tokio::spawn(async move {
                            fire_triggered_job(&db_clone, &guard_clone, &run, &def, &runs_dir).await;
                        });
                    } else {
                        tracing::warn!(
                            "pending run '{}' references unknown job '{}', marking failed",
                            &run.id[..8],
                            run.job_id
                        );
                        let now = Utc::now().timestamp();
                        let _ = runs_repo::finish_run(
                            db.pool(),
                            &run.id,
                            RunStatus::Failed,
                            now,
                            0,
                            None,
                            Some(&format!("unknown job '{}'", run.job_id)),
                        )
                        .await;
                    }
                }
            }
            _ = rescan_tick.tick() => {
                // Periodically re-read job definitions from disk.
                match load_all_defs() {
                    Ok(new_defs) => {
                        defs = new_defs;
                        if let Err(e) = sync_jobs(&db, &defs).await {
                            tracing::error!("failed to sync jobs: {e:#}");
                        }
                    }
                    Err(e) => {
                        tracing::error!("failed to reload job definitions: {e:#}");
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("scheduler shutting down");
                    break;
                }
            }
        }
    }

    Ok(())
}

fn load_all_defs() -> anyhow::Result<HashMap<String, JobDefinition>> {
    let jobs = loader::load_jobs()?;
    Ok(jobs.into_iter().collect())
}
