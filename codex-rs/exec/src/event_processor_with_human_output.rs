use std::io::IsTerminal;
use std::path::Path;
use std::path::PathBuf;

use codex_app_server_protocol::CommandExecutionStatus;
use codex_app_server_protocol::McpToolCallStatus;
use codex_app_server_protocol::PatchApplyStatus;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadItem;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::TurnStatus;
use codex_core::config::Config;
use codex_model_provider_info::WireApi;
use codex_protocol::models::PermissionProfile;
use codex_protocol::num_format::format_with_separators;
use codex_protocol::protocol::AgentMessageEvent;
use codex_protocol::protocol::AgentReasoningRawContentEvent;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::BackgroundEventEvent;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::DeprecationNoticeEvent;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::HookCompletedEvent;
use codex_protocol::protocol::HookEventName;
use codex_protocol::protocol::HookOutputEntryKind;
use codex_protocol::protocol::HookRunStatus;
use codex_protocol::protocol::HookStartedEvent;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::McpToolCallBeginEvent;
use codex_protocol::protocol::McpToolCallEndEvent;
use codex_protocol::protocol::PatchApplyBeginEvent;
use codex_protocol::protocol::PatchApplyEndEvent;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_utils_absolute_path::canonicalize_preserving_symlinks;
use owo_colors::OwoColorize;
use owo_colors::Style;

use crate::event_processor::CodexStatus;
use crate::event_processor::EventProcessor;
use crate::event_processor::handle_last_message;

pub(crate) struct EventProcessorWithHumanOutput {
    bold: Style,
    cyan: Style,
    dimmed: Style,
    green: Style,
    italic: Style,
    magenta: Style,
    red: Style,
    yellow: Style,
    show_agent_reasoning: bool,
    show_raw_agent_reasoning: bool,
    last_message_path: Option<PathBuf>,
    final_message: Option<String>,
    final_message_rendered: bool,
    emit_final_message_on_shutdown: bool,
    last_total_token_usage: Option<ThreadTokenUsage>,
}

impl EventProcessorWithHumanOutput {
    pub(crate) fn create_with_ansi(
        with_ansi: bool,
        config: &Config,
        last_message_path: Option<PathBuf>,
    ) -> Self {
        let style = |styled: Style, plain: Style| if with_ansi { styled } else { plain };
        Self {
            bold: style(Style::new().bold(), Style::new()),
            cyan: style(Style::new().cyan(), Style::new()),
            dimmed: style(Style::new().dimmed(), Style::new()),
            green: style(Style::new().green(), Style::new()),
            italic: style(Style::new().italic(), Style::new()),
            magenta: style(Style::new().magenta(), Style::new()),
            red: style(Style::new().red(), Style::new()),
            yellow: style(Style::new().yellow(), Style::new()),
            show_agent_reasoning: !config.hide_agent_reasoning,
            show_raw_agent_reasoning: config.show_raw_agent_reasoning,
            last_message_path,
            final_message: None,
            final_message_rendered: false,
            emit_final_message_on_shutdown: false,
            last_total_token_usage: None,
        }
    }

    fn render_item_started(&self, item: &ThreadItem) {
        match item {
            ThreadItem::CommandExecution { command, cwd, .. } => {
                eprintln!(
                    "{}\n{} in {}",
                    "exec".style(self.italic).style(self.magenta),
                    command.style(self.bold),
                    cwd.display()
                );
            }
            ThreadItem::McpToolCall { server, tool, .. } => {
                eprintln!(
                    "{} {} {}",
                    "mcp:".style(self.bold),
                    format!("{server}/{tool}").style(self.cyan),
                    "started".style(self.dimmed)
                );
            }
            ThreadItem::WebSearch { query, .. } => {
                eprintln!("{} {}", "web search:".style(self.bold), query);
            }
            ThreadItem::FileChange { .. } => {
                eprintln!("{}", "apply patch".style(self.bold));
            }
            ThreadItem::CollabAgentToolCall { tool, .. } => {
                eprintln!("{} {:?}", "collab:".style(self.bold), tool);
            }
            _ => {}
        }
    }

