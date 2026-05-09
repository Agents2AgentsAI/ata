// `ProviderCredential`, `ProviderOauthCredential`, and the corresponding
// `AuthDotJson` accessor methods now live in `codex_login::auth::storage` so
// the foreign type can carry the `providers` map directly. Re-export them here
// to keep the historical `crate::auth::providers::types::*` paths intact.

pub use crate::auth::storage::ProviderCredential;
pub use crate::auth::storage::ProviderOauthCredential;

/// Provider ID constants for well-known providers.
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_GEMINI: &str = "gemini";
pub const ANTHROPIC_API_KEY_ENV_VAR: &str = "ANTHROPIC_API_KEY";
pub const GOOGLE_API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAuthSource {
    Stored,
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthMethod {
    ApiKey,
    Oauth,
    ApiKeyAndOauth,
}

#[derive(Debug, Clone)]
pub struct ProviderAuthStatus {
    pub provider_id: String,
    pub source: ProviderAuthSource,
    pub method: ProviderAuthMethod,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeminiAuthSource {
    ApiKey(String),
    Oauth(ProviderOauthCredential),
    Missing,
}
