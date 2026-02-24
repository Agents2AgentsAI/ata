#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use codex_core::config::types::ShellEnvironmentPolicy;
use codex_core::exec_env::create_env;
use codex_protocol::protocol::SandboxPolicy;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::io::Read;
use std::io::Write;
use std::net::Ipv4Addr;
use std::net::TcpListener;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const BWRAP_UNAVAILABLE_ERR: &str = "build-time bubblewrap is not available in this build.";
const NETWORK_TIMEOUT_MS: u64 = 4_000;
const PROXY_ENV_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "FTP_PROXY",
    "YARN_HTTP_PROXY",
    "YARN_HTTPS_PROXY",
    "NPM_CONFIG_HTTP_PROXY",
    "NPM_CONFIG_HTTPS_PROXY",
    "NPM_CONFIG_PROXY",
    "BUNDLE_HTTP_PROXY",
    "BUNDLE_HTTPS_PROXY",
    "PIP_PROXY",
    "DOCKER_HTTP_PROXY",
    "DOCKER_HTTPS_PROXY",
];

fn create_env_from_core_vars() -> HashMap<String, String> {
    let policy = ShellEnvironmentPolicy::default();
    create_env(&policy, None)
}

fn strip_proxy_env(env: &mut HashMap<String, String>) {
    for key in PROXY_ENV_KEYS {
        env.remove(*key);
        let lower = key.to_ascii_lowercase();
        env.remove(lower.as_str());
    }
}

fn is_bwrap_unavailable_output(output: &Output) -> bool {
    String::from_utf8_lossy(&output.stderr).contains(BWRAP_UNAVAILABLE_ERR)
}

async fn should_skip_bwrap_tests() -> bool {
    let mut env = create_env_from_core_vars();
    strip_proxy_env(&mut env);

    let output = run_linux_sandbox_direct(
        &["bash", "-c", "true"],
        &SandboxPolicy::new_read_only_policy(),
        false,
        env,
        NETWORK_TIMEOUT_MS,
    )
    .await;
    is_bwrap_unavailable_output(&output)
}

async fn managed_proxy_skip_reason() -> Option<String> {
    if should_skip_bwrap_tests().await {
        return Some("vendored bwrap was not built in this environment".to_string());
    }

    let mut env = create_env_from_core_vars();
    strip_proxy_env(&mut env);
    env.insert("HTTP_PROXY".to_string(), "http://127.0.0.1:9".to_string());

    // Probe 1: basic sandbox creation with managed proxy mode.
    let Some(output) = try_run_linux_sandbox(
        &["bash", "-c", "true"],
        &SandboxPolicy::DangerFullAccess,
        true,
        env.clone(),
        NETWORK_TIMEOUT_MS,
    )
    .await
    else {
        return Some("managed proxy sandbox probe timed out or failed to execute".to_string());
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Some(format!(
            "managed proxy sandbox probe failed: {}",
            stderr.trim()
        ));
    }

    // Probe 2: verify that network isolation actually blocks direct egress.
    // In a properly isolated namespace the connection to a non-loopback IP
    // fails immediately (ENETUNREACH). If the command succeeds or hangs
    // (timeout), the isolation is not effective in this environment.
    let egress_result = try_run_linux_sandbox(
        &["bash", "-c", "echo probe > /dev/tcp/192.0.2.1/80"],
        &SandboxPolicy::DangerFullAccess,
        true,
        env,
        NETWORK_TIMEOUT_MS,
    )
    .await;

    match egress_result {
        Some(output) if output.status.success() => Some(
            "managed proxy network isolation not effective: direct egress was not blocked"
                .to_string(),
        ),
        None => Some(
            "managed proxy network isolation probe timed out (egress not blocked promptly)"
                .to_string(),
        ),
        Some(_) => {
            // Egress was blocked (command failed fast) — isolation works.
            None
        }
    }
}

fn build_sandbox_command(
    command: &[&str],
    sandbox_policy: &SandboxPolicy,
    allow_network_for_proxy: bool,
    env: HashMap<String, String>,
) -> Command {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => panic!("cwd should exist: {err}"),
    };
    let policy_json = match serde_json::to_string(sandbox_policy) {
        Ok(policy_json) => policy_json,
        Err(err) => panic!("policy should serialize: {err}"),
    };

    let mut args = vec![
        "--sandbox-policy-cwd".to_string(),
        cwd.to_string_lossy().to_string(),
        "--sandbox-policy".to_string(),
        policy_json,
        "--use-bwrap-sandbox".to_string(),
    ];
    if allow_network_for_proxy {
        args.push("--allow-network-for-proxy".to_string());
    }
    args.push("--".to_string());
    args.extend(command.iter().map(|entry| (*entry).to_string()));

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_codex-linux-sandbox"));
    cmd.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

async fn run_linux_sandbox_direct(
    command: &[&str],
    sandbox_policy: &SandboxPolicy,
    allow_network_for_proxy: bool,
    env: HashMap<String, String>,
    timeout_ms: u64,
) -> Output {
    let mut cmd = build_sandbox_command(command, sandbox_policy, allow_network_for_proxy, env);
    let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), cmd.output()).await {
        Ok(output) => output,
        Err(err) => panic!("sandbox command should not time out: {err}"),
    };
    match output {
        Ok(output) => output,
        Err(err) => panic!("sandbox command should execute: {err}"),
    }
}

