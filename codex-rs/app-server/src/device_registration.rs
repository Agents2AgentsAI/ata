//! Device registration for the app-server.
//!
//! Registers this server as a "node" in the Supabase `devices` table so that
//! remote clients (iOS, web) can discover it. A background heartbeat task
//! keeps `last_seen_at` fresh. On shutdown the row is deleted.

use std::net::UdpSocket;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use codex_core::config::types::DEFAULT_ATA_SUPABASE_ANON_KEY;
use codex_core::config::types::DEFAULT_ATA_SUPABASE_URL;
use codex_core::supabase::SupabaseAuth;
use codex_core::supabase::SupabaseClient;
use codex_core::supabase::SupabaseError;
use codex_core::supabase::SupabaseSession;
use codex_core::supabase::load_ata_session;
use codex_core::supabase::save_ata_session;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderValue;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing::warn;
use uuid::Uuid;

/// How often we PATCH `last_seen_at`.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// How far before expiry we attempt to refresh the token.
const REFRESH_MARGIN: chrono::Duration = chrono::Duration::minutes(5);

/// Mutable token state shared between the heartbeat task and the registrar.
struct TokenState {
    access_token: String,
    refresh_token: String,
    expires_at: DateTime<Utc>,
}

/// Manages this server's row in the Supabase `devices` table.
pub struct DeviceRegistrar {
    client: SupabaseClient,
    token_state: Arc<Mutex<TokenState>>,
    device_id: Uuid,
    cancel: CancellationToken,
}

/// Body sent to PostgREST for upsert.
#[derive(Serialize)]
struct DeviceUpsertBody {
    id: Uuid,
    user_id: Uuid,
    device_name: String,
    device_type: String,
    platform_info: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_endpoint: Option<String>,
    last_seen_at: String,
}