    fn render_item_completed(&mut self, item: ThreadItem) {
        match item {
            ThreadItem::AgentMessage { text, .. } => {
                eprintln!(
                    "{}\n{}",
                    "codex".style(self.italic).style(self.magenta),
                    text
                );
                self.final_message = Some(text);
                self.final_message_rendered = true;
            }
            ThreadItem::Reasoning {
                summary, content, ..
            } => {
                if self.show_agent_reasoning
                    && let Some(text) =
                        reasoning_text(&summary, &content, self.show_raw_agent_reasoning)
                    && !text.trim().is_empty()
                {
                    eprintln!("{}", text.style(self.dimmed));
                }
            }
            ThreadItem::CommandExecution {
                command: _,
                aggregated_output,
                exit_code,
                status,
                duration_ms,
                ..
            } => {
                let duration_suffix = duration_ms
                    .map(|duration_ms| format!(" in {duration_ms}ms"))
                    .unwrap_or_default();
                match status {
                    CommandExecutionStatus::Completed => {
                        eprintln!(
                            "{}",
                            format!(" succeeded{duration_suffix}:").style(self.green)
                        );
                    }
                    CommandExecutionStatus::Failed => {
                        let exit_code = exit_code.unwrap_or(1);
                        eprintln!(
                            "{}",
                            format!(" exited {exit_code}{duration_suffix}:").style(self.red)
                        );
                    }
                    CommandExecutionStatus::Declined => {
                        eprintln!(
                            "{}",
                            format!(" declined{duration_suffix}:").style(self.yellow)
                        );
                    }
                    CommandExecutionStatus::InProgress => {
                        eprintln!(
                            "{}",
                            format!(" in progress{duration_suffix}:").style(self.dimmed)
                        );
                    }
                }
                if let Some(output) = aggregated_output
                    && !output.trim().is_empty()
                {
                    eprintln!("{output}");
                }
            }
            ThreadItem::FileChange {
                changes, status, ..
            } => {
                let status_text = match status {
                    PatchApplyStatus::Completed => "completed",
                    PatchApplyStatus::Failed => "failed",
                    PatchApplyStatus::Declined => "declined",
                    PatchApplyStatus::InProgress => "in_progress",
                };
                eprintln!("{} {}", "patch:".style(self.bold), status_text);
                for change in changes {
                    eprintln!("{}", change.path.style(self.dimmed));
                }
            }
            ThreadItem::McpToolCall {
                server,
                tool,
                status,
                error,
                ..
            } => {
                let status_text = match status {
                    McpToolCallStatus::Completed => "completed".style(self.green),
                    McpToolCallStatus::Failed => "failed".style(self.red),
                    McpToolCallStatus::InProgress => "in_progress".style(self.dimmed),
                };
                eprintln!(
                    "{} {} {}",
                    "mcp:".style(self.bold),
                    format!("{server}/{tool}").style(self.cyan),
                    format!("({status_text})").style(self.dimmed)
                );
                if let Some(error) = error {
                    eprintln!("{}", error.message.style(self.red));
                }
            }
            ThreadItem::WebSearch { query, .. } => {
                eprintln!("{} {}", "web search:".style(self.bold), query);
            }
            ThreadItem::ContextCompaction { .. } => {
                eprintln!("{}", "context compacted".style(self.dimmed));
            }
            _ => {}
        }
    }
}

impl EventProcessor for EventProcessorWithHumanOutput {
    fn print_config_summary(
        &mut self,
        config: &Config,
        prompt: &str,
        session_configured_event: &SessionConfiguredEvent,
    ) {
        const VERSION: &str = env!("CARGO_PKG_VERSION");
        eprintln!("OpenAI Codex v{VERSION} (research preview)\n--------");
        for (key, value) in config_summary_entries(config, session_configured_event) {
            eprintln!("{} {}", format!("{key}:").style(self.bold), value);
        }
        eprintln!("--------");
        eprintln!("{}\n{}", "user".style(self.cyan), prompt);
    }

