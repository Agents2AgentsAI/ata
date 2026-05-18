use codex_scheduling::TaskId;
use codex_tools::ToolName;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::monitor_spec::MONITOR_WATCH_FOR_TOOL_NAME;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

use super::MonitorWatchForArgs;
use super::MonitorWatchForMatch;
use super::MonitorWatchForResponse;

pub struct MonitorWatchForHandler;

impl ToolHandler for MonitorWatchForHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(MONITOR_WATCH_FOR_TOOL_NAME)
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
                    "monitor_watch_for handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: MonitorWatchForArgs = parse_arguments(&arguments)?;
        if args.pattern.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "monitor_watch_for requires a non-empty pattern".to_string(),
            ));
        }

        let task_id: TaskId = args.task_id.clone().into();
        let pattern = args.pattern;
        let all_matches = args.all_matches;
        let max_matches = args.max_matches;
        let deadline = args
            .timeout_seconds
            .map(|s| tokio::time::Instant::now() + Duration::from_secs(s));

        let runtime = session.monitor_runtime().ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "scheduling feature is not enabled in this session".to_string(),
            )
        })?;

        // Confirm the monitor exists at all.
        if !runtime.registry.list().into_iter().any(|m| m.id == task_id) {
            return Err(FunctionCallError::RespondToModel(format!(
                "monitor_watch_for: task_id `{}` not found",
                args.task_id
            )));
        }

        let mut collected: Vec<MonitorWatchForMatch> = Vec::new();

        // First, scan the tail buffer for already-emitted matches. Single-
        // match mode returns on the first hit; multi-match mode collects
        // every hit and keeps going.
        for tail_line in runtime.registry.tail_snapshot(&task_id) {
            let (stream, line_payload) = strip_stream_prefix(&tail_line);
            if line_payload.contains(&pattern) {
                tracing::info!(
                    target: "codex_scheduling::monitor",
                    task_id = %task_id,
                    pattern = %pattern,
                    source = "tail",
                    "monitor.watch_for_match"
                );
                collected.push(MonitorWatchForMatch {
                    matching_line: line_payload.to_string(),
                    stream: stream.to_string(),
                });
                if !all_matches {
                    return ok_response(build_response(collected, all_matches, false, false));
                }
                if max_matches.is_some_and(|cap| collected.len() as u64 >= cap) {
                    return ok_response(build_response(collected, all_matches, false, false));
                }
            }
        }

        // Subscribe to future lines. `None` means the monitor has already
        // terminated and dropped its broadcast.
        let Some(mut rx) = runtime.subscribe(&task_id) else {
            tracing::info!(
                target: "codex_scheduling::monitor",
                task_id = %task_id,
                pattern = %pattern,
                "monitor.watch_for_terminated"
            );
            return ok_response(build_response(collected, all_matches, true, false));
        };

        loop {
            let sleep_until = deadline.map(tokio::time::sleep_until);
            tokio::select! {
                biased;
                recv = rx.recv() => {
                    match recv {
                        Ok((stream, line)) => {
                            if line.contains(&pattern) {
                                tracing::info!(
                                    target: "codex_scheduling::monitor",
                                    task_id = %task_id,
                                    pattern = %pattern,
                                    source = "broadcast",
                                    "monitor.watch_for_match"
                                );
                                collected.push(MonitorWatchForMatch {
                                    matching_line: line,
                                    stream,
                                });
                                if !all_matches {
                                    return ok_response(build_response(collected, all_matches, false, false));
                                }
                                if max_matches.is_some_and(|cap| collected.len() as u64 >= cap) {
                                    return ok_response(build_response(collected, all_matches, false, false));
                                }
                            }
                        }
                        Err(RecvError::Closed) => {
                            tracing::info!(
                                target: "codex_scheduling::monitor",
                                task_id = %task_id,
                                pattern = %pattern,
                                "monitor.watch_for_terminated"
                            );
                            return ok_response(build_response(collected, all_matches, true, false));
                        }
                        Err(RecvError::Lagged(skipped)) => {
                            // The broadcast buffer overran us. Fall back to
                            // inspecting the tail once before continuing —
                            // catches any matches we slipped past.
                            tracing::warn!(
                                target: "codex_scheduling::monitor",
                                task_id = %task_id,
                                skipped,
                                "monitor.watch_for_lagged"
                            );
                            for tail_line in runtime.registry.tail_snapshot(&task_id) {
                                let (stream, line_payload) = strip_stream_prefix(&tail_line);
                                if line_payload.contains(&pattern) {
                                    collected.push(MonitorWatchForMatch {
                                        matching_line: line_payload.to_string(),
                                        stream: stream.to_string(),
                                    });
                                    if !all_matches {
                                        return ok_response(build_response(collected, all_matches, false, false));
                                    }
                                    if max_matches.is_some_and(|cap| collected.len() as u64 >= cap) {
                                        return ok_response(build_response(collected, all_matches, false, false));
                                    }
                                }
                            }
                        }
                    }
                }
                _ = async {
                    match sleep_until {
                        Some(s) => s.await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    tracing::info!(
                        target: "codex_scheduling::monitor",
                        task_id = %task_id,
                        pattern = %pattern,
                        "monitor.watch_for_timeout"
                    );
                    return ok_response(build_response(collected, all_matches, false, true));
                }
            }
        }
    }
}

/// Build the response from a collected vec of matches plus the lifecycle
/// flags. Single-match mode (`all_matches=false`) keeps the `matches` array
/// empty for backward compat and only sets `matching_line`/`stream`.
fn build_response(
    collected: Vec<MonitorWatchForMatch>,
    all_matches: bool,
    terminated_without_more: bool,
    timed_out: bool,
) -> MonitorWatchForResponse {
    let matched = !collected.is_empty();
    let first_line = collected.first().map(|m| m.matching_line.clone());
    let first_stream = collected.first().map(|m| m.stream.clone());
    let terminated_without_match = terminated_without_more && !matched;
    if all_matches {
        MonitorWatchForResponse {
            matched,
            matching_line: first_line,
            stream: first_stream,
            matches: collected,
            terminated_without_match,
            timed_out,
        }
    } else {
        // Backward compat: single-match callers don't expect a `matches`
        // array. Leave it empty so the serializer skips it.
        MonitorWatchForResponse {
            matched,
            matching_line: first_line,
            stream: first_stream,
            matches: Vec::new(),
            terminated_without_match,
            timed_out,
        }
    }
}

/// Tail entries look like `[stdout] actual line` or `[stderr] actual line`.
/// Split the prefix off so pattern matching runs against the payload only.
fn strip_stream_prefix(tail_line: &str) -> (&str, &str) {
    if let Some(rest) = tail_line.strip_prefix("[stdout] ") {
        ("stdout", rest)
    } else if let Some(rest) = tail_line.strip_prefix("[stderr] ") {
        ("stderr", rest)
    } else {
        ("stdout", tail_line)
    }
}

fn ok_response(
    response: MonitorWatchForResponse,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let body = serde_json::to_string(&response).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "monitor_watch_for response serialization failed: {err}"
        ))
    })?;
    Ok(FunctionToolOutput::from_text(body, Some(true)))
}
