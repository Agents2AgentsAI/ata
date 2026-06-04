//! ATA Supabase refresh-token flow.
//!
//! When `AuthManager::refresh_token_from_authority_impl` is called on a
//! `CodexAuth::Ata` whose `expires_at` has passed (or whose current call
//! returned 401), this module:
//!   1. POSTs `<supabase>/auth/v1/token?grant_type=refresh_token` with the
//!      stored refresh token,
//!   2. rewrites `$codex_home/ata_session.json` with the fresh tokens,
//!   3. hands back the new `AtaAuth` so the caller can swap it into the
//!      cached auth slot.
//!
//! The Supabase URL + anon key duplicate the constants in
//! `codex-core::config::ata`. Duplication keeps `codex-login` independent
//! of `codex-core` (which depends on `codex-login`). The values are
//! publishable anon credentials, not secrets.

use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration as StdDuration;

use super::manager::AtaAuth;

/// Keep in sync with `codex_core::config::ata::DEFAULT_ATA_SUPABASE_URL`.
const SUPABASE_URL: &str = "https://natbqqfawsmcoeutsogu.supabase.co";
/// Keep in sync with `codex_core::config::ata::DEFAULT_ATA_SUPABASE_ANON_KEY`.
const SUPABASE_ANON_KEY: &str = "sb_publishable_MopvwXLh_k866kZvplSGGQ_IBircspG";
const SESSION_FILENAME: &str = "ata_session.json";
const REFRESH_TIMEOUT: StdDuration = StdDuration::from_secs(15);

#[derive(Debug, Serialize)]
struct RefreshRequest<'a> {
    refresh_token: &'a str,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    user: RefreshUser,
}

#[derive(Debug, Deserialize)]
struct RefreshUser {
    id: String,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AtaRefreshError {
    #[error("ATA auth has no refresh token; sign in again with /supabase-login")]
    MissingRefreshToken,
    #[error("Supabase refresh transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Supabase rejected the refresh ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("Failed to persist refreshed ATA session: {0}")]
    Persist(#[from] io::Error),
}

/// Exchange the cached `refresh_token` for a fresh access token and
/// persist the new session. Returns the rebuilt `AtaAuth` so the
/// caller can replace the cached auth slot.
pub(crate) async fn refresh_ata_session(
    codex_home: &Path,
    auth: &AtaAuth,
) -> Result<AtaAuth, AtaRefreshError> {
    let refresh_token = auth
        .refresh_token()
        .ok_or(AtaRefreshError::MissingRefreshToken)?;

    let client = crate::default_client::build_reqwest_client();
    let url = format!("{SUPABASE_URL}/auth/v1/token?grant_type=refresh_token");
    let response = client
        .post(&url)
        .header("apikey", SUPABASE_ANON_KEY)
        .header("Content-Type", "application/json")
        .timeout(REFRESH_TIMEOUT)
        .json(&RefreshRequest { refresh_token })
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        return Err(AtaRefreshError::Api {
            status: status.as_u16(),
            message,
        });
    }

    let body: RefreshResponse = response.json().await?;
    let expires_at = Utc::now() + Duration::seconds(body.expires_in);

    let new_auth = AtaAuth::new(body.access_token.clone())
        .with_refresh_token(body.refresh_token.clone())
        .with_expires_at(expires_at)
        .with_account_id(body.user.id.clone());
    let new_auth = match body.user.email.clone() {
        Some(email) => new_auth.with_email(email),
        None => new_auth,
    };

    persist_refreshed_session(codex_home, &body, expires_at)?;
    Ok(new_auth)
}

/// Rewrite `$codex_home/ata_session.json` with the refreshed tokens.
/// Schema must stay byte-for-byte compatible with the canonical writer in
/// `codex_core::supabase::session::save_ata_session` so that the next launch
/// can reload via `ata_session::load_ata_session`.
fn persist_refreshed_session(
    codex_home: &Path,
    body: &RefreshResponse,
    expires_at: DateTime<Utc>,
) -> io::Result<()> {
    let path = codex_home.join(SESSION_FILENAME);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut user_obj = serde_json::Map::new();
    user_obj.insert(
        "id".to_string(),
        serde_json::Value::String(body.user.id.clone()),
    );
    user_obj.insert(
        "email".to_string(),
        match body.user.email.clone() {
            Some(email) => serde_json::Value::String(email),
            None => serde_json::Value::Null,
        },
    );
    // The canonical writer carries `display_name` too; we don't have it on
    // the refresh response, so preserve a null and let any TUI consumer
    // treat it as unknown.
    user_obj.insert("display_name".to_string(), serde_json::Value::Null);

    let mut root = serde_json::Map::new();
    root.insert(
        "access_token".to_string(),
        serde_json::Value::String(body.access_token.clone()),
    );
    root.insert(
        "refresh_token".to_string(),
        serde_json::Value::String(body.refresh_token.clone()),
    );
    root.insert(
        "expires_at".to_string(),
        serde_json::Value::String(expires_at.to_rfc3339()),
    );
    root.insert("user".to_string(), serde_json::Value::Object(user_obj));

    let serialized = serde_json::to_string_pretty(&serde_json::Value::Object(root))?;

    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(serialized.as_bytes())?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_refresh_token_short_circuits() {
        let auth = AtaAuth::new("access".to_string());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempdir().unwrap();
        let err = rt
            .block_on(refresh_ata_session(dir.path(), &auth))
            .unwrap_err();
        assert!(matches!(err, AtaRefreshError::MissingRefreshToken));
    }

    #[test]
    fn persist_writes_file_with_expected_schema() {
        let dir = tempdir().unwrap();
        let body = RefreshResponse {
            access_token: "new-access".to_string(),
            refresh_token: "new-refresh".to_string(),
            expires_in: 3600,
            user: RefreshUser {
                id: "abc-123".to_string(),
                email: Some("alice@example.com".to_string()),
            },
        };
        let expires_at = DateTime::parse_from_rfc3339("2099-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        persist_refreshed_session(dir.path(), &body, expires_at).unwrap();

        let json = std::fs::read_to_string(dir.path().join("ata_session.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["access_token"], "new-access");
        assert_eq!(value["refresh_token"], "new-refresh");
        assert_eq!(value["user"]["email"], "alice@example.com");
        assert_eq!(value["user"]["id"], "abc-123");
    }
}