impl DeviceRegistrar {
    /// Start device registration.
    ///
    /// 1. Loads (or creates) a stable device ID from `codex_home/device_id`.
    /// 2. Loads the ATA session from `codex_home` to get the access token.
    /// 3. Upserts into the `devices` table.
    /// 4. Spawns a heartbeat task.
    ///
    /// `public_url` is the externally-reachable endpoint for this server. It is
    /// resolved in order: explicit argument > `ATA_PUBLIC_URL` env > auto-detect.
    pub async fn start(
        codex_home: &Path,
        public_url: Option<String>,
        listen_port: Option<u16>,
    ) -> anyhow::Result<Self> {
        // --- Load ATA session ---
        let session: SupabaseSession = load_ata_session(codex_home)?
            .ok_or_else(|| anyhow::anyhow!("no ATA session found; please login first"))?;
        let access_token = session.access_token.clone();

        let client = SupabaseClient::new(DEFAULT_ATA_SUPABASE_URL, DEFAULT_ATA_SUPABASE_ANON_KEY);

        // --- Device ID (persisted so the same machine always reuses the same row) ---
        let device_id = load_or_create_device_id(codex_home)?;

        // --- Hostname ---
        let device_name = gethostname::gethostname().to_string_lossy().to_string();

        // --- Public URL ---
        let relay_endpoint = resolve_public_url(public_url, listen_port.unwrap_or(19285));

        // --- Platform info ---
        let platform_info = serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });

        // --- Upsert ---
        let body = DeviceUpsertBody {
            id: device_id,
            user_id: session.user.id,
            device_name,
            device_type: "server".to_string(),
            platform_info,
            relay_endpoint: relay_endpoint.clone(),
            last_seen_at: Utc::now().to_rfc3339(),
        };

        let url = client.rest_url("devices");
        let mut headers = auth_headers(&client, &access_token);
        // PostgREST upsert: merge-duplicates on the `id` column.
        headers.insert(
            "Prefer",
            HeaderValue::from_static("resolution=merge-duplicates,return=minimal"),
        );

        let resp = client
            .http()
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("device upsert failed (HTTP {status}): {text}");
        }

        info!(
            device_id = %device_id,
            relay_endpoint = ?relay_endpoint,
            "device registered in Supabase"
        );

        // --- Shared token state ---
        let token_state = Arc::new(Mutex::new(TokenState {
            access_token: session.access_token,
            refresh_token: session.refresh_token,
            expires_at: session.expires_at,
        }));

        // --- Heartbeat loop ---
        let cancel = CancellationToken::new();
        let registrar = Self {
            client: client.clone(),
            token_state: Arc::clone(&token_state),
            device_id,
            cancel: cancel.clone(),
        };

        tokio::spawn({
            let codex_home = codex_home.to_path_buf();
            async move {
                loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {}
                    }

                    // --- Refresh token if close to expiry ---
                    maybe_refresh_token(&client, &token_state, &codex_home).await;

                    let access_token = token_state.lock().await.access_token.clone();
                    let url = format!("{}?id=eq.{}", client.rest_url("devices"), device_id,);
                    let body = serde_json::json!({
                        "last_seen_at": Utc::now().to_rfc3339(),
                    });
                    let headers = auth_headers(&client, &access_token);
                    match client
                        .http()
                        .patch(&url)
                        .headers(headers)
                        .json(&body)
                        .send()
                        .await
                    {
                        Ok(resp) if !resp.status().is_success() => {
                            warn!(
                                status = %resp.status(),
                                "device heartbeat PATCH returned non-2xx"
                            );
                        }
                        Err(e) => {
                            warn!(error = %e, "device heartbeat failed");
                        }
                        _ => {}
                    }
                }
            }
        });

        Ok(registrar)
    }

    /// Cancel the heartbeat and DELETE the device row.
    pub async fn stop(&self) {
        self.cancel.cancel();

        let access_token = self.token_state.lock().await.access_token.clone();
        let url = format!(
            "{}?id=eq.{}",
            self.client.rest_url("devices"),
            self.device_id,
        );
        let headers = auth_headers(&self.client, &access_token);
        match self
            .client
            .http()
            .delete(&url)
            .headers(headers)
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                warn!(
                    status = %resp.status(),
                    "device DELETE returned non-2xx"
                );
            }
            Err(e) => {
                warn!(error = %e, "device DELETE failed");
            }
            _ => {
                info!(device_id = %self.device_id, "device deregistered");
            }
        }
    }

    /// The device ID used for registration.
    pub fn device_id(&self) -> Uuid {
        self.device_id
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build authenticated headers (apikey + Bearer token).
fn auth_headers(client: &SupabaseClient, access_token: &str) -> HeaderMap {
    client.auth_headers(access_token)
}

/// Check whether the current access token is close to expiry and, if so,
/// refresh it via Supabase Auth and persist the new session to disk.
async fn maybe_refresh_token(
    client: &SupabaseClient,
    token_state: &Arc<Mutex<TokenState>>,
    codex_home: &Path,
) {
    let needs_refresh = {
        let state = token_state.lock().await;
        Utc::now() + REFRESH_MARGIN > state.expires_at
    };

    if !needs_refresh {
        return;
    }

    info!("access token close to expiry, attempting refresh");

    let refresh_token = { token_state.lock().await.refresh_token.clone() };

    let auth = SupabaseAuth::new(client.clone());
    match auth.refresh_session(&refresh_token).await {
        Ok(new_session) => {
            // Update in-memory state.
            {
                let mut state = token_state.lock().await;
                state.access_token = new_session.access_token.clone();
                state.refresh_token = new_session.refresh_token.clone();
                state.expires_at = new_session.expires_at;
            }

            // Persist to disk so other processes (and restarts) see the new token.
            if let Err(e) = save_ata_session(codex_home, &new_session) {
                warn!(error = %e, "failed to save refreshed ATA session to disk");
            } else {
                info!("access token refreshed successfully");
            }
        }
        Err(e) => {
            // Check if this is an auth failure (expired/invalid refresh token)
            // rather than a transient network error.
            let is_auth_failure = match &e {
                SupabaseError::Api { status, .. } => {
                    *status == 400 || *status == 401 || *status == 403
                }
                _ => false,
            };

            if is_auth_failure {
                // Write session-expired marker so TUI and other processes
                // know the user must re-authenticate.
                let marker = codex_home.join("session_expired");
                let _ = std::fs::write(&marker, "");
                warn!("refresh token expired — user must re-login");
            } else {
                warn!(error = %e, "failed to refresh access token; will retry next heartbeat");
            }
        }
    }
}

/// Load a device ID from `codex_home/device_id`, or generate and persist one.
fn load_or_create_device_id(codex_home: &Path) -> anyhow::Result<Uuid> {
    let path: PathBuf = codex_home.join("device_id");
    if path.exists() {
        let contents = std::fs::read_to_string(&path)?;
        let id = Uuid::parse_str(contents.trim())?;
        return Ok(id);
    }
    // Generate a new v4 UUID and persist it.
    let id = Uuid::new_v4();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, id.to_string())?;
    info!(device_id = %id, path = %path.display(), "created new device_id");
    Ok(id)
}

