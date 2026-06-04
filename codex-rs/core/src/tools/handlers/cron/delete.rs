use codex_scheduling::TaskId;
use codex_scheduling::os_cron;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::cron_spec::CRON_DELETE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::CronDeleteArgs;
use super::CronDeleteResponse;

pub struct CronDeleteHandler {
    pub(crate) spec: ToolSpec,
}

impl CronDeleteHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CronDeleteHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_DELETE_TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
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

        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )))
    }
}

impl CoreToolRuntime for CronDeleteHandler {}
