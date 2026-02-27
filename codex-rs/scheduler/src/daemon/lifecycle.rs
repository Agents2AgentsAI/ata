use std::path::PathBuf;

use anyhow::Context;
use tokio::sync::watch;

use crate::engine::run_scheduler;
use crate::storage::db::SchedulerDb;

const DEFAULT_MAX_CONCURRENT: usize = 4;

/// Path to the PID file (`~/.ata/scheduler/scheduler.pid`).
fn pid_file_path() -> anyhow::Result<PathBuf> {
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    Ok(home.join("scheduler").join("scheduler.pid"))
}

/// RAII guard that writes the PID file on creation and removes it on drop.
pub struct PidGuard {
    path: PathBuf,
}

impl PidGuard {
    /// Acquire the PID guard. Fails if another daemon is already running.
    pub fn acquire() -> anyhow::Result<Self> {
        let path = pid_file_path()?;

        // Check for existing PID.
        if path.exists() {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(pid) = contents.trim().parse::<u32>()
                && is_process_alive(pid)
            {
                anyhow::bail!(
                    "scheduler daemon is already running (PID {pid}). Stop it with `ata scheduler stop`."
                );
            }
            // Stale PID file — remove it.
            let _ = std::fs::remove_file(&path);
        }

        // Write our PID.
        let pid = std::process::id();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, pid.to_string())
            .with_context(|| format!("failed to write PID file at {}", path.display()))?;

        Ok(Self { path })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Check if a process with the given PID is alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Signal 0 checks for process existence without sending a signal.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Check if the daemon is currently running.
pub fn is_daemon_running() -> anyhow::Result<Option<u32>> {
    let path = pid_file_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    match contents.trim().parse::<u32>() {
        Ok(pid) if is_process_alive(pid) => Ok(Some(pid)),
        _ => Ok(None),
    }
}

/// Stop the daemon by sending SIGTERM to the PID in the PID file.
pub fn stop_daemon() -> anyhow::Result<()> {
    let path = pid_file_path()?;
    if !path.exists() {
        anyhow::bail!("no scheduler daemon is running (PID file not found)");
    }
    let contents = std::fs::read_to_string(&path)?;
    let pid: u32 = contents
        .trim()
        .parse()
        .context("invalid PID in scheduler.pid")?;

    if !is_process_alive(pid) {
        let _ = std::fs::remove_file(&path);
        anyhow::bail!("scheduler daemon (PID {pid}) is not running (stale PID file removed)");
    }

    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }

    println!("Sent stop signal to scheduler daemon (PID {pid}).");
    Ok(())
}

/// Start the scheduler daemon in the foreground.
pub async fn start_daemon() -> anyhow::Result<()> {
    let _pid_guard = PidGuard::acquire()?;

    let db = SchedulerDb::open_default().await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Set up signal handler for graceful shutdown.
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("received shutdown signal");
        let _ = shutdown_tx_clone.send(true);
    });

    #[cfg(unix)]
    {
        let shutdown_tx_term = shutdown_tx.clone();
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
            if let Some(ref mut sig) = sigterm {
                sig.recv().await;
                tracing::info!("received SIGTERM");
                let _ = shutdown_tx_term.send(true);
            }
        });
    }

    println!("Scheduler daemon started (PID {}).", std::process::id());

    run_scheduler(db, shutdown_rx, DEFAULT_MAX_CONCURRENT).await?;

    println!("Scheduler daemon stopped.");
    Ok(())
}
