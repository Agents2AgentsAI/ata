use std::path::Path;
use std::time::Duration as StdDuration;

use chrono::DateTime;
use chrono::Utc;
use once_cell::sync::Lazy;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::AuthCredentialsStoreMode;
use super::PROVIDER_COPILOT;
use super::ProviderOauthCredential;
use super::clear_provider_oauth_credential;
use super::get_provider_oauth_credential;
use super::login_with_provider_oauth;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::util::redact_error_body;

/// Public OAuth Client ID used by the GitHub Copilot CLI / VS Code extensions.
/// Reused here to satisfy GitHub's Copilot endpoints, which only accept
/// approved Copilot client IDs.
pub const COPILOT_OAUTH_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// OAuth scope requested for the device flow.
pub const COPILOT_OAUTH_SCOPE: &str = "read:user";

/// GitHub OAuth + Copilot endpoints.
pub const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
pub const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const COPILOT_TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Headers identifying us as the official Copilot Chat client. These are
/// required for both the token-exchange endpoint and Copilot API requests.
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
pub const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Refresh the Copilot access token this many seconds before its actual
/// expiry to avoid races with the server clock.
const REFRESH_SKEW_SECONDS: i64 = 300;

static COPILOT_OAUTH_REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static COPILOT_OAUTH_HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(build_reqwest_client);

/// Response from the device-code endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Serialize)]
struct DeviceCodeRequest<'a> {
    client_id: &'a str,
    scope: &'a str,
}

#[derive(Debug, Serialize)]
struct AccessTokenRequest<'a> {
    client_id: &'a str,
    device_code: &'a str,
    grant_type: &'a str,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    /// Unix seconds at which `token` expires.
    expires_at: i64,
}

