//! Supabase authentication flows for the CLI/TUI.
//!
//! Provides two login flows:
//! - **Magic Link**: sends an OTP email, starts a local callback server on port 1455,
//!   and waits for the user to click the link which redirects back with tokens.
//! - **Device Code**: calls a Supabase Edge Function to get a code, displays it to the
//!   user, and polls for completion.

use std::io;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_core::supabase::SupabaseClient;
use codex_core::supabase::SupabaseSession;

/// Decode a JWT payload and extract the `sub` claim (user ID).
/// Only decodes — does NOT verify the signature.
fn decode_jwt_sub(token: &str) -> Option<String> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims.get("sub")?.as_str().map(String::from)
}
use serde::Deserialize;
use serde::Serialize;
use tiny_http::Header;
use tiny_http::Response;
use tiny_http::Server;

const MAGIC_LINK_PORT: u16 = 1455;
const MAGIC_LINK_CALLBACK_PATH: &str = "/auth/callback";
const MAGIC_LINK_TIMEOUT: Duration = Duration::from_secs(10 * 60);

const DEVICE_CODE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

const ANSI_BLUE: &str = "\x1b[94m";
const ANSI_GRAY: &str = "\x1b[90m";
const ANSI_RESET: &str = "\x1b[0m";

/// Response from the device-code initiation endpoint.
///
/// The Edge Function returns `{ code, verification_url, expires_in }`.
/// The `code` serves as both the user-visible code and the polling identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    /// The code (used for both display to user and polling).
    pub code: String,
    /// URL where the user goes to enter the code.
    pub verification_url: String,
    /// Total expiry time in seconds (not poll interval).
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_expires_in() -> u64 {
    900 // 15 minutes
}

/// Default poll interval in seconds.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 3;

/// Response from the device-code poll endpoint.
///
/// The Edge Function returns `{ status, session? }` where `session` is a
/// nested object containing the OAuth tokens from the browser callback.
#[derive(Debug, Clone, Deserialize)]
struct DeviceCodePollResponse {
    status: String,
    /// Session tokens (present when status is "ready").
    #[serde(default)]
    session: Option<DeviceCodePollSession>,
}

/// Session tokens returned inside the poll response.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct DeviceCodePollSession {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<serde_json::Value>,
    #[serde(default)]
    token_type: Option<String>,
}

