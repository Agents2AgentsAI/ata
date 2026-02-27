use std::path::PathBuf;

use clap::Parser;

use crate::daemon;
use crate::job::loader;
use crate::storage::db::SchedulerDb;
use crate::storage::jobs_repo;
use crate::storage::runs_repo;

// ── Jobs subcommand ──────────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct JobsCli {
    #[command(subcommand)]
    pub command: JobsCommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum JobsCommand {
    /// List all scheduled jobs with status and next run time.
    List,
    /// Show details for a specific job.
    Show(JobNameArg),
    /// Create a template job TOML file.
    Create(JobNameArg),
    /// Delete a job and its run history.
    Delete(JobNameArg),
    /// Pause scheduling for a job.
    Pause(JobNameArg),
    /// Resume scheduling for a paused job.
    Resume(JobNameArg),
    /// Trigger an immediate run of a job.
    Run(JobNameArg),
    /// Show run history for a job.
    History(JobHistoryArg),
    /// Show full output of a specific run.
    Logs(RunIdArg),
}

#[derive(Debug, Parser)]
pub struct JobNameArg {
    /// Job name (filename without .toml extension).
    pub name: String,
}

#[derive(Debug, Parser)]
pub struct JobHistoryArg {
    /// Job name.
    pub name: String,
    /// Maximum number of runs to show.
    #[arg(long, default_value_t = 20)]
    pub limit: i64,
}

#[derive(Debug, Parser)]
pub struct RunIdArg {
    /// Run ID (UUID).
    pub run_id: String,
}

// ── Scheduler subcommand ─────────────────────────────────────────────

