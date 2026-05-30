use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::cron_session_spec::CRON_SESSION_LIST_TOOL_NAME;
use crate::tools::registry::CoreToolRuntime;

use super::CronSessionJobSummary;
use super::CronSessionListResponse;

pub struct CronSessionListHandler {
    pub(crate) spec: ToolSpec,
}

impl CronSessionListHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CronSessionListHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_SESSION_LIST_TOOL_NAME)
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

        match payload {
            ToolPayload::Function { .. } => {}
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "cron_list_session handler received unsupported payload".to_string(),
                ));
            }
        }

        let registry = session.cron_registry().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let jobs = registry
            .list()
            .into_iter()
            .map(|j| CronSessionJobSummary {
                task_id: j.id.to_string(),
                cron_expr: j.cron_expr,
                prompt: j.prompt,
                status: format!("{:?}", j.status),
                next_fire_at: j.next_fire_at.map(|t| t.to_rfc3339()),
                last_fired_at: j.last_fired_at.map(|t| t.to_rfc3339()),
                fire_count: j.fire_count,
            })
            .collect();

        let response = CronSessionListResponse { jobs };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_list_session response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(body, Some(true))))
    }
}

impl CoreToolRuntime for CronSessionListHandler {}
