use codex_app_server_protocol::AuthMode;
use codex_core::CodexAuth;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::CLIENT_ID;
use codex_core::auth::PROVIDER_ANTHROPIC;
use codex_core::auth::PROVIDER_GEMINI;
use codex_core::auth::PROVIDER_OPENAI;
use codex_core::auth::ProviderAuthMethod;
use codex_core::auth::ProviderAuthSource;
use codex_core::auth::get_provider_api_key;
use codex_core::auth::list_configured_providers;
use codex_core::auth::login_with_provider_api_key;
use codex_core::auth::logout_provider;
use codex_core::auth::provider_env_var;
use codex_core::config::Config;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::edit::default_model_for_provider;
use codex_core::AuthManager;
use codex_login::GeminiServerOptions;
use codex_login::ServerOptions;
use codex_login::run_device_code_login;
use codex_login::run_gemini_login_server;
use codex_login::run_login_server;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_utils_cli::CliConfigOverrides;
use std::io::IsTerminal;
use std::io::Read;
use std::path::PathBuf;

const CHATGPT_LOGIN_DISABLED_MESSAGE: &str =
    "ChatGPT login is disabled. Use API key login instead.";
const API_KEY_LOGIN_DISABLED_MESSAGE: &str =
    "API key login is disabled. Use ChatGPT login instead.";
const OAUTH_LOGIN_DISABLED_MESSAGE: &str = "OAuth login is disabled. Use ChatGPT login instead.";
const LOGIN_SUCCESS_MESSAGE: &str = "Successfully logged in";

fn print_login_server_start(actual_port: u16, auth_url: &str) {
    eprintln!(
        "Starting local login server on http://localhost:{actual_port}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{auth_url}"
    );
}

pub async fn login_with_chatgpt(
    codex_home: PathBuf,
    forced_chatgpt_workspace_id: Option<String>,
    cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let opts = ServerOptions::new(
        codex_home,
        CLIENT_ID.to_string(),
        forced_chatgpt_workspace_id,
        cli_auth_credentials_store_mode,
    );
    let server = run_login_server(opts)?;

    print_login_server_start(server.actual_port, &server.auth_url);

    server.block_until_done().await
}

