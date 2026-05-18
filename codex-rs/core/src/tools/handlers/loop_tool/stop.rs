use chrono::Utc;
use codex_scheduling::TaskId;
use codex_scheduling::TaskStatus;
use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::loop_tool_spec::LOOP_STOP_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::LoopStopArgs;
use super::LoopStopResponse;

pub struct LoopStopHandler;

impl ToolHandler for LoopStopHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_STOP_TOOL_NAME)
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
                    "loop_stop handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: LoopStopArgs = parse_arguments(&arguments)?;
        let task_id = TaskId::from(args.task_id);

        let runtime = session.loop_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let aborted = runtime.abort(&task_id);
        if aborted {
            // Stopping a loop via this tool is a graceful end (the agent
            // decided the loop's job is done), not a forced kill. Surface
            // it as Completed in `/scheduling` so the status reads honestly.
            runtime
                .registry
                .mark_terminal(&task_id, TaskStatus::Completed, Utc::now());
            let iteration_count = runtime
                .registry
                .list()
                .into_iter()
                .find(|t| t.id == task_id)
                .map(|t| t.iteration_count)
                .unwrap_or(0);
            tracing::info!(
                target: "codex_scheduling::loop",
                task_id = %task_id,
                iteration_count = iteration_count,
                "loop.stopped"
            );
            session.persist_scheduling_state();
        }
        let response = LoopStopResponse { stopped: aborted };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "loop_stop response serialization failed: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
