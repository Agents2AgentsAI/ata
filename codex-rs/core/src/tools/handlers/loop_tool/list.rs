use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::loop_tool_spec::LOOP_LIST_TOOL_NAME;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::LoopListResponse;
use super::LoopSummary;

pub struct LoopListHandler;

impl ToolHandler for LoopListHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_LIST_TOOL_NAME)
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
                    "loop_list handler received unsupported payload".to_string(),
                ));
            }
        }

        let runtime = session.loop_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let loops = runtime
            .registry
            .list()
            .into_iter()
            .map(|l| LoopSummary {
                task_id: l.id.to_string(),
                prompt: l.prompt,
                interval_seconds: l.interval.map(|d| d.as_secs()),
                status: format!("{:?}", l.status),
                last_iter_at: l.last_iter_at.map(|t| t.to_rfc3339()),
                iteration_count: l.iteration_count,
            })
            .collect();

        let response = LoopListResponse { loops };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "loop_list response serialization failed: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
