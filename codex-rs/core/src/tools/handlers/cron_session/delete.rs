use codex_scheduling::TaskId;
use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::cron_session_spec::CRON_SESSION_DELETE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::CronSessionDeleteArgs;
use super::CronSessionDeleteResponse;

pub struct CronSessionDeleteHandler;

impl ToolHandler for CronSessionDeleteHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_SESSION_DELETE_TOOL_NAME)
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "cron_delete_session handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CronSessionDeleteArgs = parse_arguments(&arguments)?;

        let registry = session.cron_registry().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let task_id = TaskId::from(args.task_id);
        let deleted = registry.remove(&task_id).is_some();
        if deleted {
            tracing::info!(
                target: "codex_scheduling::cron",
                task_id = %task_id,
                "cron.deleted (in-session)"
            );
            session.persist_scheduling_state();
        }
        let response = CronSessionDeleteResponse { deleted };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_delete_session response serialization failed: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