/// Supabase Magic Link login flow.
///
/// 1. Sends an OTP email via Supabase Auth.
/// 2. Starts a local HTTP server on port 1455 listening for `/auth/callback`.
/// 3. Waits for the callback with `access_token` and `refresh_token` in query params.
/// 4. Returns a `SupabaseSession`.
pub async fn supabase_magic_link_login(
    email: &str,
    supabase_client: &SupabaseClient,
) -> io::Result<SupabaseSession> {
    let auth = codex_core::supabase::SupabaseAuth::new(supabase_client.clone());
    let redirect_url = format!("http://localhost:{MAGIC_LINK_PORT}{MAGIC_LINK_CALLBACK_PATH}");

    // Send the OTP with redirect_to hint. The Supabase Auth API accepts
    // `options.emailRedirectTo` on the OTP endpoint but our SupabaseAuth::sign_in_with_otp
    // just sends the email. We'll call the raw endpoint with the redirect parameter.
    let otp_url = supabase_client.auth_url("otp");
    let body = serde_json::json!({
        "email": email,
        "options": {
            "emailRedirectTo": redirect_url,
        }
    });
    let resp = supabase_client
        .http()
        .post(&otp_url)
        .headers(supabase_client.default_headers())
        .json(&body)
        .send()
        .await
        .map_err(io::Error::other)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "Failed to send OTP email (HTTP {status}): {text}"
        )));
    }

    println!(
        "\n{ANSI_GRAY}A magic link has been sent to {ANSI_BLUE}{email}{ANSI_RESET}.\n\
         {ANSI_GRAY}Click the link in your email to complete sign-in.{ANSI_RESET}\n\
         {ANSI_GRAY}Waiting for callback on http://localhost:{MAGIC_LINK_PORT}...{ANSI_RESET}\n"
    );

    // Start local callback server.
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel::<CallbackResult>(1);
    let server = bind_callback_server(MAGIC_LINK_PORT)?;
    let server = Arc::new(server);

    // Spawn blocking thread to accept requests from tiny_http.
    let (req_tx, mut req_rx) = tokio::sync::mpsc::channel::<tiny_http::Request>(4);
    let server_clone = server.clone();
    std::thread::spawn(move || {
        while let Ok(request) = server_clone.recv() {
            if req_tx.blocking_send(request).is_err() {
                break;
            }
        }
    });

    let auth_clone = auth.clone();
    let email_owned = email.to_string();
    let server_ref = server.clone();
    tokio::spawn(async move {
        let deadline = Instant::now() + MAGIC_LINK_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let _ = session_tx
                    .send(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Magic link login timed out",
                    )))
                    .await;
                break;
            }

            let maybe_req = tokio::time::timeout(remaining, req_rx.recv()).await;
            let req = match maybe_req {
                Ok(Some(req)) => req,
                Ok(None) | Err(_) => {
                    let _ = session_tx
                        .send(Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Magic link login timed out",
                        )))
                        .await;
                    break;
                }
            };

            let url_raw = req.url().to_string();
            let parsed = match url::Url::parse(&format!("http://localhost{url_raw}")) {
                Ok(u) => u,
                Err(_) => {
                    let _ = req.respond(Response::from_string("Bad Request").with_status_code(400));
                    continue;
                }
            };

            if parsed.path() != MAGIC_LINK_CALLBACK_PATH {
                let _ = req.respond(Response::from_string("Not Found").with_status_code(404));
                continue;
            }

            let params: std::collections::HashMap<String, String> =
                parsed.query_pairs().into_owned().collect();

            // The callback can carry tokens directly or an OTP code.
            let result = if let Some(access_token) = params.get("access_token") {
                // Tokens provided directly via redirect fragment/query.
                let refresh_token = params.get("refresh_token").cloned().unwrap_or_default();
                let expires_in: i64 = params
                    .get("expires_in")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3600);
                Ok(build_session_from_tokens(
                    access_token.clone(),
                    refresh_token,
                    expires_in,
                    params.get("email").cloned(),
                ))
            } else if let Some(code) = params.get("code").or(params.get("token")) {
                // OTP code provided; exchange it for a session.
                match auth_clone
                    .exchange_code_for_session(&email_owned, code)
                    .await
                {
                    Ok(session) => Ok(session),
                    Err(e) => Err(io::Error::other(format!(
                        "Failed to exchange code for session: {e}"
                    ))),
                }
            } else {
                Err(io::Error::other(
                    "Callback did not contain tokens or a code",
                ))
            };

            // Respond to the browser.
            let html = if result.is_ok() {
                "<html><body><h2>Sign-in successful!</h2><p>You can close this tab.</p></body></html>"
            } else {
                "<html><body><h2>Sign-in failed</h2><p>Please try again.</p></body></html>"
            };
            let mut response = Response::from_string(html).with_status_code(200);
            if let Ok(header) = Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]) {
                response = response.with_header(header);
            }
            let _ = req.respond(response);

            let _ = session_tx.send(result).await;
            break;
        }
        server_ref.unblock();
    });

    session_rx
        .recv()
        .await
        .unwrap_or_else(|| Err(io::Error::other("Login server channel closed unexpectedly")))
}

/// Phase 1: Request a device code from the Supabase Edge Function.
///
/// POSTs to `{base_url}/functions/v1/device-code` and returns the
/// [`DeviceCodeResponse`] containing the user code, verification URL,
/// device code, and recommended poll interval.
pub async fn request_supabase_device_code(
    supabase_client: &SupabaseClient,
) -> io::Result<DeviceCodeResponse> {
    let base_url = supabase_client.base_url();
    let functions_url = format!("{base_url}/functions/v1/device-code");

    let resp = supabase_client
        .http()
        .post(&functions_url)
        .headers(supabase_client.default_headers())
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(io::Error::other)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "device-code initiation failed (HTTP {status}): {text}"
        )));
    }

    resp.json().await.map_err(io::Error::other)
}

