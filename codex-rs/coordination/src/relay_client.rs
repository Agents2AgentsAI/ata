//! Best-effort HTTP relay client for cross-machine coordination.
//!
//! Every public method silently swallows errors (logging at `warn` level) and
//! returns `Ok(empty)` so that relay failures never block local coordination.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use tracing::warn;

use crate::CoordinationMessage;
use crate::CoordinationSession;

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for the coordination relay server.
pub struct RelayClient {
    client: Client,
    base_url: String,
    secret: Option<String>,
    project_id: String,
}

impl RelayClient {
    /// Create a new relay client.
    ///
    /// * `base_url` — relay server origin, e.g. `http://relay.myteam.dev:7800`
    /// * `secret`   — optional shared secret (`X-Relay-Secret` header)
    /// * `project_id` — SHA-256 hex of the normalized git remote URL
    pub fn new(base_url: String, secret: Option<String>, project_id: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url,
            secret,
            project_id,
        }
    }

    /// Accessor for the project_id (used in tests / debugging).
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    // -- helpers --

    fn url(&self, path: &str) -> String {
        format!(
            "{}/v1/projects/{}{path}",
            self.base_url.trim_end_matches('/'),
            self.project_id,
        )
    }

    fn add_secret(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.secret {
            Some(s) => req.header("X-Relay-Secret", s.as_str()),
            None => req,
        }
    }

    // -- public API -----------------------------------------------------------

    /// Register or update a session on the relay.
    pub async fn register_session(
        &self,
        session_id: &str,
        branch: Option<&str>,
        description: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = self.url(&format!("/sessions/{session_id}"));
        let body = RegisterBody {
            branch: branch.map(String::from),
            description: description.map(String::from),
        };
        let req = self.add_secret(self.client.put(&url)).json(&body);
        if let Err(e) = req.send().await {
            warn!("relay register_session failed: {e}");
        }
        Ok(())
    }

    /// Update the session description.
    pub async fn update_description(
        &self,
        session_id: &str,
        description: &str,
    ) -> anyhow::Result<()> {
        let url = self.url(&format!("/sessions/{session_id}/description"));
        let body = DescriptionBody {
            description: description.to_string(),
        };
        let req = self.add_secret(self.client.patch(&url)).json(&body);
        if let Err(e) = req.send().await {
            warn!("relay update_description failed: {e}");
        }
        Ok(())
    }

    /// Send a heartbeat for the session.
    pub async fn heartbeat(&self, session_id: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/sessions/{session_id}/heartbeat"));
        let req = self.add_secret(self.client.post(&url));
        if let Err(e) = req.send().await {
            warn!("relay heartbeat failed: {e}");
        }
        Ok(())
    }

    /// Deregister a session from the relay.
    pub async fn deregister_session(&self, session_id: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("/sessions/{session_id}"));
        let req = self.add_secret(self.client.delete(&url));
        if let Err(e) = req.send().await {
            warn!("relay deregister_session failed: {e}");
        }
        Ok(())
    }

    /// Post a message to the relay.
    pub async fn post_message(
        &self,
        session_id: &str,
        message: &str,
        message_type: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = self.url("/messages");
        let body = PostMessageBody {
            session_id: session_id.to_string(),
            message: message.to_string(),
            message_type: message_type.map(String::from),
        };
        let req = self.add_secret(self.client.post(&url)).json(&body);
        if let Err(e) = req.send().await {
            warn!("relay post_message failed: {e}");
        }
        Ok(())
    }

    /// Fetch recent messages from the relay (incremental via `since`).
    pub async fn recent_messages(
        &self,
        since: Option<i64>,
    ) -> anyhow::Result<Vec<CoordinationMessage>> {
        let mut url = self.url("/messages");
        if let Some(id) = since {
            url = format!("{url}?since={id}");
        }
        let req = self.add_secret(self.client.get(&url));
        match req.send().await {
            Ok(resp) => match resp.json::<Vec<WireMessage>>().await {
                Ok(msgs) => Ok(msgs.into_iter().map(Into::into).collect()),
                Err(e) => {
                    warn!("relay recent_messages parse failed: {e}");
                    Ok(Vec::new())
                }
            },
            Err(e) => {
                warn!("relay recent_messages failed: {e}");
                Ok(Vec::new())
            }
        }
    }

    /// Subscribe to live coordination messages from the relay via SSE.
    ///
    /// Subscribe to live coordination messages from the relay via SSE.
    ///
    /// Spawns a background task that:
    /// 1. Opens `GET /events?since={last_id}` as a streaming response
    /// 2. Reads SSE `data:` frames and parses them into [`CoordinationMessage`]
    /// 3. Sends parsed messages via a broadcast channel
    /// 4. On disconnect, waits 5 seconds and reconnects with `since={last_id}`
    /// 5. Respects the `CancellationToken` for clean shutdown
    pub fn subscribe_events(
        self: &Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> tokio::sync::broadcast::Receiver<CoordinationMessage> {
        use tokio::sync::broadcast;

        let (tx, rx) = broadcast::channel::<CoordinationMessage>(64);
        let client_ref = Arc::clone(self);

        tokio::spawn(async move {
            // Use a client without the default 5s timeout for long-lived SSE.
            let sse_client = reqwest::Client::builder().build().unwrap_or_default();
            let mut last_id: i64 = 0;

            loop {
                if cancel.is_cancelled() {
                    break;
                }

                let url = format!("{}?since={last_id}", client_ref.url("/events"));
                let req = client_ref.add_secret(sse_client.get(&url));

                match req.send().await {
                    Ok(mut resp) => {
                        let mut buf = String::new();

                        // Read chunks via resp.chunk() (no `stream` feature needed).
                        loop {
                            tokio::select! {
                                _ = cancel.cancelled() => return,
                                chunk_result = resp.chunk() => {
                                    match chunk_result {
                                        Ok(Some(bytes)) => {
                                            buf.push_str(&String::from_utf8_lossy(&bytes));

                                            // Process complete SSE frames.
                                            while let Some(pos) = buf.find("\n\n") {
                                                let frame = buf[..pos].to_string();
                                                buf = buf[pos + 2..].to_string();

                                                // Parse SSE data lines.
                                                for line in frame.lines() {
                                                    if let Some(data) = line.strip_prefix("data:") {
                                                        let data = data.trim();
                                                        if let Ok(wire) = serde_json::from_str::<WireMessage>(data) {
                                                            tracing::debug!(
                                                                "relay SSE: received msg id={} session={}",
                                                                wire.id,
                                                                wire.session_id,
                                                            );
                                                            if wire.id > last_id {
                                                                last_id = wire.id;
                                                            }
                                                            let msg: CoordinationMessage = wire.into();
                                                            let _ = tx.send(msg);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Ok(None) => {
                                            // Stream ended (server closed).
                                            break;
                                        }
                                        Err(e) => {
                                            warn!("relay SSE stream error: {e}");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("relay SSE connect failed: {e}");
                    }
                }

                // Wait before reconnecting.
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
                }
            }
        });

        rx
    }

    /// Fetch active sessions from the relay.
    pub async fn active_sessions(&self) -> anyhow::Result<Vec<CoordinationSession>> {
        let url = self.url("/sessions");
        let req = self.add_secret(self.client.get(&url));
        match req.send().await {
            Ok(resp) => match resp.json::<Vec<WireSession>>().await {
                Ok(sessions) => Ok(sessions.into_iter().map(Into::into).collect()),
                Err(e) => {
                    warn!("relay active_sessions parse failed: {e}");
                    Ok(Vec::new())
                }
            },
            Err(e) => {
                warn!("relay active_sessions failed: {e}");
                Ok(Vec::new())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types (match the relay server JSON shapes)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct RegisterBody {
    branch: Option<String>,
    description: Option<String>,
}

#[derive(Serialize)]
struct DescriptionBody {
    description: String,
}

#[derive(Serialize)]
struct PostMessageBody {
    session_id: String,
    message: String,
    message_type: Option<String>,
}

#[derive(Deserialize)]
struct WireSession {
    session_id: String,
    branch: Option<String>,
    description: Option<String>,
    started_at: i64,
    last_heartbeat: i64,
}

#[derive(Deserialize)]
struct WireMessage {
    id: i64,
    session_id: String,
    message: String,
    message_type: String,
    created_at: i64,
}

impl From<WireSession> for CoordinationSession {
    fn from(w: WireSession) -> Self {
        Self {
            session_id: w.session_id,
            repo_path: String::new(), // relay doesn't expose repo_path
            branch: w.branch,
            description: w.description,
            started_at: w.started_at,
            last_heartbeat: w.last_heartbeat,
        }
    }
}

impl From<WireMessage> for CoordinationMessage {
    fn from(w: WireMessage) -> Self {
        Self {
            id: w.id,
            session_id: w.session_id,
            repo_path: String::new(), // relay scopes by project_id, not repo_path
            message: w.message,
            message_type: w.message_type,
            created_at: w.created_at,
        }
    }
}