/// Initiate a GitHub device-code authorization for Copilot.
pub async fn start_device_flow() -> Result<DeviceCodeResponse> {
    let body = DeviceCodeRequest {
        client_id: COPILOT_OAUTH_CLIENT_ID,
        scope: COPILOT_OAUTH_SCOPE,
    };

    let resp = COPILOT_OAUTH_HTTP_CLIENT
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("User-Agent", COPILOT_USER_AGENT)
        .json(&body)
        .send()
        .await
        .map_err(|e| CodexErr::Api(format!("Failed to request device code: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = redact_error_body(&resp.text().await.unwrap_or_default());
        return Err(CodexErr::Api(format!(
            "Device code request failed ({status}): {body}"
        )));
    }

    resp.json::<DeviceCodeResponse>()
        .await
        .map_err(|e| CodexErr::Api(format!("Invalid device code response: {e}")))
}

/// Poll the access-token endpoint until the user authorizes or the device
/// code expires. Returns the long-lived GitHub OAuth access token.
pub async fn poll_for_access_token(device: &DeviceCodeResponse) -> Result<String> {
    let interval = StdDuration::from_secs(device.interval.max(1));
    let deadline = if device.expires_in > 0 {
        Some(std::time::Instant::now() + StdDuration::from_secs(device.expires_in))
    } else {
        None
    };

    loop {
        if let Some(deadline) = deadline
            && std::time::Instant::now() >= deadline
        {
            return Err(CodexErr::Api("Device code expired".into()));
        }

        sleep(interval).await;

        let body = AccessTokenRequest {
            client_id: COPILOT_OAUTH_CLIENT_ID,
            device_code: &device.device_code,
            grant_type: "urn:ietf:params:oauth:grant-type:device_code",
        };

        let resp = COPILOT_OAUTH_HTTP_CLIENT
            .post(GITHUB_ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("User-Agent", COPILOT_USER_AGENT)
            .json(&body)
            .send()
            .await
            .map_err(|e| CodexErr::Api(format!("Failed to poll access token: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = redact_error_body(&resp.text().await.unwrap_or_default());
            return Err(CodexErr::Api(format!(
                "Access token poll failed ({status}): {body}"
            )));
        }

        let parsed: AccessTokenResponse = resp
            .json()
            .await
            .map_err(|e| CodexErr::Api(format!("Invalid access token response: {e}")))?;

        if let Some(token) = parsed.access_token {
            return Ok(token);
        }

        match parsed.error.as_deref() {
            Some("authorization_pending") | Some("slow_down") | None => continue,
            Some(err) => {
                let detail = parsed.error_description.unwrap_or_default();
                return Err(CodexErr::Api(format!(
                    "Authorization failed: {err}{}",
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(" — {detail}")
                    }
                )));
            }
        }
    }
}

/// Exchange a long-lived GitHub OAuth token for a short-lived Copilot token.
async fn exchange_for_copilot_token(github_oauth_token: &str) -> Result<(String, DateTime<Utc>)> {
    let resp = COPILOT_OAUTH_HTTP_CLIENT
        .get(COPILOT_TOKEN_EXCHANGE_URL)
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {github_oauth_token}"))
        .header("User-Agent", COPILOT_USER_AGENT)
        .header("Editor-Version", COPILOT_EDITOR_VERSION)
        .header("Editor-Plugin-Version", COPILOT_EDITOR_PLUGIN_VERSION)
        .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
        .send()
        .await
        .map_err(|e| CodexErr::Api(format!("Failed to exchange Copilot token: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = redact_error_body(&resp.text().await.unwrap_or_default());
        let hint = if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            " (does this account have an active GitHub Copilot subscription?)"
        } else {
            ""
        };
        return Err(CodexErr::Api(format!(
            "Copilot token exchange failed ({status}){hint}: {body}"
        )));
    }

    let parsed: CopilotTokenResponse = resp
        .json()
        .await
        .map_err(|e| CodexErr::Api(format!("Invalid Copilot token response: {e}")))?;

    let expires = DateTime::<Utc>::from_timestamp(parsed.expires_at, 0)
        .ok_or_else(|| CodexErr::Api("Copilot token expires_at out of range".into()))?;

    Ok((parsed.token, expires))
}

/// Persist a freshly-issued GitHub OAuth token + Copilot token to disk.
pub fn save_credentials(
    codex_home: &Path,
    store_mode: AuthCredentialsStoreMode,
    github_oauth_token: String,
    copilot_token: String,
    expires: DateTime<Utc>,
) -> Result<()> {
    let credential = ProviderOauthCredential {
        access: copilot_token,
        refresh: github_oauth_token,
        expires: Some(expires),
        email: None,
        project_id: None,
        managed_project_id: None,
    };

    login_with_provider_oauth(codex_home, PROVIDER_COPILOT, credential, store_mode)
        .map_err(|e| CodexErr::Api(format!("Failed to save Copilot credentials: {e}")))
}

/// Return a valid Copilot access token for outgoing API calls, refreshing it
/// from the long-lived GitHub OAuth token if it is expired or about to expire.
pub async fn get_or_refresh_copilot_token(
    codex_home: &Path,
    store_mode: AuthCredentialsStoreMode,
) -> Result<String> {
    let _guard = COPILOT_OAUTH_REFRESH_LOCK.lock().await;

    let credential = get_provider_oauth_credential(codex_home, PROVIDER_COPILOT, store_mode)
        .ok_or_else(|| {
            CodexErr::Api(
                "Not signed in to GitHub Copilot. Run `ata login` and choose GitHub Copilot.".into(),
            )
        })?;

    let needs_refresh = match credential.expires {
        Some(expires) => {
            let skew = chrono::Duration::seconds(REFRESH_SKEW_SECONDS);
            credential.access.is_empty() || (expires - skew) <= Utc::now()
        }
        None => true,
    };

    if !needs_refresh {
        return Ok(credential.access);
    }

    let (copilot_token, expires) = exchange_for_copilot_token(&credential.refresh).await?;

    save_credentials(
        codex_home,
        store_mode,
        credential.refresh.clone(),
        copilot_token.clone(),
        expires,
    )?;

    Ok(copilot_token)
}

/// Forget all stored Copilot credentials.
pub fn logout(codex_home: &Path, store_mode: AuthCredentialsStoreMode) -> Result<()> {
    clear_provider_oauth_credential(codex_home, PROVIDER_COPILOT, store_mode)
        .map_err(|e| CodexErr::Api(format!("Failed to clear Copilot credentials: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_is_well_known_copilot_id() {
        assert_eq!(COPILOT_OAUTH_CLIENT_ID, "Iv1.b507a08c87ecfe98");
    }

    #[test]
    fn endpoints_point_at_github() {
        assert!(GITHUB_DEVICE_CODE_URL.starts_with("https://github.com/"));
        assert!(GITHUB_ACCESS_TOKEN_URL.starts_with("https://github.com/"));
        assert!(COPILOT_TOKEN_EXCHANGE_URL.starts_with("https://api.github.com/"));
    }

    #[test]
    fn impersonation_headers_match_vscode() {
        assert!(COPILOT_USER_AGENT.starts_with("GitHubCopilotChat/"));
        assert!(COPILOT_EDITOR_VERSION.starts_with("vscode/"));
        assert_eq!(COPILOT_INTEGRATION_ID, "vscode-chat");
    }
}
