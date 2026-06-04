use codex_scheduling::os_cron;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::cron_spec::CRON_LIST_TOOL_NAME;
use crate::tools::registry::CoreToolRuntime;

use super::CronJobSummary;
use super::CronListResponse;

pub struct CronListHandler {
    pub(crate) spec: ToolSpec,
}

impl CronListHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CronListHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_LIST_TOOL_NAME)
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
                    "cron_list handler received unsupported payload".to_string(),
                ));
            }
        }

        if session.cron_registry().is_none() {
            return Err(FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            ));
        }

        let entries = os_cron::list().map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_list failed to read crontab: {err}"))
        })?;

        let jobs = entries
            .into_iter()
            .map(|e| {
                let next_fire_at = os_cron::next_fire_after_now_five_field(&e.cron_expr_five_field)
                    .map(|t| t.to_rfc3339());
                let stats = os_cron::fire_stats(&e.task_id);
                CronJobSummary {
                    task_id: e.task_id.to_string(),
                    cron_expr: e.cron_expr_five_field,
                    prompt: e.prompt,
                    next_fire_at,
                    log_path: e.log_path.display().to_string(),
                    created_at: e.created_at.map(|t| t.to_rfc3339()),
                    fire_count: stats.fire_count,
                    last_fired_at: stats.last_fired_at.map(|t| t.to_rfc3339()),
                }
            })
            .collect();

        let response = CronListResponse { jobs };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_list response serialization failed: {err}"
            ))
        })?;

        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )))
    }
}

impl CoreToolRuntime for CronListHandler {}
