use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

use super::parse_arguments;
use crate::function_tool::FunctionCallError;
use crate::git_info::get_git_repo_root;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

const RATE_LIMIT_SECS: i64 = 2;

#[derive(Deserialize)]
struct Args {
    message: String,
    message_type: Option<String>,
    #[cfg_attr(not(feature = "relay"), allow(dead_code))]
    recipient: Option<String>,
}

pub struct PlusToolHandler {
    last_post_at: AtomicI64,
}

impl PlusToolHandler {
    pub fn new() -> Self {
        Self {
            last_post_at: AtomicI64::new(0),
        }
    }
}

#[async_trait]
impl ToolHandler for PlusToolHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
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
        let args: Args = parse_arguments(&arguments)?;

        let coord_db = match session.services.plus.db() {
            Some(db) => Arc::clone(db),
            None => {
                return Ok(FunctionToolOutput::from_text(
                    "Coordination is not enabled for this session.".to_string(),
                    Some(false),
                ));
            }
        };

        let now = chrono::Utc::now().timestamp();
        let last = self.last_post_at.load(Ordering::Relaxed);
        if now - last < RATE_LIMIT_SECS {
            let wait = RATE_LIMIT_SECS - (now - last);
            return Ok(FunctionToolOutput::from_text(
                format!("Rate limited — wait {wait} seconds before posting again."),
                Some(false),
            ));
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
                #[cfg(feature = "relay")]
                if let Some(relay) = session.services.plus.relay() {
                    let _ = relay
                        .post_message(
                            &session_id,
                            &args.message,
                            args.message_type.as_deref(),
                            args.recipient.as_deref(),
                        )
                        .await;
                }
                Ok(FunctionToolOutput::from_text(
                    "Message posted to coordination channel.".to_string(),
                    Some(true),
                ))
            }
            Err(e) => {
                warn!("plus_tool failed: {e}");
                Ok(FunctionToolOutput::from_text(
                    format!("Failed to post message: {e}"),
                    Some(false),
                ))
            }
        }
    }
}
