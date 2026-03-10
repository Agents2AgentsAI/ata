use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

use super::parse_arguments;
use crate::function_tool::FunctionCallError;
use crate::git_info::get_git_repo_root;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::models::FunctionCallOutputBody;

/// Minimum seconds between consecutive team_post calls.
const RATE_LIMIT_SECS: i64 = 30;

#[derive(Deserialize)]
struct TeamPostArgs {
    message: String,
    message_type: Option<String>,
}

pub struct TeamPostHandler {
    /// Unix timestamp of the last successful post (0 = never).
    last_post_at: AtomicI64,
}

impl TeamPostHandler {
    pub fn new() -> Self {
        Self {
            last_post_at: AtomicI64::new(0),
        }
    }
}

#[async_trait]
impl ToolHandler for TeamPostHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "team_post requires Function payload".to_string(),
                ));
            }
        };
        let args: TeamPostArgs = parse_arguments(&arguments)?;

        let coord_db = match session.services.coordination.db() {
            Some(db) => Arc::clone(db),
            None => {
                return Ok(ToolOutput::Function {
                    body: FunctionCallOutputBody::Text(
                        "Coordination is not enabled for this session.".to_string(),
                    ),
                    success: Some(false),
                });
            }
        };

        // Rate limiting: at most one post per RATE_LIMIT_SECS.
        let now = chrono::Utc::now().timestamp();
        let last = self.last_post_at.load(Ordering::Relaxed);
        if now - last < RATE_LIMIT_SECS {
            let wait = RATE_LIMIT_SECS - (now - last);
            return Ok(ToolOutput::Function {
                body: FunctionCallOutputBody::Text(format!(
                    "Rate limited — wait {wait} seconds before posting again."
                )),
                success: Some(false),
            });
        }

        let session_id = session.conversation_id.to_string();
        let repo_path = crate::git_info::get_git_common_dir(&turn.cwd)
            .await
            .or_else(|| get_git_repo_root(&turn.cwd))
            .unwrap_or_else(|| turn.cwd.clone())
            .to_string_lossy()
            .to_string();

        match coord_db
            .post_message(
                &session_id,
                &repo_path,
                &args.message,
                args.message_type.as_deref(),
            )
            .await
        {
            Ok(()) => {
                self.last_post_at.store(now, Ordering::Relaxed);
                // Also post to relay (best-effort, fire-and-forget).
                #[cfg(feature = "relay")]
                if let Some(relay) = session.services.coordination.relay() {
                    let _ = relay
                        .post_message(&session_id, &args.message, args.message_type.as_deref())
                        .await;
                }
                Ok(ToolOutput::Function {
                    body: FunctionCallOutputBody::Text(
                        "Message posted to coordination channel.".to_string(),
                    ),
                    success: Some(true),
                })
            }
            Err(e) => {
                warn!("team_post failed: {e}");
                Ok(ToolOutput::Function {
                    body: FunctionCallOutputBody::Text(format!("Failed to post message: {e}")),
                    success: Some(false),
                })
            }
        }
    }
}
