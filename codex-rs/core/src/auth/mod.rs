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
//!
//! `#![allow(unused_imports, dead_code)]` covers the public surface here:
//! many `pub use` re-exports are not consumed inside `codex-core` itself
//! (they're forwarded for cli/tui/app-server consumption) and several
//! provider helper APIs (`login_with_provider_*`, `list_configured_providers`,
//! the gemini revoke helpers, etc.) are exposed for the eventual
//! `ata login provider` / `ata logout provider` CLI subcommands that are
//! still pending.
#![allow(unused_imports, dead_code)]

pub mod copilot_oauth;
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

pub use copilot_oauth::COPILOT_EDITOR_PLUGIN_VERSION;
pub use copilot_oauth::COPILOT_EDITOR_VERSION;
pub use copilot_oauth::COPILOT_INTEGRATION_ID;
pub use copilot_oauth::COPILOT_OAUTH_CLIENT_ID;
pub use copilot_oauth::COPILOT_USER_AGENT;
pub use copilot_oauth::DeviceCodeResponse as CopilotDeviceCodeResponse;
pub use copilot_oauth::complete_login as complete_copilot_login;
pub use copilot_oauth::get_or_refresh_copilot_token;
pub use copilot_oauth::logout as copilot_logout;
pub use copilot_oauth::poll_for_access_token as poll_copilot_access_token;
pub use copilot_oauth::save_credentials as save_copilot_credentials;
pub use copilot_oauth::start_device_flow as start_copilot_device_flow;
pub use providers::ANTHROPIC_API_KEY_ENV_VAR;
pub use providers::GOOGLE_API_KEY_ENV_VAR;
pub use providers::GeminiAuthSource;
pub use providers::PROVIDER_ANTHROPIC;
pub use providers::PROVIDER_COPILOT;
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

/// Extension trait that resolves an API key for a `ModelProviderInfo` using
/// both the env-var path (`api_key()`) and the per-provider auth storage path
/// (`get_provider_api_key`). Returned `Ok(None)` means "no credential
/// configured" — callers should treat that as a soft skip rather than a fatal
/// error so that providers without API keys (e.g., OAuth-only) still work.
pub trait ModelProviderApiKeyExt {
    fn api_key_with_auth(
        &self,
        codex_home: &std::path::Path,
        store_mode: AuthCredentialsStoreMode,
    ) -> codex_protocol::error::Result<Option<String>>;
}

impl ModelProviderApiKeyExt for codex_model_provider_info::ModelProviderInfo {
    fn api_key_with_auth(
        &self,
        codex_home: &std::path::Path,
        store_mode: AuthCredentialsStoreMode,
    ) -> codex_protocol::error::Result<Option<String>> {
        if let Some(key) = self.api_key()? {
            return Ok(Some(key));
        }
        let provider_id = match self.wire_api {
            codex_model_provider_info::WireApi::Responses => PROVIDER_OPENAI,
            codex_model_provider_info::WireApi::AnthropicMessages => PROVIDER_ANTHROPIC,
            codex_model_provider_info::WireApi::GeminiGenerate => PROVIDER_GEMINI,
            // Copilot uses OAuth — there's no API key in storage to return.
            // Surface "no credential" so callers fall through to the OAuth
            // bearer-token resolution path in `ModelClient`.
            codex_model_provider_info::WireApi::CopilotInline => return Ok(None),
        };
        Ok(get_provider_api_key(codex_home, provider_id, store_mode))
    }
}
