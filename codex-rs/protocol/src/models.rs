use std::collections::HashMap;
use std::path::Path;

use codex_utils_file::encode_inline_cached;
use codex_utils_file::into_owned_processed_file;
use codex_utils_image::load_and_resize_to_fit;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::ser::Serializer;
use ts_rs::TS;
use url::Url;

use crate::config_types::CollaborationMode;
use crate::config_types::SandboxMode;
use crate::protocol::AskForApproval;
use crate::protocol::COLLABORATION_MODE_CLOSE_TAG;
use crate::protocol::COLLABORATION_MODE_OPEN_TAG;
use crate::protocol::NetworkAccess;
use crate::protocol::SandboxPolicy;
use crate::protocol::WritableRoot;
use crate::user_input::UserInput;
use codex_execpolicy::Policy;
use codex_git::GhostCommit;
use codex_utils_image::error::ImageProcessingError;
use schemars::JsonSchema;

use crate::mcp::CallToolResult;

/// Controls whether a command should use the session sandbox or bypass it.
#[derive(
    Debug, Clone, Copy, Default, Eq, Hash, PartialEq, Serialize, Deserialize, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPermissions {
    /// Run with the configured sandbox
    #[default]
    UseDefault,
    /// Request to run outside the sandbox
    RequireEscalated,
}

