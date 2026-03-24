//! Remote-control mode: embeds a WebSocket server in the TUI so that mobile
//! clients (ATA-Swift) can connect and interact with the same threads.
//!
//! All remote-control logic lives in this new module to minimise the conflict
//! surface with upstream.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;

use codex_app_server::EmbeddedWebSocketConfig;
use codex_app_server::device_registration::DeviceRegistrar;
use codex_app_server::run_embedded_websocket;
use codex_arg0::Arg0DispatchPaths;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_core::config::find_codex_home;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_feedback::CodexFeedback;
use tokio_util::sync::CancellationToken;
use tracing::error;
use tracing::info;
use tracing::warn;

/// Settings parsed from CLI flags for the embedded remote-control server.
#[derive(Clone, Debug)]
pub(crate) struct RemoteControlSettings {
    pub enabled: bool,
    pub port: u16,
    /// If `None`, a random 256-bit token is generated at startup.
    pub token: Option<String>,
}

impl Default for RemoteControlSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 19285,
            token: None,
        }
    }
}

impl RemoteControlSettings {
    pub fn from_cli(cli: &crate::cli::Cli) -> Self {
        Self {
            enabled: cli.remote_control,
            port: cli.remote_control_port,
            token: cli.remote_control_token.clone(),
        }
    }
}

/// Runtime handle for the embedded remote-control server.
pub(crate) struct RemoteControlHandle {
    pub shutdown: CancellationToken,
    pub bind_addr: SocketAddr,
    pub auth_token: String,
    _discovery: Option<crate::remote_discovery::RemoteDiscoveryHandle>,
}

impl RemoteControlHandle {
    /// Human-readable connection info for display in the TUI status bar.
    pub fn connection_info(&self) -> String {
        let token_hint = &self.auth_token[..self.auth_token.len().min(8)];
        format!("Remote: ws://{}  token: {token_hint}…", self.bind_addr,)
    }
}

impl Drop for RemoteControlHandle {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Generate a random hex token (32 bytes = 256 bits).
pub(crate) fn generate_auth_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read the persisted auth token from `<codex_home>/mobile-server.token`, or
/// generate a new one and write it to that path if it does not exist yet.
///
/// This ensures the same token is reused across TUI sessions so that mobile
/// clients configured with a QR code from a previous session still work.
pub(crate) fn read_or_create_token() -> String {
    fn token_file_path() -> Option<std::path::PathBuf> {
        std::env::var("CODEX_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                #[allow(deprecated)]
                std::env::home_dir().map(|h| h.join(".ata"))
            })
            .map(|h| h.join("mobile-server.token"))
    }

    // Try to read an existing token.
    if let Some(ref path) = token_file_path()
        && let Ok(contents) = std::fs::read_to_string(path)
    {
        let trimmed = contents.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    // Generate a new token and persist it.
    let token = generate_auth_token();
    if let Some(ref path) = token_file_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, &token);
    }
    token
}

/// Detect the machine's LAN IP address (first non-loopback IPv4).
pub(crate) fn local_lan_ip() -> IpAddr {
    // Use a UDP socket trick: connect to a public IP (doesn't send data),
    // then read the local addr the OS chose.
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|sock| {
            sock.connect("8.8.8.8:53")?;
            sock.local_addr()
        })
        .map(|addr| addr.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Spawn the embedded WebSocket server as a background tokio task.
///
/// Returns a [`RemoteControlHandle`] that the TUI uses to display connection
/// info and to shut down the server when the TUI exits.
pub(crate) fn spawn_remote_control_server(
    settings: &RemoteControlSettings,
    thread_manager: Arc<ThreadManager>,
    config: &Config,
    cli_overrides: Vec<(String, toml::Value)>,
    feedback: CodexFeedback,
) -> RemoteControlHandle {
    let auth_token = settings.token.clone().unwrap_or_else(read_or_create_token);

    let lan_ip = local_lan_ip();
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), settings.port);
    let display_addr = SocketAddr::new(lan_ip, settings.port);

    let shutdown = CancellationToken::new();

    let ws_config = EmbeddedWebSocketConfig {
        bind_address: bind_addr,
        thread_manager,
        config: Arc::new(config.clone()),
        shutdown: shutdown.clone(),
        // These are used by the embedded MessageProcessor for its ConfigApi
        // and ExternalAgentConfigApi. Since the remote-control server reuses
        // the TUI's ThreadManager, these are secondary — use defaults.
        arg0_paths: Arg0DispatchPaths {
            codex_linux_sandbox_exe: None,
            main_execve_wrapper_exe: None,
        },
        cli_overrides,
        loader_overrides: LoaderOverrides::default(),
        cloud_requirements: CloudRequirementsLoader::default(),
        feedback,
        auth_token: Some(auth_token.clone()),
        owner_user_id: None,
    };

    tokio::spawn(async move {
        if let Err(err) = run_embedded_websocket(ws_config).await {
            error!("remote-control websocket server exited with error: {err}");
        }
        info!("remote-control websocket server stopped");
    });

    // Start mDNS/Bonjour advertisement so mobile clients can discover us.
    let discovery =
        crate::remote_discovery::advertise_remote_service(settings.port, &auth_token, lan_ip);

    // Start Supabase device registration so the TUI is discoverable via
    // Account Devices on the iOS app. Registration is best-effort: if it
    // fails (e.g. no ATA session) we log a warning and continue.
    // The spawned task owns the DeviceRegistrar and deregisters when the
    // shutdown token is cancelled (i.e. when RemoteControlHandle is dropped).
    {
        let registrar_port = settings.port;
        let registrar_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let codex_home = match find_codex_home() {
                Ok(h) => h.to_path_buf(),
                Err(e) => {
                    warn!("device registration skipped: cannot find codex home: {e}");
                    return;
                }
            };
            match DeviceRegistrar::start(&codex_home, None, Some(registrar_port)).await {
                Ok(registrar) => {
                    info!("device registered with Supabase for TUI remote-control");
                    // Wait for shutdown, then deregister.
                    registrar_shutdown.cancelled().await;
                    registrar.stop().await;
                }
                Err(e) => {
                    warn!("device registration failed (non-fatal): {e}");
                }
            }
        });
    }

    RemoteControlHandle {
        shutdown,
        bind_addr: display_addr,
        auth_token,
        _discovery: discovery,
    }
}
