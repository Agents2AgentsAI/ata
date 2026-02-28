use std::path::PathBuf;

use anyhow::Context;
use tokio::sync::watch;

use crate::engine::run_scheduler;
use crate::storage::db::SchedulerDb;

const DEFAULT_MAX_CONCURRENT: usize = 4;
const LAUNCHD_LABEL: &str = "com.ata.scheduler";

/// Path to the PID file (`~/.ata/scheduler/scheduler.pid`).
fn pid_file_path() -> anyhow::Result<PathBuf> {
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    Ok(home.join("scheduler").join("scheduler.pid"))
}

/// Path to the launchd plist file.
fn plist_path() -> anyhow::Result<PathBuf> {
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    Ok(home
        .join("scheduler")
        .join(format!("{LAUNCHD_LABEL}.plist")))
}

/// Check if the launchd plist is installed.
pub fn is_launchd_installed() -> anyhow::Result<bool> {
    let plist = plist_path()?;
    Ok(plist.exists())
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
///
/// Uses `kill(pid, 0)` which returns:
/// - `0` if the process exists and we can signal it
/// - `-1` with `EPERM` if the process exists but we lack permission (e.g. inside sandbox)
/// - `-1` with `ESRCH` if the process does not exist
///
/// Both success and EPERM mean the process is alive.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        // EPERM means the process exists but we can't signal it
        // (happens inside Seatbelt sandbox for launchd-owned processes).
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
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

/// Start the scheduler daemon in the background via launchd.
///
/// Requires `ata scheduler install` to have been run first. Uses
/// `launchctl kickstart` which works even from inside a Seatbelt sandbox
/// because launchd starts the daemon independently of the calling process.
pub fn start_daemon_background() -> anyhow::Result<()> {
    // Pre-flight: check if already running before starting.
    if let Some(pid) = is_daemon_running()? {
        anyhow::bail!(
            "scheduler daemon is already running (PID {pid}). Stop it with `ata scheduler stop`."
        );
    }

    #[cfg(target_os = "macos")]
    {
        let plist = plist_path()?;
        if !plist.exists() {
            anyhow::bail!("scheduler is not installed. Run `ata scheduler install` first.");
        }

        let uid = unsafe { libc::getuid() };
        let status = std::process::Command::new("launchctl")
            .args(["kickstart", "-k", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
            .status()
            .context("failed to run launchctl kickstart")?;

        if status.success() {
            println!("Scheduler daemon started via launchd.");
            println!("Use `ata scheduler status` to check, `ata scheduler stop` to stop.");
        } else {
            anyhow::bail!(
                "launchctl kickstart failed (exit {}). Try `ata scheduler uninstall` then `ata scheduler install`.",
                status.code().unwrap_or(-1)
            );
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!(
            "background daemon mode requires macOS launchd. Run `ata scheduler install` first."
        );
    }
}

/// Install the scheduler daemon as a launchd service (macOS only).
///
/// Writes a plist to `~/.ata/scheduler/com.ata.scheduler.plist` and registers
/// it with `launchctl bootstrap`. After installation, the daemon will be
/// automatically started by launchd and kept alive across reboots.
pub fn install_launchd() -> anyhow::Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("launchd installation is only supported on macOS");
    }

    #[cfg(target_os = "macos")]
    {
        let plist = plist_path()?;
        let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
        let log_path = home.join("scheduler").join("daemon.log");
        let uid = unsafe { libc::getuid() };

        // Idempotent: if already loaded, bootout first so we can re-bootstrap
        // with a potentially updated plist (e.g. new binary path).
        let bootout = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
            .output();
        if bootout.is_ok_and(|o| o.status.success()) {
            // Wait for the old daemon process to fully exit before re-bootstrapping.
            // launchctl bootout returns before the process is gone.
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if is_daemon_running()?.is_none() {
                    break;
                }
            }
        }

        let exe = std::env::current_exe().context("failed to find current executable")?;
        let exe_str = exe.display().to_string();
        let log_str = log_path.display().to_string();
        let workdir = home.join("scheduler").join("workdir");

        if let Some(parent) = plist.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(&workdir)?;

        let workdir_str = workdir.display().to_string();

        // Capture the current PATH so the daemon (and its ata exec children)
        // can find npx, node, and other tools needed by MCP servers.
        let path_env = std::env::var("PATH").unwrap_or_default();
        let home_env = std::env::var("HOME").unwrap_or_default();

        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_str}</string>
        <string>scheduler</string>
        <string>start</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{workdir_str}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path_env}</string>
        <key>HOME</key>
        <string>{home_env}</string>
    </dict>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log_str}</string>
    <key>StandardErrorPath</key>
    <string>{log_str}</string>
</dict>
</plist>
"#
        );

        std::fs::write(&plist, &plist_content)
            .with_context(|| format!("failed to write plist at {}", plist.display()))?;

        let status = std::process::Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{uid}"),
                &plist.display().to_string(),
            ])
            .status()
            .context("failed to run launchctl bootstrap")?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            anyhow::bail!("launchctl bootstrap failed (exit {code})");
        }

        // Kickstart immediately — bootstrap loads the plist but launchd may
        // throttle the initial start if a previous instance crashed.
        let kick = std::process::Command::new("launchctl")
            .args(["kickstart", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
            .status();
        if let Ok(s) = kick
            && !s.success()
        {
            eprintln!(
                "warning: launchctl kickstart exited with code {}",
                s.code().unwrap_or(-1)
            );
        }

        println!("Scheduler daemon installed and started via launchd.");
        println!("Plist: {}", plist.display());
        println!("Logs:  {}", log_path.display());

        Ok(())
    }
}

/// Uninstall the scheduler daemon from launchd (macOS only).
///
/// Runs `launchctl bootout` to stop and deregister the service, then
/// removes the plist file.
pub fn uninstall_launchd() -> anyhow::Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("launchd uninstallation is only supported on macOS");
    }

    #[cfg(target_os = "macos")]
    {
        let plist = plist_path()?;

        let uid = unsafe { libc::getuid() };
        let status = std::process::Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{LAUNCHD_LABEL}")])
            .status()
            .context("failed to run launchctl bootout")?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            // Exit code 3 means the service wasn't loaded — that's fine.
            if code != 3 {
                eprintln!("warning: launchctl bootout exited with code {code}");
            }
        }

        if plist.exists() {
            std::fs::remove_file(&plist)?;
            println!("Removed plist at {}", plist.display());
        }

        println!("Scheduler daemon uninstalled from launchd.");
        Ok(())
    }
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
