use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::env;
use std::path::Path;

use codex_app_server_protocol::AuthMode as ApiAuthMode;

use super::OPENAI_API_KEY_ENV_VAR;
use super::storage::AUTH_JSON_VERSION;
use super::storage::AuthCredentialsStoreMode;
use super::storage::AuthDotJson;
use super::storage::create_auth_storage;

/// Provider ID constants for well-known providers.
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_GEMINI: &str = "gemini";
pub const ANTHROPIC_API_KEY_ENV_VAR: &str = "ANTHROPIC_API_KEY";
pub const GOOGLE_API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";

/// Credential types that can be stored for a provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCredential {
    /// API key-based authentication.
    Api { key: String },
    /// OAuth-based authentication (for future use).
    Oauth {
        access: String,
        refresh: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires: Option<DateTime<Utc>>,
    },
}

/// Status of a provider's authentication configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAuthSource {
    /// API key is stored in auth.json
    Stored,
    /// API key is set via environment variable
    Environment,
}

/// Information about a configured provider's auth status.
#[derive(Debug, Clone)]
pub struct ProviderAuthStatus {
    pub provider_id: String,
    pub source: ProviderAuthSource,
}

impl AuthDotJson {
    /// Migrate legacy auth.json format to v2 if needed.
    /// This performs in-memory migration without modifying the file.
    pub fn migrate_if_needed(mut self) -> Self {
        if self.version.unwrap_or(1) >= AUTH_JSON_VERSION {
            return self;
        }

        if let Some(api_key) = self.openai_api_key.clone()
            && !self.providers.contains_key(PROVIDER_OPENAI)
        {
            self.set_provider_credential(PROVIDER_OPENAI, ProviderCredential::Api { key: api_key });
        }

        self.version = Some(AUTH_JSON_VERSION);
        self
    }

    /// Get API key for a specific provider from the providers map.
    pub fn get_provider_api_key(&self, provider_id: &str) -> Option<&str> {
        self.providers.get(provider_id).and_then(|cred| match cred {
            ProviderCredential::Api { key } => Some(key.as_str()),
            ProviderCredential::Oauth { .. } => None,
        })
    }

    /// Check if there are any provider API keys configured.
    pub fn has_any_provider_api_key(&self) -> bool {
        self.providers
            .values()
            .any(|cred| matches!(cred, ProviderCredential::Api { .. }))
    }

    /// Set credential for a specific provider in the providers map.
    pub fn set_provider_credential(&mut self, provider_id: &str, credential: ProviderCredential) {
        if provider_id == PROVIDER_OPENAI {
            self.openai_api_key = match &credential {
                ProviderCredential::Api { key } => Some(key.clone()),
                ProviderCredential::Oauth { .. } => None,
            };
        }

        self.providers.insert(provider_id.to_string(), credential);
        self.version = Some(AUTH_JSON_VERSION);
    }

    /// Set API key for a specific provider in the providers map.
    pub fn set_provider_api_key(&mut self, provider_id: &str, api_key: &str) {
        self.set_provider_credential(
            provider_id,
            ProviderCredential::Api {
                key: api_key.to_string(),
            },
        );
    }

    /// Remove credentials for a specific provider.
    pub fn remove_provider(&mut self, provider_id: &str) -> bool {
        let removed = self.providers.remove(provider_id).is_some();
        if provider_id == PROVIDER_OPENAI {
            self.openai_api_key = None;
        }
        removed
    }

    /// Get list of configured provider IDs.
    pub fn configured_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}

/// Get the environment variable name for a provider.
pub fn provider_env_var(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        PROVIDER_OPENAI => Some(OPENAI_API_KEY_ENV_VAR),
        PROVIDER_ANTHROPIC => Some(ANTHROPIC_API_KEY_ENV_VAR),
        PROVIDER_GEMINI => Some(GOOGLE_API_KEY_ENV_VAR),
        _ => None,
    }
}