impl SandboxPermissions {
    pub fn requires_escalated_permissions(self) -> bool {
        matches!(self, SandboxPermissions::RequireEscalated)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseInputItem {
    Message {
        role: String,
        content: Vec<ContentItem>,
    },
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    McpToolCallOutput {
        call_id: String,
        result: Result<CallToolResult, String>,
    },
    CustomToolCallOutput {
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    try_from = "UncheckedContentItem"
)]
pub enum ContentItem {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
    },
    InputFile {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        /// MIME type for the file. Stored internally for provider adapters (Anthropic, Gemini)
        /// that need it when converting to their native format. Skipped during serialization
        /// because the OpenAI Responses API does not accept this field on `input_file` blocks;
        /// the MIME type is already embedded in the `file_data` data-URI.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(skip)]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
    OutputText {
        text: String,
    },
    UrlFile {
        url: String,
        /// MIME type for provider adapters or tools that need a type hint.
        /// Not part of the stable public wire shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(skip)]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UncheckedContentItem {
    InputText {
        text: String,
    },
    InputImage {
        image_url: String,
    },
    InputFile {
        file_data: Option<String>,
        file_id: Option<String>,
        mime_type: Option<String>,
        filename: Option<String>,
    },
    OutputText {
        text: String,
    },
    UrlFile {
        url: Option<String>,
        mime_type: Option<String>,
        filename: Option<String>,
    },
}

const MAX_URL_FILE_URL_LENGTH: usize = 8192;

impl TryFrom<UncheckedContentItem> for ContentItem {
    type Error = &'static str;

    fn try_from(value: UncheckedContentItem) -> Result<Self, Self::Error> {
        match value {
            UncheckedContentItem::InputText { text } => Ok(Self::InputText { text }),
            UncheckedContentItem::InputImage { image_url } => Ok(Self::InputImage { image_url }),
            UncheckedContentItem::InputFile {
                file_data,
                file_id,
                mime_type,
                filename,
            } => {
                let file_data = file_data.filter(|value| !value.trim().is_empty());
                let file_id = file_id.filter(|value| !value.trim().is_empty());
                let has_file_data = file_data.is_some();
                let has_file_id = file_id.is_some();
                // Both present or both absent is invalid; exactly one is required.
                if has_file_data == has_file_id {
                    return Err("input_file must include exactly one of `file_data` or `file_id`");
                }

                Ok(Self::InputFile {
                    file_data,
                    file_id,
                    mime_type,
                    filename,
                })
            }
            UncheckedContentItem::OutputText { text } => Ok(Self::OutputText { text }),
            UncheckedContentItem::UrlFile {
                url,
                mime_type,
                filename,
            } => {
                let url = url
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or("url_file requires a non-empty `url`")?;
                if url.len() > MAX_URL_FILE_URL_LENGTH {
                    return Err("url_file `url` exceeds max length of 8192 characters");
                }
                let parsed =
                    Url::parse(&url).map_err(|_| "url_file `url` must be a valid absolute URL")?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err("url_file `url` must use http or https");
                }
                Ok(Self::UrlFile {
                    url,
                    mime_type,
                    filename,
                })
            }
        }
    }
}

impl ContentItem {
    pub fn inline_file(base64_data: String, mime_type: String, filename: Option<String>) -> Self {
        Self::InputFile {
            file_data: Some(base64_data),
            file_id: None,
            mime_type: Some(mime_type),
            filename,
        }
    }

    pub fn file_ref(file_id: String, mime_type: String, filename: Option<String>) -> Self {
        Self::InputFile {
            file_data: None,
            file_id: Some(file_id),
            mime_type: Some(mime_type),
            filename,
        }
    }

    pub fn url_file(url: String, mime_type: Option<String>, filename: Option<String>) -> Self {
        Self::UrlFile {
            url,
            mime_type,
            filename,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
/// Classifies an assistant message as interim commentary or final answer text.
///
/// Providers do not emit this consistently, so callers must treat `None` as
/// "phase unknown" and keep compatibility behavior for legacy models.
pub enum MessagePhase {
    /// Mid-turn assistant text (for example preamble/progress narration).
    ///
    /// Additional tool calls or assistant output may follow before turn
    /// completion.
    Commentary,
    /// The assistant's terminal answer text for the current turn.
    FinalAnswer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    Message {
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: Option<String>,
        role: String,
        content: Vec<ContentItem>,
        // Do not use directly, no available consistently across all providers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        end_turn: Option<bool>,
        // Optional output-message phase (for example: "commentary", "final_answer").
        // Availability varies by provider/model, so downstream consumers must
        // preserve fallback behavior when this is absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        phase: Option<MessagePhase>,
    },
    Reasoning {
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: String,
        summary: Vec<ReasoningItemReasoningSummary>,
        #[serde(default, skip_serializing_if = "should_serialize_reasoning_content")]
        #[ts(optional)]
        content: Option<Vec<ReasoningItemContent>>,
        encrypted_content: Option<String>,
    },
    LocalShellCall {
        /// Legacy id field retained for compatibility with older payloads.
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: Option<String>,
        /// Set when using the Responses API.
        call_id: Option<String>,
        status: LocalShellStatus,
        action: LocalShellAction,
    },
    FunctionCall {
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: Option<String>,
        name: String,
        // The Responses API returns the function call arguments as a *string* that contains
        // JSON, not as an already‑parsed object. We keep it as a raw string here and let
        // Session::handle_function_call parse it into a Value.
        arguments: String,
        call_id: String,
        /// Gemini thought signature for multi-turn reasoning.
        /// Must be preserved and passed back in subsequent requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        thought_signature: Option<String>,
    },
    // NOTE: The `output` field for `function_call_output` uses a dedicated payload type with
    // custom serialization. On the wire it is either:
    //   - a plain string (`content`)
    //   - an array of structured content items (`content_items`)
    // We keep this behavior centralized in `FunctionCallOutputPayload`.
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    CustomToolCall {
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        status: Option<String>,

        call_id: String,
        name: String,
        input: String,
    },
    CustomToolCallOutput {
        call_id: String,
        output: String,
    },
    // Emitted by the Responses API when the agent triggers a web search.
    // Example payload (from SSE `response.output_item.done`):
    // {
    //   "id":"ws_...",
    //   "type":"web_search_call",
    //   "status":"completed",
    //   "action": {"type":"search","query":"weather: San Francisco, CA"}
    // }
    WebSearchCall {
        #[serde(default, skip_serializing)]
        #[ts(skip)]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        status: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        action: Option<WebSearchAction>,
    },
    // Generated by the harness but considered exactly as a model response.
    GhostSnapshot {
        ghost_commit: GhostCommit,
    },
    #[serde(alias = "compaction_summary")]
    Compaction {
        encrypted_content: String,
    },
    #[serde(other)]
    Other,
}

pub const BASE_INSTRUCTIONS_DEFAULT: &str = include_str!("prompts/base_instructions/default.md");

/// Base instructions for the model in a thread. Corresponds to the `instructions` field in the ResponsesAPI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(rename = "base_instructions", rename_all = "snake_case")]
pub struct BaseInstructions {
    pub text: String,
}

impl Default for BaseInstructions {
    fn default() -> Self {
        Self {
            text: BASE_INSTRUCTIONS_DEFAULT.to_string(),
        }
    }
}

/// Developer-provided guidance that is injected into a turn as a developer role
/// message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(rename = "developer_instructions", rename_all = "snake_case")]
pub struct DeveloperInstructions {
    text: String,
}

const APPROVAL_POLICY_NEVER: &str = include_str!("prompts/permissions/approval_policy/never.md");
const APPROVAL_POLICY_UNLESS_TRUSTED: &str =
    include_str!("prompts/permissions/approval_policy/unless_trusted.md");
const APPROVAL_POLICY_ON_FAILURE: &str =
    include_str!("prompts/permissions/approval_policy/on_failure.md");
const APPROVAL_POLICY_ON_REQUEST: &str =
    include_str!("prompts/permissions/approval_policy/on_request.md");
const APPROVAL_POLICY_ON_REQUEST_RULE: &str =
    include_str!("prompts/permissions/approval_policy/on_request_rule.md");

const SANDBOX_MODE_DANGER_FULL_ACCESS: &str =
    include_str!("prompts/permissions/sandbox_mode/danger_full_access.md");
const SANDBOX_MODE_WORKSPACE_WRITE: &str =
    include_str!("prompts/permissions/sandbox_mode/workspace_write.md");
const SANDBOX_MODE_READ_ONLY: &str = include_str!("prompts/permissions/sandbox_mode/read_only.md");

impl DeveloperInstructions {
    pub fn new<T: Into<String>>(text: T) -> Self {
        Self { text: text.into() }
    }

    pub fn from(
        approval_policy: AskForApproval,
        exec_policy: &Policy,
        request_rule_enabled: bool,
    ) -> DeveloperInstructions {
        let text = match approval_policy {
            AskForApproval::Never => APPROVAL_POLICY_NEVER.to_string(),
            AskForApproval::UnlessTrusted => APPROVAL_POLICY_UNLESS_TRUSTED.to_string(),
            AskForApproval::OnFailure => APPROVAL_POLICY_ON_FAILURE.to_string(),
            AskForApproval::OnRequest => {
                if !request_rule_enabled {
                    APPROVAL_POLICY_ON_REQUEST.to_string()
                } else {
                    let command_prefixes =
                        format_allow_prefixes(exec_policy.get_allowed_prefixes());
                    match command_prefixes {
                        Some(prefixes) => {
                            format!(
                                "{APPROVAL_POLICY_ON_REQUEST_RULE}\n## Approved command prefixes\nThe following prefix rules have already been approved: {prefixes}"
                            )
                        }
                        None => APPROVAL_POLICY_ON_REQUEST_RULE.to_string(),
                    }
                }
            }
        };

        DeveloperInstructions::new(text)
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn concat(self, other: impl Into<DeveloperInstructions>) -> Self {
        let mut text = self.text;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&other.into().text);
        Self { text }
    }

    pub fn model_switch_message(model_instructions: String) -> Self {
        DeveloperInstructions::new(format!(
            "<model_switch>\nThe user was previously using a different model. Please continue the conversation according to the following instructions:\n\n{model_instructions}\n</model_switch>"
        ))
    }

    pub fn personality_spec_message(spec: String) -> Self {
        let message = format!(
            "<personality_spec> The user has requested a new communication style. Future messages should adhere to the following personality: \n{spec} </personality_spec>"
        );
        DeveloperInstructions::new(message)
    }

    pub fn from_policy(
        sandbox_policy: &SandboxPolicy,
        approval_policy: AskForApproval,
        exec_policy: &Policy,
        request_rule_enabled: bool,
        cwd: &Path,
    ) -> Self {
        let network_access = if sandbox_policy.has_full_network_access() {
            NetworkAccess::Enabled
        } else {
            NetworkAccess::Restricted
        };

        let (sandbox_mode, writable_roots) = match sandbox_policy {
            SandboxPolicy::DangerFullAccess => (SandboxMode::DangerFullAccess, None),
            SandboxPolicy::ReadOnly { .. } => (SandboxMode::ReadOnly, None),
            SandboxPolicy::ExternalSandbox { .. } => (SandboxMode::DangerFullAccess, None),
            SandboxPolicy::WorkspaceWrite { .. } => {
                let roots = sandbox_policy.get_writable_roots_with_cwd(cwd);
                (SandboxMode::WorkspaceWrite, Some(roots))
            }
        };

        DeveloperInstructions::from_permissions_with_network(
            sandbox_mode,
            network_access,
            approval_policy,
            exec_policy,
            request_rule_enabled,
            writable_roots,
        )
    }

    /// Returns developer instructions from a collaboration mode if they exist and are non-empty.
    pub fn from_collaboration_mode(collaboration_mode: &CollaborationMode) -> Option<Self> {
        collaboration_mode
            .settings
            .developer_instructions
            .as_ref()
            .filter(|instructions| !instructions.is_empty())
            .map(|instructions| {
                DeveloperInstructions::new(format!(
                    "{COLLABORATION_MODE_OPEN_TAG}{instructions}{COLLABORATION_MODE_CLOSE_TAG}"
                ))
            })
    }

    fn from_permissions_with_network(
        sandbox_mode: SandboxMode,
        network_access: NetworkAccess,
        approval_policy: AskForApproval,
        exec_policy: &Policy,
        request_rule_enabled: bool,
        writable_roots: Option<Vec<WritableRoot>>,
    ) -> Self {
        let start_tag = DeveloperInstructions::new("<permissions instructions>");
        let end_tag = DeveloperInstructions::new("</permissions instructions>");
        start_tag
            .concat(DeveloperInstructions::sandbox_text(
                sandbox_mode,
                network_access,
            ))
            .concat(DeveloperInstructions::from(
                approval_policy,
                exec_policy,
                request_rule_enabled,
            ))
            .concat(DeveloperInstructions::from_writable_roots(writable_roots))
            .concat(end_tag)
    }

    fn from_writable_roots(writable_roots: Option<Vec<WritableRoot>>) -> Self {
        let Some(roots) = writable_roots else {
            return DeveloperInstructions::new("");
        };

        if roots.is_empty() {
            return DeveloperInstructions::new("");
        }

        let roots_list: Vec<String> = roots
            .iter()
            .map(|r| format!("`{}`", r.root.to_string_lossy()))
            .collect();
        let text = if roots_list.len() == 1 {
            format!(" The writable root is {}.", roots_list[0])
        } else {
            format!(" The writable roots are {}.", roots_list.join(", "))
        };
        DeveloperInstructions::new(text)
    }

    fn sandbox_text(mode: SandboxMode, network_access: NetworkAccess) -> DeveloperInstructions {
        let template = match mode {
            SandboxMode::DangerFullAccess => SANDBOX_MODE_DANGER_FULL_ACCESS.trim_end(),
            SandboxMode::WorkspaceWrite => SANDBOX_MODE_WORKSPACE_WRITE.trim_end(),
            SandboxMode::ReadOnly => SANDBOX_MODE_READ_ONLY.trim_end(),
        };
        let text = template.replace("{network_access}", &network_access.to_string());

        DeveloperInstructions::new(text)
    }
}

const MAX_RENDERED_PREFIXES: usize = 100;
const MAX_ALLOW_PREFIX_TEXT_BYTES: usize = 5000;
const TRUNCATED_MARKER: &str = "...\n[Some commands were truncated]";

pub fn format_allow_prefixes(prefixes: Vec<Vec<String>>) -> Option<String> {
    let mut truncated = false;
    if prefixes.len() > MAX_RENDERED_PREFIXES {
        truncated = true;
    }

    let mut prefixes = prefixes;
    prefixes.sort_by(|a, b| {
        a.len()
            .cmp(&b.len())
            .then_with(|| prefix_combined_str_len(a).cmp(&prefix_combined_str_len(b)))
            .then_with(|| a.cmp(b))
    });

    let full_text = prefixes
        .into_iter()
        .take(MAX_RENDERED_PREFIXES)
        .map(|prefix| format!("- {}", render_command_prefix(&prefix)))
        .collect::<Vec<_>>()
        .join("\n");

    // truncate to last UTF8 char
    let mut output = full_text;
    let byte_idx = output
        .char_indices()
        .nth(MAX_ALLOW_PREFIX_TEXT_BYTES)
        .map(|(i, _)| i);
    if let Some(byte_idx) = byte_idx {
        truncated = true;
        output = output[..byte_idx].to_string();
    }

    if truncated {
        Some(format!("{output}{TRUNCATED_MARKER}"))
    } else {
        Some(output)
    }
}

fn prefix_combined_str_len(prefix: &[String]) -> usize {
    prefix.iter().map(String::len).sum()
}

fn render_command_prefix(prefix: &[String]) -> String {
    let tokens = prefix
        .iter()
        .map(|token| serde_json::to_string(token).unwrap_or_else(|_| format!("{token:?}")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{tokens}]")
}

impl From<DeveloperInstructions> for ResponseItem {
    fn from(di: DeveloperInstructions) -> Self {
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: di.into_text(),
            }],
            end_turn: None,
            phase: None,
        }
    }
}

impl From<SandboxMode> for DeveloperInstructions {
    fn from(mode: SandboxMode) -> Self {
        let network_access = match mode {
            SandboxMode::DangerFullAccess => NetworkAccess::Enabled,
            SandboxMode::WorkspaceWrite | SandboxMode::ReadOnly => NetworkAccess::Restricted,
        };

        DeveloperInstructions::sandbox_text(mode, network_access)
    }
}

fn should_serialize_reasoning_content(content: &Option<Vec<ReasoningItemContent>>) -> bool {
    match content {
        Some(content) => !content
            .iter()
            .any(|c| matches!(c, ReasoningItemContent::ReasoningText { .. })),
        None => false,
    }
}

fn local_image_error_placeholder(
    path: &std::path::Path,
    error: impl std::fmt::Display,
) -> ContentItem {
    ContentItem::InputText {
        text: format!(
            "Ata could not read the local image at `{}`: {}",
            path.display(),
            error
        ),
    }
}

pub const VIEW_IMAGE_TOOL_NAME: &str = "view_image";

const IMAGE_OPEN_TAG: &str = "<image>";
const IMAGE_CLOSE_TAG: &str = "</image>";
const LOCAL_IMAGE_OPEN_TAG_PREFIX: &str = "<image name=";
const LOCAL_IMAGE_OPEN_TAG_SUFFIX: &str = ">";
const LOCAL_IMAGE_CLOSE_TAG: &str = IMAGE_CLOSE_TAG;
const FILE_OPEN_TAG_BARE: &str = "<file_start>";
const FILE_OPEN_TAG_PREFIX: &str = "<file_start";
const FILE_OPEN_TAG_SUFFIX: &str = "</file_start>";
const FILE_CLOSE_TAG: &str = "<file_end></file_end>";

pub fn image_open_tag_text() -> String {
    IMAGE_OPEN_TAG.to_string()
}

pub fn image_close_tag_text() -> String {
    IMAGE_CLOSE_TAG.to_string()
}

pub fn local_image_label_text(label_number: usize) -> String {
    format!("[Image #{label_number}]")
}

pub fn local_file_label_text(label_number: usize) -> String {
    format!("[File #{label_number}]")
}

pub fn local_image_open_tag_text(label_number: usize) -> String {
    let label = local_image_label_text(label_number);
    format!("{LOCAL_IMAGE_OPEN_TAG_PREFIX}{label}{LOCAL_IMAGE_OPEN_TAG_SUFFIX}")
}

pub fn local_file_open_tag_text(label_number: usize) -> String {
    local_file_open_tag_text_with_filename(label_number, None)
}

pub fn local_file_open_tag_text_with_filename(
    label_number: usize,
    filename: Option<&str>,
) -> String {
    let label = local_file_label_text(label_number);
    match filename.filter(|name| !name.is_empty()) {
        Some(filename) => {
            let escaped_filename = escape_tag_attribute_value(filename);
            format!(
                r#"{FILE_OPEN_TAG_PREFIX} name="{escaped_filename}">{label}{FILE_OPEN_TAG_SUFFIX}"#
            )
        }
        None => format!("{FILE_OPEN_TAG_BARE}{label}{FILE_OPEN_TAG_SUFFIX}"),
    }
}

pub fn is_local_image_open_tag_text(text: &str) -> bool {
    text.strip_prefix(LOCAL_IMAGE_OPEN_TAG_PREFIX)
        .is_some_and(|rest| rest.ends_with(LOCAL_IMAGE_OPEN_TAG_SUFFIX))
}

pub fn is_local_file_open_tag_text(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(FILE_OPEN_TAG_PREFIX) else {
        return false;
    };
    let Some((attributes, tail)) = rest.split_once('>') else {
        return false;
    };
    if !attributes.is_empty() && !attributes.starts_with(' ') {
        return false;
    }
    tail.ends_with(FILE_OPEN_TAG_SUFFIX)
}

pub fn is_local_image_close_tag_text(text: &str) -> bool {
    is_image_close_tag_text(text)
}

pub fn is_local_file_close_tag_text(text: &str) -> bool {
    is_file_close_tag_text(text)
}

pub fn is_image_open_tag_text(text: &str) -> bool {
    text == IMAGE_OPEN_TAG
}

pub fn is_file_open_tag_text(text: &str) -> bool {
    text == FILE_OPEN_TAG_BARE || is_local_file_open_tag_text(text)
}

pub fn is_image_close_tag_text(text: &str) -> bool {
    text == IMAGE_CLOSE_TAG
}

pub fn is_file_close_tag_text(text: &str) -> bool {
    text == FILE_CLOSE_TAG
}

fn escape_tag_attribute_value(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn invalid_image_error_placeholder(
    path: &std::path::Path,
    error: impl std::fmt::Display,
) -> ContentItem {
    ContentItem::InputText {
        text: format!(
            "Image located at `{}` is invalid: {}",
            path.display(),
            error
        ),
    }
}

fn unsupported_image_error_placeholder(path: &std::path::Path, mime: &str) -> ContentItem {
    ContentItem::InputText {
        text: format!(
            "Ata cannot attach image at `{}`: unsupported image format `{}`.",
            path.display(),
            mime
        ),
    }
}

pub fn local_image_content_items_with_label_number(
    path: &std::path::Path,
    label_number: Option<usize>,
) -> Vec<ContentItem> {
    match load_and_resize_to_fit(path) {
        Ok(image) => {
            let mut items = Vec::with_capacity(3);
            if let Some(label_number) = label_number {
                items.push(ContentItem::InputText {
                    text: local_image_open_tag_text(label_number),
                });
            }
            items.push(ContentItem::InputImage {
                image_url: image.into_data_url(),
            });
            if label_number.is_some() {
                items.push(ContentItem::InputText {
                    text: LOCAL_IMAGE_CLOSE_TAG.to_string(),
                });
            }
            items
        }
        Err(err) => {
            if matches!(&err, ImageProcessingError::Read { .. }) {
                vec![local_image_error_placeholder(path, &err)]
            } else if err.is_invalid_image() {
                vec![invalid_image_error_placeholder(path, &err)]
            } else {
                let Some(mime_guess) = mime_guess::from_path(path).first() else {
                    return vec![local_image_error_placeholder(
                        path,
                        "unsupported MIME type (unknown)",
                    )];
                };
                let mime = mime_guess.essence_str().to_owned();
                if !mime.starts_with("image/") {
                    return vec![local_image_error_placeholder(
                        path,
                        format!("unsupported MIME type `{mime}`"),
                    )];
                }
                vec![unsupported_image_error_placeholder(path, &mime)]
            }
        }
    }
}

pub fn local_file_content_items(path: &Path, label_number: Option<usize>) -> Vec<ContentItem> {
    match encode_inline_cached(path) {
        Ok(processed) => {
            let processed = into_owned_processed_file(processed);
            let filename = processed.filename.clone();
            let file_item = ContentItem::inline_file(
                processed.base64,
                processed.mime_type,
                Some(filename.clone()),
            );
            wrap_file_content_items(file_item, label_number, Some(filename.as_str()))
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "local file encoding failed; inserting error placeholder"
            );
            vec![ContentItem::InputText {
                text: format!(
                    "Codex could not read the local file at `{}`: {error}",
                    path.display()
                ),
            }]
        }
    }
}

fn wrap_file_content_items(
    file_item: ContentItem,
    label_number: Option<usize>,
    filename: Option<&str>,
) -> Vec<ContentItem> {
    let mut items = Vec::with_capacity(if label_number.is_some() { 3 } else { 1 });
    if let Some(label_number) = label_number {
        items.push(ContentItem::InputText {
            text: local_file_open_tag_text_with_filename(label_number, filename),
        });
    }
    items.push(file_item);
    if label_number.is_some() {
        items.push(ContentItem::InputText {
            text: FILE_CLOSE_TAG.to_string(),
        });
    }
    items
}

impl From<ResponseInputItem> for ResponseItem {
    fn from(item: ResponseInputItem) -> Self {
        match item {
            ResponseInputItem::Message { role, content } => Self::Message {
                role,
                content,
                id: None,
                end_turn: None,
                phase: None,
            },
            ResponseInputItem::FunctionCallOutput { call_id, output } => {
                Self::FunctionCallOutput { call_id, output }
            }
            ResponseInputItem::McpToolCallOutput { call_id, result } => {
                let output = match result {
                    Ok(result) => FunctionCallOutputPayload::from(&result),
                    Err(tool_call_err) => FunctionCallOutputPayload {
                        body: FunctionCallOutputBody::Text(format!("err: {tool_call_err:?}")),
                        success: Some(false),
                    },
                };
                Self::FunctionCallOutput { call_id, output }
            }
            ResponseInputItem::CustomToolCallOutput { call_id, output } => {
                Self::CustomToolCallOutput { call_id, output }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum LocalShellStatus {
    Completed,
    InProgress,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalShellAction {
    Exec(LocalShellExecAction),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
pub struct LocalShellExecAction {
    pub command: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub working_directory: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebSearchAction {
    Search {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        queries: Option<Vec<String>>,
    },
    OpenPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        url: Option<String>,
    },
    FindInPage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        pattern: Option<String>,
    },

    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningItemReasoningSummary {
    SummaryText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningItemContent {
    ReasoningText { text: String },
    Text { text: String },
}

impl From<Vec<UserInput>> for ResponseInputItem {
    fn from(items: Vec<UserInput>) -> Self {
        // Use a single attachment counter so images and files share the same
        // numbering sequence, matching the composer's combined label counter.
        let mut attachment_index = 0;
        Self::Message {
            role: "user".to_string(),
            content: items
                .into_iter()
                .flat_map(|c| match c {
                    UserInput::Text { text, .. } => vec![ContentItem::InputText { text }],
                    UserInput::Image { image_url } => vec![
                        ContentItem::InputText {
                            text: image_open_tag_text(),
                        },
                        ContentItem::InputImage { image_url },
                        ContentItem::InputText {
                            text: image_close_tag_text(),
                        },
                    ],
                    UserInput::LocalImage { path } => {
                        attachment_index += 1;
                        local_image_content_items_with_label_number(&path, Some(attachment_index))
                    }
                    UserInput::LocalFile { path } => {
                        attachment_index += 1;
                        local_file_content_items(&path, Some(attachment_index))
                    }
                    UserInput::UploadedFile {
                        file_id,
                        mime_type,
                        filename,
                        ..
                    } => {
                        attachment_index += 1;
                        let file_item =
                            ContentItem::file_ref(file_id, mime_type, Some(filename.clone()));
                        wrap_file_content_items(
                            file_item,
                            Some(attachment_index),
                            Some(filename.as_str()),
                        )
                    }
                    UserInput::Skill { .. } | UserInput::Mention { .. } => Vec::new(), // Tool bodies are injected later in core
                })
                .collect::<Vec<ContentItem>>(),
        }
    }
}

/// If the `name` of a `ResponseItem::FunctionCall` is either `container.exec`
/// or `shell`, the `arguments` field should deserialize to this struct.
#[derive(Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
pub struct ShellToolCallParams {
    pub command: Vec<String>,
    pub workdir: Option<String>,

    /// This is the maximum time in milliseconds that the command is allowed to run.
    #[serde(alias = "timeout")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sandbox_permissions: Option<SandboxPermissions>,
    /// Suggests a command prefix to persist for future sessions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prefix_rule: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

/// If the `name` of a `ResponseItem::FunctionCall` is `shell_command`, the
/// `arguments` field should deserialize to this struct.
#[derive(Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
pub struct ShellCommandToolCallParams {
    pub command: String,
    pub workdir: Option<String>,

    /// Whether to run the shell with login shell semantics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<bool>,
    /// This is the maximum time in milliseconds that the command is allowed to run.
    #[serde(alias = "timeout")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sandbox_permissions: Option<SandboxPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prefix_rule: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
}

/// Responses API compatible content items that can be returned by a tool call.
/// This is a subset of ContentItem with the types we support as function call outputs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FunctionCallOutputContentItem {
    // Do not rename, these are serialized and used directly in the responses API.
    InputText { text: String },
    // Do not rename, these are serialized and used directly in the responses API.
    InputImage { image_url: String },
}

/// Converts structured function-call output content into plain text for
/// human-readable surfaces.
///
/// This conversion is intentionally lossy:
/// - only `input_text` items are included
/// - image items are ignored
///
/// We use this helper where callers still need a string representation (for
/// example telemetry previews or legacy string-only output paths) while keeping
/// the original multimodal `content_items` as the authoritative payload sent to
/// the model.
pub fn function_call_output_content_items_to_text(
    content_items: &[FunctionCallOutputContentItem],
) -> Option<String> {
    let text_segments = content_items
        .iter()
        .filter_map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } if !text.trim().is_empty() => {
                Some(text.as_str())
            }
            FunctionCallOutputContentItem::InputText { .. }
            | FunctionCallOutputContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>();

    if text_segments.is_empty() {
        None
    } else {
        Some(text_segments.join("\n"))
    }
}

impl From<crate::dynamic_tools::DynamicToolCallOutputContentItem>
    for FunctionCallOutputContentItem
{
    fn from(item: crate::dynamic_tools::DynamicToolCallOutputContentItem) -> Self {
        match item {
            crate::dynamic_tools::DynamicToolCallOutputContentItem::InputText { text } => {
                Self::InputText { text }
            }
            crate::dynamic_tools::DynamicToolCallOutputContentItem::InputImage { image_url } => {
                Self::InputImage { image_url }
            }
        }
    }
}

/// The payload we send back to OpenAI when reporting a tool call result.
///
/// `body` serializes directly as the wire value for `function_call_output.output`.
/// `success` remains internal metadata for downstream handling.
#[derive(Debug, Default, Clone, PartialEq, JsonSchema, TS)]
pub struct FunctionCallOutputPayload {
    pub body: FunctionCallOutputBody,
    pub success: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema, TS)]
#[serde(untagged)]
pub enum FunctionCallOutputBody {
    Text(String),
    ContentItems(Vec<FunctionCallOutputContentItem>),
}

impl FunctionCallOutputBody {
    /// Best-effort conversion of a function-call output body to plain text for
    /// human-readable surfaces.
    ///
    /// This conversion is intentionally lossy when the body contains content
    /// items: image entries are dropped and text entries are joined with
    /// newlines.
    pub fn to_text(&self) -> Option<String> {
        match self {
            Self::Text(content) => Some(content.clone()),
            Self::ContentItems(items) => function_call_output_content_items_to_text(items),
        }
    }
}

impl Default for FunctionCallOutputBody {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl FunctionCallOutputPayload {
    pub fn from_text(content: String) -> Self {
        Self {
            body: FunctionCallOutputBody::Text(content),
            success: None,
        }
    }

    pub fn from_content_items(content_items: Vec<FunctionCallOutputContentItem>) -> Self {
        Self {
            body: FunctionCallOutputBody::ContentItems(content_items),
            success: None,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        match &self.body {
            FunctionCallOutputBody::Text(content) => Some(content),
            FunctionCallOutputBody::ContentItems(_) => None,
        }
    }

    pub fn text_content_mut(&mut self) -> Option<&mut String> {
        match &mut self.body {
            FunctionCallOutputBody::Text(content) => Some(content),
            FunctionCallOutputBody::ContentItems(_) => None,
        }
    }

    pub fn content_items(&self) -> Option<&[FunctionCallOutputContentItem]> {
        match &self.body {
            FunctionCallOutputBody::Text(_) => None,
            FunctionCallOutputBody::ContentItems(items) => Some(items),
        }
    }

    pub fn content_items_mut(&mut self) -> Option<&mut Vec<FunctionCallOutputContentItem>> {
        match &mut self.body {
            FunctionCallOutputBody::Text(_) => None,
            FunctionCallOutputBody::ContentItems(items) => Some(items),
        }
    }
}

// `function_call_output.output` is encoded as either:
//   - an array of structured content items
//   - a plain string
impl Serialize for FunctionCallOutputPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.body {
            FunctionCallOutputBody::Text(content) => serializer.serialize_str(content),
            FunctionCallOutputBody::ContentItems(items) => items.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for FunctionCallOutputPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let body = FunctionCallOutputBody::deserialize(deserializer)?;
        Ok(FunctionCallOutputPayload {
            body,
            success: None,
        })
    }
}

impl From<&CallToolResult> for FunctionCallOutputPayload {
    fn from(call_tool_result: &CallToolResult) -> Self {
        let CallToolResult {
            content,
            structured_content,
            is_error,
            meta: _,
        } = call_tool_result;

        let is_success = is_error != &Some(true);

        if let Some(structured_content) = structured_content
            && !structured_content.is_null()
        {
            match serde_json::to_string(structured_content) {
                Ok(serialized_structured_content) => {
                    return FunctionCallOutputPayload {
                        body: FunctionCallOutputBody::Text(serialized_structured_content),
                        success: Some(is_success),
                    };
                }
                Err(err) => {
                    return FunctionCallOutputPayload {
                        body: FunctionCallOutputBody::Text(err.to_string()),
                        success: Some(false),
                    };
                }
            }
        }

        let serialized_content = match serde_json::to_string(content) {
            Ok(serialized_content) => serialized_content,
            Err(err) => {
                return FunctionCallOutputPayload {
                    body: FunctionCallOutputBody::Text(err.to_string()),
                    success: Some(false),
                };
            }
        };

        let content_items = convert_mcp_content_to_items(content);

        let body = match content_items {
            Some(content_items) => FunctionCallOutputBody::ContentItems(content_items),
            None => FunctionCallOutputBody::Text(serialized_content),
        };

        FunctionCallOutputPayload {
            body,
            success: Some(is_success),
        }
    }
}

fn convert_mcp_content_to_items(
    contents: &[serde_json::Value],
) -> Option<Vec<FunctionCallOutputContentItem>> {
    #[derive(serde::Deserialize)]
    #[serde(tag = "type")]
    enum McpContent {
        #[serde(rename = "text")]
        Text { text: String },
        #[serde(rename = "image")]
        Image {
            data: String,
            #[serde(rename = "mimeType", alias = "mime_type")]
            mime_type: Option<String>,
        },
        #[serde(other)]
        Unknown,
    }

    let mut saw_image = false;
    let mut items = Vec::with_capacity(contents.len());

    for content in contents {
        let item = match serde_json::from_value::<McpContent>(content.clone()) {
            Ok(McpContent::Text { text }) => FunctionCallOutputContentItem::InputText { text },
            Ok(McpContent::Image { data, mime_type }) => {
                saw_image = true;
                let image_url = if data.starts_with("data:") {
                    data
                } else {
                    let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".into());
                    format!("data:{mime_type};base64,{data}")
                };
                FunctionCallOutputContentItem::InputImage { image_url }
            }
            Ok(McpContent::Unknown) | Err(_) => FunctionCallOutputContentItem::InputText {
                text: serde_json::to_string(content).unwrap_or_else(|_| "<content>".to_string()),
            },
        };
        items.push(item);
    }

    if saw_image { Some(items) } else { None }
}

// Implement Display so callers can treat the payload like a plain string when logging or doing
// trivial substring checks in tests (existing tests call `.contains()` on the output). For
// `ContentItems`, Display emits a JSON representation.

impl std::fmt::Display for FunctionCallOutputPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.body {
            FunctionCallOutputBody::Text(content) => f.write_str(content),
            FunctionCallOutputBody::ContentItems(items) => {
                let content = serde_json::to_string(items).unwrap_or_default();
                f.write_str(content.as_str())
            }
        }
    }
}

// (Moved event mapping logic into codex-core to avoid coupling protocol to UI-facing events.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_types::SandboxMode;
    use crate::protocol::AskForApproval;
    use anyhow::Result;
    use codex_execpolicy::Policy;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn convert_mcp_content_to_items_preserves_data_urls() {
        let contents = vec![serde_json::json!({
            "type": "image",
            "data": "data:image/png;base64,Zm9v",
            "mimeType": "image/png",
        })];

        let items = convert_mcp_content_to_items(&contents).expect("expected image items");
        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,Zm9v".to_string(),
            }]
        );
    }

    #[test]
    fn convert_mcp_content_to_items_builds_data_urls_when_missing_prefix() {
        let contents = vec![serde_json::json!({
            "type": "image",
            "data": "Zm9v",
            "mimeType": "image/png",
        })];

        let items = convert_mcp_content_to_items(&contents).expect("expected image items");
        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,Zm9v".to_string(),
            }]
        );
    }

