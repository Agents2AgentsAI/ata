use chrono::Utc;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::Submission;
use codex_protocol::user_input::UserInput;
use codex_scheduling::LoopTask;
use codex_scheduling::TaskId;
use codex_tools::ToolName;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::loop_tool_spec::LOOP_START_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::LoopStartArgs;
use super::LoopStartResponse;

const MIN_INTERVAL_SECONDS: u64 = 5;

#[derive(Debug, Clone, Copy)]
enum LoopMode {
    Fixed(Duration),
    Dynamic(Duration),
}

pub struct LoopStartHandler;

impl ToolHandler for LoopStartHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_START_TOOL_NAME)
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
                    "loop_start handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: LoopStartArgs = parse_arguments(&arguments)?;
        if args.prompt.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "loop_start requires a non-empty prompt".to_string(),
            ));
        }

        // Mode selection: fixed (interval_seconds) vs dynamic
        // (initial_delay_seconds). Exactly one must be set.
        let mode = match (args.interval_seconds, args.initial_delay_seconds) {
            (Some(_), Some(_)) => {
                return Err(FunctionCallError::RespondToModel(
                    "loop_start: pass either `interval_seconds` (fixed mode) OR `initial_delay_seconds` (dynamic mode), not both".to_string(),
                ));
            }
            (None, None) => {
                return Err(FunctionCallError::RespondToModel(
                    "loop_start: must pass either `interval_seconds` (fixed mode) or `initial_delay_seconds` (dynamic mode)".to_string(),
                ));
            }
            (Some(secs), None) => {
                if secs < MIN_INTERVAL_SECONDS {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "loop_start requires interval_seconds >= {MIN_INTERVAL_SECONDS}"
                    )));
                }
                LoopMode::Fixed(Duration::from_secs(secs))
            }
            (None, Some(secs)) => {
                if secs < MIN_INTERVAL_SECONDS {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "loop_start requires initial_delay_seconds >= {MIN_INTERVAL_SECONDS}"
                    )));
                }
                LoopMode::Dynamic(Duration::from_secs(secs))
            }
        };

        let runtime = session.loop_runtime().cloned().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let task = match mode {
            LoopMode::Fixed(interval) => {
                LoopTask::new_fixed_with_background(args.prompt.clone(), interval, args.background)
            }
            LoopMode::Dynamic(_) => {
                let mut t = LoopTask::new_dynamic(args.prompt.clone());
                t.background = args.background;
                t
            }
        };
        let task_id = runtime.registry.insert(task);
        match mode {
            LoopMode::Fixed(interval) => tracing::info!(
                target: "codex_scheduling::loop",
                task_id = %task_id,
                mode = "fixed",
                interval_seconds = interval.as_secs(),
                background = args.background,
                "loop.created"
            ),
            LoopMode::Dynamic(initial) => tracing::info!(
                target: "codex_scheduling::loop",
                task_id = %task_id,
                mode = "dynamic",
                initial_delay_seconds = initial.as_secs(),
                background = args.background,
                "loop.created"
            ),
        };
        session.persist_scheduling_state();

        let tx_sub = session.submission_tx();
        let registry = runtime.registry.clone();
        let task_id_for_task = task_id.clone();
        let prompt = args.prompt;
        let background = args.background;

        let join_handle = match mode {
            LoopMode::Fixed(interval) => tokio::spawn(async move {
                run_loop(
                    task_id_for_task,
                    prompt,
                    interval,
                    background,
                    registry,
                    tx_sub,
                )
                .await;
            }),
            LoopMode::Dynamic(initial_delay) => {
                // Seed the first wake-up so the dynamic loop's polling
                // task knows when to fire iteration #1.
                if let Ok(initial_chrono) = chrono::Duration::from_std(initial_delay) {
                    runtime
                        .registry
                        .set_next_wakeup(&task_id, Utc::now() + initial_chrono);
                }
                tokio::spawn(async move {
                    run_loop_dynamic(task_id_for_task, background, registry, tx_sub).await;
                })
            }
        };
        runtime.store_handle(task_id.clone(), join_handle.abort_handle());

        let response = LoopStartResponse {
            task_id: task_id.to_string(),
        };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "loop_start response serialization failed: {err}"
            ))
        })?;
        Ok(FunctionToolOutput::from_text(body, Some(true)))
    }
}

