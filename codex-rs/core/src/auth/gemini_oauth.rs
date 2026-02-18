use std::path::Path;
use std::time::Duration as StdDuration;

use chrono::Duration;
use chrono::Utc;
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;

use super::AuthCredentialsStoreMode;
use super::PROVIDER_GEMINI;
use super::ProviderOauthCredential;
use super::clear_provider_oauth_credential;
use super::get_provider_oauth_credential;
use super::login_with_provider_oauth;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::util::redact_error_body;

pub const GEMINI_OAUTH_CLIENT_ID_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_CLIENT_ID";
pub const GEMINI_OAUTH_CLIENT_SECRET_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_CLIENT_SECRET";
pub const GEMINI_OAUTH_TOKEN_URL_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_TOKEN_URL";
const GEMINI_CODE_ASSIST_BASE_URL_ENV_VAR: &str = "CODEX_GEMINI_CODE_ASSIST_BASE_URL";
const GEMINI_CODE_ASSIST_API_VERSION_ENV_VAR: &str = "CODEX_GEMINI_CODE_ASSIST_API_VERSION";
const GEMINI_PROJECT_ENV_VAR: &str = "GOOGLE_CLOUD_PROJECT";
const GEMINI_PROJECT_ID_ENV_VAR: &str = "GOOGLE_CLOUD_PROJECT_ID";

pub const DEFAULT_GEMINI_OAUTH_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
pub const DEFAULT_GEMINI_OAUTH_CLIENT_SECRET: &str = "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";
pub const DEFAULT_GEMINI_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DEFAULT_GEMINI_CODE_ASSIST_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
const DEFAULT_GEMINI_CODE_ASSIST_API_VERSION: &str = "v1internal";

const REFRESH_SKEW_SECONDS: i64 = 300;
const ONBOARD_POLL_INITIAL_INTERVAL: StdDuration = StdDuration::from_secs(1);
const ONBOARD_POLL_MAX_INTERVAL: StdDuration = StdDuration::from_secs(8);
const ONBOARD_POLL_TIMEOUT: StdDuration = StdDuration::from_secs(60);