    #[test]
    fn convert_mcp_content_to_items_returns_none_without_images() {
        let contents = vec![serde_json::json!({
            "type": "text",
            "text": "hello",
        })];

        assert_eq!(convert_mcp_content_to_items(&contents), None);
    }

    #[test]
    fn function_call_output_content_items_to_text_joins_text_segments() {
        let content_items = vec![
            FunctionCallOutputContentItem::InputText {
                text: "line 1".to_string(),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,AAA".to_string(),
            },
            FunctionCallOutputContentItem::InputText {
                text: "line 2".to_string(),
            },
        ];

        let text = function_call_output_content_items_to_text(&content_items);
        assert_eq!(text, Some("line 1\nline 2".to_string()));
    }

    #[test]
    fn function_call_output_content_items_to_text_ignores_blank_text_and_images() {
        let content_items = vec![
            FunctionCallOutputContentItem::InputText {
                text: "   ".to_string(),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,AAA".to_string(),
            },
        ];

        let text = function_call_output_content_items_to_text(&content_items);
        assert_eq!(text, None);
    }

    #[test]
    fn function_call_output_body_to_text_returns_plain_text_content() {
        let body = FunctionCallOutputBody::Text("ok".to_string());
        let text = body.to_text();
        assert_eq!(text, Some("ok".to_string()));
    }