#[derive(Debug, Parser)]
pub struct SchedulerCli {
    #[command(subcommand)]
    pub command: SchedulerCommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum SchedulerCommand {
    /// Start the scheduler daemon.
    Start(SchedulerStartArgs),
    /// Stop the running scheduler daemon.
    Stop,
    /// Show scheduler daemon status.
    Status,
}

#[derive(Debug, Parser)]
pub struct SchedulerStartArgs {
    /// Run the daemon in the background (fork and detach).
    #[arg(long, short = 'd')]
    pub daemon: bool,
}

// ── Dispatch ─────────────────────────────────────────────────────────

pub async fn run_jobs_command(cli: JobsCli) -> anyhow::Result<()> {
    match cli.command {
        JobsCommand::List => cmd_list().await,
        JobsCommand::Show(arg) => cmd_show(&arg.name).await,
        JobsCommand::Create(arg) => cmd_create(&arg.name).await,
        JobsCommand::Delete(arg) => cmd_delete(&arg.name).await,
        JobsCommand::Pause(arg) => cmd_pause(&arg.name).await,
        JobsCommand::Resume(arg) => cmd_resume(&arg.name).await,
        JobsCommand::Run(arg) => cmd_run(&arg.name).await,
        JobsCommand::History(arg) => cmd_history(&arg.name, arg.limit).await,
        JobsCommand::Logs(arg) => cmd_logs(&arg.run_id).await,
    }
}

pub async fn run_scheduler_command(cli: SchedulerCli) -> anyhow::Result<()> {
    match cli.command {
        SchedulerCommand::Start(args) => {
            if args.daemon {
                daemon::start_daemon_background()
            } else {
                daemon::start_daemon().await
            }
        }
        SchedulerCommand::Stop => daemon::stop_daemon(),
        SchedulerCommand::Status => cmd_status(),
    }
}

// ── Command implementations ──────────────────────────────────────────

async fn cmd_list() -> anyhow::Result<()> {
    let db = SchedulerDb::open_default().await?;
    let records = jobs_repo::list_jobs(db.pool()).await?;
    let defs = loader::load_jobs()?;
    let defs_map: std::collections::HashMap<_, _> = defs.into_iter().collect();

    if records.is_empty() && defs_map.is_empty() {
        println!("No jobs found. Create one with `ata jobs create <name>`.");
        return Ok(());
    }

    println!(
        "{:<20} {:<10} {:<8} {:<6} {:<24} {:<24}",
        "NAME", "STATUS", "RUNS", "FAILS", "LAST RUN", "NEXT RUN"
    );
    println!("{}", "-".repeat(92));

    // Show jobs from disk that may not be in the DB yet.
    let mut seen = std::collections::HashSet::new();
    for record in &records {
        seen.insert(record.id.clone());
        let status = if !record.enabled {
            "disabled"
        } else if record.paused {
            "paused"
        } else {
            "active"
        };
        let last_run = record
            .last_run_at
            .map(format_timestamp)
            .unwrap_or_else(|| "never".to_string());
        let next_run = record
            .next_run_at
            .map(format_timestamp)
            .unwrap_or_else(|| "-".to_string());

        println!(
            "{:<20} {:<10} {:<8} {:<6} {:<24} {:<24}",
            record.id, status, record.run_count, record.consecutive_failures, last_run, next_run
        );
    }

    // Show jobs on disk not yet synced to DB.
    for (id, def) in &defs_map {
        if !seen.contains(id) {
            let status = if def.enabled { "new" } else { "disabled" };
            println!(
                "{:<20} {:<10} {:<8} {:<6} {:<24} {:<24}",
                id, status, 0, 0, "never", "-"
            );
        }
    }

    Ok(())
}

async fn cmd_show(name: &str) -> anyhow::Result<()> {
    let jobs_dir = loader::jobs_dir()?;
    let toml_path = jobs_dir.join(format!("{name}.toml"));

    if toml_path.exists() {
        let contents = std::fs::read_to_string(&toml_path)?;
        println!("── Definition ({}) ──\n", toml_path.display());
        println!("{contents}");
    } else {
        println!("No TOML file found at {}", toml_path.display());
    }

    let db = SchedulerDb::open_default().await?;
    if let Some(record) = jobs_repo::get_job(db.pool(), name).await? {
        println!("\n── Status ──");
        println!("  Enabled: {}", record.enabled);
        println!("  Paused:  {}", record.paused);
        println!("  Runs:    {}", record.run_count);
        println!("  Failures (consecutive): {}", record.consecutive_failures);
        if let Some(ts) = record.last_run_at {
            println!("  Last run: {}", format_timestamp(ts));
        }
        if let Some(ts) = record.next_run_at {
            println!("  Next run: {}", format_timestamp(ts));
        }
    }

    // Show recent runs.
    let runs = runs_repo::list_runs(db.pool(), name, 5).await?;
    if !runs.is_empty() {
        println!("\n── Recent Runs ──");
        for run in runs {
            let dur = run
                .duration_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_default();
            println!(
                "  {} | {} | {} | {}",
                &run.id[..8],
                run.status,
                format_timestamp(run.started_at),
                dur
            );
        }
    }

    Ok(())
}

async fn cmd_create(name: &str) -> anyhow::Result<()> {
    let jobs_dir = loader::jobs_dir()?;
    std::fs::create_dir_all(&jobs_dir)?;

    let path = jobs_dir.join(format!("{name}.toml"));
    if path.exists() {
        anyhow::bail!("job '{name}' already exists at {}", path.display());
    }

    let template = format!(
        r#"name = "{name}"
description = ""
enabled = true

[task]
# Use either `skill` or `prompt` (not both).
# skill = "my-skill"
prompt = """
Describe what this job should do.
"""
# context = "Additional instructions"
# cwd = "/path/to/working/directory"

[task.config]
# model = "o3"
# sandbox_mode = "danger-full-access"

[schedule]
# cron = "0 7 * * *"           # daily at 7am
# interval_minutes = 360       # every 6 hours

[execution]
timeout_minutes = 30
max_retries = 0
skip_if_running = true
"#
    );

    std::fs::write(&path, template)?;
    println!("Created job template at {}", path.display());
    println!("Edit the file and run `ata jobs run {name}` to test it.");
    Ok(())
}

async fn cmd_delete(name: &str) -> anyhow::Result<()> {
    let jobs_dir = loader::jobs_dir()?;
    let path = jobs_dir.join(format!("{name}.toml"));

    if path.exists() {
        std::fs::remove_file(&path)?;
        println!("Deleted {}", path.display());
    }

    let db = SchedulerDb::open_default().await?;
    if jobs_repo::delete_job(db.pool(), name).await? {
        println!("Removed '{name}' from scheduler database.");
    }

    Ok(())
}

async fn cmd_pause(name: &str) -> anyhow::Result<()> {
    let db = SchedulerDb::open_default().await?;
    if jobs_repo::set_paused(db.pool(), name, true).await? {
        println!("Paused job '{name}'.");
    } else {
        println!("Job '{name}' not found in scheduler database.");
    }
    Ok(())
}

async fn cmd_resume(name: &str) -> anyhow::Result<()> {
    let db = SchedulerDb::open_default().await?;
    if jobs_repo::set_paused(db.pool(), name, false).await? {
        println!("Resumed job '{name}'.");
    } else {
        println!("Job '{name}' not found in scheduler database.");
    }
    Ok(())
}

async fn cmd_run(name: &str) -> anyhow::Result<()> {
    let jobs_dir = loader::jobs_dir()?;
    let path = jobs_dir.join(format!("{name}.toml"));

    let (_, def) = loader::load_single_job(&path)?;

    let db = SchedulerDb::open_default().await?;
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    let runs_dir = home.join("scheduler").join("runs");

    // Ensure the job exists in the DB (the runs table has a FK to jobs).
    if jobs_repo::get_job(db.pool(), name).await?.is_none() {
        let now = chrono::Utc::now().timestamp();
        let serialized = toml::to_string(&def).unwrap_or_default();
        let hash = {
            use std::hash::Hash;
            use std::hash::Hasher;
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            serialized.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        };
        let job_record = jobs_repo::JobRecord {
            id: name.to_string(),
            definition_hash: hash,
            enabled: def.enabled,
            paused: false,
            created_at: now,
            updated_at: now,
            last_run_at: None,
            next_run_at: None,
            run_count: 0,
            consecutive_failures: 0,
        };
        jobs_repo::upsert_job(db.pool(), &job_record).await?;
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();

    // Insert run record.
    let run_record = runs_repo::RunRecord {
        id: run_id.clone(),
        job_id: name.to_string(),
        status: "running".to_string(),
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
    runs_repo::insert_run(db.pool(), &run_record).await?;

    println!("Running job '{name}' (run {})...", &run_id[..8]);

    let result = crate::engine::runner::execute_job(&db, &run_id, name, &def, &runs_dir).await?;

    let finished_at = chrono::Utc::now().timestamp();
    runs_repo::finish_run(
        db.pool(),
        &run_id,
        result.status,
        finished_at,
        result.duration_ms,
        result.output_preview.as_deref(),
        result.error_message.as_deref(),
    )
    .await?;

    match result.status {
        crate::storage::RunStatus::Success => {
            println!(
                "Job '{name}' completed successfully in {}ms.",
                result.duration_ms
            );
        }
        _ => {
            eprintln!("Job '{name}' finished with status: {}", result.status);
            if let Some(err) = &result.error_message {
                eprintln!("Error: {err}");
            }
        }
    }

    Ok(())
}

async fn cmd_history(name: &str, limit: i64) -> anyhow::Result<()> {
    let db = SchedulerDb::open_default().await?;
    let runs = runs_repo::list_runs(db.pool(), name, limit).await?;

    if runs.is_empty() {
        println!("No runs found for job '{name}'.");
        return Ok(());
    }

    println!(
        "{:<10} {:<10} {:<24} {:<12} ERROR",
        "RUN ID", "STATUS", "STARTED", "DURATION"
    );
    println!("{}", "-".repeat(80));

    for run in runs {
        let dur = run
            .duration_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "-".to_string());
        let err = run.error_message.as_deref().unwrap_or("");
        println!(
            "{:<10} {:<10} {:<24} {:<12} {}",
            &run.id[..8],
            run.status,
            format_timestamp(run.started_at),
            dur,
            err
        );
    }

    Ok(())
}

async fn cmd_logs(run_id: &str) -> anyhow::Result<()> {
    let db = SchedulerDb::open_default().await?;

    // Try to find the run by prefix match.
    let run = runs_repo::get_run(db.pool(), run_id).await?;
    let run = match run {
        Some(r) => r,
        None => {
            // Try prefix match — user might pass short ID.
            anyhow::bail!("run '{run_id}' not found");
        }
    };

    if let Some(path) = &run.output_path {
        let output_path = PathBuf::from(path);
        if output_path.exists() {
            let contents = std::fs::read_to_string(&output_path)?;
            println!("{contents}");
        } else {
            println!("Output file not found at {path}");
            if let Some(preview) = &run.output_preview {
                println!("\n── Preview ──\n{preview}");
            }
        }
    } else if let Some(preview) = &run.output_preview {
        println!("{preview}");
    } else {
        println!("No output available for this run.");
    }

    Ok(())
}

fn cmd_status() -> anyhow::Result<()> {
    match daemon::is_daemon_running()? {
        Some(pid) => println!("Scheduler daemon is running (PID {pid})."),
        None => println!("Scheduler daemon is not running."),
    }
    Ok(())
}

fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}