/// Read API key from environment variable for a specific provider.
pub fn read_api_key_from_env(provider_id: &str) -> Option<String> {
    provider_env_var(provider_id).and_then(|env_var| {
        env::var(env_var)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// Store credentials for a specific provider.
/// Updates existing auth.json if present, preserving other provider credentials.
pub(super) fn set_provider_credential(
    codex_home: &Path,
    provider_id: &str,
    credential: ProviderCredential,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let mut auth_dot_json = storage.load()?.unwrap_or_default();
    let is_api_credential = matches!(credential, ProviderCredential::Api { .. });
    auth_dot_json.set_provider_credential(provider_id, credential);
    if is_api_credential {
        auth_dot_json.auth_mode = Some(ApiAuthMode::ApiKey);
    }
    storage.save(&auth_dot_json)
}

/// Store an API key for a specific provider.
/// Updates existing auth.json if present, preserving other provider credentials.
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

/// Get the API key for a specific provider.
/// Checks environment variable first, then stored credentials.
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

/// List all configured providers with their auth status.
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
                });
            }

            for provider_id in auth.configured_providers() {
                if !result.iter().any(|p| p.provider_id == provider_id) {
                    result.push(ProviderAuthStatus {
                        provider_id,
                        source: ProviderAuthSource::Stored,
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

/// Remove credentials for a specific provider.
/// Returns true if the provider was removed, false if it wasn't configured.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::storage::AUTH_JSON_VERSION;
    use crate::auth::storage::AuthStorageBackend;
    use crate::auth::storage::FileAuthStorage;
    use crate::auth::storage::get_auth_file;
    use crate::auth::test_utils::EnvVarGuard;
    use codex_app_server_protocol::AuthMode;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use serial_test::serial;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[test]
    fn provider_credential_api_serialization() {
        let cred = ProviderCredential::Api {
            key: "sk-test".to_string(),
        };
        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("\"type\":\"api\""));
        assert!(json.contains("\"key\":\"sk-test\""));

        let deserialized: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(cred, deserialized);
    }

    #[test]
    fn provider_credential_oauth_serialization() {
        let cred = ProviderCredential::Oauth {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: Some(Utc::now()),
        };
        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("\"type\":\"oauth\""));
        assert!(json.contains("\"access\":\"access-token\""));

        let deserialized: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(cred, deserialized);
    }

    #[test]
    fn auth_dot_json_migrate_legacy_format() {
        let legacy = AuthDotJson {
            version: None,
            auth_mode: Some(AuthMode::ApiKey),
            openai_api_key: Some("sk-legacy".to_string()),
            tokens: None,
            last_refresh: None,
            providers: HashMap::new(),
        };

        let migrated = legacy.migrate_if_needed();

        assert_eq!(migrated.version, Some(AUTH_JSON_VERSION));
        assert!(migrated.providers.contains_key(PROVIDER_OPENAI));
        assert_eq!(
            migrated.get_provider_api_key(PROVIDER_OPENAI),
            Some("sk-legacy")
        );
        assert_eq!(migrated.openai_api_key, Some("sk-legacy".to_string()));
    }

    #[test]
    fn auth_dot_json_no_migration_for_v2() {
        let mut providers = HashMap::new();
        providers.insert(
            PROVIDER_ANTHROPIC.to_string(),
            ProviderCredential::Api {
                key: "sk-ant-test".to_string(),
            },
        );

        let v2 = AuthDotJson {
            version: Some(AUTH_JSON_VERSION),
            auth_mode: Some(AuthMode::ApiKey),
            openai_api_key: None,
            tokens: None,
            last_refresh: None,
            providers,
        };

        let result = v2.migrate_if_needed();

        assert!(!result.providers.contains_key(PROVIDER_OPENAI));
        assert_eq!(
            result.get_provider_api_key(PROVIDER_ANTHROPIC),
            Some("sk-ant-test")
        );
    }

    #[test]
    fn auth_dot_json_set_provider_api_key() {
        let mut auth = AuthDotJson::default();

        auth.set_provider_api_key(PROVIDER_ANTHROPIC, "sk-ant-new");

        assert_eq!(
            auth.get_provider_api_key(PROVIDER_ANTHROPIC),
            Some("sk-ant-new")
        );
        assert_eq!(auth.version, Some(AUTH_JSON_VERSION));
        assert!(auth.openai_api_key.is_none());
    }

    #[test]
    fn auth_dot_json_set_openai_api_key_updates_legacy_field() {
        let mut auth = AuthDotJson::default();

        auth.set_provider_api_key(PROVIDER_OPENAI, "sk-openai-new");

        assert_eq!(
            auth.get_provider_api_key(PROVIDER_OPENAI),
            Some("sk-openai-new")
        );
        assert_eq!(auth.openai_api_key, Some("sk-openai-new".to_string()));
    }

    #[test]
    fn auth_dot_json_remove_provider() {
        let mut auth = AuthDotJson::default();
        auth.set_provider_api_key(PROVIDER_ANTHROPIC, "sk-ant-test");
        auth.set_provider_api_key(PROVIDER_OPENAI, "sk-openai-test");

        let removed = auth.remove_provider(PROVIDER_ANTHROPIC);
        assert!(removed);
        assert!(auth.get_provider_api_key(PROVIDER_ANTHROPIC).is_none());
        assert!(auth.get_provider_api_key(PROVIDER_OPENAI).is_some());

        let removed = auth.remove_provider(PROVIDER_OPENAI);
        assert!(removed);
        assert!(auth.get_provider_api_key(PROVIDER_OPENAI).is_none());
        assert!(auth.openai_api_key.is_none());
    }

    #[test]
    fn auth_dot_json_configured_providers() {
        let mut auth = AuthDotJson::default();
        auth.set_provider_api_key(PROVIDER_OPENAI, "sk-openai");
        auth.set_provider_api_key(PROVIDER_ANTHROPIC, "sk-ant");
        auth.set_provider_api_key(PROVIDER_GEMINI, "AIza");

        let providers = auth.configured_providers();
        assert_eq!(providers.len(), 3);
        assert!(providers.contains(&PROVIDER_OPENAI.to_string()));
        assert!(providers.contains(&PROVIDER_ANTHROPIC.to_string()));
        assert!(providers.contains(&PROVIDER_GEMINI.to_string()));
    }

    #[test]
    fn file_storage_loads_and_migrates_legacy_format() -> anyhow::Result<()> {
        let codex_home = tempdir()?;
        let auth_file = get_auth_file(codex_home.path());

        let legacy_json = json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "sk-legacy-file"
        });
        std::fs::write(&auth_file, serde_json::to_string_pretty(&legacy_json)?)?;

        let storage = FileAuthStorage::new(codex_home.path().to_path_buf());
        let loaded = storage.load()?.expect("should load auth");

        assert_eq!(loaded.version, Some(AUTH_JSON_VERSION));
        assert_eq!(
            loaded.get_provider_api_key(PROVIDER_OPENAI),
            Some("sk-legacy-file")
        );
        Ok(())
    }

    #[test]
    fn auth_dot_json_v2_serialization_roundtrip() -> anyhow::Result<()> {
        let mut auth = AuthDotJson::default();
        auth.set_provider_api_key(PROVIDER_OPENAI, "sk-openai");
        auth.set_provider_api_key(PROVIDER_ANTHROPIC, "sk-ant");
        auth.auth_mode = Some(AuthMode::ApiKey);

        let json = serde_json::to_string_pretty(&auth)?;
        let deserialized: AuthDotJson = serde_json::from_str(&json)?;

        assert_eq!(auth.version, deserialized.version);
        assert_eq!(auth.openai_api_key, deserialized.openai_api_key);
        assert_eq!(
            auth.get_provider_api_key(PROVIDER_OPENAI),
            deserialized.get_provider_api_key(PROVIDER_OPENAI)
        );
        assert_eq!(
            auth.get_provider_api_key(PROVIDER_ANTHROPIC),
            deserialized.get_provider_api_key(PROVIDER_ANTHROPIC)
        );
        Ok(())
    }

    #[test]
    #[serial(codex_api_key)]
    fn login_with_provider_api_key_stores_key() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-ant-test",
            AuthCredentialsStoreMode::File,
        )
        .expect("should store key");

        let key = get_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            AuthCredentialsStoreMode::File,
        );
        assert_eq!(key, Some("sk-ant-test".to_string()));
    }

    #[test]
    #[serial(codex_api_key)]
    fn login_with_provider_api_key_preserves_existing_providers() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_OPENAI,
            "sk-openai",
            AuthCredentialsStoreMode::File,
        )
        .expect("should store openai key");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-ant",
            AuthCredentialsStoreMode::File,
        )
        .expect("should store anthropic key");

        assert_eq!(
            get_provider_api_key(dir.path(), PROVIDER_OPENAI, AuthCredentialsStoreMode::File),
            Some("sk-openai".to_string())
        );
        assert_eq!(
            get_provider_api_key(
                dir.path(),
                PROVIDER_ANTHROPIC,
                AuthCredentialsStoreMode::File
            ),
            Some("sk-ant".to_string())
        );
    }

    #[test]
    #[serial(codex_api_key)]
    fn get_provider_api_key_env_takes_precedence() {
        let dir = tempdir().unwrap();

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-stored",
            AuthCredentialsStoreMode::File,
        )
        .expect("should store key");

        let _guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "sk-env");

        let key = get_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            AuthCredentialsStoreMode::File,
        );
        assert_eq!(key, Some("sk-env".to_string()));
    }

    #[test]
    #[serial(codex_api_key)]
    fn list_configured_providers_shows_stored() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_OPENAI,
            "sk-openai",
            AuthCredentialsStoreMode::File,
        )
        .expect("store openai");
        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-ant",
            AuthCredentialsStoreMode::File,
        )
        .expect("store anthropic");

        let providers = list_configured_providers(dir.path(), AuthCredentialsStoreMode::File);
        assert_eq!(providers.len(), 2);

        let openai = providers.iter().find(|p| p.provider_id == PROVIDER_OPENAI);
        assert!(openai.is_some());
        assert_eq!(openai.unwrap().source, ProviderAuthSource::Stored);

        let anthropic = providers
            .iter()
            .find(|p| p.provider_id == PROVIDER_ANTHROPIC);
        assert!(anthropic.is_some());
        assert_eq!(anthropic.unwrap().source, ProviderAuthSource::Stored);
    }

    #[test]
    #[serial(codex_api_key)]
    fn list_configured_providers_shows_env() {
        let dir = tempdir().unwrap();
        let _guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "sk-env");

        let providers = list_configured_providers(dir.path(), AuthCredentialsStoreMode::File);

        let anthropic = providers
            .iter()
            .find(|p| p.provider_id == PROVIDER_ANTHROPIC);
        assert!(anthropic.is_some());
        assert_eq!(anthropic.unwrap().source, ProviderAuthSource::Environment);
    }

    #[test]
    #[serial(codex_api_key)]
    fn remove_provider_removes_single_provider() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_OPENAI,
            "sk-openai",
            AuthCredentialsStoreMode::File,
        )
        .expect("store openai");
        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-ant",
            AuthCredentialsStoreMode::File,
        )
        .expect("store anthropic");

        let removed = remove_provider(
            dir.path(),
            PROVIDER_ANTHROPIC,
            AuthCredentialsStoreMode::File,
        )
        .expect("remove should succeed");
        assert!(removed);

        assert!(
            get_provider_api_key(
                dir.path(),
                PROVIDER_ANTHROPIC,
                AuthCredentialsStoreMode::File
            )
            .is_none()
        );
        assert!(
            get_provider_api_key(dir.path(), PROVIDER_OPENAI, AuthCredentialsStoreMode::File)
                .is_some()
        );
    }

    #[test]
    #[serial(codex_api_key)]
    fn remove_provider_deletes_file_when_empty() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_ANTHROPIC,
            "sk-ant",
            AuthCredentialsStoreMode::File,
        )
        .expect("store anthropic");

        let auth_file = get_auth_file(dir.path());
        assert!(auth_file.exists());

        remove_provider(
            dir.path(),
            PROVIDER_ANTHROPIC,
            AuthCredentialsStoreMode::File,
        )
        .expect("remove should succeed");

        assert!(!auth_file.exists());
    }

    #[test]
    fn provider_env_var_returns_correct_vars() {
        assert_eq!(
            provider_env_var(PROVIDER_OPENAI),
            Some(OPENAI_API_KEY_ENV_VAR)
        );
        assert_eq!(
            provider_env_var(PROVIDER_ANTHROPIC),
            Some(ANTHROPIC_API_KEY_ENV_VAR)
        );
        assert_eq!(
            provider_env_var(PROVIDER_GEMINI),
            Some(GOOGLE_API_KEY_ENV_VAR)
        );
        assert_eq!(provider_env_var("unknown"), None);
    }
}
