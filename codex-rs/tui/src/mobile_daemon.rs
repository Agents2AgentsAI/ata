//! Lifecycle management for the persistent mobile WebSocket daemon.
//!
//! When the user enables "Background server" in the `/mobile` setup, we spawn
//! `codex-app-server --listen ws://0.0.0.0:PORT` as a detached background
//! process so that mobile clients can connect even when no TUI session is
//! running. The daemon's PID is stored at `~/.ata/mobile-server.pid`.

use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Resolve `~/.ata/mobile-server.pid`.
fn pid_file_path() -> Option<PathBuf> {
    // Honor CODEX_HOME env var, otherwise default to ~/.ata.
    let codex_home = std::env::var("CODEX_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            #[allow(deprecated)]
            std::env::home_dir().map(|h| h.join(".ata"))
        })?;
    Some(codex_home.join("mobile-server.pid"))
}

/// Check if a process with the given PID is alive (Unix only).
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Check if the mobile daemon is currently running. Returns the PID if alive.
pub(crate) fn is_daemon_running() -> Option<u32> {
    let path = pid_file_path()?;
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(&path).ok()?;
    let pid: u32 = contents.trim().parse().ok()?;
    if is_process_alive(pid) {
        Some(pid)
    } else {
        // Stale PID file — clean up.
        let _ = std::fs::remove_file(&path);
        None
    }
}

/// Start the mobile daemon. Returns a status message.
pub(crate) fn start_daemon(port: u16) -> String {
    // Stop existing daemon first if running.
    if is_daemon_running().is_some() {
        stop_daemon();
    }

    // Find the codex-app-server binary. Try the same directory as the current
    // binary first, then fall back to PATH.
    let binary = find_app_server_binary();
    let Some(binary) = binary else {
        return "Failed to start daemon: codex-app-server binary not found. Build it with `cargo build -p codex-app-server`.".to_string();
    };

    let listen_url = format!("ws://0.0.0.0:{port}");
    let mut cmd = Command::new(&binary);
    cmd.arg("--listen")
        .arg(&listen_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Detach the child into its own session so it survives TUI exit.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let result = cmd.spawn();

    match result {
        Ok(child) => {
            let pid = child.id();
            // Write PID file.
            if let Some(path) = pid_file_path() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, pid.to_string());
            }
            format!("Background server started (PID {pid}) on port {port}")
        }
        Err(err) => {
            format!("Failed to start daemon: {err}")
        }
    }
}

/// Stop the mobile daemon. Returns a status message.
pub(crate) fn stop_daemon() -> String {
    let Some(path) = pid_file_path() else {
        return "No daemon PID file found.".to_string();
    };
    if !path.exists() {
        return "No background server is running.".to_string();
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return "Failed to read PID file.".to_string(),
    };
    let pid: u32 = match contents.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return "Invalid PID file (removed).".to_string();
        }
    };

    #[cfg(unix)]
    {
        let ret = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        let _ = std::fs::remove_file(&path);
        if ret == 0 {
            return format!("Background server stopped (PID {pid}).");
        }
        "Background server was not running (stale PID file removed).".to_string()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        let _ = std::fs::remove_file(&path);
        "Daemon stop not supported on this platform.".to_string()
    }
}

/// Find the `codex-app-server` binary.
fn find_app_server_binary() -> Option<PathBuf> {
    // 1. Same directory as the current executable.
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join("codex-app-server");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 2. Check PATH via `which`.
    if let Ok(output) = Command::new("which").arg("codex-app-server").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    None
}