    #[test]
    fn function_call_output_body_to_text_uses_content_item_fallback() {
        let body = FunctionCallOutputBody::ContentItems(vec![
            FunctionCallOutputContentItem::InputText {
                text: "line 1".to_string(),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,AAA".to_string(),
            },
        ]);

        let text = body.to_text();
        assert_eq!(text, Some("line 1".to_string()));
    }

    #[test]
    fn converts_sandbox_mode_into_developer_instructions() {
        let workspace_write: DeveloperInstructions = SandboxMode::WorkspaceWrite.into();
        assert_eq!(
            workspace_write,
            DeveloperInstructions::new(
                "Filesystem sandboxing defines which files can be read or written. `sandbox_mode` is `workspace-write`: The sandbox permits reading files, and editing files in `cwd` and `writable_roots`. Editing files in other directories requires approval. Network access is restricted."
            )
        );

        let read_only: DeveloperInstructions = SandboxMode::ReadOnly.into();
        assert_eq!(
            read_only,
            DeveloperInstructions::new(
                "Filesystem sandboxing defines which files can be read or written. `sandbox_mode` is `read-only`: The sandbox only permits reading files. Network access is restricted."
            )
        );
    }

    #[test]
    fn builds_permissions_with_network_access_override() {
        let instructions = DeveloperInstructions::from_permissions_with_network(
            SandboxMode::WorkspaceWrite,
            NetworkAccess::Enabled,
            AskForApproval::OnRequest,
            &Policy::empty(),
            false,
            None,
        );

        let text = instructions.into_text();
        assert!(
            text.contains("Network access is enabled."),
            "expected network access to be enabled in message"
        );
        assert!(
            text.contains("`approval_policy` is `on-request`"),
            "expected approval guidance to be included"
        );
    }