/// Phase 2: Poll until the device code login completes.
///
/// GETs `{base_url}/functions/v1/device-code?code={device_code}` every
/// `interval` seconds (from the [`DeviceCodeResponse`]) until the server
/// reports "ready" / "complete", then returns a [`SupabaseSession`].
///
/// Does **not** print anything to stdout — callers are responsible for
/// displaying the code and URL to the user.
pub async fn complete_supabase_device_code_login(
    supabase_client: &SupabaseClient,
    device_resp: &DeviceCodeResponse,
) -> io::Result<SupabaseSession> {
    let base_url = supabase_client.base_url();
    let functions_url = format!("{base_url}/functions/v1/device-code");
    let client = supabase_client.http();

    let poll_interval = Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS);
    let deadline = Instant::now() + DEVICE_CODE_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Device code login timed out after 15 minutes",
            ));
        }

        tokio::time::sleep(poll_interval).await;

        let poll_url = format!("{functions_url}?code={}", device_resp.code);
        let poll_resp = client
            .get(&poll_url)
            .headers(supabase_client.default_headers())
            .send()
            .await
            .map_err(io::Error::other)?;

        let status = poll_resp.status();

        if status == reqwest::StatusCode::ACCEPTED || status == reqwest::StatusCode::FORBIDDEN {
            // Still pending.
            continue;
        }

        if !status.is_success() {
            let text = poll_resp.text().await.unwrap_or_default();
            return Err(io::Error::other(format!(
                "device-code poll failed (HTTP {status}): {text}"
            )));
        }

        let poll_data: DeviceCodePollResponse = poll_resp.json().await.map_err(io::Error::other)?;

        if poll_data.status == "ready" || poll_data.status == "complete" {
            let session = poll_data.session.ok_or_else(|| {
                io::Error::other("device-code ready but no session data returned")
            })?;
            let access_token = session.access_token.ok_or_else(|| {
                io::Error::other("device-code ready but no access_token in session")
            })?;
            let refresh_token = session.refresh_token.unwrap_or_default();
            // expires_in can be a number or string from the browser callback
            let expires_in = session
                .expires_in
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => n.as_i64(),
                    serde_json::Value::String(s) => s.parse().ok(),
                    _ => None,
                })
                .unwrap_or(3600);

            return Ok(build_session_from_tokens(
                access_token,
                refresh_token,
                expires_in,
                None, // email not available from OAuth callback tokens
            ));
        }

        // Any other status: keep polling.
    }
}

/// Supabase device-code login flow (convenience wrapper).
///
/// 1. Calls [`request_supabase_device_code`] to obtain a user code and verification URL.
/// 2. Displays the code to the user via stdout.
/// 3. Calls [`complete_supabase_device_code_login`] to poll until status is "ready".
/// 4. Returns a [`SupabaseSession`].
pub async fn supabase_device_code_login(
    supabase_client: &SupabaseClient,
) -> io::Result<SupabaseSession> {
    let device_resp = request_supabase_device_code(supabase_client).await?;

    print_supabase_device_code_prompt(&device_resp.verification_url, &device_resp.code);

    complete_supabase_device_code_login(supabase_client, &device_resp).await
}

fn print_supabase_device_code_prompt(verification_url: &str, code: &str) {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "\nWelcome to Ata [v{ANSI_GRAY}{version}{ANSI_RESET}]\n\
         {ANSI_GRAY}ATA account sign-in via device code{ANSI_RESET}\n\
         \n1. Open this link in your browser:\n   {ANSI_BLUE}{verification_url}{ANSI_RESET}\n\
         \n2. Enter this one-time code {ANSI_GRAY}(expires in 15 minutes){ANSI_RESET}\n   \
         {ANSI_BLUE}{code}{ANSI_RESET}\n\
         \n{ANSI_GRAY}Never share this code with anyone.{ANSI_RESET}\n"
    );
}

fn build_session_from_tokens(
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    email: Option<String>,
) -> SupabaseSession {
    use chrono::Duration;
    use chrono::Utc;

    // Extract the real user ID from the JWT's `sub` claim.
    let user_id = decode_jwt_sub(&access_token)
        .and_then(|s| uuid::Uuid::parse_str(&s).ok())
        .unwrap_or_else(uuid::Uuid::nil);

    let expires_at = Utc::now() + Duration::seconds(expires_in);
    SupabaseSession {
        access_token,
        refresh_token,
        expires_at,
        user: codex_core::supabase::SupabaseUser {
            id: user_id,
            email: email.unwrap_or_default(),
            display_name: None,
        },
    }
}

/// Send an OTP email via Supabase Auth (no browser needed).
pub async fn send_ata_otp(supabase_client: &SupabaseClient, email: &str) -> io::Result<()> {
    let otp_url = supabase_client.auth_url("otp");
    let body = serde_json::json!({
        "email": email,
        "options": { "shouldCreateUser": true }
    });
    let resp = supabase_client
        .http()
        .post(&otp_url)
        .headers(supabase_client.default_headers())
        .json(&body)
        .send()
        .await
        .map_err(io::Error::other)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "Failed to send OTP (HTTP {status}): {text}"
        )));
    }
    Ok(())
}