pub async fn run_login_with_chatgpt(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();

    match login_with_chatgpt(
        config.codex_home,
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    )
    .await
    {
        Ok(_) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
) -> ! {
    run_login_with_provider_api_key(cli_config_overrides, api_key, None).await
}

/// Login with an API key for a specific provider.
/// If provider is None, defaults to OpenAI.
pub async fn run_login_with_provider_api_key(
    cli_config_overrides: CliConfigOverrides,
    api_key: String,
    provider: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Chatgpt)) {
        eprintln!("{API_KEY_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let provider_id = validate_provider_id(provider.as_deref());

    match login_with_provider_api_key(
        &config.codex_home,
        provider_id,
        &api_key,
        config.cli_auth_credentials_store_mode,
    ) {
        Ok(_) => {
            let default_model = default_model_for_provider(provider_id);
            if let Err(err) = ConfigEditsBuilder::new(&config.codex_home)
                .set_model(default_model, None, Some(provider_id.to_string()))
                .apply_blocking()
            {
                eprintln!("Warning: failed to set default model for provider {provider_id}: {err}");
            }
            eprintln!("{LOGIN_SUCCESS_MESSAGE} for provider: {provider_id}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

/// Login with OAuth for a specific provider.
/// Currently supported only for Gemini.
pub async fn run_login_with_provider_oauth(
    cli_config_overrides: CliConfigOverrides,
    provider: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Chatgpt)) {
        eprintln!("{OAUTH_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let provider_id = validate_provider_id(provider.as_deref());
    if provider_id != PROVIDER_GEMINI {
        eprintln!(
            "OAuth login is currently supported only for provider: {PROVIDER_GEMINI}. Use --with-api-key for {provider_id}."
        );
        std::process::exit(1);
    }

    let opts = GeminiServerOptions::new(
        config.codex_home.clone(),
        config.cli_auth_credentials_store_mode,
    );
    match run_gemini_login_server(opts) {
        Ok(server) => {
            print_login_server_start(server.actual_port, &server.auth_url);
            match server.block_until_done().await {
                Ok(()) => {
                    let default_model = default_model_for_provider(provider_id);
                    if let Err(err) = ConfigEditsBuilder::new(&config.codex_home)
                        .set_model(default_model, None, Some(provider_id.to_string()))
                        .apply_blocking()
                    {
                        eprintln!(
                            "Warning: failed to set default model for provider {provider_id}: {err}"
                        );
                    }
                    eprintln!("{LOGIN_SUCCESS_MESSAGE} for provider: {provider_id}");
                    std::process::exit(0);
                }
                Err(err) => {
                    eprintln!("Error logging in: {err}");
                    std::process::exit(1);
                }
            }
        }
        Err(err) => {
            eprintln!("Error logging in: {err}");
            std::process::exit(1);
        }
    }
}

/// Validate and normalize provider ID.
fn validate_provider_id(provider: Option<&str>) -> &str {
    match provider {
        None => PROVIDER_OPENAI,
        Some(p) => match p.to_lowercase().as_str() {
            "openai" => PROVIDER_OPENAI,
            "anthropic" => PROVIDER_ANTHROPIC,
            "gemini" | "google" | "google-gemini" => PROVIDER_GEMINI,
            _ => {
                eprintln!("Unknown provider: {p}. Valid providers: openai, anthropic, gemini");
                std::process::exit(1);
            }
        },
    }
}

pub fn read_api_key_from_stdin() -> String {
    let mut stdin = std::io::stdin();

    if stdin.is_terminal() {
        eprintln!(
            "--with-api-key expects the API key on stdin. Try piping it, e.g. `printenv OPENAI_API_KEY | ata login --with-api-key`."
        );
        std::process::exit(1);
    }

    eprintln!("Reading API key from stdin...");

    let mut buffer = String::new();
    if let Err(err) = stdin.read_to_string(&mut buffer) {
        eprintln!("Failed to read API key from stdin: {err}");
        std::process::exit(1);
    }

    let api_key = buffer.trim().to_string();
    if api_key.is_empty() {
        eprintln!("No API key provided via stdin.");
        std::process::exit(1);
    }

    api_key
}

/// Login using the OAuth device code flow.
pub async fn run_login_with_device_code(
    cli_config_overrides: CliConfigOverrides,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }
    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    match run_device_code_login(opts).await {
        Ok(()) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in with device code: {e}");
            std::process::exit(1);
        }
    }
}

/// Prefers device-code login (with `open_browser = false`) when headless environment is detected, but keeps
/// `codex login` working in environments where device-code may be disabled/feature-gated.
/// If `run_device_code_login` returns `ErrorKind::NotFound` ("device-code unsupported"), this
/// falls back to starting the local browser login server.
pub async fn run_login_with_device_code_fallback_to_browser(
    cli_config_overrides: CliConfigOverrides,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    opts.open_browser = false;

    match run_device_code_login(opts.clone()).await {
        Ok(()) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                eprintln!("Device code login is not enabled; falling back to browser login.");
                match run_login_server(opts) {
                    Ok(server) => {
                        print_login_server_start(server.actual_port, &server.auth_url);
                        match server.block_until_done().await {
                            Ok(()) => {
                                eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                                std::process::exit(0);
                            }
                            Err(e) => {
                                eprintln!("Error logging in: {e}");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error logging in: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("Error logging in with device code: {e}");
                std::process::exit(1);
            }
        }
    }
}

pub async fn run_login_status(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    // Show ChatGPT auth status if available
    let mut chatgpt_auth = false;
    match CodexAuth::from_auth_storage(&config.codex_home, config.cli_auth_credentials_store_mode) {
        Ok(Some(auth)) => match auth.api_auth_mode() {
            AuthMode::ApiKey => {
                // Don't show legacy status, fall through to provider list
            }
            AuthMode::Chatgpt => {
                eprintln!("Logged in using ChatGPT");
                chatgpt_auth = true;
            }
            AuthMode::ChatgptAuthTokens => {
                eprintln!("Logged in using ChatGPT (external tokens)");
                chatgpt_auth = true;
            }
        },
        Ok(None) => {}
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    }

    // Show configured providers
    let providers =
        list_configured_providers(&config.codex_home, config.cli_auth_credentials_store_mode);

    if providers.is_empty() {
        if chatgpt_auth {
            std::process::exit(0);
        }
        eprintln!("No provider credentials configured");
        std::process::exit(1);
    }

    eprintln!("Configured provider credentials:");
    for provider in &providers {
        let source = match provider.source {
            ProviderAuthSource::Stored => "stored",
            ProviderAuthSource::Environment => "env",
        };
        eprintln!(
            "  {} ({source}, {})",
            provider.provider_id,
            auth_method_label(provider.method)
        );
    }
    std::process::exit(0);
}

/// List all configured providers.
pub async fn run_list_providers(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    let providers =
        list_configured_providers(&config.codex_home, config.cli_auth_credentials_store_mode);

    if providers.is_empty() {
        eprintln!("No providers configured");
        eprintln!();
        eprintln!("To configure a provider, run:");
        eprintln!("  ata login --provider <provider>");
        eprintln!();
        eprintln!("Available providers: openai, anthropic, gemini");
        std::process::exit(0);
    }

    eprintln!("Configured providers:");
    for provider in &providers {
        let source = match provider.source {
            ProviderAuthSource::Stored => "stored",
            ProviderAuthSource::Environment => "env",
        };

        // Show env var hint for environment-sourced keys
        let hint = if provider.source == ProviderAuthSource::Environment {
            provider_env_var(&provider.provider_id)
                .map(|v| format!(" (${v})"))
                .unwrap_or_default()
        } else {
            String::new()
        };

        eprintln!(
            "  {} ({source}, {}){}",
            provider.provider_id,
            auth_method_label(provider.method),
            hint
        );
    }
    std::process::exit(0);
}

/// Logout a specific provider or all providers.
pub async fn run_logout_provider(
    cli_config_overrides: CliConfigOverrides,
    provider: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    match provider {
        Some(p) => {
            let provider_id = validate_provider_id(Some(&p));

            // Check if this provider is set via environment variable
            if get_provider_api_key(
                &config.codex_home,
                provider_id,
                config.cli_auth_credentials_store_mode,
            )
            .is_some()
                && let Some(env_var) = provider_env_var(provider_id)
                && std::env::var(env_var).is_ok()
            {
                eprintln!(
                    "Note: {provider_id} API key is set via ${env_var} environment variable."
                );
                eprintln!("Removing stored credentials will not affect the environment variable.");
            }

            match logout_provider(
                &config.codex_home,
                provider_id,
                config.cli_auth_credentials_store_mode,
            ) {
                Ok(true) => {
                    eprintln!("Successfully logged out of {provider_id}");
                    std::process::exit(0);
                }
                Ok(false) => {
                    eprintln!("No stored credentials found for {provider_id}");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error logging out: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            // Logout all (existing behavior)
            let auth_manager = AuthManager::new(
                config.codex_home.clone(),
                false,
                config.cli_auth_credentials_store_mode,
            );
            match auth_manager.logout() {
                Ok(true) => {
                    eprintln!("Successfully logged out");
                    std::process::exit(0);
                }
                Ok(false) => {
                    eprintln!("Not logged in");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Error logging out: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

pub async fn run_logout(cli_config_overrides: CliConfigOverrides) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    let auth_manager = AuthManager::new(
        config.codex_home.clone(),
        false,
        config.cli_auth_credentials_store_mode,
    );
    match auth_manager.logout() {
        Ok(true) => {
            eprintln!("Successfully logged out");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

async fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    match Config::load_with_cli_overrides(cli_overrides).await {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

#[allow(dead_code)]
fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

fn auth_method_label(method: ProviderAuthMethod) -> &'static str {
    match method {
        ProviderAuthMethod::ApiKey => "api_key",
        ProviderAuthMethod::Oauth => "oauth",
        ProviderAuthMethod::ApiKeyAndOauth => "api_key+oauth",
    }
}

#[cfg(test)]
mod tests {
    use super::safe_format_key;

    #[test]
    fn formats_long_key() {
        let key = "sk-proj-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "sk-proj-***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "sk-proj-12345";
        assert_eq!(safe_format_key(key), "***");
    }
}
