use codex_scheduling::TaskId;
use codex_scheduling::os_cron;
use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::cron_spec::CRON_DELETE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::CronDeleteArgs;
use super::CronDeleteResponse;

pub struct CronDeleteHandler;

impl ToolHandler for CronDeleteHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_DELETE_TOOL_NAME)
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
                    "cron_delete handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CronDeleteArgs = parse_arguments(&arguments)?;

        if session.cron_registry().is_none() {
            return Err(FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            ));
        }

        let task_id = TaskId::from(args.task_id);
        let deleted = os_cron::delete(&task_id).map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_delete failed: {err}"))
        })?;

        if deleted {
            tracing::info!(
                target: "codex_scheduling::cron",
                task_id = %task_id,
                "cron.deleted (os-cron)"
            );
        }

        let response = CronDeleteResponse { deleted };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_delete response serialization failed: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
