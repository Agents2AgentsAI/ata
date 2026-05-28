use codex_scheduling::TaskId;
use codex_scheduling::TaskStatus;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use std::time::Duration;
use tokio::time::Instant;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::monitor_spec::MONITOR_WAIT_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::MonitorWaitArgs;
use super::MonitorWaitResponse;

const POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct MonitorWaitHandler {
    pub(crate) spec: ToolSpec,
}

impl MonitorWaitHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for MonitorWaitHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_WAIT_TOOL_NAME)
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
                    "monitor_wait handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: MonitorWaitArgs = parse_arguments(&arguments)?;
        let task_id: TaskId = args.task_id.clone().into();
        // `timeout_seconds = None` means wait forever — the sub-agent's turn
        // stays alive as long as it takes the subprocess to terminate. The
        // tokio runtime keeps the future polled at no LLM cost since this
        // is pure Rust polling against an in-memory registry.
        let deadline = args
            .timeout_seconds
            .map(|secs| Instant::now() + Duration::from_secs(secs));

        let runtime = session.monitor_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let registry = runtime.registry.clone();

        // Confirm the monitor exists at all before we start polling.
        if !registry.list().into_iter().any(|m| m.id == task_id) {
            return Err(FunctionCallError::RespondToModel(format!(
                "monitor_wait: task_id `{}` not found",
                args.task_id
            )));
        }

        let (final_status, timed_out) = loop {
            let snapshot = registry.list();
            let Some(task) = snapshot.into_iter().find(|m| m.id == task_id) else {
                // Monitor was removed mid-wait (e.g. by monitor_stop).
                break (TaskStatus::Killed, false);
            };
            match task.status {
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Killed => {
                    break (task.status, false);
                }
                _ => {}
            }
            if let Some(d) = deadline
                && Instant::now() >= d
            {
                break (task.status, true);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        };

        let tail = registry.tail_snapshot(&task_id);
        let response = MonitorWaitResponse {
            status: if timed_out {
                "Running".to_string()
            } else {
                format!("{final_status:?}")
            },
            timed_out,
            tail,
        };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "monitor_wait response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(body, Some(true))))
    }
}

impl CoreToolRuntime for MonitorWaitHandler {}