/// Verify an OTP code and return a session.
pub async fn verify_ata_otp(
    supabase_client: &SupabaseClient,
    email: &str,
    otp: &str,
) -> io::Result<SupabaseSession> {
    let verify_url = supabase_client.auth_url("verify");
    let body = serde_json::json!({
        "email": email,
        "token": otp,
        "type": "email"
    });
    let resp = supabase_client
        .http()
        .post(&verify_url)
        .headers(supabase_client.default_headers())
        .json(&body)
        .send()
        .await
        .map_err(io::Error::other)?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(io::Error::other(format!(
            "OTP verification failed (HTTP {status}): {text}"
        )));
    }

    // Supabase returns { access_token, token_type, expires_in, refresh_token, user }
    let auth_resp: codex_core::supabase::types::AuthTokenResponse =
        resp.json().await.map_err(io::Error::other)?;

    Ok(build_session_from_tokens(
        auth_resp.access_token,
        auth_resp.refresh_token,
        auth_resp.expires_in,
        auth_resp.user.email,
    ))
}

type CallbackResult = io::Result<SupabaseSession>;

fn bind_callback_server(port: u16) -> io::Result<Server> {
    let addr = format!("127.0.0.1:{port}");
    Server::http(&addr).map_err(|e| io::Error::other(format!("Failed to bind {addr}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_session_from_tokens_sets_expiry() {
        let session = build_session_from_tokens(
            "access_abc".to_string(),
            "refresh_xyz".to_string(),
            3600,
            Some("user@test.com".to_string()),
        );
        assert_eq!(session.access_token, "access_abc");
        assert_eq!(session.refresh_token, "refresh_xyz");
        assert_eq!(session.user.email, "user@test.com");
        let diff = session.expires_at - chrono::Utc::now();
        assert!(diff.num_seconds() > 3590 && diff.num_seconds() <= 3600);
    }

    #[test]
    fn build_session_no_email() {
        let session = build_session_from_tokens("a".to_string(), "r".to_string(), 60, None);
        assert_eq!(session.user.email, "");
        assert!(session.user.display_name.is_none());
    }

    #[test]
    fn device_code_response_deserialize() {
        let json = serde_json::json!({
            "code": "ABCD-1234",
            "verification_url": "https://example.com/verify",
            "expires_in": 900
        });
        let resp: DeviceCodeResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.code, "ABCD-1234");
        assert_eq!(resp.verification_url, "https://example.com/verify");
        assert_eq!(resp.expires_in, 900);
    }

    #[test]
    fn device_code_response_default_expires_in() {
        let json = serde_json::json!({
            "code": "CODE-1234",
            "verification_url": "https://example.com"
        });
        let resp: DeviceCodeResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.expires_in, 900); // default 15 minutes
    }

    #[test]
    fn device_code_poll_response_ready() {
        let json = serde_json::json!({
            "status": "ready",
            "session": {
                "access_token": "at_123",
                "refresh_token": "rt_456",
                "expires_in": 7200,
                "token_type": "bearer"
            }
        });
        let resp: DeviceCodePollResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.status, "ready");
        let session = resp.session.unwrap();
        assert_eq!(session.access_token.as_deref(), Some("at_123"));
        assert_eq!(session.refresh_token.as_deref(), Some("rt_456"));
    }

    #[test]
    fn device_code_poll_response_ready_with_string_expires() {
        // Browser callback may send expires_in as a string
        let json = serde_json::json!({
            "status": "ready",
            "session": {
                "access_token": "at_abc",
                "refresh_token": "rt_xyz",
                "expires_in": "3600"
            }
        });
        let resp: DeviceCodePollResponse = serde_json::from_value(json).unwrap();
        let session = resp.session.unwrap();
        assert_eq!(session.access_token.as_deref(), Some("at_abc"));
        // expires_in as string should be handled in the parsing code
        match session.expires_in.unwrap() {
            serde_json::Value::String(s) => assert_eq!(s, "3600"),
            _ => panic!("expected string expires_in"),
        }
    }

    #[test]
    fn device_code_poll_response_pending() {
        let json = serde_json::json!({
            "status": "pending"
        });
        let resp: DeviceCodePollResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.status, "pending");
        assert!(resp.session.is_none());
    }
}
