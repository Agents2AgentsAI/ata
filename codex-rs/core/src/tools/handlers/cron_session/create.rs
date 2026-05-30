use chrono::Utc;
use codex_scheduling::CronJob;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::cron_session_spec::CRON_SESSION_CREATE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::CronSessionCreateArgs;
use super::CronSessionCreateResponse;

pub struct CronSessionCreateHandler {
    pub(crate) spec: ToolSpec,
}

impl CronSessionCreateHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CronSessionCreateHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_SESSION_CREATE_TOOL_NAME)
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
                    "cron_create_session handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CronSessionCreateArgs = parse_arguments(&arguments)?;

        let registry = session.cron_registry().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let cron_expr = args.cron_expr.clone();
        let background = args.background;
        let max_firings = args.max_firings;
        let until = match args.until.as_deref() {
            None => None,
            Some(s) => Some(
                chrono::DateTime::parse_from_rfc3339(s)
                    .map_err(|err| {
                        FunctionCallError::RespondToModel(format!(
                            "cron_create_session: `until` must be RFC3339 (e.g. \"2026-05-16T17:00:00Z\"): {err}"
                        ))
                    })?
                    .with_timezone(&Utc),
            ),
        };
        let timezone = args.timezone.clone();

        let job = CronJob::new_with_full_options(
            args.cron_expr,
            args.prompt,
            background,
            max_firings,
            until,
            timezone.clone(),
        )
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_create_session rejected: {err}"))
        })?
        .with_name(args.name);

        let now = Utc::now();
        let task_id = registry.insert(job, now);
        tracing::info!(
            target: "codex_scheduling::cron",
            task_id = %task_id,
            cron_expr = %cron_expr,
            background = background,
            max_firings = ?max_firings,
            until = ?until,
            timezone = ?timezone,
            "cron.created (in-session)"
        );
        session.persist_scheduling_state();

        let next_fire_at = registry
            .list()
            .into_iter()
            .find(|j| j.id == task_id)
            .and_then(|j| j.next_fire_at)
            .map(|t| t.to_rfc3339());

        let response = CronSessionCreateResponse {
            task_id: task_id.to_string(),
            next_fire_at,
        };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_create_session response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            body,
            Some(true),
        )))
    }
}

impl CoreToolRuntime for CronSessionCreateHandler {}
