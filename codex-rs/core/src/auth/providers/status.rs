use std::path::Path;

use codex_app_server_protocol::AuthMode as ApiAuthMode;

use crate::auth::storage::AuthCredentialsStoreMode;
use crate::auth::storage::create_auth_storage;

use super::env::read_api_key_from_env;
use super::storage_ops::get_provider_api_key;
use super::storage_ops::get_provider_oauth_credential;
use super::types::GeminiAuthSource;
use super::types::PROVIDER_ANTHROPIC;
use super::types::PROVIDER_GEMINI;
use super::types::PROVIDER_OPENAI;
use super::types::ProviderAuthMethod;
use super::types::ProviderAuthSource;
use super::types::ProviderAuthStatus;
use super::types::ProviderCredential;

pub fn resolve_gemini_auth_source(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> GeminiAuthSource {
    if let Some(api_key) =
        get_provider_api_key(codex_home, PROVIDER_GEMINI, auth_credentials_store_mode)
    {
        return GeminiAuthSource::ApiKey(api_key);
    }

    match get_provider_oauth_credential(codex_home, PROVIDER_GEMINI, auth_credentials_store_mode) {
        Some(oauth) => GeminiAuthSource::Oauth(oauth),
        None => GeminiAuthSource::Missing,
    }
}

pub fn list_configured_providers(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> Vec<ProviderAuthStatus> {
    let mut result = Vec::new();

    for provider_id in [PROVIDER_OPENAI, PROVIDER_ANTHROPIC, PROVIDER_GEMINI] {
        if read_api_key_from_env(provider_id).is_some() {
            result.push(ProviderAuthStatus {
                provider_id: provider_id.to_string(),
                source: ProviderAuthSource::Environment,
                method: ProviderAuthMethod::ApiKey,
            });
        }
    }

    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    match storage.load() {
        Ok(Some(auth)) => {
            if auth.auth_mode == Some(ApiAuthMode::Chatgpt)
                && auth.tokens.is_some()
                && !result.iter().any(|p| p.provider_id == PROVIDER_OPENAI)
            {
                result.push(ProviderAuthStatus {
                    provider_id: PROVIDER_OPENAI.to_string(),
                    source: ProviderAuthSource::Stored,
                    method: ProviderAuthMethod::Oauth,
                });
            }

            for provider_id in auth.configured_providers() {
                if !result.iter().any(|p| p.provider_id == provider_id) {
                    let method = auth
                        .providers
                        .get(&provider_id)
                        .map(|credential| match credential {
                            ProviderCredential::Api { .. } => ProviderAuthMethod::ApiKey,
                            ProviderCredential::Oauth { .. } => ProviderAuthMethod::Oauth,
                            ProviderCredential::ApiAndOauth { .. } => {
                                ProviderAuthMethod::ApiKeyAndOauth
                            }
                        })
                        .unwrap_or(ProviderAuthMethod::ApiKey);
                    result.push(ProviderAuthStatus {
                        provider_id,
                        source: ProviderAuthSource::Stored,
                        method,
                    });
                }
            }
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(
                error = %err,
                "Failed to load auth storage when listing providers. Check file permissions or keyring access."
            );
        }
    }

    result
}
