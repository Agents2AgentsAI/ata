use codex_scheduling::CronJob;
use codex_scheduling::os_cron;
use codex_tools::ToolName;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::cron_spec::CRON_CREATE_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::CronCreateArgs;
use super::CronCreateResponse;

pub struct CronCreateHandler;

impl ToolHandler for CronCreateHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(CRON_CREATE_TOOL_NAME)
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
                    "cron_create handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CronCreateArgs = parse_arguments(&arguments)?;

        // The CronRegistry handle is now used purely as a feature-flag check.
        // OS cron has no in-memory state; persistence is the user's crontab.
        if session.cron_registry().is_none() {
            return Err(FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            ));
        }

        // The tool spec documents 5-field POSIX cron (no seconds column) but
        // the underlying `cron` crate validates 6-field expressions. Normalize
        // by prepending `0 ` so seconds=0 when the caller sent 5 fields.
        let normalized_expr = match args.cron_expr.split_whitespace().count() {
            5 => format!("0 {}", args.cron_expr),
            _ => args.cron_expr.clone(),
        };

        // Validate the cron expression and confirm OS cron can express it.
        let _five_field = os_cron::six_field_to_five(&normalized_expr).map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_create rejected: {err}"))
        })?;

        let job = CronJob::new(normalized_expr.clone(), args.prompt).map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_create rejected: {err}"))
        })?;

        os_cron::insert(&job).map_err(|err| {
            FunctionCallError::RespondToModel(format!("cron_create failed to write crontab: {err}"))
        })?;

        let next_fire_at =
            os_cron::next_fire_after_now(&normalized_expr).map(|t| t.to_rfc3339());

        let log_path = os_cron::data_dir()
            .map(|d| d.join(format!("{}.log", job.id)).display().to_string())
            .unwrap_or_default();

        tracing::info!(
            target: "codex_scheduling::cron",
            task_id = %job.id,
            cron_expr = %args.cron_expr,
            "cron.created (os-cron)"
        );

        let response = CronCreateResponse {
            task_id: job.id.to_string(),
            next_fire_at,
            log_path,
        };

        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "cron_create response serialization failed: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}