    #[test]
    fn builds_permissions_from_policy() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            read_only_access: Default::default(),
            network_access: true,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        };

        let instructions = DeveloperInstructions::from_policy(
            &policy,
            AskForApproval::UnlessTrusted,
            &Policy::empty(),
            false,
            &PathBuf::from("/tmp"),
        );
        let text = instructions.into_text();
        assert!(text.contains("Network access is enabled."));
        assert!(text.contains("`approval_policy` is `unless-trusted`"));
    }

    #[test]
    fn includes_request_rule_instructions_when_enabled() {
        let mut exec_policy = Policy::empty();
        exec_policy
            .add_prefix_rule(
                &["git".to_string(), "pull".to_string()],
                codex_execpolicy::Decision::Allow,
            )
            .expect("add rule");
        let instructions = DeveloperInstructions::from_permissions_with_network(
            SandboxMode::WorkspaceWrite,
            NetworkAccess::Enabled,
            AskForApproval::OnRequest,
            &exec_policy,
            true,
            None,
        );

        let text = instructions.into_text();
        assert!(text.contains("prefix_rule"));
        assert!(text.contains("Approved command prefixes"));
        assert!(text.contains(r#"["git", "pull"]"#));
    }

    #[test]
    fn render_command_prefix_list_sorts_by_len_then_total_len_then_alphabetical() {
        let prefixes = vec![
            vec!["b".to_string(), "zz".to_string()],
            vec!["aa".to_string()],
            vec!["b".to_string()],
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            vec!["a".to_string()],
            vec!["b".to_string(), "a".to_string()],
        ];

        let output = format_allow_prefixes(prefixes).expect("rendered list");
        assert_eq!(
            output,
            r#"- ["a"]
- ["b"]
- ["aa"]
- ["b", "a"]
- ["b", "zz"]
- ["a", "b", "c"]"#
                .to_string(),
        );
    }

    #[test]
    fn render_command_prefix_list_limits_output_to_max_prefixes() {
        let prefixes = (0..(MAX_RENDERED_PREFIXES + 5))
            .map(|i| vec![format!("{i:03}")])
            .collect::<Vec<_>>();

        let output = format_allow_prefixes(prefixes).expect("rendered list");
        assert_eq!(output.ends_with(TRUNCATED_MARKER), true);
        eprintln!("output: {output}");
        assert_eq!(output.lines().count(), MAX_RENDERED_PREFIXES + 1);
    }

    #[test]
    fn format_allow_prefixes_limits_output() {
        let mut exec_policy = Policy::empty();
        for i in 0..200 {
            exec_policy
                .add_prefix_rule(
                    &[format!("tool-{i:03}"), "x".repeat(500)],
                    codex_execpolicy::Decision::Allow,
                )
                .expect("add rule");
        }

        let output =
            format_allow_prefixes(exec_policy.get_allowed_prefixes()).expect("formatted prefixes");
        assert!(
            output.len() <= MAX_ALLOW_PREFIX_TEXT_BYTES + TRUNCATED_MARKER.len(),
            "output length exceeds expected limit: {output}",
        );
    }

    #[test]
    fn serializes_success_as_plain_string() -> Result<()> {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload::from_text("ok".into()),
        };

        let json = serde_json::to_string(&item)?;
        let v: serde_json::Value = serde_json::from_str(&json)?;

        // Success case -> output should be a plain string
        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "ok");
        Ok(())
    }

    #[test]
    fn serializes_failure_as_string() -> Result<()> {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload {
                body: FunctionCallOutputBody::Text("bad".into()),
                success: Some(false),
            },
        };

        let json = serde_json::to_string(&item)?;
        let v: serde_json::Value = serde_json::from_str(&json)?;

        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "bad");
        Ok(())
    }

    #[test]
    fn serializes_image_outputs_as_array() -> Result<()> {
        let call_tool_result = CallToolResult {
            content: vec![
                serde_json::json!({"type":"text","text":"caption"}),
                serde_json::json!({"type":"image","data":"BASE64","mimeType":"image/png"}),
            ],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        };

        let payload = FunctionCallOutputPayload::from(&call_tool_result);
        assert_eq!(payload.success, Some(true));
        let Some(items) = payload.content_items() else {
            panic!("expected content items");
        };
        let items = items.to_vec();
        assert_eq!(
            items,
            vec![
                FunctionCallOutputContentItem::InputText {
                    text: "caption".into(),
                },
                FunctionCallOutputContentItem::InputImage {
                    image_url: "data:image/png;base64,BASE64".into(),
                },
            ]
        );

        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: payload,
        };

        let json = serde_json::to_string(&item)?;
        let v: serde_json::Value = serde_json::from_str(&json)?;

        let output = v.get("output").expect("output field");
        assert!(output.is_array(), "expected array output");

        Ok(())
    }

    #[test]
    fn preserves_existing_image_data_urls() -> Result<()> {
        let call_tool_result = CallToolResult {
            content: vec![serde_json::json!({
                "type": "image",
                "data": "data:image/png;base64,BASE64",
                "mimeType": "image/png"
            })],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        };

        let payload = FunctionCallOutputPayload::from(&call_tool_result);
        let Some(items) = payload.content_items() else {
            panic!("expected content items");
        };
        let items = items.to_vec();
        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,BASE64".into(),
            }]
        );

        Ok(())
    }

    #[test]
    fn deserializes_array_payload_into_items() -> Result<()> {
        let json = r#"[
            {"type": "input_text", "text": "note"},
            {"type": "input_image", "image_url": "data:image/png;base64,XYZ"}
        ]"#;

        let payload: FunctionCallOutputPayload = serde_json::from_str(json)?;

        assert_eq!(payload.success, None);
        let expected_items = vec![
            FunctionCallOutputContentItem::InputText {
                text: "note".into(),
            },
            FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,XYZ".into(),
            },
        ];
        assert_eq!(
            payload.body,
            FunctionCallOutputBody::ContentItems(expected_items.clone())
        );
        assert_eq!(
            serde_json::to_string(&payload)?,
            serde_json::to_string(&expected_items)?
        );

        Ok(())
    }

    #[test]
    fn deserializes_compaction_alias() -> Result<()> {
        let json = r#"{"type":"compaction_summary","encrypted_content":"abc"}"#;

        let item: ResponseItem = serde_json::from_str(json)?;

        assert_eq!(
            item,
            ResponseItem::Compaction {
                encrypted_content: "abc".into(),
            }
        );
        Ok(())
    }

    #[test]
    fn roundtrips_web_search_call_actions() -> Result<()> {
        let cases = vec![
            (
                r#"{
                    "type": "web_search_call",
                    "status": "completed",
                    "action": {
                        "type": "search",
                        "query": "weather seattle",
                        "queries": ["weather seattle", "seattle weather now"]
                    }
                }"#,
                None,
                Some(WebSearchAction::Search {
                    query: Some("weather seattle".into()),
                    queries: Some(vec!["weather seattle".into(), "seattle weather now".into()]),
                }),
                Some("completed".into()),
                true,
            ),
            (
                r#"{
                    "type": "web_search_call",
                    "status": "open",
                    "action": {
                        "type": "open_page",
                        "url": "https://example.com"
                    }
                }"#,
                None,
                Some(WebSearchAction::OpenPage {
                    url: Some("https://example.com".into()),
                }),
                Some("open".into()),
                true,
            ),
            (
                r#"{
                    "type": "web_search_call",
                    "status": "in_progress",
                    "action": {
                        "type": "find_in_page",
                        "url": "https://example.com/docs",
                        "pattern": "installation"
                    }
                }"#,
                None,
                Some(WebSearchAction::FindInPage {
                    url: Some("https://example.com/docs".into()),
                    pattern: Some("installation".into()),
                }),
                Some("in_progress".into()),
                true,
            ),
            (
                r#"{
                    "type": "web_search_call",
                    "status": "in_progress",
                    "id": "ws_partial"
                }"#,
                Some("ws_partial".into()),
                None,
                Some("in_progress".into()),
                false,
            ),
        ];

        for (json_literal, expected_id, expected_action, expected_status, expect_roundtrip) in cases
        {
            let parsed: ResponseItem = serde_json::from_str(json_literal)?;
            let expected = ResponseItem::WebSearchCall {
                id: expected_id.clone(),
                status: expected_status.clone(),
                action: expected_action.clone(),
            };
            assert_eq!(parsed, expected);

            let serialized = serde_json::to_value(&parsed)?;
            let mut expected_serialized: serde_json::Value = serde_json::from_str(json_literal)?;
            if !expect_roundtrip && let Some(obj) = expected_serialized.as_object_mut() {
                obj.remove("id");
            }
            assert_eq!(serialized, expected_serialized);
        }

        Ok(())
    }

    #[test]
    fn deserialize_shell_tool_call_params() -> Result<()> {
        let json = r#"{
            "command": ["ls", "-l"],
            "workdir": "/tmp",
            "timeout": 1000
        }"#;

        let params: ShellToolCallParams = serde_json::from_str(json)?;
        assert_eq!(
            ShellToolCallParams {
                command: vec!["ls".to_string(), "-l".to_string()],
                workdir: Some("/tmp".to_string()),
                timeout_ms: Some(1000),
                sandbox_permissions: None,
                prefix_rule: None,
                justification: None,
            },
            params
        );
        Ok(())
    }

    #[test]
    fn wraps_image_user_input_with_tags() -> Result<()> {
        let image_url = "data:image/png;base64,abc".to_string();

        let item = ResponseInputItem::from(vec![UserInput::Image {
            image_url: image_url.clone(),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                let expected = vec![
                    ContentItem::InputText {
                        text: image_open_tag_text(),
                    },
                    ContentItem::InputImage { image_url },
                    ContentItem::InputText {
                        text: image_close_tag_text(),
                    },
                ];
                assert_eq!(content, expected);
            }
            other => panic!("expected message response but got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn local_image_read_error_adds_placeholder() -> Result<()> {
        let dir = tempdir()?;
        let missing_path = dir.path().join("missing-image.png");

        let item = ResponseInputItem::from(vec![UserInput::LocalImage {
            path: missing_path.clone(),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    ContentItem::InputText { text } => {
                        let display_path = missing_path.display().to_string();
                        assert!(
                            text.contains(&display_path),
                            "placeholder should mention missing path: {text}"
                        );
                        assert!(
                            text.contains("could not read"),
                            "placeholder should mention read issue: {text}"
                        );
                    }
                    other => panic!("expected placeholder text but found {other:?}"),
                }
            }
            other => panic!("expected message response but got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn local_image_non_image_adds_placeholder() -> Result<()> {
        let dir = tempdir()?;
        let json_path = dir.path().join("example.json");
        std::fs::write(&json_path, br#"{"hello":"world"}"#)?;

        let item = ResponseInputItem::from(vec![UserInput::LocalImage {
            path: json_path.clone(),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(content.len(), 1);
                match &content[0] {
                    ContentItem::InputText { text } => {
                        assert!(
                            text.contains("unsupported MIME type `application/json`"),
                            "placeholder should mention unsupported MIME: {text}"
                        );
                        assert!(
                            text.contains(&json_path.display().to_string()),
                            "placeholder should mention path: {text}"
                        );
                    }
                    other => panic!("expected placeholder text but found {other:?}"),
                }
            }
            other => panic!("expected message response but got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn local_image_unsupported_image_format_adds_placeholder() -> Result<()> {
        let dir = tempdir()?;
        let svg_path = dir.path().join("example.svg");
        std::fs::write(
            &svg_path,
            br#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
        )?;

        let item = ResponseInputItem::from(vec![UserInput::LocalImage {
            path: svg_path.clone(),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(content.len(), 1);
                let expected = format!(
                    "Ata cannot attach image at `{}`: unsupported image format `image/svg+xml`.",
                    svg_path.display()
                );
                match &content[0] {
                    ContentItem::InputText { text } => assert_eq!(text, &expected),
                    other => panic!("expected placeholder text but found {other:?}"),
                }
            }
            other => panic!("expected message response but got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn input_file_inline_has_file_data_only() {
        let item = ContentItem::inline_file(
            "JVBERi0xLjQ=".to_string(),
            "application/pdf".to_string(),
            Some("report.pdf".to_string()),
        );

        match item {
            ContentItem::InputFile {
                file_data,
                file_id,
                mime_type,
                filename,
            } => {
                assert_eq!(file_data, Some("JVBERi0xLjQ=".to_string()));
                assert_eq!(file_id, None);
                assert_eq!(mime_type, Some("application/pdf".to_string()));
                assert_eq!(filename, Some("report.pdf".to_string()));
            }
            other => panic!("expected input file but found {other:?}"),
        }
    }

    #[test]
    fn input_file_ref_has_file_id_only() {
        let item = ContentItem::file_ref(
            "file-123".to_string(),
            "application/pdf".to_string(),
            Some("report.pdf".to_string()),
        );

        match item {
            ContentItem::InputFile {
                file_data,
                file_id,
                mime_type,
                filename,
            } => {
                assert_eq!(file_data, None);
                assert_eq!(file_id, Some("file-123".to_string()));
                assert_eq!(mime_type, Some("application/pdf".to_string()));
                assert_eq!(filename, Some("report.pdf".to_string()));
            }
            other => panic!("expected input file but found {other:?}"),
        }
    }

    #[test]
    fn local_file_open_tag_with_filename_is_detected() {
        let tag = local_file_open_tag_text_with_filename(1, Some("report.pdf"));
        assert!(is_local_file_open_tag_text(&tag));
        assert!(is_file_open_tag_text(&tag));
    }

    #[test]
    fn input_file_deserialization_rejects_both_file_data_and_file_id() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "input_file",
            "file_data": "JVBERi0xLjQ=",
            "file_id": "file-123",
            "mime_type": "application/pdf",
            "filename": "report.pdf",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("input_file must include exactly one of `file_data` or `file_id`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn input_file_deserialization_rejects_missing_file_data_and_file_id() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "input_file",
            "mime_type": "application/pdf",
            "filename": "report.pdf",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("input_file must include exactly one of `file_data` or `file_id`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn input_file_deserialization_rejects_empty_file_data() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "input_file",
            "file_data": "",
            "mime_type": "application/pdf",
            "filename": "report.pdf",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("input_file must include exactly one of `file_data` or `file_id`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn input_file_deserialization_rejects_empty_file_id() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "input_file",
            "file_id": "",
            "mime_type": "application/pdf",
            "filename": "report.pdf",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("input_file must include exactly one of `file_data` or `file_id`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_deserialization_accepts_non_empty_url() {
        let item = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": " https://example.com/report.pdf ",
            "filename": "report.pdf",
            "mime_type": "application/pdf",
        }))
        .expect("deserialization should succeed");

        assert_eq!(
            item,
            ContentItem::UrlFile {
                url: "https://example.com/report.pdf".to_string(),
                mime_type: Some("application/pdf".to_string()),
                filename: Some("report.pdf".to_string()),
            }
        );
    }

    #[test]
    fn url_file_deserialization_rejects_invalid_scheme() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": "file:///tmp/report.pdf",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("url_file `url` must use http or https"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_deserialization_rejects_non_url_values() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": "not-a-url",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("url_file `url` must be a valid absolute URL"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_deserialization_rejects_overly_long_urls() {
        let long_url = format!(
            "https://example.com/{}",
            "a".repeat(MAX_URL_FILE_URL_LENGTH)
        );
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": long_url,
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("url_file `url` exceeds max length of 8192 characters"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_deserialization_rejects_empty_url() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": "",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("url_file requires a non-empty `url`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_deserialization_rejects_whitespace_url() {
        let err = serde_json::from_value::<ContentItem>(serde_json::json!({
            "type": "url_file",
            "url": "   ",
        }))
        .expect_err("deserialization should fail");

        assert!(
            err.to_string()
                .contains("url_file requires a non-empty `url`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn url_file_round_trip_preserves_fields() {
        let original = ContentItem::UrlFile {
            url: "https://example.com/docs/report.pdf".to_string(),
            mime_type: Some("application/pdf".to_string()),
            filename: Some("report.pdf".to_string()),
        };
        let serialized = serde_json::to_value(&original).expect("serialize");
        let parsed = serde_json::from_value::<ContentItem>(serialized).expect("deserialize");

        assert_eq!(parsed, original);
    }

    #[test]
    fn wraps_local_file_user_input_with_tags() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"%PDF-1.4\nhello")?;

        let item = ResponseInputItem::from(vec![UserInput::LocalFile { path }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(content.len(), 3);
                assert_eq!(
                    content[0],
                    ContentItem::InputText {
                        text: local_file_open_tag_text_with_filename(1, Some("report.pdf")),
                    }
                );
                match &content[1] {
                    ContentItem::InputFile {
                        file_data,
                        file_id,
                        mime_type,
                        filename,
                    } => {
                        assert!(file_data.is_some(), "inline file should include file_data");
                        assert_eq!(file_id, &None);
                        assert_eq!(mime_type, &Some("application/pdf".to_string()));
                        assert_eq!(filename, &Some("report.pdf".to_string()));
                    }
                    other => panic!("expected input file content but found {other:?}"),
                }
                assert_eq!(
                    content[2],
                    ContentItem::InputText {
                        text: FILE_CLOSE_TAG.to_string(),
                    }
                );
            }
            other => panic!("expected message response but got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn local_file_read_error_inserts_error_placeholder() {
        let dir = tempdir().expect("temp dir");
        let missing = dir.path().join("missing-report.pdf");
        let item = ResponseInputItem::from(vec![UserInput::LocalFile {
            path: missing.clone(),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(content.len(), 1, "should have exactly one placeholder item");
                match &content[0] {
                    ContentItem::InputText { text } => {
                        assert!(
                            text.contains(&missing.display().to_string()),
                            "placeholder should mention the file path"
                        );
                        assert!(
                            text.starts_with("Codex could not read"),
                            "placeholder should start with error prefix"
                        );
                    }
                    other => panic!("expected InputText placeholder but got {other:?}"),
                }
            }
            other => panic!("expected message response but got {other:?}"),
        }
    }

    #[test]
    fn uploaded_file_user_input_produces_tagged_file_ref_content_item() {
        let item = ResponseInputItem::from(vec![UserInput::UploadedFile {
            file_id: "file-abc".to_string(),
            mime_type: "application/pdf".to_string(),
            filename: "report.pdf".to_string(),
            source_path: PathBuf::from("/tmp/report.pdf"),
        }]);

        match item {
            ResponseInputItem::Message { content, .. } => {
                assert_eq!(
                    content,
                    vec![
                        ContentItem::InputText {
                            text: local_file_open_tag_text_with_filename(1, Some("report.pdf")),
                        },
                        ContentItem::InputFile {
                            file_data: None,
                            file_id: Some("file-abc".to_string()),
                            mime_type: Some("application/pdf".to_string()),
                            filename: Some("report.pdf".to_string()),
                        },
                        ContentItem::InputText {
                            text: FILE_CLOSE_TAG.to_string(),
                        }
                    ]
                );
            }
            other => panic!("expected message response but got {other:?}"),
        }
    }
}
