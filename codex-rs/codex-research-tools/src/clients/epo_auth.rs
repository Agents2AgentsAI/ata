use std::time::Duration;
use std::time::Instant;

use tokio::sync::Mutex;

use crate::error::ResearchError;
use crate::error::Result;

/// Manages OAuth2 client-credentials tokens for EPO OPS.
///
/// Tokens are lazily acquired on first use and cached until near expiry.
/// The auth endpoint is called directly via `reqwest` (bypassing the patent
/// rate limiter, since it is not a patent data request).
#[derive(Debug)]
pub(crate) struct EpoAuthManager {
    consumer_key: String,
    consumer_secret: String,
    auth_url: String,
    cached: Mutex<Option<CachedToken>>,
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Buffer before actual expiry to trigger a refresh (5 minutes).
const REFRESH_BUFFER: Duration = Duration::from_secs(5 * 60);

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    /// EPO returns `expires_in` as a JSON string (e.g. `"1199"`), not an integer.
    #[serde(deserialize_with = "deserialize_string_u64")]
    expires_in: u64,
}

fn deserialize_string_u64<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    match &value {
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| serde::de::Error::custom("expected u64")),
        serde_json::Value::String(s) => s
            .parse::<u64>()
            .map_err(|_| serde::de::Error::custom(format!("cannot parse \"{s}\" as u64"))),
        _ => Err(serde::de::Error::custom("expected number or string")),
    }
}

impl EpoAuthManager {
    pub(crate) fn new(consumer_key: String, consumer_secret: String, base_url: &str) -> Self {
        let auth_url = format!("{base_url}/auth/accesstoken");
        Self {
            consumer_key,
            consumer_secret,
            auth_url,
            cached: Mutex::new(None),
        }
    }

    /// Returns a valid bearer token, fetching or refreshing as needed.
    pub(crate) async fn get_token(&self, client: &reqwest::Client) -> Result<String> {
        {
            let guard = self.cached.lock().await;
            if let Some(cached) = guard.as_ref()
                && Instant::now() < cached.expires_at
            {
                return Ok(cached.access_token.clone());
            }
        }

        // Token missing or expired — fetch a new one.
        let token = self.fetch_token(client).await?;
        let access_token = token.access_token.clone();

        let expires_at = Instant::now()
            + Duration::from_secs(token.expires_in)
                .checked_sub(REFRESH_BUFFER)
                .unwrap_or(Duration::from_secs(0));

        let mut guard = self.cached.lock().await;
        *guard = Some(CachedToken {
            access_token: access_token.clone(),
            expires_at,
        });

        Ok(access_token)
    }

    async fn fetch_token(&self, client: &reqwest::Client) -> Result<TokenResponse> {
        let response = client
            .post(&self.auth_url)
            .basic_auth(&self.consumer_key, Some(&self.consumer_secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|err| ResearchError::Internal(format!("EPO auth request failed: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ResearchError::Internal(format!(
                "EPO auth returned {status}: {body}"
            )));
        }

        response.json::<TokenResponse>().await.map_err(|err| {
            ResearchError::Internal(format!("EPO auth response parse failed: {err}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    use super::*;

    #[tokio::test]
    async fn fetches_and_caches_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/accesstoken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "test-token-123",
                "token_type": "Bearer",
                "expires_in": 1200
            })))
            .expect(1)
            .mount(&server)
            .await;

        let auth = EpoAuthManager::new("key".to_string(), "secret".to_string(), &server.uri());
        let client = reqwest::Client::new();

        let token1 = auth.get_token(&client).await.expect("first token");
        assert_eq!(token1, "test-token-123");

        // Second call should use cache (mock expects exactly 1 call).
        let token2 = auth.get_token(&client).await.expect("cached token");
        assert_eq!(token2, "test-token-123");
    }

    #[tokio::test]
    async fn returns_error_on_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/auth/accesstoken"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let auth = EpoAuthManager::new(
            "bad-key".to_string(),
            "bad-secret".to_string(),
            &server.uri(),
        );
        let client = reqwest::Client::new();

        let err = auth.get_token(&client).await.expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("401"), "unexpected error: {msg}");
    }
}