static GEMINI_OAUTH_REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static GEMINI_OAUTH_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(build_reqwest_client);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeminiOauthRuntimeContext {
    pub access_token: String,
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorResponse {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientMetadata {
    ide_type: &'static str,
    platform: &'static str,
    plugin_type: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duet_project: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadCodeAssistRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloudaicompanion_project: Option<String>,
    metadata: ClientMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadCodeAssistResponse {
    #[serde(default)]
    current_tier: Option<GeminiTier>,
    #[serde(default)]
    allowed_tiers: Option<Vec<GeminiTier>>,
    #[serde(default)]
    ineligible_tiers: Option<Vec<IneligibleTier>>,
    #[serde(default)]
    cloudaicompanion_project: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTier {
    id: String,
    #[serde(default)]
    is_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IneligibleTier {
    #[serde(default)]
    reason_message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OnboardUserRequest {
    tier_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloudaicompanion_project: Option<String>,
    metadata: ClientMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LongRunningOperationResponse {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    done: Option<bool>,
    #[serde(default)]
    response: Option<OnboardUserResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardUserResponse {
    #[serde(default)]
    cloudaicompanion_project: Option<OnboardedProject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardedProject {
    #[serde(default)]
    id: Option<String>,
}

pub(crate) fn code_assist_base_url() -> String {
    env_string_or_default(
        GEMINI_CODE_ASSIST_BASE_URL_ENV_VAR,
        DEFAULT_GEMINI_CODE_ASSIST_BASE_URL,
    )
}

fn code_assist_api_version() -> String {
    env_string_or_default(
        GEMINI_CODE_ASSIST_API_VERSION_ENV_VAR,
        DEFAULT_GEMINI_CODE_ASSIST_API_VERSION,
    )
}

pub fn env_string_or_default(env_var: &str, default_value: &str) -> String {
    std::env::var(env_var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_value.to_string())
}

fn gemini_oauth_http_client() -> &'static reqwest::Client {
    &GEMINI_OAUTH_HTTP_CLIENT
}

pub(crate) fn code_assist_method_url(method: &str) -> String {
    let base_url = code_assist_base_url();
    let version = code_assist_api_version();
    format!("{base_url}/{version}:{method}")
}

fn code_assist_operation_url(operation_name: &str) -> String {
    let operation_name = operation_name.trim_start_matches('/');
    let base_url = code_assist_base_url();
    let version = code_assist_api_version();
    format!("{base_url}/{version}/{operation_name}")
}

pub(crate) async fn ensure_gemini_oauth_context(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    initial_credential: ProviderOauthCredential,
) -> Result<GeminiOauthRuntimeContext> {
    ensure_gemini_oauth_context_with_refresh(
        codex_home,
        auth_credentials_store_mode,
        initial_credential,
        false,
    )
    .await
}

pub(crate) async fn force_refresh_gemini_oauth_context(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    initial_credential: ProviderOauthCredential,
) -> Result<GeminiOauthRuntimeContext> {
    ensure_gemini_oauth_context_with_refresh(
        codex_home,
        auth_credentials_store_mode,
        initial_credential,
        true,
    )
    .await
}

async fn ensure_gemini_oauth_context_with_refresh(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    initial_credential: ProviderOauthCredential,
    force_refresh: bool,
) -> Result<GeminiOauthRuntimeContext> {
    let mut credential =
        get_provider_oauth_credential(codex_home, PROVIDER_GEMINI, auth_credentials_store_mode)
            .unwrap_or(initial_credential);

    if force_refresh || should_refresh_credential(&credential) {
        let _refresh_guard = GEMINI_OAUTH_REFRESH_LOCK.lock().await;
        credential =
            get_provider_oauth_credential(codex_home, PROVIDER_GEMINI, auth_credentials_store_mode)
                .unwrap_or_else(|| credential.clone());

        if force_refresh || should_refresh_credential(&credential) {
            credential =
                refresh_access_token(codex_home, auth_credentials_store_mode, credential).await?;
            login_with_provider_oauth(
                codex_home,
                PROVIDER_GEMINI,
                credential.clone(),
                auth_credentials_store_mode,
            )
            .map_err(|err| {
                CodexErr::Api(format!(
                    "Failed to persist Gemini OAuth credential updates: {err}"
                ))
            })?;
        }
    }

    let effective_project_id = ensure_project_context(&credential).await?;
    let configured_project_id = configured_project_id_from_env();
    let should_persist_managed_project = credential.project_id.is_none()
        && configured_project_id.as_deref() != Some(effective_project_id.as_str())
        && credential.managed_project_id.as_deref() != Some(effective_project_id.as_str());
    if should_persist_managed_project {
        credential.managed_project_id = Some(effective_project_id.clone());
        login_with_provider_oauth(
            codex_home,
            PROVIDER_GEMINI,
            credential.clone(),
            auth_credentials_store_mode,
        )
        .map_err(|err| {
            CodexErr::Api(format!(
                "Failed to persist Gemini OAuth credential updates: {err}"
            ))
        })?;
    }

    Ok(GeminiOauthRuntimeContext {
        access_token: credential.access,
        project_id: effective_project_id,
    })
}

fn should_refresh_credential(credential: &ProviderOauthCredential) -> bool {
    match credential.expires {
        Some(expires_at) => {
            let refresh_after = Utc::now() + Duration::seconds(REFRESH_SKEW_SECONDS);
            expires_at <= refresh_after
        }
        None => true,
    }
}

async fn refresh_access_token(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    credential: ProviderOauthCredential,
) -> Result<ProviderOauthCredential> {
    let token_url = env_string_or_default(
        GEMINI_OAUTH_TOKEN_URL_ENV_VAR,
        DEFAULT_GEMINI_OAUTH_TOKEN_URL,
    );
    let client_id = env_string_or_default(
        GEMINI_OAUTH_CLIENT_ID_ENV_VAR,
        DEFAULT_GEMINI_OAUTH_CLIENT_ID,
    );
    let client_secret = env_string_or_default(
        GEMINI_OAUTH_CLIENT_SECRET_ENV_VAR,
        DEFAULT_GEMINI_OAUTH_CLIENT_SECRET,
    );

    let response = gemini_oauth_http_client()
        .post(&token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=refresh_token&client_id={client_id}&client_secret={client_secret}&refresh_token={refresh_token}",
            client_id = urlencoding::encode(&client_id),
            client_secret = urlencoding::encode(&client_secret),
            refresh_token = urlencoding::encode(&credential.refresh)
        ))
        .send()
        .await
        .map_err(|err| CodexErr::Api(format!("Gemini OAuth token refresh request failed: {err}")))?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        CodexErr::Api(format!("Failed to read Gemini OAuth token response: {err}"))
    })?;

    if !status.is_success() {
        if is_invalid_grant_response(status, &body) {
            let _ = clear_provider_oauth_credential(
                codex_home,
                PROVIDER_GEMINI,
                auth_credentials_store_mode,
            );
            return Err(CodexErr::Api(
                "Gemini OAuth session expired or revoked. Run `ata login --provider gemini --with-oauth` again."
                    .to_string(),
            ));
        }

        let redacted_body = redact_error_body(&body);
        return Err(CodexErr::Api(format!(
            "Gemini OAuth token refresh failed ({status}): {redacted_body}"
        )));
    }

    let refresh_response: RefreshTokenResponse = serde_json::from_str(&body).map_err(|err| {
        CodexErr::Api(format!(
            "Failed to parse Gemini OAuth token refresh response JSON: {err}"
        ))
    })?;

    let mut refreshed = credential;
    refreshed.access = refresh_response.access_token;
    if let Some(rotated_refresh_token) = refresh_response
        .refresh_token
        .filter(|token| !token.trim().is_empty())
    {
        refreshed.refresh = rotated_refresh_token;
    }
    refreshed.expires = refresh_response
        .expires_in
        .map(|seconds| Utc::now() + Duration::seconds(seconds));
    Ok(refreshed)
}

fn is_invalid_grant_response(status: StatusCode, response_body: &str) -> bool {
    if status != StatusCode::BAD_REQUEST && status != StatusCode::UNAUTHORIZED {
        return false;
    }

    if response_body.contains("invalid_grant") {
        return true;
    }

    serde_json::from_str::<OAuthErrorResponse>(response_body)
        .ok()
        .is_some_and(|error| {
            error.error.as_deref() == Some("invalid_grant")
                || error
                    .error_description
                    .as_deref()
                    .is_some_and(|description| description.contains("invalid_grant"))
        })
}

async fn ensure_project_context(credential: &ProviderOauthCredential) -> Result<String> {
    if let Some(project_id) = credential.project_id.as_ref() {
        return Ok(project_id.clone());
    }
    if let Some(configured_project_id) = configured_project_id_from_env() {
        return Ok(configured_project_id);
    }
    if let Some(managed_project_id) = credential.managed_project_id.as_ref() {
        return Ok(managed_project_id.clone());
    }

    resolve_project_via_code_assist(&credential.access).await
}

fn configured_project_id_from_env() -> Option<String> {
    for env_var in [GEMINI_PROJECT_ENV_VAR, GEMINI_PROJECT_ID_ENV_VAR] {
        if let Ok(value) = std::env::var(env_var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn build_client_metadata(configured_project_id: Option<&str>) -> ClientMetadata {
    ClientMetadata {
        ide_type: "IDE_UNSPECIFIED",
        platform: "PLATFORM_UNSPECIFIED",
        plugin_type: "GEMINI",
        duet_project: configured_project_id.map(std::string::ToString::to_string),
    }
}

fn select_onboarding_tier_id(load_response: &LoadCodeAssistResponse) -> String {
    if let Some(current_tier) = load_response.current_tier.as_ref()
        && !current_tier.id.trim().is_empty()
    {
        return current_tier.id.clone();
    }

    if let Some(default_tier) = load_response
        .allowed_tiers
        .as_ref()
        .and_then(|tiers| tiers.iter().find(|tier| tier.is_default))
        && !default_tier.id.trim().is_empty()
    {
        return default_tier.id.clone();
    }

    "legacy-tier".to_string()
}

fn ineligible_tier_messages(load_response: &LoadCodeAssistResponse) -> Vec<String> {
    load_response
        .ineligible_tiers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|tier| tier.reason_message.clone())
        .filter(|message| !message.trim().is_empty())
        .collect()
}

async fn resolve_project_via_code_assist(access_token: &str) -> Result<String> {
    let configured_project_id = configured_project_id_from_env();
    let metadata = build_client_metadata(configured_project_id.as_deref());
    let load_request = LoadCodeAssistRequest {
        cloudaicompanion_project: configured_project_id.clone(),
        metadata,
    };

    let load_response: LoadCodeAssistResponse = gemini_oauth_http_client()
        .post(code_assist_method_url("loadCodeAssist"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&load_request)
        .send()
        .await
        .map_err(|err| {
            CodexErr::Api(format!(
                "Gemini Code Assist loadCodeAssist request failed: {err}"
            ))
        })?
        .error_for_status()
        .map_err(|err| CodexErr::Api(format!("Gemini Code Assist loadCodeAssist failed: {err}")))?
        .json()
        .await
        .map_err(|err| {
            CodexErr::Api(format!(
                "Failed to parse Gemini Code Assist loadCodeAssist response: {err}"
            ))
        })?;

    if let Some(project_id) = load_response
        .cloudaicompanion_project
        .as_ref()
        .map(|project_id| project_id.trim())
        .filter(|project_id| !project_id.is_empty())
    {
        return Ok(project_id.to_string());
    }

    if let Some(configured_project_id) = configured_project_id.as_ref()
        && load_response.current_tier.is_some()
    {
        return Ok(configured_project_id.clone());
    }

    let tier_id = select_onboarding_tier_id(&load_response);
    let metadata = build_client_metadata(configured_project_id.as_deref());
    let onboard_request = OnboardUserRequest {
        tier_id: tier_id.clone(),
        cloudaicompanion_project: if tier_id == "free-tier" {
            None
        } else {
            configured_project_id.clone()
        },
        metadata,
    };

    let mut operation_response: LongRunningOperationResponse = gemini_oauth_http_client()
        .post(code_assist_method_url("onboardUser"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .json(&onboard_request)
        .send()
        .await
        .map_err(|err| {
            CodexErr::Api(format!(
                "Gemini Code Assist onboardUser request failed: {err}"
            ))
        })?
        .error_for_status()
        .map_err(|err| CodexErr::Api(format!("Gemini Code Assist onboardUser failed: {err}")))?
        .json()
        .await
        .map_err(|err| CodexErr::Api(format!("Failed to parse onboardUser response: {err}")))?;

    let poll_started = tokio::time::Instant::now();
    let mut poll_interval = ONBOARD_POLL_INITIAL_INTERVAL;
    let mut poll_timed_out = false;
    let mut attempts = 0;
    while !operation_response.done.unwrap_or(false)
        && operation_response
            .name
            .as_ref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        let elapsed = poll_started.elapsed();
        if elapsed >= ONBOARD_POLL_TIMEOUT {
            poll_timed_out = true;
            break;
        }

        attempts += 1;
        let remaining = ONBOARD_POLL_TIMEOUT.saturating_sub(elapsed);
        let delay = std::cmp::min(poll_interval, remaining);
        tracing::info!(
            attempt = attempts,
            delay_secs = delay.as_secs(),
            remaining_secs = remaining.as_secs(),
            "Polling Gemini Code Assist onboarding operation"
        );
        tokio::time::sleep(delay).await;
        let operation_name = operation_response.name.clone().unwrap_or_default();
        operation_response = gemini_oauth_http_client()
            .get(code_assist_operation_url(&operation_name))
            .header("Authorization", format!("Bearer {access_token}"))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|err| {
                CodexErr::Api(format!("Gemini Code Assist operation poll failed: {err}"))
            })?
            .error_for_status()
            .map_err(|err| CodexErr::Api(format!("Gemini Code Assist operation failed: {err}")))?
            .json()
            .await
            .map_err(|err| CodexErr::Api(format!("Failed to parse operation response: {err}")))?;
        poll_interval = std::cmp::min(poll_interval.saturating_mul(2), ONBOARD_POLL_MAX_INTERVAL);
    }

    if poll_timed_out {
        return Err(CodexErr::Api(
            "Gemini Code Assist onboarding timed out while waiting for project setup. Retry in a minute, or set GOOGLE_CLOUD_PROJECT / GOOGLE_CLOUD_PROJECT_ID explicitly."
                .to_string(),
        ));
    }

    if let Some(onboarded_project_id) = operation_response
        .response
        .as_ref()
        .and_then(|response| response.cloudaicompanion_project.as_ref())
        .and_then(|project| project.id.as_ref())
        .map(|project_id| project_id.trim())
        .filter(|project_id| !project_id.is_empty())
    {
        return Ok(onboarded_project_id.to_string());
    }

    if let Some(configured_project_id) = configured_project_id {
        return Ok(configured_project_id);
    }

    let ineligible_messages = ineligible_tier_messages(&load_response);
    if !ineligible_messages.is_empty() {
        return Err(CodexErr::Api(format!(
            "Gemini Code Assist project resolution failed: {}",
            ineligible_messages.join("; ")
        )));
    }

    Err(CodexErr::Api(
        "Gemini OAuth could not resolve a Google Cloud project. Set GOOGLE_CLOUD_PROJECT (or GOOGLE_CLOUD_PROJECT_ID), or complete Gemini Code Assist onboarding."
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::get_provider_api_key;
    use crate::auth::login_with_provider_api_key;
    use crate::auth::login_with_provider_oauth;
    use crate::auth::test_utils::EnvVarGuard;
    use pretty_assertions::assert_eq;
    use serial_test::serial;
    use tempfile::tempdir;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[test]
    fn select_onboarding_tier_prefers_current_tier() {
        let response = LoadCodeAssistResponse {
            current_tier: Some(GeminiTier {
                id: "standard-tier".to_string(),
                is_default: false,
            }),
            allowed_tiers: Some(vec![GeminiTier {
                id: "free-tier".to_string(),
                is_default: true,
            }]),
            ineligible_tiers: None,
            cloudaicompanion_project: None,
        };
        assert_eq!(select_onboarding_tier_id(&response), "standard-tier");
    }

    #[test]
    fn select_onboarding_tier_uses_default_allowed_tier() {
        let response = LoadCodeAssistResponse {
            current_tier: None,
            allowed_tiers: Some(vec![
                GeminiTier {
                    id: "legacy-tier".to_string(),
                    is_default: false,
                },
                GeminiTier {
                    id: "free-tier".to_string(),
                    is_default: true,
                },
            ]),
            ineligible_tiers: None,
            cloudaicompanion_project: None,
        };
        assert_eq!(select_onboarding_tier_id(&response), "free-tier");
    }

    #[tokio::test]
    #[serial(gemini_oauth_env)]
    async fn refresh_updates_access_and_rotated_refresh_token() {
        let codex_home = tempdir().expect("tempdir");
        let token_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
            ))
            .mount(&token_server)
            .await;

        let _token_url_guard = EnvVarGuard::set(
            GEMINI_OAUTH_TOKEN_URL_ENV_VAR,
            &format!("{}/token", token_server.uri()),
        );
        let credential = ProviderOauthCredential {
            access: "old-access".to_string(),
            refresh: "old-refresh".to_string(),
            expires: Some(Utc::now() - Duration::seconds(30)),
            email: None,
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };
        login_with_provider_oauth(
            codex_home.path(),
            PROVIDER_GEMINI,
            credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("store oauth");

        let context = ensure_gemini_oauth_context(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            credential,
        )
        .await
        .expect("refresh should succeed");

        assert_eq!(context.access_token, "new-access");
        assert_eq!(context.project_id, "managed-project");
        let persisted = get_provider_oauth_credential(
            codex_home.path(),
            PROVIDER_GEMINI,
            AuthCredentialsStoreMode::File,
        )
        .expect("oauth credential should exist");
        assert_eq!(persisted.refresh, "new-refresh");
        assert_eq!(persisted.access, "new-access");
        assert!(persisted.expires.is_some());
    }

    #[tokio::test]
    #[serial(gemini_oauth_env)]
    async fn invalid_grant_clears_stored_oauth_credential() {
        let codex_home = tempdir().expect("tempdir");
        let _google_guard = EnvVarGuard::set(crate::auth::GOOGLE_API_KEY_ENV_VAR, "");
        let token_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"invalid_grant","error_description":"expired"}"#),
            )
            .mount(&token_server)
            .await;

        let _token_url_guard = EnvVarGuard::set(
            GEMINI_OAUTH_TOKEN_URL_ENV_VAR,
            &format!("{}/token", token_server.uri()),
        );
        let credential = ProviderOauthCredential {
            access: "old-access".to_string(),
            refresh: "old-refresh".to_string(),
            expires: Some(Utc::now() - Duration::seconds(30)),
            email: None,
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };
        login_with_provider_oauth(
            codex_home.path(),
            PROVIDER_GEMINI,
            credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("store oauth");

        let err = ensure_gemini_oauth_context(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            credential,
        )
        .await
        .expect_err("invalid grant should fail");
        assert!(
            err.to_string()
                .contains("Run `ata login --provider gemini --with-oauth` again"),
            "unexpected error: {err}"
        );

        assert_eq!(
            get_provider_oauth_credential(
                codex_home.path(),
                PROVIDER_GEMINI,
                AuthCredentialsStoreMode::File,
            ),
            None,
        );
    }

    #[tokio::test]
    #[serial(gemini_oauth_env)]
    async fn invalid_grant_keeps_stored_api_key() {
        let codex_home = tempdir().expect("tempdir");
        let _google_guard = EnvVarGuard::set(crate::auth::GOOGLE_API_KEY_ENV_VAR, "");
        let token_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string(r#"{"error":"invalid_grant","error_description":"expired"}"#),
            )
            .mount(&token_server)
            .await;

        let _token_url_guard = EnvVarGuard::set(
            GEMINI_OAUTH_TOKEN_URL_ENV_VAR,
            &format!("{}/token", token_server.uri()),
        );
        login_with_provider_api_key(
            codex_home.path(),
            PROVIDER_GEMINI,
            "AIza-test",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");

        let credential = ProviderOauthCredential {
            access: "old-access".to_string(),
            refresh: "old-refresh".to_string(),
            expires: Some(Utc::now() - Duration::seconds(30)),
            email: None,
            project_id: None,
            managed_project_id: Some("managed-project".to_string()),
        };
        login_with_provider_oauth(
            codex_home.path(),
            PROVIDER_GEMINI,
            credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("store oauth");

        let err = ensure_gemini_oauth_context(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            credential,
        )
        .await
        .expect_err("invalid grant should fail");
        assert!(
            err.to_string()
                .contains("Run `ata login --provider gemini --with-oauth` again"),
            "unexpected error: {err}"
        );

        assert_eq!(
            get_provider_api_key(
                codex_home.path(),
                PROVIDER_GEMINI,
                AuthCredentialsStoreMode::File,
            ),
            Some("AIza-test".to_string()),
        );
        assert_eq!(
            get_provider_oauth_credential(
                codex_home.path(),
                PROVIDER_GEMINI,
                AuthCredentialsStoreMode::File,
            ),
            None,
        );
    }

    #[tokio::test]
    #[serial(gemini_oauth_env)]
    async fn load_code_assist_project_is_persisted_as_managed_project() {
        let codex_home = tempdir().expect("tempdir");
        let code_assist_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{
                    "cloudaicompanionProject":"managed-from-load",
                    "currentTier":{"id":"free-tier","isDefault":true}
                }"#,
            ))
            .expect(1)
            .mount(&code_assist_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1internal:onboardUser"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&code_assist_server)
            .await;

        let _base_url_guard = EnvVarGuard::set(
            GEMINI_CODE_ASSIST_BASE_URL_ENV_VAR,
            &code_assist_server.uri(),
        );
        let _project_guard = EnvVarGuard::set(GEMINI_PROJECT_ENV_VAR, "");
        let _project_id_guard = EnvVarGuard::set(GEMINI_PROJECT_ID_ENV_VAR, "");

        let credential = ProviderOauthCredential {
            access: "existing-access".to_string(),
            refresh: "existing-refresh".to_string(),
            expires: Some(Utc::now() + Duration::hours(1)),
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: None,
        };
        login_with_provider_oauth(
            codex_home.path(),
            PROVIDER_GEMINI,
            credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("store oauth");

        let context = ensure_gemini_oauth_context(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            credential,
        )
        .await
        .expect("context should resolve");
        assert_eq!(context.project_id, "managed-from-load");

        let persisted = get_provider_oauth_credential(
            codex_home.path(),
            PROVIDER_GEMINI,
            AuthCredentialsStoreMode::File,
        )
        .expect("oauth credential should exist");
        assert_eq!(
            persisted.managed_project_id.as_deref(),
            Some("managed-from-load")
        );
    }

    #[tokio::test]
    #[serial(gemini_oauth_env)]
    async fn onboard_user_polling_resolves_and_persists_project() {
        let codex_home = tempdir().expect("tempdir");
        let code_assist_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{
                    "currentTier":{"id":"free-tier","isDefault":true},
                    "allowedTiers":[{"id":"free-tier","isDefault":true}]
                }"#,
            ))
            .expect(1)
            .mount(&code_assist_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1internal:onboardUser"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"name":"operations/op-1","done":false}"#),
            )
            .expect(1)
            .mount(&code_assist_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1internal/operations/op-1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{
                    "name":"operations/op-1",
                    "done":true,
                    "response":{"cloudaicompanionProject":{"id":"managed-from-onboard"}}
                }"#,
            ))
            .expect(1)
            .mount(&code_assist_server)
            .await;

        let _base_url_guard = EnvVarGuard::set(
            GEMINI_CODE_ASSIST_BASE_URL_ENV_VAR,
            &code_assist_server.uri(),
        );
        let _project_guard = EnvVarGuard::set(GEMINI_PROJECT_ENV_VAR, "");
        let _project_id_guard = EnvVarGuard::set(GEMINI_PROJECT_ID_ENV_VAR, "");

        let credential = ProviderOauthCredential {
            access: "existing-access".to_string(),
            refresh: "existing-refresh".to_string(),
            expires: Some(Utc::now() + Duration::hours(1)),
            email: Some("user@example.com".to_string()),
            project_id: None,
            managed_project_id: None,
        };
        login_with_provider_oauth(
            codex_home.path(),
            PROVIDER_GEMINI,
            credential.clone(),
            AuthCredentialsStoreMode::File,
        )
        .expect("store oauth");

        let context = ensure_gemini_oauth_context(
            codex_home.path(),
            AuthCredentialsStoreMode::File,
            credential,
        )
        .await
        .expect("context should resolve");
        assert_eq!(context.project_id, "managed-from-onboard");

        let persisted = get_provider_oauth_credential(
            codex_home.path(),
            PROVIDER_GEMINI,
            AuthCredentialsStoreMode::File,
        )
        .expect("oauth credential should exist");
        assert_eq!(
            persisted.managed_project_id.as_deref(),
            Some("managed-from-onboard")
        );
    }
}
