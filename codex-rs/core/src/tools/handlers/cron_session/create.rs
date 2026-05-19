use chrono::Utc;
use codex_scheduling::CronJob;
use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::cron_session_spec::CRON_SESSION_CREATE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::CronSessionCreateArgs;
use super::CronSessionCreateResponse;

pub struct CronSessionCreateHandler;

impl ToolHandler for CronSessionCreateHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_SESSION_CREATE_TOOL_NAME)
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
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
