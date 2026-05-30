// ATA-only Supabase auth (`supabase_auth.rs`) is still temporarily disabled
// for the rust-v0.129.0 upstream merge — the ATA email-OTP flow now lives
// behind `app-server-protocol::v2::LoginAccountParams::AtaSendOtp/AtaVerifyOtp`
// and is driven from the TUI's onboarding picker.
//
// `gemini_server.rs` was re-introduced from `main` as part of restoring the
// four-option onboarding picker (Gemini Code Assist OAuth). It lives behind
// `LoginAccountParams::GeminiOauth`.

pub mod auth;
pub mod auth_env_telemetry;
pub mod token_data;

mod device_code_auth;
// === ATA: Gemini Code Assist OAuth login server. ===
mod gemini_server;
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
// === ATA: Gemini OAuth public surface (used by app-server's
//   `account/login/start` handler for `LoginAccountParams::GeminiOauth`). ===
pub use gemini_server::GeminiServerOptions;
pub use gemini_server::run_gemini_login_server;

pub use auth::AuthConfig;
pub use auth::AuthDotJson;
pub use auth::AuthManager;
pub use auth::AuthManagerConfig;
pub use auth::CLIENT_ID;
pub use auth::CODEX_ACCESS_TOKEN_ENV_VAR;
pub use auth::CODEX_API_KEY_ENV_VAR;
pub use auth::ATA_SUPABASE_TOKEN_ENV_VAR;
pub use auth::AtaAuth;
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
pub use auth::read_ata_supabase_token_from_env;
pub use auth::read_codex_access_token_from_env;
pub use auth::read_openai_api_key_from_env;
pub use auth::save_auth;
pub use auth_env_telemetry::AuthEnvTelemetry;
pub use auth_env_telemetry::collect_auth_env_telemetry;
pub use token_data::TokenData;
