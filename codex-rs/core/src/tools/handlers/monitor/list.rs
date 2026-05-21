use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::monitor_spec::MONITOR_LIST_TOOL_NAME;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::MonitorListResponse;
use super::MonitorSummary;

pub struct MonitorListHandler;

impl ToolHandler for MonitorListHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_LIST_TOOL_NAME)
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
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
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
