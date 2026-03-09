use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use codex_app_server_protocol::AuthMode as ApiAuthMode;

use crate::auth::OPENAI_API_KEY_ENV_VAR;
use crate::auth::storage::AUTH_JSON_VERSION;
use crate::auth::storage::AuthDotJson;

/// Provider ID constants for well-known providers.
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_ANTHROPIC: &str = "anthropic";
pub const PROVIDER_GEMINI: &str = "gemini";
pub const ANTHROPIC_API_KEY_ENV_VAR: &str = "ANTHROPIC_API_KEY";
pub const GOOGLE_API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";

/// OAuth credential payload stored for provider-based login.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ProviderOauthCredential {
    pub access: String,
    pub refresh: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_project_id: Option<String>,
}

/// Credential types that can be stored for a provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderCredential {
    Api { key: String },
    Oauth {
        #[serde(flatten)]
        credential: ProviderOauthCredential,
    },
    ApiAndOauth {
        key: String,
        #[serde(flatten)]
        credential: ProviderOauthCredential,
    },
}

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

impl AuthDotJson {
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

    pub fn get_provider_api_key(&self, provider_id: &str) -> Option<&str> {
        self.providers.get(provider_id).and_then(|cred| match cred {
            ProviderCredential::Api { key } => Some(key.as_str()),
            ProviderCredential::Oauth { .. } => None,
            ProviderCredential::ApiAndOauth { key, .. } => Some(key.as_str()),
        })
    }

    pub fn get_provider_oauth_credential(
        &self,
        provider_id: &str,
    ) -> Option<ProviderOauthCredential> {
        self.providers.get(provider_id).and_then(|cred| match cred {
            ProviderCredential::Api { .. } => None,
            ProviderCredential::Oauth { credential } => Some(credential.clone()),
            ProviderCredential::ApiAndOauth { credential, .. } => Some(credential.clone()),
        })
    }

    pub fn has_any_provider_api_key(&self) -> bool {
        self.providers.values().any(|cred| {
            matches!(
                cred,
                ProviderCredential::Api { .. } | ProviderCredential::ApiAndOauth { .. }
            )
        })
    }

    pub fn has_any_provider_oauth_credential(&self) -> bool {
        self.providers.values().any(|cred| {
            matches!(
                cred,
                ProviderCredential::Oauth { .. } | ProviderCredential::ApiAndOauth { .. }
            )
        })
    }

    pub fn set_provider_credential(&mut self, provider_id: &str, credential: ProviderCredential) {
        let merged = match (self.providers.get(provider_id), credential) {
            (
                Some(ProviderCredential::Oauth {
                    credential: existing_oauth,
                }),
                ProviderCredential::Api { key },
            ) => ProviderCredential::ApiAndOauth {
                key,
                credential: existing_oauth.clone(),
            },
            (
                Some(ProviderCredential::ApiAndOauth {
                    credential: existing_oauth,
                    ..
                }),
                ProviderCredential::Api { key },
            ) => ProviderCredential::ApiAndOauth {
                key,
                credential: existing_oauth.clone(),
            },
            (
                Some(ProviderCredential::Api { key }),
                ProviderCredential::Oauth {
                    credential: incoming_oauth,
                },
            ) => ProviderCredential::ApiAndOauth {
                key: key.clone(),
                credential: incoming_oauth,
            },
            (
                Some(ProviderCredential::ApiAndOauth {
                    key: existing_key, ..
                }),
                ProviderCredential::Oauth {
                    credential: incoming_oauth,
                },
            ) => ProviderCredential::ApiAndOauth {
                key: existing_key.clone(),
                credential: incoming_oauth,
            },
            (_, incoming) => incoming,
        };

        if provider_id == PROVIDER_OPENAI {
            self.openai_api_key = match &merged {
                ProviderCredential::Api { key } => Some(key.clone()),
                ProviderCredential::Oauth { .. } => None,
                ProviderCredential::ApiAndOauth { key, .. } => Some(key.clone()),
            };
        }

        self.providers.insert(provider_id.to_string(), merged);
        self.version = Some(AUTH_JSON_VERSION);
    }

    pub fn set_provider_api_key(&mut self, provider_id: &str, api_key: &str) {
        self.set_provider_credential(
            provider_id,
            ProviderCredential::Api {
                key: api_key.to_string(),
            },
        );
    }

    pub fn set_provider_oauth_credential(
        &mut self,
        provider_id: &str,
        credential: ProviderOauthCredential,
    ) {
        self.set_provider_credential(provider_id, ProviderCredential::Oauth { credential });
    }

    pub fn clear_provider_oauth_credential(&mut self, provider_id: &str) -> bool {
        let Some(existing) = self.providers.get(provider_id).cloned() else {
            return false;
        };

        let changed = match existing {
            ProviderCredential::Api { .. } => false,
            ProviderCredential::Oauth { .. } => {
                self.providers.remove(provider_id);
                true
            }
            ProviderCredential::ApiAndOauth { key, .. } => {
                self.providers
                    .insert(provider_id.to_string(), ProviderCredential::Api { key });
                true
            }
        };

        if changed {
            if provider_id == PROVIDER_OPENAI {
                self.openai_api_key = self
                    .get_provider_api_key(PROVIDER_OPENAI)
                    .map(std::string::ToString::to_string);
            }
            self.version = Some(AUTH_JSON_VERSION);
        }

        changed
    }

    pub fn remove_provider(&mut self, provider_id: &str) -> bool {
        let removed = self.providers.remove(provider_id).is_some();
        if provider_id == PROVIDER_OPENAI {
            self.openai_api_key = None;
        }
        removed
    }

    pub fn configured_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}
