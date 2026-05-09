// ATA-only Gemini OAuth (`gemini_server.rs`) and Supabase auth (`supabase_auth.rs`)
// flows are temporarily disabled for the rust-v0.129.0 upstream merge. They depend on
// fork-only modules (`codex_core::auth::gemini_oauth`, `codex_core::auth::storage`,
// `codex_core::supabase`) that were relocated/removed upstream. Re-enable as a follow-up
// per `merge_info/local_upstream_feature_analysis.md`.

pub mod auth;
pub mod auth_env_telemetry;
pub mod token_data;

mod device_code_auth;
mod pkce;
mod server;

pub use codex_client::BuildCustomCaTransportError as BuildLoginHttpClientError;
pub use codex_client::CodexHttpClient;
pub use codex_config::types::AuthCredentialsStoreMode;
pub use device_code_auth::DeviceCode;
pub use device_code_auth::complete_device_code_login;
pub use device_code_auth::request_device_code;
pub use device_code_auth::run_device_code_login;
pub use server::LoginServer;
pub use server::ServerOptions;
pub use server::ShutdownHandle;
pub use server::run_login_server;

pub use auth::AuthConfig;
pub use auth::AuthDotJson;
pub use auth::AuthManager;
pub use auth::AuthManagerConfig;
pub use auth::CLIENT_ID;
pub use auth::CODEX_ACCESS_TOKEN_ENV_VAR;
pub use auth::CODEX_API_KEY_ENV_VAR;
pub use auth::CodexAuth;
pub use auth::ExternalAuth;
pub use auth::ExternalAuthChatgptMetadata;
pub use auth::ExternalAuthRefreshContext;
pub use auth::ExternalAuthRefreshReason;
pub use auth::ExternalAuthTokens;
pub use auth::OPENAI_API_KEY_ENV_VAR;
pub use auth::REFRESH_TOKEN_EXPIRED_MESSAGE;
pub use auth::REFRESH_TOKEN_INVALIDATED_MESSAGE;
pub use auth::REFRESH_TOKEN_REUSED_MESSAGE;
pub use auth::REFRESH_TOKEN_UNKNOWN_MESSAGE;
pub use auth::REFRESH_TOKEN_URL;
pub use auth::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR;
pub use auth::REVOKE_TOKEN_URL_OVERRIDE_ENV_VAR;
pub use auth::RefreshTokenError;
pub use auth::UnauthorizedRecovery;
pub use auth::default_client;
pub use auth::enforce_login_restrictions;
pub use auth::load_auth_dot_json;
pub use auth::login_with_access_token;
pub use auth::login_with_api_key;
pub use auth::logout;
pub use auth::logout_with_revoke;
pub use auth::read_codex_access_token_from_env;
pub use auth::read_openai_api_key_from_env;
pub use auth::save_auth;
pub use auth_env_telemetry::AuthEnvTelemetry;
pub use auth_env_telemetry::collect_auth_env_telemetry;
pub use token_data::TokenData;
