use super::*;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum CommandExecutionApprovalDecision {
    /// User approved the command.
    Accept,
    /// User approved the command and future prompts in the same session-scoped
    /// approval cache should run without prompting.
    AcceptForSession,
    /// User approved the command, and wants to apply the proposed execpolicy amendment so future
    /// matching commands can run without prompting.
    AcceptWithExecpolicyAmendment {
        execpolicy_amendment: ExecPolicyAmendment,
    },
    /// User chose a persistent network policy rule (allow/deny) for this host.
    ApplyNetworkPolicyAmendment {
        network_policy_amendment: NetworkPolicyAmendment,
    },
    /// User denied the command. The agent will continue the turn.
    Decline,
    /// User denied the command. The turn will also be immediately interrupted.
    Cancel,
}

impl From<CoreReviewDecision> for CommandExecutionApprovalDecision {
    fn from(value: CoreReviewDecision) -> Self {
        match value {
            CoreReviewDecision::Approved => Self::Accept,
            CoreReviewDecision::ApprovedExecpolicyAmendment {
                proposed_execpolicy_amendment,
            } => Self::AcceptWithExecpolicyAmendment {
                execpolicy_amendment: proposed_execpolicy_amendment.into(),
            },
            CoreReviewDecision::ApprovedForSession => Self::AcceptForSession,
            CoreReviewDecision::NetworkPolicyAmendment {
                network_policy_amendment,
            } => Self::ApplyNetworkPolicyAmendment {
                network_policy_amendment: network_policy_amendment.into(),
            },
            CoreReviewDecision::Abort => Self::Cancel,
            CoreReviewDecision::Denied => Self::Decline,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum FileChangeApprovalDecision {
    /// User approved the file changes.
    Accept,
    /// User approved the file changes and future changes to the same files should run without prompting.
    AcceptForSession,
    /// User denied the file changes. The agent will continue the turn.
    Decline,
    /// User denied the file changes. The turn will also be immediately interrupted.
    Cancel,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum CommandAction {
    Read {
        command: String,
        name: String,
        path: PathBuf,
    },
    ListFiles {
        command: String,
        path: Option<String>,
    },
    Search {
        command: String,
        query: Option<String>,
        path: Option<String>,
    },
    Unknown {
        command: String,
    },
}

impl CommandAction {
    pub fn into_core(self) -> CoreParsedCommand {
        match self {
            CommandAction::Read {
                command: cmd,
                name,
                path,
            } => CoreParsedCommand::Read { cmd, name, path },
            CommandAction::ListFiles { command: cmd, path } => {
                CoreParsedCommand::ListFiles { cmd, path }
            }
            CommandAction::Search {
                command: cmd,
                query,
                path,
            } => CoreParsedCommand::Search { cmd, query, path },
            CommandAction::Unknown { command: cmd } => CoreParsedCommand::Unknown { cmd },
        }
    }
}

impl From<CoreParsedCommand> for CommandAction {
    fn from(value: CoreParsedCommand) -> Self {
        match value {
            CoreParsedCommand::Read { cmd, name, path } => CommandAction::Read {
                command: cmd,
                name,
                path,
            },
            CoreParsedCommand::ListFiles { cmd, path } => {
                CommandAction::ListFiles { command: cmd, path }
            }
            CoreParsedCommand::Search { cmd, query, path } => CommandAction::Search {
                command: cmd,
                query,
                path,
            },
            CoreParsedCommand::Unknown { cmd } => CommandAction::Unknown { command: cmd },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum ThreadItem {
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    UserMessage { id: String, content: Vec<UserInput> },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    AgentMessage {
        id: String,
        text: String,
        #[serde(default)]
        phase: Option<MessagePhase>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    /// EXPERIMENTAL - proposed plan item content. The completed plan item is
    /// authoritative and may not match the concatenation of `PlanDelta` text.
    Plan { id: String, text: String },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Reasoning {
        id: String,
        #[serde(default)]
        summary: Vec<String>,
        #[serde(default)]
        content: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    CommandExecution {
        id: String,
        /// The command to be executed.
        command: String,
        /// The command's working directory.
        cwd: PathBuf,
        /// Identifier for the underlying PTY process (when available).
        process_id: Option<String>,
        status: CommandExecutionStatus,
        /// A best-effort parsing of the command to understand the action(s) it will perform.
        /// This returns a list of CommandAction objects because a single shell command may
        /// be composed of many commands piped together.
        command_actions: Vec<CommandAction>,
        /// The command's output, aggregated from stdout and stderr.
        aggregated_output: Option<String>,
        /// The command's exit code.
        exit_code: Option<i32>,
        /// The duration of the command execution in milliseconds.
        #[ts(type = "number | null")]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    FileChange {
        id: String,
        changes: Vec<FileUpdateChange>,
        status: PatchApplyStatus,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        status: McpToolCallStatus,
        arguments: JsonValue,
        result: Option<McpToolCallResult>,
        error: Option<McpToolCallError>,
        /// The duration of the MCP tool call in milliseconds.
        #[ts(type = "number | null")]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    DynamicToolCall {
        id: String,
        tool: String,
        arguments: JsonValue,
        status: DynamicToolCallStatus,
        content_items: Option<Vec<DynamicToolCallOutputContentItem>>,
        success: Option<bool>,
        /// The duration of the dynamic tool call in milliseconds.
        #[ts(type = "number | null")]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    CollabAgentToolCall {
        /// Unique identifier for this collab tool call.
        id: String,
        /// Name of the collab tool that was invoked.
        tool: CollabAgentTool,
        /// Current status of the collab tool call.
        status: CollabAgentToolCallStatus,
        /// Thread ID of the agent issuing the collab request.
        sender_thread_id: String,
        /// Thread ID of the receiving agent, when applicable. In case of spawn operation,
        /// this corresponds to the newly spawned agent.
        receiver_thread_ids: Vec<String>,
        /// Prompt text sent as part of the collab tool call, when available.
        prompt: Option<String>,
        /// Model requested for the spawned agent, when applicable.
        model: Option<String>,
        /// Reasoning effort requested for the spawned agent, when applicable.
        reasoning_effort: Option<ReasoningEffort>,
        /// Last known status of the target agents, when available.
        agents_states: HashMap<String, CollabAgentState>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    WebSearch {
        id: String,
        query: String,
        action: Option<WebSearchAction>,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ImageView { id: String, path: String },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        result: String,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    EnteredReviewMode { id: String, review: String },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ExitedReviewMode { id: String, review: String },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ContextCompaction { id: String },
}

impl ThreadItem {
    pub fn id(&self) -> &str {
        match self {
            ThreadItem::UserMessage { id, .. }
            | ThreadItem::AgentMessage { id, .. }
            | ThreadItem::Plan { id, .. }
            | ThreadItem::Reasoning { id, .. }
            | ThreadItem::CommandExecution { id, .. }
            | ThreadItem::FileChange { id, .. }
            | ThreadItem::McpToolCall { id, .. }
            | ThreadItem::DynamicToolCall { id, .. }
            | ThreadItem::CollabAgentToolCall { id, .. }
            | ThreadItem::WebSearch { id, .. }
            | ThreadItem::ImageView { id, .. }
            | ThreadItem::ImageGeneration { id, .. }
            | ThreadItem::EnteredReviewMode { id, .. }
            | ThreadItem::ExitedReviewMode { id, .. }
            | ThreadItem::ContextCompaction { id, .. } => id,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// [UNSTABLE] Lifecycle state for a guardian approval review.
pub enum GuardianApprovalReviewStatus {
    InProgress,
    Approved,
    Denied,
    Aborted,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "v2/")]
/// [UNSTABLE] Risk level assigned by guardian approval review.
pub enum GuardianRiskLevel {
    Low,
    Medium,
    High,
}

impl From<CoreGuardianRiskLevel> for GuardianRiskLevel {
    fn from(value: CoreGuardianRiskLevel) -> Self {
        match value {
            CoreGuardianRiskLevel::Low => Self::Low,
            CoreGuardianRiskLevel::Medium => Self::Medium,
            CoreGuardianRiskLevel::High => Self::High,
        }
    }
}

/// [UNSTABLE] Temporary guardian approval review payload used by
/// `item/autoApprovalReview/*` notifications. This shape is expected to change
/// soon.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GuardianApprovalReview {
    pub status: GuardianApprovalReviewStatus,
    #[serde(alias = "risk_score")]
    #[ts(type = "number | null")]
    pub risk_score: Option<u8>,
    #[serde(alias = "risk_level")]
    pub risk_level: Option<GuardianRiskLevel>,
    pub rationale: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type", rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum WebSearchAction {
    Search {
        query: Option<String>,
        queries: Option<Vec<String>>,
    },
    OpenPage {
        url: Option<String>,
    },
    FindInPage {
        url: Option<String>,
        pattern: Option<String>,
    },
    #[serde(other)]
    Other,
}

impl From<codex_protocol::models::WebSearchAction> for WebSearchAction {
    fn from(value: codex_protocol::models::WebSearchAction) -> Self {
        match value {
            codex_protocol::models::WebSearchAction::Search { query, queries } => {
                WebSearchAction::Search { query, queries }
            }
            codex_protocol::models::WebSearchAction::OpenPage { url } => {
                WebSearchAction::OpenPage { url }
            }
            codex_protocol::models::WebSearchAction::FindInPage { url, pattern } => {
                WebSearchAction::FindInPage { url, pattern }
            }
            codex_protocol::models::WebSearchAction::Other => WebSearchAction::Other,
        }
    }
}

impl From<CoreTurnItem> for ThreadItem {
    fn from(value: CoreTurnItem) -> Self {
        match value {
            CoreTurnItem::UserMessage(user) => ThreadItem::UserMessage {
                id: user.id,
                content: user.content.into_iter().map(UserInput::from).collect(),
            },
            CoreTurnItem::AgentMessage(agent) => {
                let text = agent
                    .content
                    .into_iter()
                    .map(|entry| match entry {
                        CoreAgentMessageContent::Text { text } => text,
                    })
                    .collect::<String>();
                ThreadItem::AgentMessage {
                    id: agent.id,
                    text,
                    phase: agent.phase,
                }
            }
            CoreTurnItem::Plan(plan) => ThreadItem::Plan {
                id: plan.id,
                text: plan.text,
            },
            CoreTurnItem::Reasoning(reasoning) => ThreadItem::Reasoning {
                id: reasoning.id,
                summary: reasoning.summary_text,
                content: reasoning.raw_content,
            },
            CoreTurnItem::WebSearch(search) => ThreadItem::WebSearch {
                id: search.id,
                query: search.query,
                action: Some(WebSearchAction::from(search.action)),
            },
            CoreTurnItem::ImageGeneration(image) => ThreadItem::ImageGeneration {
                id: image.id,
                status: image.status,
                revised_prompt: image.revised_prompt,
                result: image.result,
            },
            CoreTurnItem::ContextCompaction(compaction) => {
                ThreadItem::ContextCompaction { id: compaction.id }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum CommandExecutionStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

impl From<CoreExecCommandStatus> for CommandExecutionStatus {
    fn from(value: CoreExecCommandStatus) -> Self {
        Self::from(&value)
    }
}

impl From<&CoreExecCommandStatus> for CommandExecutionStatus {
    fn from(value: &CoreExecCommandStatus) -> Self {
        match value {
            CoreExecCommandStatus::Completed => CommandExecutionStatus::Completed,
            CoreExecCommandStatus::Failed => CommandExecutionStatus::Failed,
            CoreExecCommandStatus::Declined => CommandExecutionStatus::Declined,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum CollabAgentTool {
    SpawnAgent,
    SendInput,
    ResumeAgent,
    Wait,
    CloseAgent,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: PatchChangeKind,
    pub diff: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum PatchChangeKind {
    Add,
    Delete,
    Update { move_path: Option<PathBuf> },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum PatchApplyStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

impl From<CorePatchApplyStatus> for PatchApplyStatus {
    fn from(value: CorePatchApplyStatus) -> Self {
        Self::from(&value)
    }
}

impl From<&CorePatchApplyStatus> for PatchApplyStatus {
    fn from(value: &CorePatchApplyStatus) -> Self {
        match value {
            CorePatchApplyStatus::Completed => PatchApplyStatus::Completed,
            CorePatchApplyStatus::Failed => PatchApplyStatus::Failed,
            CorePatchApplyStatus::Declined => PatchApplyStatus::Declined,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum McpToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum DynamicToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum CollabAgentToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum CollabAgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CollabAgentState {
    pub status: CollabAgentStatus,
    pub message: Option<String>,
}

impl From<CoreAgentStatus> for CollabAgentState {
    fn from(value: CoreAgentStatus) -> Self {
        match value {
            CoreAgentStatus::PendingInit => Self {
                status: CollabAgentStatus::PendingInit,
                message: None,
            },
            CoreAgentStatus::Running => Self {
                status: CollabAgentStatus::Running,
                message: None,
            },
            CoreAgentStatus::Interrupted => Self {
                status: CollabAgentStatus::Interrupted,
                message: None,
            },
            CoreAgentStatus::Completed(message) => Self {
                status: CollabAgentStatus::Completed,
                message,
            },
            CoreAgentStatus::Errored(message) => Self {
                status: CollabAgentStatus::Errored,
                message: Some(message),
            },
            CoreAgentStatus::Shutdown => Self {
                status: CollabAgentStatus::Shutdown,
                message: None,
            },
            CoreAgentStatus::NotFound => Self {
                status: CollabAgentStatus::NotFound,
                message: None,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ItemStartedNotification {
    pub item: ThreadItem,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// [UNSTABLE] Temporary notification payload for guardian automatic approval
/// review. This shape is expected to change soon.
///
/// TODO(ccunningham): Attach guardian review state to the reviewed tool item's
/// lifecycle instead of sending separate standalone review notifications so the
/// app-server API can persist and replay review state via `thread/read`.
pub struct ItemGuardianApprovalReviewStartedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub target_item_id: String,
    pub review: GuardianApprovalReview,
    pub action: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// [UNSTABLE] Temporary notification payload for guardian automatic approval
/// review. This shape is expected to change soon.
///
/// TODO(ccunningham): Attach guardian review state to the reviewed tool item's
/// lifecycle instead of sending separate standalone review notifications so the
/// app-server API can persist and replay review state via `thread/read`.
pub struct ItemGuardianApprovalReviewCompletedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub target_item_id: String,
    pub review: GuardianApprovalReview,
    pub action: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ItemCompletedNotification {
    pub item: ThreadItem,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RawResponseItemCompletedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item: ResponseItem,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AgentMessageDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL - proposed plan streaming deltas for plan items. Clients should
/// not assume concatenated deltas match the completed plan item content.
pub struct PlanDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ReasoningSummaryTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    #[ts(type = "number")]
    pub summary_index: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ReasoningSummaryPartAddedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    #[ts(type = "number")]
    pub summary_index: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ReasoningTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    #[ts(type = "number")]
    pub content_index: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct TerminalInteractionNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub process_id: String,
    pub stdin: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CommandExecutionOutputDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FileChangeOutputDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CommandExecutionRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    /// Unique identifier for this specific approval callback.
    ///
    /// For regular shell/unified_exec approvals, this is null.
    ///
    /// For zsh-exec-bridge subcommand approvals, multiple callbacks can belong to
    /// one parent `itemId`, so `approvalId` is a distinct opaque callback id
    /// (a UUID) used to disambiguate routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub approval_id: Option<String>,
    /// Optional explanatory reason (e.g. request for network access).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub reason: Option<String>,
    /// Optional context for a managed-network approval prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub network_approval_context: Option<NetworkApprovalContext>,
    /// The command to be executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub command: Option<String>,
    /// The command's working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub cwd: Option<PathBuf>,
    /// Best-effort parsed command actions for friendly display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub command_actions: Option<Vec<CommandAction>>,
    /// Optional additional permissions requested for this command.
    #[experimental("item/commandExecution/requestApproval.additionalPermissions")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    /// Optional skill metadata when the approval was triggered by a skill script.
    #[experimental("item/commandExecution/requestApproval.skillMetadata")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub skill_metadata: Option<CommandExecutionRequestApprovalSkillMetadata>,
    /// Optional proposed execpolicy amendment to allow similar commands without prompting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    /// Optional proposed network policy amendments (allow/deny host) for future requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub proposed_network_policy_amendments: Option<Vec<NetworkPolicyAmendment>>,
    /// Ordered list of decisions the client may present for this prompt.
    #[experimental("item/commandExecution/requestApproval.availableDecisions")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub available_decisions: Option<Vec<CommandExecutionApprovalDecision>>,
}

impl CommandExecutionRequestApprovalParams {
    pub fn strip_experimental_fields(&mut self) {
        // TODO: Avoid hardcoding individual experimental fields here.
        // We need a generic outbound compatibility design for stripping or
        // otherwise handling experimental server->client payloads.
        self.additional_permissions = None;
        self.skill_metadata = None;
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CommandExecutionRequestApprovalResponse {
    pub decision: CommandExecutionApprovalDecision,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct FileChangeRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    /// Optional explanatory reason (e.g. request for extra write access).
    #[ts(optional = nullable)]
    pub reason: Option<String>,
    /// [UNSTABLE] When set, the agent is asking the user to allow writes under this root
    /// for the remainder of the session (unclear if this is honored today).
    #[ts(optional = nullable)]
    pub grant_root: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[ts(export_to = "v2/")]
pub struct FileChangeRequestApprovalResponse {
    pub decision: FileChangeApprovalDecision,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct DynamicToolCallParams {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub tool: String,
    pub arguments: JsonValue,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct DynamicToolCallResponse {
    pub content_items: Vec<DynamicToolCallOutputContentItem>,
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum DynamicToolCallOutputContentItem {
    #[serde(rename_all = "camelCase")]
    InputText { text: String },
    #[serde(rename_all = "camelCase")]
    InputImage { image_url: String },
}

impl From<DynamicToolCallOutputContentItem>
    for codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem
{
    fn from(item: DynamicToolCallOutputContentItem) -> Self {
        match item {
            DynamicToolCallOutputContentItem::InputText { text } => Self::InputText { text },
            DynamicToolCallOutputContentItem::InputImage { image_url } => {
                Self::InputImage { image_url }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL. Defines a single selectable option for request_user_input.
pub struct ToolRequestUserInputOption {
    pub label: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL. Represents one request_user_input question and its required options.
pub struct ToolRequestUserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    pub options: Option<Vec<ToolRequestUserInputOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL. Params sent with a request_user_input event.
pub struct ToolRequestUserInputParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub questions: Vec<ToolRequestUserInputQuestion>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL. Captures a user's answer to a request_user_input question.
pub struct ToolRequestUserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
/// EXPERIMENTAL. Response payload mapping question ids to answers.
pub struct ToolRequestUserInputResponse {
    pub answers: HashMap<String, ToolRequestUserInputAnswer>,
}
