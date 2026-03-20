use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

/// Supabase session returned after successful authentication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupabaseSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub user: SupabaseUser,
}

/// Supabase user profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupabaseUser {
    pub id: Uuid,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// Device registration record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceRegistration {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_name: String,
    pub device_type: String,
    #[serde(default)]
    pub platform_info: Option<serde_json::Value>,
    #[serde(default)]
    pub relay_endpoint: Option<String>,
    #[serde(default)]
    pub relay_token: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// User profile stored in the `profiles` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub id: Uuid,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub shared_keys_enabled: bool,
    #[serde(default)]
    pub invite_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Supabase auth token response (from `/auth/v1/token`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: String,
    pub user: AuthUserResponse,
}

/// User object within the Supabase auth token response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserResponse {
    pub id: Uuid,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub user_metadata: Option<serde_json::Value>,
}

/// Request body for OTP (magic-link) sign-in.
#[derive(Debug, Serialize)]
pub struct OtpRequest {
    pub email: String,
}

/// Request body for exchanging an OTP code for a session.
#[derive(Debug, Serialize)]
pub struct VerifyOtpRequest {
    pub email: String,
    pub token: String,
    #[serde(rename = "type")]
    pub otp_type: String,
}

/// Request body for refreshing a session.
#[derive(Debug, Serialize)]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn supabase_session_roundtrip() {
        let session = SupabaseSession {
            access_token: "eyJabc123".to_string(),
            refresh_token: "rt_abc123".to_string(),
            expires_at: DateTime::parse_from_rfc3339("2026-03-12T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            user: SupabaseUser {
                id: Uuid::nil(),
                email: "test@example.com".to_string(),
                display_name: Some("Test User".to_string()),
            },
        };

        let json_str = serde_json::to_string(&session).unwrap();
        let deserialized: SupabaseSession = serde_json::from_str(&json_str).unwrap();
        assert_eq!(session, deserialized);
    }

    #[test]
    fn supabase_user_minimal() {
        let json = json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "email": "user@test.com"
        });
        let user: SupabaseUser = serde_json::from_value(json).unwrap();
        assert_eq!(user.email, "user@test.com");
        assert!(user.display_name.is_none());
    }

    #[test]
    fn auth_token_response_deserialize() {
        let json = json!({
            "access_token": "eyJ...",
            "token_type": "bearer",
            "expires_in": 3600,
            "refresh_token": "rt_...",
            "user": {
                "id": "00000000-0000-0000-0000-000000000000",
                "email": "user@test.com",
                "user_metadata": { "full_name": "Test" }
            }
        });
        let resp: AuthTokenResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.access_token, "eyJ...");
        assert_eq!(resp.expires_in, 3600);
        assert_eq!(resp.user.email.as_deref(), Some("user@test.com"));
    }

    #[test]
    fn otp_request_serialize() {
        let req = OtpRequest {
            email: "user@test.com".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["email"], "user@test.com");
    }

    #[test]
    fn verify_otp_request_serialize() {
        let req = VerifyOtpRequest {
            email: "user@test.com".to_string(),
            token: "123456".to_string(),
            otp_type: "email".to_string(),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["email"], "user@test.com");
        assert_eq!(json["token"], "123456");
        assert_eq!(json["type"], "email");
    }

    #[test]
    fn device_registration_roundtrip() {
        let now = Utc::now();
        let device = DeviceRegistration {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            device_name: "macbook".to_string(),
            device_type: "laptop".to_string(),
            platform_info: Some(json!({"os": "macos"})),
            relay_endpoint: None,
            relay_token: None,
            last_seen_at: now,
            created_at: now,
        };
        let json_str = serde_json::to_string(&device).unwrap();
        let deserialized: DeviceRegistration = serde_json::from_str(&json_str).unwrap();
        assert_eq!(device, deserialized);
    }

    #[test]
    fn profile_roundtrip() {
        let now = Utc::now();
        let profile = Profile {
            id: Uuid::nil(),
            display_name: Some("Test".to_string()),
            email: Some("test@example.com".to_string()),
            avatar_url: None,
            is_admin: false,
            shared_keys_enabled: false,
            invite_code: None,
            created_at: now,
            updated_at: now,
        };
        let json_str = serde_json::to_string(&profile).unwrap();
        let deserialized: Profile = serde_json::from_str(&json_str).unwrap();
        assert_eq!(profile, deserialized);
    }
}