    fn process_server_notification(&mut self, notification: ServerNotification) -> CodexStatus {
        match notification {
            ServerNotification::ConfigWarning(notification) => {
                let details = notification
                    .details
                    .map(|details| format!(" ({details})"))
                    .unwrap_or_default();
                eprintln!(
                    "{} {}{}",
                    "warning:".style(self.yellow).style(self.bold),
                    notification.summary,
                    details
                );
                CodexStatus::Running
            }
            EventMsg::GuardianAssessment(_) => {}
            EventMsg::ModelReroute(_) => {}
            EventMsg::DeprecationNotice(DeprecationNoticeEvent { summary, details }) => {
                ts_msg!(
                    self,
                    "{} {summary}",
                    "deprecated:".style(self.magenta).style(self.bold)
                );
                if let Some(details) = details {
                    ts_msg!(self, "  {}", details.style(self.dimmed));
                }
            }
            EventMsg::McpStartupUpdate(update) => {
                let status_text = match update.status {
                    codex_protocol::protocol::McpStartupStatus::Starting => "starting".to_string(),
                    codex_protocol::protocol::McpStartupStatus::Ready => "ready".to_string(),
                    codex_protocol::protocol::McpStartupStatus::Cancelled => {
                        "cancelled".to_string()
                    }
                    codex_protocol::protocol::McpStartupStatus::Failed { ref error } => {
                        format!("failed: {error}")
                    }
                };
                ts_msg!(
                    self,
                    "{} {} {}",
                    "mcp:".style(self.cyan),
                    update.server,
                    status_text
                );
            }
            EventMsg::McpStartupComplete(summary) => {
                let mut parts = Vec::new();
                if !summary.ready.is_empty() {
                    parts.push(format!("ready: {}", summary.ready.join(", ")));
                }
                if !summary.failed.is_empty() {
                    let servers: Vec<_> = summary.failed.iter().map(|f| f.server.clone()).collect();
                    parts.push(format!("failed: {}", servers.join(", ")));
                }
                if !summary.cancelled.is_empty() {
                    parts.push(format!("cancelled: {}", summary.cancelled.join(", ")));
                }
                let joined = if parts.is_empty() {
                    "no servers".to_string()
                } else {
                    parts.join("; ")
                };
                ts_msg!(self, "{} {}", "mcp startup:".style(self.cyan), joined);
            }
            EventMsg::BackgroundEvent(BackgroundEventEvent { message }) => {
                ts_msg!(self, "{}", message.style(self.dimmed));
            }
            EventMsg::StreamError(StreamErrorEvent {
                message,
                additional_details,
                ..
            }) => {
                let message = match additional_details {
                    Some(details) if !details.trim().is_empty() => format!("{message} ({details})"),
                    _ => message,
                };
                ts_msg!(self, "{}", message.style(self.dimmed));
            }
            EventMsg::TurnStarted(_) => {
                // Ignore.
            }
            EventMsg::ElicitationRequest(ev) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "ERROR:".style(self.red).style(self.bold),
                    notification.error
                );
                CodexStatus::Running
            }
            ServerNotification::DeprecationNotice(notification) => {
                eprintln!(
                    "{} {}",
                    "deprecated:".style(self.yellow).style(self.bold),
                    notification.summary
                );
                if let Some(details) = notification.details {
                    eprintln!("{}", details.style(self.dimmed));
                }
                CodexStatus::Running
            }
            ServerNotification::HookStarted(notification) => {
                eprintln!(
                    "{} {}",
                    "hook:".style(self.bold),
                    format!("{:?}", notification.run.event_name).style(self.dimmed)
                );
                CodexStatus::Running
            }
            ServerNotification::HookCompleted(notification) => {
                eprintln!(
                    "{} {} {:?}",
                    "hook:".style(self.bold),
                    format!("{:?}", notification.run.event_name).style(self.dimmed),
                    notification.run.status
                );
                CodexStatus::Running
            }
            ServerNotification::ItemStarted(notification) => {
                self.render_item_started(&notification.item);
                CodexStatus::Running
            }
            ServerNotification::ItemCompleted(notification) => {
                self.render_item_completed(notification.item);
                CodexStatus::Running
            }
            ServerNotification::ModelRerouted(notification) => {
                eprintln!(
                    "{} {} -> {}",
                    "model rerouted:".style(self.yellow).style(self.bold),
                    notification.from_model,
                    notification.to_model
                );
                CodexStatus::Running
            }
            ServerNotification::ModelVerification(_) => CodexStatus::Running,
            ServerNotification::ThreadTokenUsageUpdated(notification) => {
                self.last_total_token_usage = Some(notification.token_usage);
                CodexStatus::Running
            }
            ServerNotification::TurnCompleted(notification) => match notification.turn.status {
                TurnStatus::Completed => {
                    let rendered_message = self
                        .final_message_rendered
                        .then(|| self.final_message.clone())
                        .flatten();
                    if let Some(final_message) =
                        final_message_from_turn_items(notification.turn.items.as_slice())
                    {
                        self.final_message_rendered =
                            rendered_message.as_deref() == Some(final_message.as_str());
                        self.final_message = Some(final_message);
                    }
                    self.emit_final_message_on_shutdown = true;
                    CodexStatus::InitiateShutdown
                }
            }
            EventMsg::WebSearchBegin(_) => {
                ts_msg!(self, "🌐 Searching the web...");
            }
            EventMsg::WebSearchEnd(WebSearchEndEvent {
                call_id: _,
                query,
                action,
            }) => {
                let detail = web_search_detail(Some(&action), &query);
                if detail.is_empty() {
                    ts_msg!(self, "🌐 Searched the web");
                } else {
                    ts_msg!(self, "🌐 Searched: {detail}");
                }
            }
            EventMsg::ImageGenerationBegin(generated) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "image generation started".style(self.magenta),
                    generated.call_id
                );
            }
            EventMsg::ImageGenerationEnd(generated) => {
                if !generated.result.is_empty()
                    && !generated.result.starts_with("data:")
                    && !generated.result.starts_with("http://")
                    && !generated.result.starts_with("https://")
                    && !generated.result.starts_with("file://")
                {
                    ts_msg!(
                        self,
                        "{} {} {}",
                        "generated image".style(self.magenta),
                        generated.call_id,
                        generated.result.style(self.dimmed)
                    );
                } else {
                    ts_msg!(
                        self,
                        "{} {}",
                        "generated image".style(self.magenta),
                        generated.call_id
                    );
                }
            }
            EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
                call_id,
                auto_approved,
                changes,
                ..
            }) => {
                // Store metadata so we can calculate duration later when we
                // receive the corresponding PatchApplyEnd event.
                self.call_id_to_patch.insert(
                    call_id,
                    PatchApplyBegin {
                        start_time: Instant::now(),
                        auto_approved,
                    },
                );

                ts_msg!(
                    self,
                    "{}",
                    "file update".style(self.magenta).style(self.italic),
                );

                // Pretty-print the patch summary with colored diff markers so
                // it's easy to scan in the terminal output.
                for (path, change) in changes.iter() {
                    match change {
                        FileChange::Add { content } => {
                            let header = format!(
                                "{} {}",
                                format_file_change(change),
                                path.to_string_lossy()
                            );
                            eprintln!("{}", header.style(self.magenta));
                            for line in content.lines() {
                                eprintln!("{}", line.style(self.green));
                            }
                        }
                        FileChange::Delete { content } => {
                            let header = format!(
                                "{} {}",
                                format_file_change(change),
                                path.to_string_lossy()
                            );
                            eprintln!("{}", header.style(self.magenta));
                            for line in content.lines() {
                                eprintln!("{}", line.style(self.red));
                            }
                        }
                        FileChange::Update {
                            unified_diff,
                            move_path,
                        } => {
                            let header = if let Some(dest) = move_path {
                                format!(
                                    "{} {} -> {}",
                                    format_file_change(change),
                                    path.to_string_lossy(),
                                    dest.to_string_lossy()
                                )
                            } else {
                                format!("{} {}", format_file_change(change), path.to_string_lossy())
                            };
                            eprintln!("{}", header.style(self.magenta));

                            // Colorize diff lines. We keep file header lines
                            // (--- / +++) without extra coloring so they are
                            // still readable.
                            for diff_line in unified_diff.lines() {
                                if diff_line.starts_with('+') && !diff_line.starts_with("+++") {
                                    eprintln!("{}", diff_line.style(self.green));
                                } else if diff_line.starts_with('-')
                                    && !diff_line.starts_with("---")
                                {
                                    eprintln!("{}", diff_line.style(self.red));
                                } else {
                                    eprintln!("{diff_line}");
                                }
                            }
                        }
                    }
                    CodexStatus::InitiateShutdown
                }
                TurnStatus::Interrupted => {
                    self.final_message = None;
                    self.final_message_rendered = false;
                    self.emit_final_message_on_shutdown = false;
                    eprintln!("{}", "turn interrupted".style(self.dimmed));
                    CodexStatus::InitiateShutdown
                }
                TurnStatus::InProgress => CodexStatus::Running,
            },
            ServerNotification::TurnDiffUpdated(notification) => {
                if !notification.diff.trim().is_empty() {
                    eprintln!("{}", notification.diff);
                }
                CodexStatus::Running
            }
            ServerNotification::TurnPlanUpdated(notification) => {
                if let Some(explanation) = notification.explanation {
                    eprintln!("{}", explanation.style(self.italic));
                }
                for step in notification.plan {
                    match step.status {
                        codex_app_server_protocol::TurnPlanStepStatus::Completed => {
                            eprintln!("  {} {}", "✓".style(self.green), step.step);
                        }
                        codex_app_server_protocol::TurnPlanStepStatus::InProgress => {
                            eprintln!("  {} {}", "→".style(self.cyan), step.step);
                        }
                        codex_app_server_protocol::TurnPlanStepStatus::Pending => {
                            eprintln!(
                                "  {} {}",
                                "•".style(self.dimmed),
                                step.step.style(self.dimmed)
                            );
                        }
                    }
                }
                CodexStatus::Running
            }
            EventMsg::ViewImageToolCall(view) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "viewed image".style(self.magenta),
                    view.path.display()
                );
            }
            EventMsg::TurnAborted(abort_reason) => {
                match abort_reason.reason {
                    TurnAbortReason::Interrupted => {
                        ts_msg!(self, "task interrupted");
                    }
                    TurnAbortReason::Replaced => {
                        ts_msg!(self, "task aborted: replaced by a new task");
                    }
                    TurnAbortReason::ReviewEnded => {
                        ts_msg!(self, "task aborted: review ended");
                    }
                }
                return CodexStatus::InitiateShutdown;
            }
            EventMsg::ContextCompacted(_) => {
                ts_msg!(self, "context compacted");
            }
            EventMsg::CollabAgentSpawnBegin(CollabAgentSpawnBeginEvent {
                call_id,
                sender_thread_id: _,
                prompt,
                ..
            }) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "collab".style(self.magenta),
                    format_collab_invocation("spawn_agent", &call_id, Some(&prompt))
                        .style(self.bold)
                );
            }
            EventMsg::CollabAgentSpawnEnd(CollabAgentSpawnEndEvent {
                call_id,
                sender_thread_id: _,
                new_thread_id,
                prompt,
                status,
                ..
            }) => {
                let success = new_thread_id.is_some() && !is_collab_status_failure(&status);
                let title_style = if success { self.green } else { self.red };
                let title = format!(
                    "{} {}:",
                    format_collab_invocation("spawn_agent", &call_id, Some(&prompt)),
                    format_collab_status(&status)
                );
                ts_msg!(self, "{}", title.style(title_style));
                if let Some(new_thread_id) = new_thread_id {
                    eprintln!("  agent: {}", new_thread_id.to_string().style(self.dimmed));
                }
            }
            EventMsg::CollabAgentInteractionBegin(CollabAgentInteractionBeginEvent {
                call_id,
                sender_thread_id: _,
                receiver_thread_id,
                prompt,
            }) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "collab".style(self.magenta),
                    format_collab_invocation("send_input", &call_id, Some(&prompt))
                        .style(self.bold)
                );
                eprintln!(
                    "  receiver: {}",
                    receiver_thread_id.to_string().style(self.dimmed)
                );
            }
            EventMsg::CollabAgentInteractionEnd(CollabAgentInteractionEndEvent {
                call_id,
                sender_thread_id: _,
                receiver_thread_id,
                prompt,
                status,
                ..
            }) => {
                let success = !is_collab_status_failure(&status);
                let title_style = if success { self.green } else { self.red };
                let title = format!(
                    "{} {}:",
                    format_collab_invocation("send_input", &call_id, Some(&prompt)),
                    format_collab_status(&status)
                );
                ts_msg!(self, "{}", title.style(title_style));
                eprintln!(
                    "  receiver: {}",
                    receiver_thread_id.to_string().style(self.dimmed)
                );
            }
            EventMsg::CollabWaitingBegin(CollabWaitingBeginEvent {
                sender_thread_id: _,
                receiver_thread_ids,
                call_id,
                ..
            }) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "collab".style(self.magenta),
                    format_collab_invocation("wait", &call_id, None).style(self.bold)
                );
                eprintln!(
                    "  receivers: {}",
                    format_receiver_list(&receiver_thread_ids).style(self.dimmed)
                );
            }
            EventMsg::CollabWaitingEnd(CollabWaitingEndEvent {
                sender_thread_id: _,
                call_id,
                statuses,
                ..
            }) => {
                if statuses.is_empty() {
                    ts_msg!(
                        self,
                        "{} {}:",
                        format_collab_invocation("wait", &call_id, None),
                        "timed out".style(self.yellow)
                    );
                    return CodexStatus::Running;
                }
                let success = !statuses.values().any(is_collab_status_failure);
                let title_style = if success { self.green } else { self.red };
                let title = format!(
                    "{} {} agents complete:",
                    format_collab_invocation("wait", &call_id, None),
                    statuses.len()
                );
                ts_msg!(self, "{}", title.style(title_style));
                let mut sorted = statuses
                    .into_iter()
                    .map(|(thread_id, status)| (thread_id.to_string(), status))
                    .collect::<Vec<_>>();
                sorted.sort_by(|(left, _), (right, _)| left.cmp(right));
                for (thread_id, status) in sorted {
                    eprintln!(
                        "  {} {}",
                        thread_id.style(self.dimmed),
                        format_collab_status(&status).style(style_for_agent_status(&status, self))
                    );
                }
            }
            EventMsg::CollabCloseBegin(CollabCloseBeginEvent {
                call_id,
                sender_thread_id: _,
                receiver_thread_id,
            }) => {
                ts_msg!(
                    self,
                    "{} {}",
                    "collab".style(self.magenta),
                    format_collab_invocation("close_agent", &call_id, None).style(self.bold)
                );
                eprintln!(
                    "  receiver: {}",
                    receiver_thread_id.to_string().style(self.dimmed)
                );
            }
            EventMsg::CollabCloseEnd(CollabCloseEndEvent {
                call_id,
                sender_thread_id: _,
                receiver_thread_id,
                status,
                ..
            }) => {
                let success = !is_collab_status_failure(&status);
                let title_style = if success { self.green } else { self.red };
                let title = format!(
                    "{} {}:",
                    format_collab_invocation("close_agent", &call_id, None),
                    format_collab_status(&status)
                );
                ts_msg!(self, "{}", title.style(title_style));
                eprintln!(
                    "  receiver: {}",
                    receiver_thread_id.to_string().style(self.dimmed)
                );
            }
            EventMsg::HookStarted(event) => self.render_hook_started(event),
            EventMsg::HookCompleted(event) => self.render_hook_completed(event),
            EventMsg::ShutdownComplete => return CodexStatus::Shutdown,
            EventMsg::ThreadNameUpdated(_)
            | EventMsg::ExecApprovalRequest(_)
            | EventMsg::ApplyPatchApprovalRequest(_)
            | EventMsg::TerminalInteraction(_)
            | EventMsg::ExecCommandOutputDelta(_)
            | EventMsg::GetHistoryEntryResponse(_)
            | EventMsg::McpListToolsResponse(_)
            | EventMsg::ListCustomPromptsResponse(_)
            | EventMsg::ListSkillsResponse(_)
            | EventMsg::ListRemoteSkillsResponse(_)
            | EventMsg::RemoteSkillDownloaded(_)
            | EventMsg::RawResponseItem(_)
            | EventMsg::UserMessage(_)
            | EventMsg::EnteredReviewMode(_)
            | EventMsg::ExitedReviewMode(_)
            | EventMsg::AgentMessageDelta(_)
            | EventMsg::AgentReasoningDelta(_)
            | EventMsg::AgentReasoningRawContentDelta(_)
            | EventMsg::ItemStarted(_)
            | EventMsg::ItemCompleted(_)
            | EventMsg::AgentMessageContentDelta(_)
            | EventMsg::PlanDelta(_)
            | EventMsg::ReasoningContentDelta(_)
            | EventMsg::ReasoningRawContentDelta(_)
            | EventMsg::SkillsUpdateAvailable
            | EventMsg::UndoCompleted(_)
            | EventMsg::UndoStarted(_)
            | EventMsg::ThreadRolledBack(_)
            | EventMsg::RequestUserInput(_)
            | EventMsg::RequestPermissions(_)
            | EventMsg::CollabResumeBegin(_)
            | EventMsg::CollabResumeEnd(_)
            | EventMsg::RealtimeConversationStarted(_)
            | EventMsg::RealtimeConversationRealtime(_)
            | EventMsg::RealtimeConversationClosed(_)
            | EventMsg::DynamicToolCallRequest(_)
            | EventMsg::PresentDocument(_)
            | EventMsg::UpdateDocumentSection(_)
            | EventMsg::AppendDocumentSection(_)
            | EventMsg::AddDocumentSection(_)
            | EventMsg::PatchDocumentSection(_)
            | EventMsg::DynamicToolCallResponse(_) => {}
        }
    }

    fn process_warning(&mut self, message: String) -> CodexStatus {
        eprintln!(
            "{} {message}",
            "warning:".style(self.yellow).style(self.bold)
        );
        CodexStatus::Running
    }

    fn print_final_output(&mut self) {
        if self.emit_final_message_on_shutdown
            && let Some(path) = self.last_message_path.as_deref()
        {
            handle_last_message(self.final_message.as_deref(), path);
        }

        if let Some(usage) = &self.last_total_token_usage {
            eprintln!(
                "{}\n{}",
                "tokens used".style(self.dimmed),
                format_with_separators(blended_total(usage))
            );
        }

        #[allow(clippy::print_stdout)]
        if should_print_final_message_to_stdout(
            self.emit_final_message_on_shutdown
                .then_some(self.final_message.as_deref())
                .flatten(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        ) && let Some(message) = self.final_message.as_deref()
        {
            println!("{message}");
        } else if should_print_final_message_to_tty(
            self.emit_final_message_on_shutdown
                .then_some(self.final_message.as_deref())
                .flatten(),
            self.final_message_rendered,
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal(),
        ) && let Some(message) = self.final_message.as_deref()
        {
            eprintln!(
                "{}\n{}",
                "codex".style(self.italic).style(self.magenta),
                message
            );
        }
    }
}

