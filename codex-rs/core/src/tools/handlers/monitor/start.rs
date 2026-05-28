use chrono::Utc;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SchedulingMonitorOutputDeltaEvent;
use codex_protocol::protocol::Submission;
use codex_protocol::user_input::UserInput;
use codex_scheduling::MonitorTask;
use codex_scheduling::TaskId;
use codex_scheduling::TaskStatus;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use uuid::Uuid;

use crate::session::session::Session;

use crate::function_tool::FunctionCallError;
use crate::scheduling_runtime::MonitorRuntime;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::monitor_spec::MONITOR_START_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;

use super::MonitorStartArgs;
use super::MonitorStartResponse;

pub struct MonitorStartHandler {
    pub(crate) spec: ToolSpec,
}

impl MonitorStartHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for MonitorStartHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_START_TOOL_NAME)
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
                    "monitor_start handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: MonitorStartArgs = parse_arguments(&arguments)?;
        if args.command.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "monitor_start requires a non-empty command".to_string(),
            ));
        }

        let runtime = session.monitor_runtime().cloned().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        let task = MonitorTask::new_with_options(
            args.command.clone(),
            args.background,
            args.restart_on_resume,
        )
        .with_name(args.name);
        let task_id = runtime.registry.insert(task);
        tracing::info!(
            target: "codex_scheduling::monitor",
            task_id = %task_id,
            command = %args.command,
            background = args.background,
            restart_on_resume = args.restart_on_resume,
            "monitor.started"
        );
        session.persist_scheduling_state();

        // Create the broadcast channel **before** spawning the streaming
        // task so any `monitor_watch_for` call that races in immediately can
        // subscribe. Lines emitted before a subscriber attaches are also
        // captured in the registry's tail buffer, so the watcher catches up
        // by inspecting the tail before subscribing.
        let watch_tx = runtime.register_watcher_channel(task_id.clone());

        let tx_sub = session.submission_tx();
        let session_for_task = session;
        let registry = runtime.registry.clone();
        let runtime_for_task = runtime.clone();
        let task_id_for_task = task_id.clone();
        let command = args.command;

        let background = args.background;
        let join_handle = tokio::spawn(async move {
            run_monitor(
                task_id_for_task,
                command,
                registry,
                runtime_for_task,
                tx_sub,
                session_for_task,
                background,
                watch_tx,
            )
            .await;
        });
        runtime.store_handle(task_id.clone(), join_handle.abort_handle());

        let response = MonitorStartResponse {
            task_id: task_id.to_string(),
        };
        let body = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "monitor_start response serialization failed: {err}"
            ))
        })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(body, Some(true))))
    }
}