pub(crate) async fn run_loop(
    task_id: TaskId,
    prompt: String,
    interval: Duration,
    background: bool,
    registry: Arc<codex_scheduling::LoopRegistry>,
    tx_sub: async_channel::Sender<Submission>,
) {
    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick so we don't fire at start time.
    tick.tick().await;
    // Record when the first real fire is expected so the `/scheduling` panel
    // can show a countdown ("next in 58s") instead of a static "every 60s".
    if let Ok(interval_chrono) = chrono::Duration::from_std(interval) {
        registry.set_next_wakeup(&task_id, Utc::now() + interval_chrono);
    }
    loop {
        tick.tick().await;
        // If `loop_stop` already marked the task terminal but the tokio
        // abort hasn't unwound this task yet (or the interval is in
        // burst-catch-up mode), don't send another submission.
        if registry.status(&task_id).is_none_or(|s| s.is_terminal()) {
            tracing::debug!(
                target: "codex_scheduling::loop",
                task_id = %task_id,
                reason = "task_terminal",
                "loop.fire_dropped"
            );
            return;
        }
        let now = Utc::now();
        registry.record_iteration(&task_id, now);
        let iteration_count = registry
            .list()
            .into_iter()
            .find(|t| t.id == task_id)
            .map(|t| t.iteration_count)
            .unwrap_or(0);
        tracing::info!(
            target: "codex_scheduling::loop",
            task_id = %task_id,
            iteration_count = iteration_count,
            interval_seconds = interval.as_secs(),
            "loop.fired"
        );
        let op = Op::UserInput {
            items: vec![UserInput::Text {
                text: format!("[loop {task_id}] {prompt}"),
                text_elements: Vec::new(),
            }],
            environments: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        };
        // Encode task_id + background flag in the submission id.
        // `loop__` / `loopbg__` lets `submission_loop` drop queued firings
        // after `loop_stop` (the `__<task_id>__` part) and the TUI hide the
        // agent's natural-language reply for background firings (the prefix).
        // `__` is unambiguous because UUIDs never contain underscores.
        let prefix = if background { "loopbg" } else { "loop" };
        let sub = Submission {
            id: format!("{prefix}__{task_id}__{}", Uuid::now_v7()),
            op,
            trace: None,
        };
        if tx_sub.send(sub).await.is_err() {
            return; // submission channel closed
        }
    }
}

/// Dynamic-pacing loop runner. Polls the registry's `next_wakeup_at`
/// field every 500ms; when it crosses the current time, fires one
/// iteration and clears the wake-up so the agent must call
/// `loop_wakeup` to schedule the next one. If the agent never schedules,
/// the loop sits idle until `loop_stop` or session resume.
pub(crate) async fn run_loop_dynamic(
    task_id: TaskId,
    background: bool,
    registry: Arc<codex_scheduling::LoopRegistry>,
    tx_sub: async_channel::Sender<Submission>,
) {
    const POLL_INTERVAL: Duration = Duration::from_millis(500);
    loop {
        // Bail if loop_stop or removal happened.
        let snapshot = registry.list().into_iter().find(|t| t.id == task_id);
        let Some(task) = snapshot else {
            return; // removed
        };
        if task.status.is_terminal() {
            return;
        }
        let Some(wakeup) = task.next_wakeup_at else {
            // No pending wake-up. Wait for the agent to call loop_wakeup
            // (or for loop_stop to terminate us).
            tokio::time::sleep(POLL_INTERVAL).await;
            continue;
        };
        let now = Utc::now();
        if now < wakeup {
            // Sleep until the scheduled time (capped at POLL_INTERVAL so
            // stop signals are detected within ~500ms).
            let remaining = (wakeup - now).to_std().unwrap_or(POLL_INTERVAL);
            tokio::time::sleep(remaining.min(POLL_INTERVAL)).await;
            continue;
        }

        // It's time to fire. Clear the wake-up first so we don't refire
        // before the agent has had a chance to set a new one (or stop).
        registry.clear_next_wakeup(&task_id);
        registry.record_iteration(&task_id, now);
        let prompt = task.prompt.clone();
        let iteration_count = registry
            .list()
            .into_iter()
            .find(|t| t.id == task_id)
            .map(|t| t.iteration_count)
            .unwrap_or(0);
        tracing::info!(
            target: "codex_scheduling::loop",
            task_id = %task_id,
            iteration_count = iteration_count,
            mode = "dynamic",
            "loop.fired"
        );
        let op = Op::UserInput {
            items: vec![UserInput::Text {
                text: format!("[loop {task_id}] {prompt}"),
                text_elements: Vec::new(),
            }],
            environments: None,
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        };
        let prefix = if background { "loopbg" } else { "loop" };
        let sub = Submission {
            id: format!("{prefix}__{task_id}__{}", Uuid::now_v7()),
            op,
            trace: None,
        };
        if tx_sub.send(sub).await.is_err() {
            return; // submission channel closed
        }
    }
}
