use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use tokio::process::Command;

use crate::job::JobDefinition;
use crate::storage::RunStatus;
use crate::storage::db::SchedulerDb;
use crate::storage::runs_repo;

/// Result of executing a single job run.
pub struct RunResult {
    pub status: RunStatus,
    pub duration_ms: i64,
    pub output_preview: Option<String>,
    pub error_message: Option<String>,
}

/// Resolve the full prompt for a job. If the job uses a skill, prepend the
/// context and reference the skill name in the prompt. If inline prompt,
/// prepend context directly.
fn resolve_prompt(def: &JobDefinition) -> String {
    let base = if let Some(skill) = &def.task.skill {
        format!("Run the '{skill}' skill.")
    } else if let Some(prompt) = &def.task.prompt {
        prompt.clone()
    } else {
        "No task specified.".to_string()
    };

    match &def.task.context {
        Some(ctx) => format!("{ctx}\n\n{base}"),
        None => base,
    }
}

/// Find the `ata` binary. Prefers the same directory as the current executable,
/// falling back to `PATH`.
fn find_ata_binary() -> PathBuf {
    if let Ok(current) = std::env::current_exe() {
        let candidate = current.with_file_name("ata");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("ata")
}

/// Execute a job by spawning `ata exec --full-auto` as a subprocess.
pub async fn execute_job(
    _db: &SchedulerDb,
    run_id: &str,
    _job_id: &str,
    def: &JobDefinition,
    runs_dir: &PathBuf,
) -> anyhow::Result<RunResult> {
    let prompt = resolve_prompt(def);
    let output_path = runs_dir.join(format!("{run_id}.md"));

    tokio::fs::create_dir_all(runs_dir).await?;

    let ata_bin = find_ata_binary();
    let mut cmd = Command::new(&ata_bin);
    cmd.arg("exec")
        .arg("--full-auto")
        .arg("--json")
        .arg("--ephemeral")
        .arg("--skip-git-repo-check");

    if let Some(cwd) = &def.task.cwd {
        cmd.arg("--cd").arg(cwd);
    } else {
        // Default to scheduler workdir so jobs don't write files into
        // random directories (launchd sets cwd to / by default).
        let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
        let workdir = home.join("scheduler").join("workdir");
        tokio::fs::create_dir_all(&workdir).await?;
        cmd.arg("--cd").arg(&workdir);
    }
    if let Some(model) = &def.task.config.model {
        cmd.arg("-m").arg(model);
    }
    if let Some(sandbox) = &def.task.config.sandbox_mode {
        cmd.arg("-s").arg(sandbox);
    }

    cmd.arg(&prompt);

    // Capture stdout/stderr.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let start = Instant::now();
    let timeout = tokio::time::Duration::from_secs(def.execution.timeout_minutes * 60);

    let result = tokio::time::timeout(timeout, cmd.output()).await;

    let elapsed = start.elapsed();
    let duration_ms = elapsed.as_millis() as i64;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Write full output to file.
            let full_output = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n\n--- stderr ---\n{stderr}")
            };
            tokio::fs::write(&output_path, &full_output)
                .await
                .with_context(|| format!("failed to write output to {}", output_path.display()))?;

            // Truncate preview to 4KB.
            let preview = if full_output.len() > 4096 {
                full_output[..4096].to_string()
            } else {
                full_output.clone()
            };

            if output.status.success() {
                Ok(RunResult {
                    status: RunStatus::Success,
                    duration_ms,
                    output_preview: Some(preview),
                    error_message: None,
                })
            } else {
                let code = output.status.code().unwrap_or(-1);
                Ok(RunResult {
                    status: RunStatus::Failed,
                    duration_ms,
                    output_preview: Some(preview),
                    error_message: Some(format!("process exited with code {code}")),
                })
            }
        }
        Ok(Err(e)) => Ok(RunResult {
            status: RunStatus::Failed,
            duration_ms,
            output_preview: None,
            error_message: Some(format!("failed to spawn ata exec: {e}")),
        }),
        Err(_) => {
            // Timeout — the child process is dropped and killed.
            Ok(RunResult {
                status: RunStatus::Timeout,
                duration_ms,
                output_preview: None,
                error_message: Some(format!(
                    "job timed out after {} minutes",
                    def.execution.timeout_minutes
                )),
            })
        }
    }
}

/// Create a run record marked as "skipped" (used when `skip_if_running` fires).
pub async fn record_skipped_run(db: &SchedulerDb, job_id: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().timestamp();
    let run = runs_repo::RunRecord {
        id: uuid::Uuid::new_v4().to_string(),
        job_id: job_id.to_string(),
        status: RunStatus::Skipped.as_str().to_string(),
        attempt: 1,
        started_at: now,
        finished_at: Some(now),
        duration_ms: Some(0),
        output_path: None,
        output_preview: None,
        error_message: Some("skipped: previous run still in progress".to_string()),
        trigger_data: None,
        delivery_results: None,
    };
    runs_repo::insert_run(db.pool(), &run).await
}
