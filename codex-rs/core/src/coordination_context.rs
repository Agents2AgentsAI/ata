use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use codex_coordination::CoordinationDb;
use codex_coordination::CoordinationMessage;
use codex_coordination::CoordinationSession;
use codex_protocol::models::ContentItem;
use codex_protocol::models::DeveloperInstructions;
use codex_protocol::models::ResponseItem;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::git_info::get_git_repo_root;

const INSTRUCTIONS: &str = include_str!("../templates/coordination/instructions.md");

// ---------------------------------------------------------------------------
// Handle — opaque coordination lifecycle manager
// ---------------------------------------------------------------------------

/// Session-scoped coordination handle.  All methods are no-ops when
/// coordination is disabled, so callers never need feature-flag checks.
pub(crate) struct Handle {
    inner: Option<HandleInner>,
}

struct HandleInner {
    db: Arc<CoordinationDb>,
    cancel: CancellationToken,
}

impl Handle {
    /// Create a handle that is unconditionally disabled (used in tests).
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Initialize coordination: open DB, register session, start heartbeat.
    /// Returns a disabled handle if anything goes wrong.
    pub async fn init(codex_home: &Path, cwd: &Path, session_id: &str) -> Self {
        let cancel = CancellationToken::new();
        let db = match CoordinationDb::open(codex_home).await {
            Ok(db) => Arc::new(db),
            Err(e) => {
                warn!("failed to open coordination db: {e}");
                return Self::disabled();
            }
        };
        db.prune_old().await;

        let repo_path = resolve_repo_path(cwd).await;

        let branch = tokio::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(cwd)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        if let Err(e) = db
            .register_session(session_id, &repo_path, branch.as_deref(), None)
            .await
        {
            warn!("failed to register coordination session: {e}");
        }

        // Heartbeat loop — cancelled via the stored token.
        let hb_db = Arc::clone(&db);
        let hb_sid = session_id.to_string();
        let hb_cancel = cancel.child_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(
                codex_coordination::HEARTBEAT_INTERVAL_SECS,
            ));
            loop {
                tokio::select! {
                    _ = hb_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        if let Err(e) = hb_db.heartbeat(&hb_sid).await {
                            warn!("coordination heartbeat failed: {e}");
                        }
                    }
                }
            }
        });

        Self {
            inner: Some(HandleInner { db, cancel }),
        }
    }

    /// Borrow the underlying DB (used by `team_post` tool handler).
    pub fn db(&self) -> Option<&Arc<CoordinationDb>> {
        self.inner.as_ref().map(|i| &i.db)
    }

    /// Developer instructions to inject into the initial context.
    pub fn developer_instructions(&self) -> Option<ResponseItem> {
        self.inner
            .as_ref()
            .map(|_| DeveloperInstructions::new(INSTRUCTIONS).into())
    }

    /// Stop heartbeat and deregister the session.
    pub async fn shutdown(&self, session_id: &str) {
        if let Some(ref inner) = self.inner {
            inner.cancel.cancel();
            if let Err(e) = inner.db.deregister_session(session_id).await {
                warn!("failed to deregister coordination session: {e}");
            }
        }
    }

    /// Update session description from the user's first prompt.
    pub async fn update_description(
        &self,
        session_id: &str,
        input: &[codex_protocol::user_input::UserInput],
    ) {
        let Some(ref inner) = self.inner else {
            return;
        };
        let desc: String = input
            .iter()
            .filter_map(|ui| match ui {
                codex_protocol::user_input::UserInput::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        if !desc.is_empty() {
            let truncated: String = desc.chars().take(200).collect();
            if let Err(e) = inner.db.update_description(session_id, &truncated).await {
                warn!("failed to update coordination session description: {e}");
            }
        }
    }

    /// Build coordination context if peer state changed. Returns `None` when
    /// coordination is disabled or nothing changed since the last call.
    pub async fn build_if_changed(
        &self,
        tracker: &mut CoordinationTracker,
        session_id: &str,
        cwd: &Path,
    ) -> Option<ResponseItem> {
        let inner = self.inner.as_ref()?;
        let repo_path = resolve_repo_path(cwd).await;

        let peers = match inner.db.active_sessions(&repo_path).await {
            Ok(p) => p,
            Err(e) => {
                warn!("coordination active_sessions query failed: {e}");
                return None;
            }
        };
        let messages = match inner.db.recent_messages(&repo_path).await {
            Ok(m) => m,
            Err(e) => {
                warn!("coordination recent_messages query failed: {e}");
                return None;
            }
        };

        // Fingerprint: sorted peer IDs (excluding self) + latest peer message ID.
        let mut cur_peer_ids: Vec<String> = peers
            .iter()
            .filter(|s| s.session_id != session_id)
            .map(|s| s.session_id.clone())
            .collect();
        cur_peer_ids.sort();
        let cur_latest_msg_id = messages
            .iter()
            .filter(|m| m.session_id != session_id)
            .map(|m| m.id)
            .max()
            .unwrap_or(-1);

        if !tracker.first
            && cur_peer_ids == tracker.peer_ids
            && cur_latest_msg_id == tracker.latest_msg_id
        {
            return None;
        }

        tracing::debug!(
            peers = cur_peer_ids.len(),
            latest_peer_msg_id = cur_latest_msg_id,
            "injecting coordination context (changed)"
        );
        tracker.first = false;
        tracker.peer_ids = cur_peer_ids;
        tracker.latest_msg_id = cur_latest_msg_id;

        let ctx = CoordinationContext {
            session_id: session_id.to_string(),
            peer_sessions: peers,
            recent_messages: messages,
        };
        Some(ctx.into())
    }
}

// ---------------------------------------------------------------------------
// Per-turn change-detection tracker
// ---------------------------------------------------------------------------

pub(crate) struct CoordinationTracker {
    first: bool,
    peer_ids: Vec<String>,
    latest_msg_id: i64,
}

impl CoordinationTracker {
    pub fn new() -> Self {
        Self {
            first: true,
            peer_ids: Vec::new(),
            latest_msg_id: -1,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn resolve_repo_path(cwd: &Path) -> String {
    crate::git_info::get_git_common_dir(cwd)
        .await
        .or_else(|| get_git_repo_root(cwd))
        .unwrap_or_else(|| cwd.to_path_buf())
        .to_string_lossy()
        .to_string()
}

// ---------------------------------------------------------------------------
// CoordinationContext (XML serialization + ResponseItem conversion)
// ---------------------------------------------------------------------------

struct CoordinationContext {
    session_id: String,
    peer_sessions: Vec<CoordinationSession>,
    recent_messages: Vec<CoordinationMessage>,
}

impl CoordinationContext {
    fn serialize_to_xml(&self) -> String {
        let mut lines = vec!["<coordination>".to_string()];

        let peers: Vec<_> = self
            .peer_sessions
            .iter()
            .filter(|s| s.session_id != self.session_id)
            .collect();

        let mut peer_labels: HashMap<&str, String> = HashMap::new();
        for (i, p) in peers.iter().enumerate() {
            peer_labels.insert(&p.session_id, format!("peer-{}", i + 1));
        }

        let mut latest_msg_by_peer: HashMap<&str, &str> = HashMap::new();
        for m in &self.recent_messages {
            if m.session_id != self.session_id {
                latest_msg_by_peer.insert(&m.session_id, &m.message);
            }
        }

        if peers.is_empty() {
            lines.push("  <peers>none</peers>".to_string());
        } else {
            lines.push("  <peers>".to_string());
            for p in &peers {
                let label = &peer_labels[p.session_id.as_str()];
                let branch = p.branch.as_deref().unwrap_or("unknown");
                let desc = p.description.as_deref().unwrap_or("no description");
                if let Some(latest) = latest_msg_by_peer.get(p.session_id.as_str()) {
                    let summary: String = latest.chars().take(120).collect();
                    lines.push(format!(
                        "    <peer id=\"{label}\" branch=\"{branch}\" task=\"{desc}\" latest=\"{summary}\" />"
                    ));
                } else {
                    lines.push(format!(
                        "    <peer id=\"{label}\" branch=\"{branch}\" task=\"{desc}\" />"
                    ));
                }
            }
            lines.push("  </peers>".to_string());
        }

        let peer_messages: Vec<_> = self
            .recent_messages
            .iter()
            .filter(|m| m.session_id != self.session_id)
            .collect();

        if peer_messages.is_empty() {
            lines.push("  <messages>none</messages>".to_string());
        } else {
            lines.push("  <messages>".to_string());
            for m in &peer_messages {
                let from_label = peer_labels
                    .get(m.session_id.as_str())
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                lines.push(format!(
                    "    <msg from=\"{from_label}\">{}</msg>",
                    m.message
                ));
            }
            lines.push("  </messages>".to_string());
        }

        lines.push("</coordination>".to_string());
        lines.join("\n")
    }
}

impl From<CoordinationContext> for ResponseItem {
    fn from(ctx: CoordinationContext) -> Self {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: ctx.serialize_to_xml(),
            }],
            end_turn: None,
            phase: None,
        }
    }
}
