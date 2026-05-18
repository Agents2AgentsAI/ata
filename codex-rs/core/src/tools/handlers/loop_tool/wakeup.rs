use chrono::Utc;
use codex_scheduling::TaskId;
use codex_tools::ToolName;
use std::time::Duration;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::loop_tool_spec::LOOP_WAKEUP_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::LoopWakeupArgs;
use super::LoopWakeupResponse;

const MIN_DELAY_SECONDS: u64 = 5;

pub struct LoopWakeupHandler;

impl ToolHandler for LoopWakeupHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_WAKEUP_TOOL_NAME)
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
                    "loop_wakeup handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: LoopWakeupArgs = parse_arguments(&arguments)?;
        if args.delay_seconds < MIN_DELAY_SECONDS {
            return Err(FunctionCallError::RespondToModel(format!(
                "loop_wakeup requires delay_seconds >= {MIN_DELAY_SECONDS}"
            )));
        }

        let task_id: TaskId = args.task_id.clone().into();
        let runtime = session.loop_runtime().cloned().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        // Confirm the loop exists and is dynamic.
        let task = runtime
            .registry
            .list()
            .into_iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "loop_wakeup: task_id `{}` not found",
                    args.task_id
                ))
            })?;
        if task.status.is_terminal() {
            return Err(FunctionCallError::RespondToModel(format!(
                "loop_wakeup: loop `{}` is already terminal",
                args.task_id
            )));
        }
        if !task.is_dynamic() {
            return Err(FunctionCallError::RespondToModel(format!(
                "loop_wakeup: loop `{}` is fixed-interval, not dynamic — cannot reschedule",
                args.task_id
            )));
        }

        let delay = Duration::from_secs(args.delay_seconds);
        let next_fire_at = Utc::now()
            + chrono::Duration::from_std(delay).map_err(|err| {
                FunctionCallError::RespondToModel(format!("loop_wakeup: invalid delay: {err}"))
            })?;
        runtime.registry.set_next_wakeup(&task_id, next_fire_at);
        if let Some(new_prompt) = args.prompt {
            runtime.registry.update_prompt(&task_id, new_prompt);
        }
        tracing::info!(
            target: "codex_scheduling::loop",
            task_id = %task_id,
            delay_seconds = args.delay_seconds,
            next_fire_at = %next_fire_at,
            "loop.wakeup_scheduled"
        );
        session.persist_scheduling_state();

        let response = LoopWakeupResponse {
            next_fire_at: next_fire_at.to_rfc3339(),
        };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "loop_wakeup response serialization failed: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