fn config_summary_entries(
    config: &Config,
    session_configured_event: &SessionConfiguredEvent,
) -> Vec<(&'static str, String)> {
    let mut entries = vec![
        ("workdir", config.cwd.display().to_string()),
        ("model", session_configured_event.model.clone()),
        (
            "provider",
            session_configured_event.model_provider_id.clone(),
        ),
        (
            "approval",
            config.permissions.approval_policy.value().to_string(),
        ),
        (
            "sandbox",
            summarize_permission_profile(
                config.permissions.permission_profile.get(),
                config.cwd.as_path(),
            ),
        ),
    ];
    if config.model_provider.wire_api == WireApi::Responses {
        entries.push((
            "reasoning effort",
            config
                .model_reasoning_effort
                .map(|effort| effort.to_string())
                .unwrap_or_else(|| "none".to_string()),
        ));
        entries.push((
            "reasoning summaries",
            config
                .model_reasoning_summary
                .map(|summary| summary.to_string())
                .unwrap_or_else(|| "none".to_string()),
        ));
    }
    entries.push((
        "session id",
        session_configured_event.session_id.to_string(),
    ));
    entries
}

    fn render_hook_started(&self, event: HookStartedEvent) {
        if !Self::should_print_hook_started(&event) {
            return;
        }
        let event_name = Self::hook_event_name(event.run.event_name);
        if let Some(status_message) = event.run.status_message
            && !status_message.trim().is_empty()
        {
            ts_msg!(
                self,
                "{} {}: {}",
                "hook".style(self.magenta),
                event_name,
                status_message
            );
        }
    }

    fn render_hook_completed(&self, event: HookCompletedEvent) {
        if !Self::should_print_hook_completed(&event) {
            return;
        }

        let event_name = Self::hook_event_name(event.run.event_name);
        let status = Self::hook_status_name(event.run.status);
        ts_msg!(
            self,
            "{} {} ({status})",
            "hook".style(self.magenta),
            event_name
        );

        for entry in event.run.entries {
            let prefix = Self::hook_entry_prefix(entry.kind);
            eprintln!("  {prefix} {}", entry.text);
        }
    }

    fn should_print_hook_started(event: &HookStartedEvent) -> bool {
        event
            .run
            .status_message
            .as_deref()
            .is_some_and(|status_message| !status_message.trim().is_empty())
    }

    fn should_print_hook_completed(event: &HookCompletedEvent) -> bool {
        event.run.status != HookRunStatus::Completed || !event.run.entries.is_empty()
    }

    fn hook_event_name(event_name: HookEventName) -> &'static str {
        match event_name {
            HookEventName::SessionStart => "SessionStart",
            HookEventName::Stop => "Stop",
        }
    }

    fn hook_status_name(status: HookRunStatus) -> &'static str {
        match status {
            HookRunStatus::Running => "running",
            HookRunStatus::Completed => "completed",
            HookRunStatus::Failed => "failed",
            HookRunStatus::Blocked => "blocked",
            HookRunStatus::Stopped => "stopped",
        }
    }

    fn hook_entry_prefix(kind: HookOutputEntryKind) -> &'static str {
        match kind {
            HookOutputEntryKind::Warning => "warning:",
            HookOutputEntryKind::Stop => "stop:",
            HookOutputEntryKind::Feedback => "feedback:",
            HookOutputEntryKind::Context => "context:",
            HookOutputEntryKind::Error => "error:",
        }
    }

    fn is_silent_event(msg: &EventMsg) -> bool {
        match msg {
            EventMsg::HookStarted(event) => !Self::should_print_hook_started(event),
            EventMsg::HookCompleted(event) => !Self::should_print_hook_completed(event),
            _ => matches!(
                msg,
                EventMsg::ThreadNameUpdated(_)
                    | EventMsg::TokenCount(_)
                    | EventMsg::TurnStarted(_)
                    | EventMsg::ExecApprovalRequest(_)
                    | EventMsg::ApplyPatchApprovalRequest(_)
                    | EventMsg::TerminalInteraction(_)
                    | EventMsg::ExecCommandOutputDelta(_)
                    | EventMsg::GetHistoryEntryResponse(_)
                    | EventMsg::McpListToolsResponse(_)
                    | EventMsg::ListCustomPromptsResponse(_)
                    | EventMsg::ListSkillsResponse(_)
                    | EventMsg::ListRemoteSkillsResponse(_)
                    | EventMsg::RemoteSkillDownloaded(_)
                    | EventMsg::RawResponseItem(_)
                    | EventMsg::UserMessage(_)
                    | EventMsg::EnteredReviewMode(_)
                    | EventMsg::ExitedReviewMode(_)
                    | EventMsg::AgentMessageDelta(_)
                    | EventMsg::AgentReasoningDelta(_)
                    | EventMsg::AgentReasoningRawContentDelta(_)
                    | EventMsg::ItemStarted(_)
                    | EventMsg::ItemCompleted(_)
                    | EventMsg::AgentMessageContentDelta(_)
                    | EventMsg::PlanDelta(_)
                    | EventMsg::ReasoningContentDelta(_)
                    | EventMsg::ReasoningRawContentDelta(_)
                    | EventMsg::SkillsUpdateAvailable
                    | EventMsg::UndoCompleted(_)
                    | EventMsg::UndoStarted(_)
                    | EventMsg::ThreadRolledBack(_)
                    | EventMsg::RequestUserInput(_)
                    | EventMsg::RequestPermissions(_)
                    | EventMsg::DynamicToolCallRequest(_)
                    | EventMsg::DynamicToolCallResponse(_)
                    | EventMsg::GuardianAssessment(_)
            ),
        }
    }

    fn should_interrupt_progress(msg: &EventMsg) -> bool {
        if let EventMsg::HookCompleted(event) = msg {
            return Self::should_print_hook_completed(event);
        }
        matches!(
            msg,
            EventMsg::Error(_)
                | EventMsg::Warning(_)
                | EventMsg::GuardianAssessment(_)
                | EventMsg::DeprecationNotice(_)
                | EventMsg::StreamError(_)
                | EventMsg::TurnComplete(_)
                | EventMsg::ShutdownComplete
        )
    }

    fn finish_progress_line(&mut self) {
        if self.progress_active {
            self.progress_active = false;
            self.progress_last_len = 0;
            self.progress_done = false;
            if self.use_ansi_cursor {
                if self.progress_anchor {
                    eprintln!("\u{1b}[1A\u{1b}[1G\u{1b}[2K");
                } else {
                    eprintln!("\u{1b}[1G\u{1b}[2K");
                }
            } else {
                eprintln!();
            }

            let writable_roots = file_system_policy.get_writable_roots_with_cwd(cwd);
            if writable_roots.is_empty() {
                let mut summary = "read-only".to_string();
                append_network_summary(&mut summary, network_policy);
                return summary;
            }

            let mut summary = "workspace-write".to_string();
            let writable_entries = writable_roots
                .iter()
                .map(|root| writable_root_label(root.root.as_path(), cwd))
                .collect::<Vec<_>>();
            summary.push_str(&format!(" [{}]", writable_entries.join(", ")));
            append_network_summary(&mut summary, network_policy);
            summary
        }
    }
}

