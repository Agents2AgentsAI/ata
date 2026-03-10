//! CLI subcommand for managing the mobile background server.
//!
//! Reimplements daemon utilities locally to minimize conflict surface with
//! upstream when merging.

use std::fmt::Write as _;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use clap::Parser;
use qrcode::QrCode;
use rand::Rng;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

// ---------------------------------------------------------------------------
// CLI types
// ---------------------------------------------------------------------------

#[derive(Debug, Parser)]
pub(crate) struct MobileCommand {
    #[command(subcommand)]
    pub sub: Option<MobileSubcommand>,
}

#[derive(Debug, clap::Subcommand)]
pub(crate) enum MobileSubcommand {
    /// Start the mobile background server.
    On {
        #[arg(long, default_value = "19285")]
        port: u16,
    },
    /// Stop the mobile background server.
    Off,
    /// Show mobile server status (default if no subcommand).
    Status,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub(crate) fn run_mobile_command(cmd: MobileCommand) -> anyhow::Result<()> {
    match cmd.sub.unwrap_or(MobileSubcommand::Status) {
        MobileSubcommand::On { port } => cmd_on(port),
        MobileSubcommand::Off => cmd_off(),
        MobileSubcommand::Status => cmd_status(),
    }
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

fn cmd_on(port: u16) -> anyhow::Result<()> {
    // Stop any existing daemon first.
    if is_daemon_running().is_some() {
        stop_daemon();
    }

    let token = read_or_create_token()?;
    let msg = start_daemon(port);
    println!("{msg}");

    if is_daemon_running().is_none() {
        // Daemon failed to start — don't print QR.
        return Ok(());
    }

    let ip = local_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let url = build_connect_url(&ip, port, &token);
    println!("\nConnect URL: {url}\n");
    print_qr(&url);

    Ok(())
}

fn cmd_off() -> anyhow::Result<()> {
    let msg = stop_daemon();
    println!("{msg}");

    // Remove token file.
    if let Some(path) = token_file_path() {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

fn cmd_status() -> anyhow::Result<()> {
    match is_daemon_running() {
        Some(pid) => {
            println!("Mobile server is running (PID {pid}).");
            if let Some(token) = read_token() {
                let ip = local_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
                // Try to read port from how it was started — default 19285.
                let port: u16 = 19285;
                let url = build_connect_url(&ip, port, &token);
                println!("Connect URL: {url}\n");
                print_qr(&url);
            }
        }
        None => {
            println!("Mobile server is not running.");
            println!("Start it with: ata mobile on");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Daemon lifecycle (reimplemented locally to reduce conflict surface)
// ---------------------------------------------------------------------------

fn ata_home() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            #[allow(deprecated)]
            std::env::home_dir().map(|h| h.join(".ata"))
        })
}

fn pid_file_path() -> Option<PathBuf> {
    ata_home().map(|h| h.join("mobile-server.pid"))
}

fn token_file_path() -> Option<PathBuf> {
    ata_home().map(|h| h.join("mobile-server.token"))
}

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

fn is_daemon_running() -> Option<u32> {
    let path = pid_file_path()?;
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(&path).ok()?;
    let pid: u32 = contents.trim().parse().ok()?;
    if is_process_alive(pid) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(&path);
        None
    }
}

fn find_app_server_binary() -> Option<PathBuf> {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let candidate = dir.join("codex-app-server");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Ok(output) = Command::new("which").arg("codex-app-server").output()
        && output.status.success()
    {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            return Some(PathBuf::from(path_str));
        }
    }
    None
}

fn start_daemon(port: u16) -> String {
    if is_daemon_running().is_some() {
        stop_daemon();
    }

    let Some(binary) = find_app_server_binary() else {
        return "Failed to start daemon: codex-app-server binary not found. \
                Build it with `cargo build -p codex-app-server`."
            .to_string();
    };

    let listen_url = format!("ws://0.0.0.0:{port}");
    let mut cmd = Command::new(&binary);
    cmd.arg("--listen")
        .arg(&listen_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
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

fn stop_daemon() -> String {
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

// ---------------------------------------------------------------------------
// Token management
// ---------------------------------------------------------------------------

fn generate_auth_token() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    let mut hex = String::with_capacity(64);
    for b in &bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn read_token() -> Option<String> {
    let path = token_file_path()?;
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_or_create_token() -> anyhow::Result<String> {
    if let Some(token) = read_token() {
        return Ok(token);
    }
    let token = generate_auth_token();
    let path = token_file_path()
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory for token file"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &token)?;
    Ok(token)
}

// ---------------------------------------------------------------------------
// Networking
// ---------------------------------------------------------------------------

fn local_lan_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

fn build_connect_url(ip: &str, port: u16, token: &str) -> String {
    format!("ata://connect?host={ip}&port={port}&token={token}")
}

// ---------------------------------------------------------------------------
// QR code rendering
// ---------------------------------------------------------------------------

fn render_qr_to_lines(data: &str) -> Vec<String> {
    let code = match QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => return vec!["(failed to generate QR code)".to_string()],
    };

    let matrix = code.to_colors();
    let width = code.width();
    let height = matrix.len() / width;

    let mut lines = Vec::new();

    // Process two rows at a time using Unicode half-block characters.
    let mut y = 0;
    while y < height {
        let mut line = String::new();
        for x in 0..width {
            let top = matrix[y * width + x];
            let bottom = if y + 1 < height {
                matrix[(y + 1) * width + x]
            } else {
                qrcode::Color::Light
            };

            match (top, bottom) {
                (qrcode::Color::Dark, qrcode::Color::Dark) => line.push('\u{2588}'), // █
                (qrcode::Color::Dark, qrcode::Color::Light) => line.push('\u{2580}'), // ▀
                (qrcode::Color::Light, qrcode::Color::Dark) => line.push('\u{2584}'), // ▄
                (qrcode::Color::Light, qrcode::Color::Light) => line.push(' '),
            }
        }
        lines.push(line);
        y += 2;
    }

    lines
}

fn print_qr(data: &str) {
    for line in render_qr_to_lines(data) {
        println!("{line}");
    }
}
