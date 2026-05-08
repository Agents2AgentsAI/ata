use clap::Parser;
use codex_app_server::AppServerRuntimeOptions;
use codex_app_server::AppServerTransport;
use codex_app_server::AppServerWebsocketAuthArgs;
use codex_app_server::PluginStartupTasks;
use codex_app_server::run_main_with_transport_options;
use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_config::LoaderOverrides;
use codex_protocol::protocol::SessionSource;
use codex_utils_cli::CliConfigOverrides;
use std::path::PathBuf;

// Debug-only test hook: lets integration tests point the server at a temporary
// managed config file without writing to /etc.
const MANAGED_CONFIG_PATH_ENV_VAR: &str = "CODEX_APP_SERVER_MANAGED_CONFIG_PATH";
const DISABLE_MANAGED_CONFIG_ENV_VAR: &str = "CODEX_APP_SERVER_DISABLE_MANAGED_CONFIG";

#[derive(Debug, Parser)]
struct AppServerArgs {
    /// Transport endpoint URL. Supported values: `stdio://` (default),
    /// `unix://`, `unix://PATH`, `ws://IP:PORT`, `off`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: AppServerTransport,

    /// Optional auth token for WebSocket connections. When set, clients must
    /// provide `?token=<value>` on the upgrade request.
    #[arg(long = "token", value_name = "TOKEN")]
    token: Option<String>,

    /// Public URL for this server (e.g. ngrok URL). Used for device registration
    /// so remote clients can discover and connect to this node.
    #[arg(long = "public-url", value_name = "URL")]
    public_url: Option<String>,
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|arg0_paths: Arg0DispatchPaths| async move {
        let args = AppServerArgs::parse();
        let loader_overrides = if disable_managed_config_from_debug_env() {
            LoaderOverrides::without_managed_config_for_tests()
        } else {
            managed_config_path_from_debug_env()
                .map(LoaderOverrides::with_managed_config_path_for_tests)
                .unwrap_or_default()
        };
        let transport = args.listen;
        let session_source = args.session_source;
        let auth = args.auth.try_into_settings()?;
        let mut runtime_options = AppServerRuntimeOptions::default();
        #[cfg(debug_assertions)]
        if args.disable_plugin_startup_tasks_for_tests {
            runtime_options.plugin_startup_tasks = PluginStartupTasks::Skip;
        }

        run_main_with_transport_options(
            arg0_paths,
            CliConfigOverrides::default(),
            loader_overrides,
            /*default_analytics_enabled*/ false,
            transport,
            args.token,
            None, // owner_user_id — device registration handled separately
        )
        .await?;
        Ok(())
    })
}

fn disable_managed_config_from_debug_env() -> bool {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(DISABLE_MANAGED_CONFIG_ENV_VAR) {
            return matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES");
        }
    }

    false
}

fn managed_config_path_from_debug_env() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        if let Ok(value) = std::env::var(MANAGED_CONFIG_PATH_ENV_VAR) {
            return if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            };
        }
    }

    None
}