/// Determine the relay endpoint URL.
///
/// Priority: explicit `--public-url` > `ATA_PUBLIC_URL` env var > auto-detect LAN IP.
fn resolve_public_url(explicit: Option<String>, port: u16) -> Option<String> {
    if let Some(url) = explicit.filter(|u| !u.is_empty()) {
        return Some(url);
    }
    if let Some(url) = std::env::var("ATA_PUBLIC_URL")
        .ok()
        .filter(|u| !u.is_empty())
    {
        return Some(url);
    }
    // Auto-detect: create a UDP socket pointed at a public address.
    // This doesn't actually send traffic — it just lets the OS pick the
    // outgoing interface so we can read the local IP.
    detect_local_ip().map(|ip| format!("ws://{ip}:{port}"))
}

/// Detect the machine's LAN IP by opening a UDP socket toward 8.8.8.8:80.
fn detect_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_create_device_id_persists() {
        let dir = tempdir().unwrap();
        let id1 = load_or_create_device_id(dir.path()).unwrap();
        let id2 = load_or_create_device_id(dir.path()).unwrap();
        assert_eq!(id1, id2, "device_id should be stable across calls");
    }

    #[test]
    fn load_or_create_device_id_reads_existing() {
        let dir = tempdir().unwrap();
        let expected = Uuid::new_v4();
        std::fs::write(dir.path().join("device_id"), expected.to_string()).unwrap();
        let loaded = load_or_create_device_id(dir.path()).unwrap();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn resolve_public_url_explicit_wins() {
        let result = resolve_public_url(Some("https://my-server.example.com".to_string()), 19285);
        assert_eq!(result, Some("https://my-server.example.com".to_string()));
    }

    #[test]
    fn resolve_public_url_empty_explicit_falls_through() {
        // With empty explicit, it should try env / auto-detect.
        // We cannot easily control env in tests, so just verify it doesn't
        // return the empty string.
        let result = resolve_public_url(Some(String::new()), 19285);
        assert_ne!(result, Some(String::new()));
    }

    #[test]
    fn detect_local_ip_returns_something() {
        // On CI or sandboxed environments this might be None, so we just
        // verify the function does not panic.
        let _ip = detect_local_ip();
    }

    #[test]
    fn device_upsert_body_serializes() {
        let body = DeviceUpsertBody {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            device_name: "test-host".to_string(),
            device_type: "server".to_string(),
            platform_info: serde_json::json!({"os": "linux", "arch": "x86_64"}),
            relay_endpoint: Some("http://192.168.1.10:3284".to_string()),
            last_seen_at: "2026-03-15T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["device_type"], "server");
        assert_eq!(json["device_name"], "test-host");
        assert!(json.get("relay_endpoint").is_some());
    }

    #[test]
    fn device_upsert_body_omits_none_relay() {
        let body = DeviceUpsertBody {
            id: Uuid::nil(),
            user_id: Uuid::nil(),
            device_name: "host".to_string(),
            device_type: "server".to_string(),
            platform_info: serde_json::json!({}),
            relay_endpoint: None,
            last_seen_at: "2026-03-15T00:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("relay_endpoint").is_none());
    }
}
