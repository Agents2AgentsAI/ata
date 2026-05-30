use codex_scheduling::TaskId;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::cron_session_spec::CRON_SESSION_DELETE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::CronSessionDeleteArgs;
use super::CronSessionDeleteResponse;

pub struct CronSessionDeleteHandler {
    pub(crate) spec: ToolSpec,
}

impl CronSessionDeleteHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CronSessionDeleteHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_SESSION_DELETE_TOOL_NAME)
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
        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )))
    }
}

impl CoreToolRuntime for CronSessionDeleteHandler {}
