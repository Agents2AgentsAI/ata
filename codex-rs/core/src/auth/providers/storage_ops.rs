use std::path::Path;

use codex_app_server_protocol::AuthMode as ApiAuthMode;

use crate::auth::storage::AuthCredentialsStoreMode;
use crate::auth::storage::create_auth_storage;

use super::env::read_api_key_from_env;
use super::types::ProviderCredential;
use super::types::ProviderOauthCredential;

pub fn login_with_provider_api_key(
    codex_home: &Path,
    provider_id: &str,
    api_key: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    set_provider_credential(
        codex_home,
        provider_id,
        ProviderCredential::Api {
            key: api_key.to_string(),
        },
        auth_credentials_store_mode,
    )
}

pub fn login_with_provider_oauth(
    codex_home: &Path,
    provider_id: &str,
    credential: ProviderOauthCredential,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    set_provider_credential(
        codex_home,
        provider_id,
        ProviderCredential::Oauth { credential },
        auth_credentials_store_mode,
    )
}

pub fn get_provider_api_key(
    codex_home: &Path,
    provider_id: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> Option<String> {
    if let Some(key) = read_api_key_from_env(provider_id) {
        return Some(key);
    }

    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    match storage.load() {
        Ok(Some(auth)) => auth
            .get_provider_api_key(provider_id)
            .map(std::string::ToString::to_string),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                provider_id = provider_id,
                error = %err,
                "Failed to load auth storage for provider. Check file permissions or keyring access."
            );
            None
        }
    }
}

pub fn get_provider_oauth_credential(
    codex_home: &Path,
    provider_id: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> Option<ProviderOauthCredential> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    match storage.load() {
        Ok(Some(auth)) => auth.get_provider_oauth_credential(provider_id),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                provider_id = provider_id,
                error = %err,
                "Failed to load auth storage for provider OAuth credential. Check file permissions or keyring access."
            );
            None
        }
    }
}

pub(super) fn remove_provider(
    codex_home: &Path,
    provider_id: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<bool> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let Some(mut auth) = storage.load()? else {
        return Ok(false);
    };

    let removed = auth.remove_provider(provider_id);
    if removed {
        if auth.providers.is_empty() && auth.tokens.is_none() {
            storage.delete()?;
        } else {
            storage.save(&auth)?;
        }
    }
    Ok(removed)
}

pub fn clear_provider_oauth_credential(
    codex_home: &Path,
    provider_id: &str,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<bool> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let Some(mut auth) = storage.load()? else {
        return Ok(false);
    };

    let cleared = auth.clear_provider_oauth_credential(provider_id);
    if cleared {
        if auth.providers.is_empty() && auth.tokens.is_none() {
            storage.delete()?;
        } else {
            storage.save(&auth)?;
        }
    }
    Ok(cleared)
}

fn set_provider_credential(
    codex_home: &Path,
    provider_id: &str,
    credential: ProviderCredential,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let mut auth_dot_json = storage.load()?.unwrap_or_default();
    let is_api_credential = matches!(
        &credential,
        ProviderCredential::Api { .. } | ProviderCredential::ApiAndOauth { .. }
    );
    auth_dot_json.set_provider_credential(provider_id, credential);
    if is_api_credential {
        auth_dot_json.auth_mode = Some(ApiAuthMode::ApiKey);
    }
    storage.save(&auth_dot_json)
}
