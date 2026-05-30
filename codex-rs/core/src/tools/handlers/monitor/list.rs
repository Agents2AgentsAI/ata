use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::monitor_spec::MONITOR_LIST_TOOL_NAME;
use crate::tools::registry::CoreToolRuntime;

use super::MonitorListResponse;
use super::MonitorSummary;

pub struct MonitorListHandler {
    pub(crate) spec: ToolSpec,
}

impl MonitorListHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for MonitorListHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_LIST_TOOL_NAME)
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
                    "monitor_list handler received unsupported payload".to_string(),
                ));
            }
        }

        let runtime = session.monitor_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let monitors = runtime
            .registry
            .list()
            .into_iter()
            .map(|m| MonitorSummary {
                task_id: m.id.to_string(),
                command: m.command,
                status: format!("{:?}", m.status),
                started_at: m.started_at.map(|t| t.to_rfc3339()),
                stopped_at: m.stopped_at.map(|t| t.to_rfc3339()),
                lines_emitted: m.lines_emitted,
            })
            .collect();

        let response = MonitorListResponse { monitors };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "monitor_list response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(body, Some(true))))
    }
}

impl CoreToolRuntime for MonitorListHandler {}
