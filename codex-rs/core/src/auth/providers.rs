mod env;
mod status;
mod storage_ops;
mod types;

pub use env::provider_env_var;
pub use env::read_api_key_from_env;
pub use status::list_configured_providers;
pub use status::resolve_gemini_auth_source;
pub use storage_ops::clear_provider_oauth_credential;
pub use storage_ops::get_provider_api_key;
pub use storage_ops::get_provider_oauth_credential;
pub use storage_ops::login_with_provider_api_key;
pub use storage_ops::login_with_provider_oauth;
pub(super) use storage_ops::remove_provider;
pub use types::ANTHROPIC_API_KEY_ENV_VAR;
pub use types::GOOGLE_API_KEY_ENV_VAR;
pub use types::GeminiAuthSource;
pub use types::PROVIDER_ANTHROPIC;
pub use types::PROVIDER_GEMINI;
pub use types::PROVIDER_OPENAI;
pub use types::ProviderAuthMethod;
pub use types::ProviderAuthSource;
pub use types::ProviderAuthStatus;
pub use types::ProviderCredential;
pub use types::ProviderOauthCredential;

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
            credential: ProviderOauthCredential {
                access: "access-token".to_string(),
                refresh: "refresh-token".to_string(),
                expires: Some(Utc::now()),
                email: Some("user@example.com".to_string()),
                project_id: Some("project-id".to_string()),
                managed_project_id: Some("managed-project-id".to_string()),
            },
        };
        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("\"type\":\"oauth\""));
        assert!(json.contains("\"access\":\"access-token\""));

        let deserialized: ProviderCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(cred, deserialized);
    }

    #[test]
    fn provider_credential_api_and_oauth_serialization() {
        let cred = ProviderCredential::ApiAndOauth {
            key: "AIza-1".to_string(),
            credential: ProviderOauthCredential {
                access: "access-token".to_string(),
                refresh: "refresh-token".to_string(),
                expires: Some(Utc::now()),
                email: Some("user@example.com".to_string()),
                project_id: Some("project-id".to_string()),
                managed_project_id: Some("managed-project-id".to_string()),
            },
        };
        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("\"type\":\"api_and_oauth\""));
        assert!(json.contains("\"key\":\"AIza-1\""));
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
    fn auth_dot_json_set_provider_oauth_credential() {
        let mut auth = AuthDotJson::default();
        let credential = ProviderOauthCredential {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: Some("project-id".to_string()),
            managed_project_id: Some("managed-project-id".to_string()),
        };

        auth.set_provider_oauth_credential(PROVIDER_GEMINI, credential.clone());

        assert_eq!(auth.get_provider_api_key(PROVIDER_GEMINI), None);
        assert_eq!(
            auth.get_provider_oauth_credential(PROVIDER_GEMINI),
            Some(credential),
        );
    }

    #[test]
    fn auth_dot_json_set_provider_oauth_preserves_existing_provider_api_key() {
        let mut auth = AuthDotJson::default();
        let oauth = ProviderOauthCredential {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };

        auth.set_provider_api_key(PROVIDER_GEMINI, "AIza-1");
        auth.set_provider_oauth_credential(PROVIDER_GEMINI, oauth.clone());

        assert_eq!(auth.get_provider_api_key(PROVIDER_GEMINI), Some("AIza-1"));
        assert_eq!(
            auth.get_provider_oauth_credential(PROVIDER_GEMINI),
            Some(oauth),
        );
    }

    #[test]
    fn auth_dot_json_set_provider_api_key_preserves_existing_provider_oauth() {
        let mut auth = AuthDotJson::default();
        let oauth = ProviderOauthCredential {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };

        auth.set_provider_oauth_credential(PROVIDER_GEMINI, oauth.clone());
        auth.set_provider_api_key(PROVIDER_GEMINI, "AIza-1");

        assert_eq!(auth.get_provider_api_key(PROVIDER_GEMINI), Some("AIza-1"));
        assert_eq!(
            auth.get_provider_oauth_credential(PROVIDER_GEMINI),
            Some(oauth),
        );
    }

    #[test]
    fn auth_dot_json_clear_provider_oauth_credential_removes_oauth_only() {
        let mut auth = AuthDotJson::default();
        let oauth = ProviderOauthCredential {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };
        auth.set_provider_oauth_credential(PROVIDER_GEMINI, oauth);

        let cleared = auth.clear_provider_oauth_credential(PROVIDER_GEMINI);

        assert!(cleared);
        assert_eq!(auth.get_provider_oauth_credential(PROVIDER_GEMINI), None);
        assert_eq!(auth.get_provider_api_key(PROVIDER_GEMINI), None);
    }

    #[test]
    fn auth_dot_json_clear_provider_oauth_credential_preserves_api_key() {
        let mut auth = AuthDotJson::default();
        let oauth = ProviderOauthCredential {
            access: "access-token".to_string(),
            refresh: "refresh-token".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };
        auth.set_provider_api_key(PROVIDER_GEMINI, "AIza-test");
        auth.set_provider_oauth_credential(PROVIDER_GEMINI, oauth);

        let cleared = auth.clear_provider_oauth_credential(PROVIDER_GEMINI);

        assert!(cleared);
        assert_eq!(
            auth.get_provider_api_key(PROVIDER_GEMINI),
            Some("AIza-test")
        );
        assert_eq!(auth.get_provider_oauth_credential(PROVIDER_GEMINI), None);
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
    fn resolve_gemini_auth_source_prefers_api_key_over_oauth() {
        let dir = tempdir().unwrap();
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");
        let oauth_credential = ProviderOauthCredential {
            access: "oauth-access".to_string(),
            refresh: "oauth-refresh".to_string(),
            expires: None,
            email: None,
            project_id: None,
            managed_project_id: None,
        };

        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            oauth_credential,
            AuthCredentialsStoreMode::File,
        )
        .expect("should store oauth credential");

        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "AIza-env");
        let source = resolve_gemini_auth_source(dir.path(), AuthCredentialsStoreMode::File);
        assert_eq!(source, GeminiAuthSource::ApiKey("AIza-env".to_string()));
    }

    #[test]
    #[serial(codex_api_key)]
    fn resolve_gemini_auth_source_uses_oauth_when_api_key_missing() {
        let dir = tempdir().unwrap();
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");
        let oauth_credential = ProviderOauthCredential {
            access: "oauth-access".to_string(),
            refresh: "oauth-refresh".to_string(),
            expires: None,
            email: Some("user@example.com".to_string()),
            project_id: Some("project-id".to_string()),
            managed_project_id: Some("managed-project-id".to_string()),
        };

        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            oauth_credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("should store oauth credential");

        let source = resolve_gemini_auth_source(dir.path(), AuthCredentialsStoreMode::File);
        assert_eq!(source, GeminiAuthSource::Oauth(oauth_credential));
    }

    #[test]
    #[serial(codex_api_key)]
    fn login_with_provider_oauth_preserves_existing_provider_api_key() {
        let dir = tempdir().unwrap();
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_GEMINI,
            "AIza-stored",
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini api key");
        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            ProviderOauthCredential {
                access: "oauth-access".to_string(),
                refresh: "oauth-refresh".to_string(),
                expires: None,
                email: Some("user@example.com".to_string()),
                project_id: None,
                managed_project_id: Some("managed-project".to_string()),
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini oauth");

        assert_eq!(
            get_provider_api_key(dir.path(), PROVIDER_GEMINI, AuthCredentialsStoreMode::File),
            Some("AIza-stored".to_string())
        );
        assert!(
            get_provider_oauth_credential(
                dir.path(),
                PROVIDER_GEMINI,
                AuthCredentialsStoreMode::File
            )
            .is_some()
        );
    }

    #[test]
    #[serial(codex_api_key)]
    fn login_with_provider_api_key_preserves_existing_provider_oauth() {
        let dir = tempdir().unwrap();
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            ProviderOauthCredential {
                access: "oauth-access".to_string(),
                refresh: "oauth-refresh".to_string(),
                expires: None,
                email: Some("user@example.com".to_string()),
                project_id: None,
                managed_project_id: Some("managed-project".to_string()),
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini oauth");
        login_with_provider_api_key(
            dir.path(),
            PROVIDER_GEMINI,
            "AIza-stored",
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini api key");

        assert_eq!(
            get_provider_api_key(dir.path(), PROVIDER_GEMINI, AuthCredentialsStoreMode::File),
            Some("AIza-stored".to_string())
        );
        let oauth = get_provider_oauth_credential(
            dir.path(),
            PROVIDER_GEMINI,
            AuthCredentialsStoreMode::File,
        );
        assert!(oauth.is_some());
        assert_eq!(oauth.unwrap().refresh, "oauth-refresh");
    }

    #[test]
    #[serial(codex_api_key)]
    fn resolve_gemini_auth_source_missing_when_unconfigured() {
        let dir = tempdir().unwrap();
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        let source = resolve_gemini_auth_source(dir.path(), AuthCredentialsStoreMode::File);
        assert_eq!(source, GeminiAuthSource::Missing);
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
        assert_eq!(openai.unwrap().method, ProviderAuthMethod::ApiKey);

        let anthropic = providers
            .iter()
            .find(|p| p.provider_id == PROVIDER_ANTHROPIC);
        assert!(anthropic.is_some());
        assert_eq!(anthropic.unwrap().source, ProviderAuthSource::Stored);
        assert_eq!(anthropic.unwrap().method, ProviderAuthMethod::ApiKey);
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
        assert_eq!(anthropic.unwrap().method, ProviderAuthMethod::ApiKey);
    }

    #[test]
    #[serial(codex_api_key)]
    fn list_configured_providers_shows_oauth_method() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            ProviderOauthCredential {
                access: "oauth-access".to_string(),
                refresh: "oauth-refresh".to_string(),
                expires: None,
                email: Some("user@example.com".to_string()),
                project_id: None,
                managed_project_id: Some("managed-project".to_string()),
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini oauth");

        let providers = list_configured_providers(dir.path(), AuthCredentialsStoreMode::File);
        let gemini = providers.iter().find(|p| p.provider_id == PROVIDER_GEMINI);
        assert!(gemini.is_some());
        assert_eq!(gemini.unwrap().source, ProviderAuthSource::Stored);
        assert_eq!(gemini.unwrap().method, ProviderAuthMethod::Oauth);
    }

    #[test]
    #[serial(codex_api_key)]
    fn list_configured_providers_shows_api_and_oauth_method() {
        let dir = tempdir().unwrap();
        let _openai_guard = EnvVarGuard::set(OPENAI_API_KEY_ENV_VAR, "");
        let _anthropic_guard = EnvVarGuard::set(ANTHROPIC_API_KEY_ENV_VAR, "");
        let _google_guard = EnvVarGuard::set(GOOGLE_API_KEY_ENV_VAR, "");

        login_with_provider_api_key(
            dir.path(),
            PROVIDER_GEMINI,
            "AIza-1",
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini api key");
        login_with_provider_oauth(
            dir.path(),
            PROVIDER_GEMINI,
            ProviderOauthCredential {
                access: "oauth-access".to_string(),
                refresh: "oauth-refresh".to_string(),
                expires: None,
                email: Some("user@example.com".to_string()),
                project_id: None,
                managed_project_id: Some("managed-project".to_string()),
            },
            AuthCredentialsStoreMode::File,
        )
        .expect("store gemini oauth");

        let providers = list_configured_providers(dir.path(), AuthCredentialsStoreMode::File);
        let gemini = providers.iter().find(|p| p.provider_id == PROVIDER_GEMINI);
        assert!(gemini.is_some());
        assert_eq!(gemini.unwrap().source, ProviderAuthSource::Stored);
        assert_eq!(gemini.unwrap().method, ProviderAuthMethod::ApiKeyAndOauth);
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