fn append_network_summary(summary: &mut String, network_policy: NetworkSandboxPolicy) {
    if network_policy.is_enabled() {
        summary.push_str(" (network access enabled)");
    }
}

fn writable_root_label(root: &Path, cwd: &Path) -> String {
    if paths_match_after_canonicalization(root, cwd) {
        return "workdir".to_string();
    }
    if paths_match_after_canonicalization(root, Path::new("/tmp")) {
        return "/tmp".to_string();
    }
    if std::env::var_os("TMPDIR")
        .filter(|tmpdir| !tmpdir.is_empty())
        .is_some_and(|tmpdir| paths_match_after_canonicalization(root, Path::new(&tmpdir)))
    {
        return "$TMPDIR".to_string();
    }
    display_path_label(root)
}

fn paths_match_after_canonicalization(left: &Path, right: &Path) -> bool {
    match (
        canonicalize_preserving_symlinks(left),
        canonicalize_preserving_symlinks(right),
    ) {
        (Ok(left), Ok(right)) if left == right => true,
        _ => display_path_label(left) == display_path_label(right),
    }
}

fn display_path_label(path: &Path) -> String {
    path.strip_prefix("/private/tmp")
        .ok()
        .map(|suffix| Path::new("/tmp").join(suffix))
        .unwrap_or_else(|| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn reasoning_text(
    summary: &[String],
    content: &[String],
    show_raw_agent_reasoning: bool,
) -> Option<String> {
    let entries = if show_raw_agent_reasoning && !content.is_empty() {
        content
    } else {
        summary
    };
    if entries.is_empty() {
        None
    } else {
        Some(entries.join("\n"))
    }
}

fn final_message_from_turn_items(items: &[ThreadItem]) -> Option<String> {
    items
        .iter()
        .rev()
        .find_map(|item| match item {
            ThreadItem::AgentMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .or_else(|| {
            items.iter().rev().find_map(|item| match item {
                ThreadItem::Plan { text, .. } => Some(text.clone()),
                _ => None,
            })
        })
}

fn blended_total(usage: &ThreadTokenUsage) -> i64 {
    let cached_input = usage.total.cached_input_tokens.max(0);
    let non_cached_input = (usage.total.input_tokens - cached_input).max(0);
    (non_cached_input + usage.total.output_tokens.max(0)).max(0)
}

fn should_print_final_message_to_stdout(
    final_message: Option<&str>,
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    final_message.is_some() && !(stdout_is_terminal && stderr_is_terminal)
}

struct AgentJobProgressStats {
    processed: usize,
    total: usize,
    percent: i64,
    failed: usize,
    running: usize,
    pending: usize,
}

fn format_agent_job_progress_line(
    columns: Option<usize>,
    job_label: &str,
    stats: AgentJobProgressStats,
    eta: &str,
) -> String {
    let rest = format!(
        "{processed}/{total} {percent}% f{failed} r{running} p{pending} eta {eta}",
        processed = stats.processed,
        total = stats.total,
        percent = stats.percent,
        failed = stats.failed,
        running = stats.running,
        pending = stats.pending
    );
    let prefix = format!("job {job_label}");
    let base_len = prefix.len() + rest.len() + 4;
    let mut bar_width = columns
        .and_then(|columns| columns.checked_sub(base_len))
        .filter(|available| *available > 0)
        .unwrap_or(20usize);
    let with_bar = |width: usize| {
        let filled = ((stats.processed as f64 / stats.total as f64) * width as f64)
            .round()
            .clamp(0.0, width as f64) as usize;
        let mut bar = "#".repeat(filled);
        bar.push_str(&"-".repeat(width - filled));
        format!("{prefix} [{bar}] {rest}")
    };
    let mut line = with_bar(bar_width);
    if let Some(columns) = columns
        && line.len() > columns
    {
        let min_line = format!("{prefix} {rest}");
        if min_line.len() > columns {
            let mut truncated = min_line;
            if columns > 2 && truncated.len() > columns {
                truncated.truncate(columns - 2);
                truncated.push_str("..");
            }
            return truncated;
        }
        let available = columns.saturating_sub(base_len);
        if available == 0 {
            return min_line;
        }
        bar_width = available.min(bar_width).max(1);
        line = with_bar(bar_width);
    }
    line
}

fn escape_command(command: &[String]) -> String {
    try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

fn format_file_change(change: &FileChange) -> &'static str {
    match change {
        FileChange::Add { .. } => "A",
        FileChange::Delete { .. } => "D",
        FileChange::Update {
            move_path: Some(_), ..
        } => "R",
        FileChange::Update {
            move_path: None, ..
        } => "M",
    }
}

fn format_collab_invocation(tool: &str, call_id: &str, prompt: Option<&str>) -> String {
    let prompt = prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .map(|prompt| truncate_preview(prompt, 120));
    match prompt {
        Some(prompt) => format!("{tool}({call_id}, prompt=\"{prompt}\")"),
        None => format!("{tool}({call_id})"),
    }
}

fn format_collab_status(status: &AgentStatus) -> String {
    match status {
        AgentStatus::PendingInit => "pending init".to_string(),
        AgentStatus::Running => "running".to_string(),
        AgentStatus::Interrupted => "interrupted".to_string(),
        AgentStatus::Completed(Some(message)) => {
            let preview = truncate_preview(message.trim(), 120);
            if preview.is_empty() {
                "completed".to_string()
            } else {
                format!("completed: \"{preview}\"")
            }
        }
        AgentStatus::Completed(None) => "completed".to_string(),
        AgentStatus::Errored(message) => {
            let preview = truncate_preview(message.trim(), 120);
            if preview.is_empty() {
                "errored".to_string()
            } else {
                format!("errored: \"{preview}\"")
            }
        }
        AgentStatus::Shutdown => "shutdown".to_string(),
        AgentStatus::NotFound => "not found".to_string(),
    }
}

fn style_for_agent_status(
    status: &AgentStatus,
    processor: &EventProcessorWithHumanOutput,
) -> Style {
    match status {
        AgentStatus::PendingInit | AgentStatus::Shutdown => processor.dimmed,
        AgentStatus::Running => processor.cyan,
        AgentStatus::Interrupted => processor.yellow,
        AgentStatus::Completed(_) => processor.green,
        AgentStatus::Errored(_) | AgentStatus::NotFound => processor.red,
    }
}

fn is_collab_status_failure(status: &AgentStatus) -> bool {
    matches!(status, AgentStatus::Errored(_) | AgentStatus::NotFound)
}

fn format_receiver_list(ids: &[codex_protocol::ThreadId]) -> String {
    if ids.is_empty() {
        return "none".to_string();
    }
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let preview = text.chars().take(max_chars).collect::<String>();
    format!("{preview}…")
}

fn format_mcp_invocation(invocation: &McpInvocation) -> String {
    // Build fully-qualified tool name: server.tool
    let fq_tool_name = format!("{}.{}", invocation.server, invocation.tool);

    // Format arguments as compact JSON so they fit on one line.
    let args_str = invocation
        .arguments
        .as_ref()
        .map(|v: &serde_json::Value| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()))
        .unwrap_or_default();

    if args_str.is_empty() {
        format!("{fq_tool_name}()")
    } else {
        format!("{fq_tool_name}({args_str})")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::HookCompletedEvent;
    use codex_protocol::protocol::HookEventName;
    use codex_protocol::protocol::HookExecutionMode;
    use codex_protocol::protocol::HookHandlerType;
    use codex_protocol::protocol::HookOutputEntry;
    use codex_protocol::protocol::HookRunStatus;
    use codex_protocol::protocol::HookRunSummary;
    use codex_protocol::protocol::HookScope;
    use codex_protocol::protocol::HookStartedEvent;

    use super::EventProcessorWithHumanOutput;
    use super::should_print_final_message_to_stdout;
    use pretty_assertions::assert_eq;

    #[test]
    fn suppresses_final_stdout_message_when_both_streams_are_terminals() {
        assert_eq!(
            should_print_final_message_to_stdout(Some("hello"), true, true),
            false
        );
    }

    #[test]
    fn prints_final_stdout_message_when_stdout_is_not_terminal() {
        assert_eq!(
            should_print_final_message_to_stdout(Some("hello"), false, true),
            true
        );
    }

    #[test]
    fn prints_final_stdout_message_when_stderr_is_not_terminal() {
        assert_eq!(
            should_print_final_message_to_stdout(Some("hello"), true, false),
            true
        );
    }

    #[test]
    fn does_not_print_when_message_is_missing() {
        assert_eq!(
            should_print_final_message_to_stdout(None, false, false),
            false
        );
    }

    #[test]
    fn hook_started_with_status_message_is_not_silent() {
        let event = HookStartedEvent {
            turn_id: Some("turn-1".to_string()),
            run: hook_run(
                HookRunStatus::Running,
                Some("running hook"),
                Vec::new(),
                HookEventName::Stop,
            ),
        };

        assert!(!EventProcessorWithHumanOutput::is_silent_event(
            &EventMsg::HookStarted(event)
        ));
    }

    #[test]
    fn hook_completed_failure_interrupts_progress() {
        let event = HookCompletedEvent {
            turn_id: Some("turn-1".to_string()),
            run: hook_run(HookRunStatus::Failed, None, Vec::new(), HookEventName::Stop),
        };

        assert!(EventProcessorWithHumanOutput::should_interrupt_progress(
            &EventMsg::HookCompleted(event)
        ));
    }

    #[test]
    fn hook_completed_success_without_entries_stays_silent() {
        let event = HookCompletedEvent {
            turn_id: Some("turn-1".to_string()),
            run: hook_run(
                HookRunStatus::Completed,
                None,
                Vec::new(),
                HookEventName::Stop,
            ),
        };

        assert!(EventProcessorWithHumanOutput::is_silent_event(
            &EventMsg::HookCompleted(event)
        ));
    }

    fn hook_run(
        status: HookRunStatus,
        status_message: Option<&str>,
        entries: Vec<HookOutputEntry>,
        event_name: HookEventName,
    ) -> HookRunSummary {
        HookRunSummary {
            id: "hook-run-1".to_string(),
            event_name,
            handler_type: HookHandlerType::Command,
            execution_mode: HookExecutionMode::Sync,
            scope: HookScope::Turn,
            source_path: PathBuf::from("/tmp/hooks.json"),
            display_order: 0,
            status,
            status_message: status_message.map(ToOwned::to_owned),
            started_at: 0,
            completed_at: Some(1),
            duration_ms: Some(1),
            entries,
        }
    }
}
