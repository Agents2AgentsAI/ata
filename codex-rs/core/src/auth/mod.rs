//! ATA-side auth surface, layered on `codex-login`.
//!
//! `codex-login` owns the `auth.json` storage backend, OpenAI auth-token
//! refresh, the `AuthManager`, and the device-code login server. ATA layers
//! per-provider credential storage (OpenAI / Anthropic / Gemini), Gemini
//! Code Assist OAuth, and a refresh adapter on top.
//!
//! Naming convention: `crate::auth::storage` re-exports codex-login's
//! storage primitives so the stranded provider modules below can keep their
//! original `crate::auth::storage::*` import paths.

pub mod gemini_oauth;
pub mod gemini_revoke;
pub mod providers;
pub mod refresh;
#[cfg(test)]
pub(crate) mod test_utils;

/// Re-exports from `codex_login::auth::storage` so internal modules can use
/// the familiar `crate::auth::storage::*` paths without re-implementing the
/// file/keyring backend.
pub mod storage {
    pub use codex_config::types::AuthCredentialsStoreMode;
    pub use codex_login::auth::storage::AUTH_JSON_VERSION;
    pub use codex_login::auth::storage::AgentIdentityAuthRecord;
    pub use codex_login::auth::storage::AuthDotJson;
    pub use codex_login::auth::storage::AuthStorageBackend;
    pub use codex_login::auth::storage::FileAuthStorage;
    pub use codex_login::auth::storage::ProviderCredential;
    pub use codex_login::auth::storage::ProviderOauthCredential;
    pub use codex_login::auth::storage::create_auth_storage;
    pub use codex_login::auth::storage::delete_file_if_exists;
    pub use codex_login::auth::storage::get_auth_file;
}

// Common pubs that downstream modules expect at the `crate::auth::` path.
pub use codex_config::types::AuthCredentialsStoreMode;
pub use codex_login::AuthManager;
pub use codex_login::CodexAuth;
pub use codex_login::OPENAI_API_KEY_ENV_VAR;
pub use codex_login::auth::storage::AuthDotJson;

pub use providers::ANTHROPIC_API_KEY_ENV_VAR;
pub use providers::GOOGLE_API_KEY_ENV_VAR;
pub use providers::GeminiAuthSource;
pub use providers::PROVIDER_ANTHROPIC;
pub use providers::PROVIDER_GEMINI;
pub use providers::PROVIDER_OPENAI;
pub use providers::ProviderAuthMethod;
pub use providers::ProviderAuthSource;
pub use providers::ProviderAuthStatus;
pub use providers::ProviderCredential;
pub use providers::ProviderOauthCredential;
pub use providers::clear_provider_oauth_credential;
pub use providers::get_provider_api_key;
pub use providers::get_provider_oauth_credential;
pub use providers::list_configured_providers;
pub use providers::login_with_provider_api_key;
pub use providers::login_with_provider_oauth;
pub use providers::provider_env_var;
pub use providers::read_api_key_from_env;
pub use providers::resolve_gemini_auth_source;