impl CoreToolRuntime for MonitorStartHandler {}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_monitor(
    task_id: TaskId,
    command: String,
    registry: Arc<codex_scheduling::MonitorRegistry>,
    runtime: Arc<MonitorRuntime>,
    tx_sub: async_channel::Sender<Submission>,
    session: Arc<Session>,
    background: bool,
    watch_tx: tokio::sync::broadcast::Sender<crate::scheduling_runtime::MonitorLine>,
) {
    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(err) => {
            registry.mark_terminal(&task_id, TaskStatus::Failed, Utc::now());
            runtime.drop_watcher_channel(&task_id);
            tracing::warn!(
                target: "codex_scheduling::monitor",
                task_id = %task_id,
                command = %command,
                error = %err,
                "monitor.spawn_failed"
            );
            emit_terminate_summary(
                &task_id,
                &command,
                TaskStatus::Failed,
                Vec::new(),
                &tx_sub,
                Some(format!("failed to spawn subprocess: {err}")),
            )
            .await;
            return;
        }
    };

    registry.mark_running(&task_id, Utc::now());

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            registry.mark_terminal(&task_id, TaskStatus::Failed, Utc::now());
            runtime.drop_watcher_channel(&task_id);
            emit_terminate_summary(
                &task_id,
                &command,
                TaskStatus::Failed,
                Vec::new(),
                &tx_sub,
                Some("subprocess stdout pipe was not captured".to_string()),
            )
            .await;
            return;
        }
    };
    let stderr = child.stderr.take();

    let mut stdout_reader = BufReader::new(stdout).lines();

    let stderr_handle = if let Some(stderr) = stderr {
        let registry_for_stderr = registry.clone();
        let task_id_for_stderr = task_id.clone();
        let session_for_stderr = session.clone();
        let watch_tx_for_stderr = watch_tx.clone();
        Some(tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                emit_line(
                    &task_id_for_stderr,
                    "stderr",
                    &line,
                    &registry_for_stderr,
                    &session_for_stderr,
                    background,
                    &watch_tx_for_stderr,
                )
                .await;
            }
        }))
    } else {
        None
    };

    loop {
        match stdout_reader.next_line().await {
            Ok(Some(line)) => {
                emit_line(
                    &task_id, "stdout", &line, &registry, &session, background, &watch_tx,
                )
                .await;
            }
            Ok(None) => break,
            Err(err) => {
                tracing::warn!("monitor stdout read error: {err}");
                break;
            }
        }
    }

    if let Some(h) = stderr_handle {
        let _ = h.await;
    }

    let status = match child.wait().await {
        Ok(status) if status.success() => TaskStatus::Completed,
        Ok(_) => TaskStatus::Failed,
        Err(_) => TaskStatus::Failed,
    };
    registry.mark_terminal(&task_id, status, Utc::now());
    // Phase 4: persist the final status so resume sees Completed/Failed
    // instead of the stale Running.
    session.persist_scheduling_state();
    // Drop the per-monitor broadcast channel. Active `monitor_watch_for`
    // receivers wake up with `RecvError::Closed` and report
    // `terminated_without_match` (or their own buffered match, if any).
    runtime.drop_watcher_channel(&task_id);

    let lines_emitted = registry
        .list()
        .into_iter()
        .find(|m| m.id == task_id)
        .map(|m| m.lines_emitted)
        .unwrap_or(0);
    tracing::info!(
        target: "codex_scheduling::monitor",
        task_id = %task_id,
        command = %command,
        status = ?status,
        lines_emitted = lines_emitted,
        "monitor.terminated"
    );

    let tail = registry.tail_snapshot(&task_id);
    emit_terminate_summary(&task_id, &command, status, tail, &tx_sub, None).await;
}

async fn emit_line(
    task_id: &TaskId,
    stream: &str,
    line: &str,
    registry: &Arc<codex_scheduling::MonitorRegistry>,
    session: &Arc<Session>,
    background: bool,
    watch_tx: &tokio::sync::broadcast::Sender<crate::scheduling_runtime::MonitorLine>,
) {
    // Always record the line so the `/scheduling` panel's `lines N` counter
    // climbs and the terminate-summary tail still has data. Only the
    // user-facing chat cell is gated by `background`.
    registry.record_line(task_id);
    registry.record_tail_line(task_id, format!("[{stream}] {line}"));
    // Broadcast to any active `monitor_watch_for` subscribers regardless of
    // `background` — watching is a tool-level concern, separate from chat
    // visibility. `send` errors only when there are no subscribers, which
    // is the common case; we ignore it.
    let _ = watch_tx.send((stream.to_string(), line.to_string()));
    if background {
        return;
    }

    let event = Event {
        id: format!("monitor-{}", Uuid::now_v7()),
        msg: EventMsg::SchedulingMonitorOutputDelta(SchedulingMonitorOutputDeltaEvent {
            task_id: task_id.to_string(),
            stream: stream.to_string(),
            line: line.to_string(),
        }),
    };
    // Route to the **root** session's event channel so output appears in the
    // user-facing chat instead of an invisible sub-agent's session. The
    // event is marked non-persisted in `rollout::policy`, so we deliver it
    // directly via the channel rather than going through `send_event_raw`.
    let _ = session.scheduling_event_tx().send(event).await;
}

async fn emit_terminate_summary(
    task_id: &TaskId,
    command: &str,
    status: TaskStatus,
    tail: Vec<String>,
    tx_sub: &async_channel::Sender<Submission>,
    error: Option<String>,
) {
    let status_text = match status {
        TaskStatus::Completed => "completed",
        TaskStatus::Failed => "failed",
        _ => "ended",
    };
    let mut body = format!("[monitor {task_id}] `{command}` {status_text}.");
    if let Some(err) = error.as_ref() {
        body.push_str(&format!("\nError: {err}"));
    }
    if !tail.is_empty() {
        body.push_str("\nLast output:\n");
        for line in tail {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let op = Op::UserInput {
        items: vec![UserInput::Text {
            text: body,
            text_elements: Vec::new(),
        }],
        environments: None,
        final_output_json_schema: None,
        responsesapi_client_metadata: None,
    };
    let sub = Submission {
        id: format!("monitor-{}", Uuid::now_v7()),
        op,
        trace: None,
    };
    let _ = tx_sub.send(sub).await;
}
