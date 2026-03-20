use chrono::Duration;
use chrono::Utc;

use super::client::SupabaseClient;
use super::error::SupabaseError;
use super::types::AuthTokenResponse;
use super::types::OtpRequest;
use super::types::RefreshTokenRequest;
use super::types::SupabaseSession;
use super::types::SupabaseUser;
use super::types::VerifyOtpRequest;

/// High-level authentication operations against Supabase Auth.
#[derive(Debug, Clone)]
pub struct SupabaseAuth {
    client: SupabaseClient,
}

impl SupabaseAuth {
    pub fn new(client: SupabaseClient) -> Self {
        Self { client }
    }

    /// Request a magic-link / OTP to be sent to the given email address.
    pub async fn sign_in_with_otp(&self, email: &str) -> Result<(), SupabaseError> {
        let url = self.client.auth_url("otp");
        let body = OtpRequest {
            email: email.to_string(),
        };
        let resp = self
            .client
            .http()
            .post(&url)
            .headers(self.client.default_headers())
            .json(&body)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Err(SupabaseError::api(status, text))
        }
    }

    /// Exchange an OTP code (from the user's email) for a session.
    pub async fn exchange_code_for_session(
        &self,
        email: &str,
        code: &str,
    ) -> Result<SupabaseSession, SupabaseError> {
        let url = self.client.auth_url("verify");
        let body = VerifyOtpRequest {
            email: email.to_string(),
            token: code.to_string(),
            otp_type: "email".to_string(),
        };
        let resp = self
            .client
            .http()
            .post(&url)
            .headers(self.client.default_headers())
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(SupabaseError::api(status, text));
        }

        let token_resp: AuthTokenResponse = resp.json().await?;
        Ok(token_response_to_session(token_resp))
    }

    /// Refresh a session using a refresh token.
    pub async fn refresh_session(
        &self,
        refresh_token: &str,
    ) -> Result<SupabaseSession, SupabaseError> {
        let url = self.client.auth_url("token?grant_type=refresh_token");
        let body = RefreshTokenRequest {
            refresh_token: refresh_token.to_string(),
        };
        let resp = self
            .client
            .http()
            .post(&url)
            .headers(self.client.default_headers())
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(SupabaseError::api(status, text));
        }

        let token_resp: AuthTokenResponse = resp.json().await?;
        Ok(token_response_to_session(token_resp))
    }

    /// Sign out the current session (invalidate the refresh token server-side).
    pub async fn sign_out(&self, access_token: &str) -> Result<(), SupabaseError> {
        let url = self.client.auth_url("logout");
        let resp = self
            .client
            .http()
            .post(&url)
            .headers(self.client.auth_headers(access_token))
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            Err(SupabaseError::api(status, text))
        }
    }

    /// Access the underlying `SupabaseClient`.
    pub fn client(&self) -> &SupabaseClient {
        &self.client
    }
}

/// Convert a raw token response into a `SupabaseSession`.
fn token_response_to_session(resp: AuthTokenResponse) -> SupabaseSession {
    let expires_at = Utc::now() + Duration::seconds(resp.expires_in);
    let display_name = resp.user.user_metadata.as_ref().and_then(|m| {
        m.get("full_name")
            .or_else(|| m.get("name"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string)
    });

    SupabaseSession {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at,
        user: SupabaseUser {
            id: resp.user.id,
            email: resp.user.email.unwrap_or_default(),
            display_name,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn token_response_conversion() {
        let resp = AuthTokenResponse {
            access_token: "access_123".to_string(),
            token_type: "bearer".to_string(),
            expires_in: 3600,
            refresh_token: "refresh_456".to_string(),
            user: super::super::types::AuthUserResponse {
                id: Uuid::nil(),
                email: Some("user@test.com".to_string()),
                user_metadata: Some(json!({"full_name": "Test User"})),
            },
        };

        let session = token_response_to_session(resp);
        assert_eq!(session.access_token, "access_123");
        assert_eq!(session.refresh_token, "refresh_456");
        assert_eq!(session.user.email, "user@test.com");
        assert_eq!(session.user.display_name.as_deref(), Some("Test User"));
        // expires_at should be roughly 1 hour from now
        let diff = session.expires_at - Utc::now();
        assert!(diff.num_seconds() > 3590 && diff.num_seconds() <= 3600);
    }

    #[test]
    fn token_response_no_metadata() {
        let resp = AuthTokenResponse {
            access_token: "a".to_string(),
            token_type: "bearer".to_string(),
            expires_in: 60,
            refresh_token: "r".to_string(),
            user: super::super::types::AuthUserResponse {
                id: Uuid::nil(),
                email: None,
                user_metadata: None,
            },
        };

        let session = token_response_to_session(resp);
        assert_eq!(session.user.email, "");
        assert!(session.user.display_name.is_none());
    }

    #[test]
    fn supabase_auth_construction() {
        let client = SupabaseClient::new("https://example.supabase.co", "key");
        let auth = SupabaseAuth::new(client);
        assert_eq!(auth.client().base_url(), "https://example.supabase.co");
    }
}
