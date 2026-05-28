use chrono::Utc;
use codex_scheduling::TaskId;
use codex_scheduling::TaskStatus;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::monitor_spec::MONITOR_STOP_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::MonitorStopArgs;
use super::MonitorStopResponse;

pub struct MonitorStopHandler {
    pub(crate) spec: ToolSpec,
}

impl MonitorStopHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for MonitorStopHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_STOP_TOOL_NAME)
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
                    "monitor_stop handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: MonitorStopArgs = parse_arguments(&arguments)?;
        let task_id = TaskId::from(args.task_id);

        let runtime = session.monitor_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let aborted = runtime.abort(&task_id);
        if aborted {
            runtime
                .registry
                .mark_terminal(&task_id, TaskStatus::Killed, Utc::now());
            tracing::info!(
                target: "codex_scheduling::monitor",
                task_id = %task_id,
                "monitor.stopped"
            );
            session.persist_scheduling_state();
        }
        let response = MonitorStopResponse { stopped: aborted };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "monitor_stop response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(body, Some(true))))
    }
}

impl CoreToolRuntime for MonitorStopHandler {}