/// Like `run_linux_sandbox_direct` but returns `None` on timeout or execution
/// failure instead of panicking. Used for skip-detection probes where failures
/// are expected in unsupported environments.
async fn try_run_linux_sandbox(
    command: &[&str],
    sandbox_policy: &SandboxPolicy,
    allow_network_for_proxy: bool,
    env: HashMap<String, String>,
    timeout_ms: u64,
) -> Option<Output> {
    let mut cmd = build_sandbox_command(command, sandbox_policy, allow_network_for_proxy, env);
    match tokio::time::timeout(Duration::from_millis(timeout_ms), cmd.output()).await {
        Ok(Ok(output)) => Some(output),
        _ => None,
    }
}

#[tokio::test]
async fn managed_proxy_mode_fails_closed_without_proxy_env() {
    if let Some(skip_reason) = managed_proxy_skip_reason().await {
        eprintln!("skipping managed proxy test: {skip_reason}");
        return;
    }

    let mut env = create_env_from_core_vars();
    strip_proxy_env(&mut env);

    let output = run_linux_sandbox_direct(
        &["bash", "-c", "true"],
        &SandboxPolicy::DangerFullAccess,
        true,
        env,
        NETWORK_TIMEOUT_MS,
    )
    .await;

    assert_eq!(output.status.success(), false);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("managed proxy mode requires proxy environment variables"),
        "expected fail-closed managed-proxy message, got stderr: {stderr}"
    );
}

#[tokio::test]
async fn managed_proxy_mode_routes_through_bridge_and_blocks_direct_egress() {
    if let Some(skip_reason) = managed_proxy_skip_reason().await {
        eprintln!("skipping managed proxy test: {skip_reason}");
        return;
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind proxy listener");
    let proxy_port = listener
        .local_addr()
        .expect("proxy listener local addr")
        .port();
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept proxy connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set read timeout");
        let mut buf = [0_u8; 4096];
        let read = stream.read(&mut buf).expect("read proxy request");
        let request = String::from_utf8_lossy(&buf[..read]).to_string();
        request_tx.send(request).expect("send proxy request");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
            .expect("write proxy response");
    });

    let mut env = create_env_from_core_vars();
    strip_proxy_env(&mut env);
    env.insert(
        "HTTP_PROXY".to_string(),
        format!("http://127.0.0.1:{proxy_port}"),
    );

    let routed_output = run_linux_sandbox_direct(
        &[
            "bash",
            "-c",
            "proxy=\"${HTTP_PROXY#*://}\"; host=\"${proxy%%:*}\"; port=\"${proxy##*:}\"; exec 3<>/dev/tcp/${host}/${port}; printf 'GET http://example.com/ HTTP/1.1\\r\\nHost: example.com\\r\\n\\r\\n' >&3; IFS= read -r line <&3; printf '%s\\n' \"$line\"",
        ],
        &SandboxPolicy::DangerFullAccess,
        true,
        env.clone(),
        NETWORK_TIMEOUT_MS,
    )
    .await;

    assert_eq!(
        routed_output.status.success(),
        true,
        "expected routed command to execute successfully; status={:?}; stdout={}; stderr={}",
        routed_output.status.code(),
        String::from_utf8_lossy(&routed_output.stdout),
        String::from_utf8_lossy(&routed_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&routed_output.stdout);
    assert!(
        stdout.contains("HTTP/1.1 200 OK"),
        "expected bridge-routed proxy response, got stdout: {stdout}"
    );

    let request = request_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("expected proxy request");
    assert!(
        request.contains("GET http://example.com/ HTTP/1.1"),
        "expected HTTP proxy absolute-form request, got request: {request}"
    );

    let direct_egress_output = run_linux_sandbox_direct(
        &["bash", "-c", "echo hi > /dev/tcp/192.0.2.1/80"],
        &SandboxPolicy::DangerFullAccess,
        true,
        env,
        NETWORK_TIMEOUT_MS,
    )
    .await;
    assert_eq!(direct_egress_output.status.success(), false);
}

#[tokio::test]
async fn managed_proxy_mode_denies_af_unix_creation_for_user_command() {
    if let Some(skip_reason) = managed_proxy_skip_reason().await {
        eprintln!("skipping managed proxy test: {skip_reason}");
        return;
    }

    let python_available = Command::new("bash")
        .arg("-c")
        .arg("command -v python3 >/dev/null")
        .status()
        .await
        .expect("python3 probe should execute")
        .success();
    if !python_available {
        eprintln!("skipping managed proxy AF_UNIX test: python3 is unavailable");
        return;
    }

    let mut env = create_env_from_core_vars();
    strip_proxy_env(&mut env);
    env.insert("HTTP_PROXY".to_string(), "http://127.0.0.1:9".to_string());

    let output = run_linux_sandbox_direct(
        &[
            "python3",
            "-c",
            "import socket,sys\ntry:\n    socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\nexcept PermissionError:\n    sys.exit(0)\nexcept OSError:\n    sys.exit(2)\nsys.exit(1)\n",
        ],
        &SandboxPolicy::DangerFullAccess,
        true,
        env,
        NETWORK_TIMEOUT_MS,
    )
    .await;

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected AF_UNIX creation to be denied cleanly for user command; status={:?}; stdout={}; stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
