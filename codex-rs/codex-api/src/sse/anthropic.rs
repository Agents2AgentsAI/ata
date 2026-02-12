//! Anthropic SSE streaming response parser.
//!
//! Anthropic uses standard SSE with specific event types for content blocks.
//! This module handles the interleaved streaming of text and tool calls.

use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;

use serde::Deserialize;
use serde_json::Value;

use crate::common::ResponseEvent;
use crate::error::ApiError;
use crate::file_support::map_user_facing_file_error_from_message;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;

/// Anthropic SSE event types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicEventType {
    MessageStart,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    MessageDelta,
    MessageStop,
    Ping,
    Error,
}

impl FromStr for AnthropicEventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "message_start" => Ok(Self::MessageStart),
            "content_block_start" => Ok(Self::ContentBlockStart),
            "content_block_delta" => Ok(Self::ContentBlockDelta),
            "content_block_stop" => Ok(Self::ContentBlockStop),
            "message_delta" => Ok(Self::MessageDelta),
            "message_stop" => Ok(Self::MessageStop),
            "ping" => Ok(Self::Ping),
            "error" => Ok(Self::Error),
            _ => Err(()),
        }
    }
}

/// State tracker for Anthropic streaming.
///
/// Anthropic streams content blocks with indices, and tool calls arrive
/// in interleaved chunks. This state tracks the tool call info per index
/// so we can associate delta chunks with the correct tool.
#[derive(Default)]
pub struct AnthropicStreamState {
    /// Maps content block index to (tool_id, tool_name).
    tool_state: HashMap<u32, (String, String)>,
    /// Accumulated arguments for each tool call index.
    tool_arguments: HashMap<u32, String>,
    /// Accumulated text content.
    text_content: String,
    /// Whether we've sent the Created event.
    created_sent: bool,
    /// Response ID from message_start.
    response_id: String,
    /// Stable id used for the assistant message turn item (so start/done match).
    assistant_message_id: Option<String>,

    // Thinking/reasoning state
    /// Block indices that are thinking blocks.
    thinking_blocks: HashSet<u32>,
    /// Current thinking summary index (incremented per thinking block).
    thinking_index: i64,
    /// Whether we are currently inside a thinking content block.
    in_thinking_section: bool,
    /// Accumulated thinking text for the current thinking block.
    accumulated_thinking_text: String,
    /// Accumulated signature for the current thinking block.
    accumulated_signature: String,
    /// Whether the current thinking block is a redacted_thinking block.
    is_redacted_thinking: bool,
    /// The encrypted data from a redacted_thinking block.
    redacted_thinking_data: String,
    /// Whether we've emitted the Reasoning OutputItemAdded event.
    reasoning_item_started: bool,
    /// Stable id for the currently-streaming reasoning item (per thinking block).
    current_reasoning_id: Option<String>,
    /// Whether we've emitted the Message OutputItemAdded event for text.
    text_item_started: bool,

    // Token usage
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
}

impl AnthropicStreamState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a new tool call at the given content block index.
    pub fn register_tool(&mut self, index: u32, id: String, name: String) {
        self.tool_state.insert(index, (id, name));
        self.tool_arguments.insert(index, String::new());
    }

    /// Gets the tool info for a content block index.
    pub fn get_tool(&self, index: u32) -> Option<&(String, String)> {
        self.tool_state.get(&index)
    }

    /// Appends arguments to a tool call.
    pub fn append_tool_arguments(&mut self, index: u32, args: &str) {
        if let Some(accumulated) = self.tool_arguments.get_mut(&index) {
            accumulated.push_str(args);
        }
    }

    /// Gets the accumulated arguments for a tool call.
    pub fn get_tool_arguments(&self, index: u32) -> Option<&String> {
        self.tool_arguments.get(&index)
    }

    fn ensure_text_item_started(&mut self, events: &mut Vec<ResponseEvent>) {
        if self.text_item_started {
            return;
        }

        let id = self
            .assistant_message_id
            .get_or_insert_with(|| format!("{}:assistant", self.response_id))
            .clone();

        self.text_item_started = true;
        events.push(ResponseEvent::OutputItemAdded(ResponseItem::Message {
            id: Some(id),
            role: "assistant".to_string(),
            content: vec![],
            end_turn: None,
            phase: None,
        }));
    }

    fn ensure_reasoning_item_started(&mut self, events: &mut Vec<ResponseEvent>) {
        if self.reasoning_item_started {
            return;
        }

        let id = self
            .current_reasoning_id
            .get_or_insert_with(|| format!("{}:thinking:{}", self.response_id, self.thinking_index))
            .clone();

        self.reasoning_item_started = true;
        events.push(ResponseEvent::OutputItemAdded(ResponseItem::Reasoning {
            id,
            summary: vec![],
            content: None,
            encrypted_content: None,
        }));
    }
}

/// Message start payload.
#[derive(Debug, Deserialize)]
pub struct MessageStartPayload {
    pub message: AnthropicMessage,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub role: String,
    pub model: String,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<i64>,
}

/// Content block start payload.
#[derive(Debug, Deserialize)]
pub struct ContentBlockStartPayload {
    pub index: u32,
    pub content_block: ContentBlock,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
    /// Catch-all for unknown/new content block types (e.g. `server_tool_use`,
    /// `web_search_tool_result`). Gracefully ignored.
    #[serde(other)]
    Unknown,
}

/// Content block delta payload — parsed manually to handle unknown delta types gracefully.
#[derive(Debug)]
pub struct ContentBlockDeltaPayload {
    pub index: u32,
    pub delta: Option<ContentDelta>,
}

#[derive(Debug)]
pub enum ContentDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
    ThinkingDelta { thinking: String },
    SignatureDelta { signature: String },
}

impl ContentBlockDeltaPayload {
    /// Parse from a `serde_json::Value`, returning `None` for the delta
    /// if the delta type is unknown rather than failing.
    fn from_value(v: &Value) -> Result<Self, ApiError> {
        let index = v
            .get("index")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| ApiError::Stream("Missing index in content_block_delta".to_string()))?
            as u32;

        let delta = v.get("delta").and_then(|d| {
            let delta_type = d.get("type").and_then(|t| t.as_str())?;
            match delta_type {
                "text_delta" => {
                    let text = d.get("text").and_then(|t| t.as_str())?.to_string();
                    Some(ContentDelta::TextDelta { text })
                }
                "input_json_delta" => {
                    let partial_json = d.get("partial_json").and_then(|t| t.as_str())?.to_string();
                    Some(ContentDelta::InputJsonDelta { partial_json })
                }
                "thinking_delta" => {
                    let thinking = d.get("thinking").and_then(|t| t.as_str())?.to_string();
                    Some(ContentDelta::ThinkingDelta { thinking })
                }
                "signature_delta" => {
                    let signature = d.get("signature").and_then(|t| t.as_str())?.to_string();
                    Some(ContentDelta::SignatureDelta { signature })
                }
                other => {
                    tracing::debug!(delta_type = other, "Skipping unknown content delta type");
                    None
                }
            }
        });

        Ok(Self { index, delta })
    }
}

/// Message delta payload (for stop reason and final usage).
#[derive(Debug, Deserialize)]
pub struct MessageDeltaPayload {
    pub delta: MessageDelta,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct MessageDelta {
    pub stop_reason: Option<String>,
    pub stop_sequence: Option<String>,
}

/// Error payload.
#[derive(Debug, Deserialize)]
pub struct ErrorPayload {
    pub error: AnthropicError,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Parses an Anthropic SSE event into ResponseEvents.
///
/// # Arguments
/// * `event_type` - The SSE event type (e.g., "message_start")
/// * `data` - The JSON data payload
/// * `state` - Mutable state tracker for tool calls
///
/// # Returns
/// A vector of ResponseEvents extracted from the event.
pub fn parse_anthropic_event(
    event_type: &str,
    data: &str,
    state: &mut AnthropicStreamState,
) -> Result<Vec<ResponseEvent>, ApiError> {
    let parsed_event_type = match AnthropicEventType::from_str(event_type) {
        Ok(et) => et,
        Err(()) => {
            tracing::debug!(event_type, "Skipping unknown Anthropic SSE event type");
            return Ok(vec![]);
        }
    };

    let mut events = Vec::new();

    match parsed_event_type {
        AnthropicEventType::MessageStart => {
            let payload: MessageStartPayload = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse message_start: {e}")))?;

            state.response_id = payload.message.id;
            if !state.text_item_started {
                state.assistant_message_id = Some(format!("{}:assistant", state.response_id));
            }

            // Capture initial usage from message_start
            if let Some(usage) = &payload.message.usage {
                state.input_tokens = usage.input_tokens;
                state.output_tokens = usage.output_tokens;
                state.cache_creation_input_tokens = usage.cache_creation_input_tokens;
                state.cache_read_input_tokens = usage.cache_read_input_tokens;
            }

            if !state.created_sent {
                events.push(ResponseEvent::Created);
                state.created_sent = true;
            }
        }

        AnthropicEventType::ContentBlockStart => {
            let payload: ContentBlockStartPayload = serde_json::from_str(data).map_err(|e| {
                ApiError::Stream(format!("Failed to parse content_block_start: {e}"))
            })?;

            match payload.content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    state.register_tool(payload.index, id, name);
                }
                ContentBlock::Text { text } => {
                    // Intentionally do not emit `OutputItemAdded` here. Some Anthropic streams
                    // can start a text block before any text deltas arrive (for example when
                    // thinking blocks are streamed first). We only start the message item once
                    // we actually see text content to ensure `active_item` aligns with deltas.
                    if !text.is_empty() {
                        state.ensure_text_item_started(&mut events);
                        state.text_content.push_str(&text);
                        events.push(ResponseEvent::OutputTextDelta(text));
                    }
                }
                ContentBlock::Thinking { thinking } => {
                    state.thinking_blocks.insert(payload.index);
                    state.in_thinking_section = true;
                    state.is_redacted_thinking = false;
                    state.accumulated_thinking_text.clear();
                    state.accumulated_signature.clear();
                    state.current_reasoning_id = None;

                    state.ensure_reasoning_item_started(&mut events);
                    if !thinking.is_empty() {
                        state.accumulated_thinking_text.push_str(&thinking);
                        events.push(ResponseEvent::ReasoningSummaryDelta {
                            delta: thinking,
                            summary_index: state.thinking_index,
                        });
                    }
                }
                ContentBlock::RedactedThinking { data } => {
                    state.thinking_blocks.insert(payload.index);
                    state.in_thinking_section = true;
                    state.is_redacted_thinking = true;
                    state.redacted_thinking_data = data;
                    state.accumulated_thinking_text.clear();
                    state.accumulated_signature.clear();
                    state.current_reasoning_id = None;

                    state.ensure_reasoning_item_started(&mut events);
                }
                ContentBlock::Unknown => {
                    tracing::debug!(index = payload.index, "Skipping unknown content block type");
                }
            }
        }

        AnthropicEventType::ContentBlockDelta => {
            let parsed: Value = serde_json::from_str(data).map_err(|e| {
                ApiError::Stream(format!("Failed to parse content_block_delta: {e}"))
            })?;
            let payload = ContentBlockDeltaPayload::from_value(&parsed)?;

            if let Some(delta) = payload.delta {
                match delta {
                    ContentDelta::TextDelta { text } => {
                        if !text.is_empty() {
                            state.ensure_text_item_started(&mut events);
                            state.text_content.push_str(&text);
                            events.push(ResponseEvent::OutputTextDelta(text));
                        }
                    }
                    ContentDelta::InputJsonDelta { partial_json } => {
                        state.append_tool_arguments(payload.index, &partial_json);
                    }
                    ContentDelta::ThinkingDelta { thinking } => {
                        if !thinking.is_empty() {
                            state.ensure_reasoning_item_started(&mut events);
                            state.accumulated_thinking_text.push_str(&thinking);
                            events.push(ResponseEvent::ReasoningSummaryDelta {
                                delta: thinking,
                                summary_index: state.thinking_index,
                            });
                        }
                    }
                    ContentDelta::SignatureDelta { signature } => {
                        state.accumulated_signature.push_str(&signature);
                    }
                }
            }
        }

        AnthropicEventType::ContentBlockStop => {
            // Parse to get the index
            #[derive(Deserialize)]
            struct ContentBlockStopPayload {
                index: u32,
            }

            let payload: ContentBlockStopPayload = serde_json::from_str(data).map_err(|e| {
                ApiError::Stream(format!("Failed to parse content_block_stop: {e}"))
            })?;

            if state.thinking_blocks.contains(&payload.index) {
                // Finalize the thinking block
                let summary_index = state.thinking_index;
                let reasoning_id = state.current_reasoning_id.take().unwrap_or_else(|| {
                    format!("{}:thinking:{}", state.response_id, state.thinking_index)
                });
                events.push(ResponseEvent::ReasoningSummaryPartAdded { summary_index });

                if state.is_redacted_thinking {
                    // Redacted thinking: store data in encrypted_content, empty summary
                    let data = std::mem::take(&mut state.redacted_thinking_data);
                    events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                        id: reasoning_id,
                        summary: vec![],
                        content: None,
                        encrypted_content: Some(data),
                    }));
                } else {
                    // Normal thinking: text in summary, signature in encrypted_content
                    let text = std::mem::take(&mut state.accumulated_thinking_text);
                    let signature = std::mem::take(&mut state.accumulated_signature);
                    events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                        id: reasoning_id,
                        summary: vec![ReasoningItemReasoningSummary::SummaryText { text }],
                        content: None,
                        encrypted_content: if signature.is_empty() {
                            None
                        } else {
                            Some(signature)
                        },
                    }));
                }

                state.thinking_index += 1;
                state.in_thinking_section = false;
                state.is_redacted_thinking = false;
                state.reasoning_item_started = false;
                state.current_reasoning_id = None;
            } else if let Some((id, name)) = state.get_tool(payload.index).cloned() {
                // If this was a tool call block, emit the completed tool call
                let arguments = state
                    .get_tool_arguments(payload.index)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string());

                events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                    id: None,
                    call_id: id,
                    name,
                    arguments,
                    thought_signature: None,
                }));
            }
        }

        AnthropicEventType::MessageDelta => {
            let payload: MessageDeltaPayload = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse message_delta: {e}")))?;

            // Capture final output_tokens from message_delta usage
            if let Some(usage) = &payload.usage
                && let Some(output_tokens) = usage.output_tokens
            {
                state.output_tokens = Some(output_tokens);
            }
        }

        AnthropicEventType::MessageStop => {
            // Flush any in-progress thinking section
            if state.in_thinking_section
                && (!state.accumulated_thinking_text.is_empty() || state.is_redacted_thinking)
            {
                let summary_index = state.thinking_index;
                let reasoning_id = state.current_reasoning_id.take().unwrap_or_else(|| {
                    format!("{}:thinking:{}", state.response_id, state.thinking_index)
                });
                events.push(ResponseEvent::ReasoningSummaryPartAdded { summary_index });

                if state.is_redacted_thinking {
                    let data = std::mem::take(&mut state.redacted_thinking_data);
                    events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                        id: reasoning_id,
                        summary: vec![],
                        content: None,
                        encrypted_content: Some(data),
                    }));
                } else {
                    let text = std::mem::take(&mut state.accumulated_thinking_text);
                    let signature = std::mem::take(&mut state.accumulated_signature);
                    events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                        id: reasoning_id,
                        summary: vec![ReasoningItemReasoningSummary::SummaryText { text }],
                        content: None,
                        encrypted_content: if signature.is_empty() {
                            None
                        } else {
                            Some(signature)
                        },
                    }));
                }

                state.thinking_index += 1;
                state.in_thinking_section = false;
                state.is_redacted_thinking = false;
                state.reasoning_item_started = false;
                state.current_reasoning_id = None;
            }

            // Build the final message if we have text content
            if !state.text_content.is_empty() {
                let id = state
                    .assistant_message_id
                    .get_or_insert_with(|| format!("{}:assistant", state.response_id))
                    .clone();
                events.push(ResponseEvent::OutputItemDone(ResponseItem::Message {
                    id: Some(id),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: std::mem::take(&mut state.text_content),
                    }],
                    end_turn: Some(true),
                    phase: None,
                }));
            }

            // Build token usage from accumulated state
            let input_tokens = state.input_tokens.unwrap_or(0);
            let output_tokens = state.output_tokens.unwrap_or(0);
            let cached_input_tokens = state
                .cache_read_input_tokens
                .unwrap_or(0)
                .saturating_add(state.cache_creation_input_tokens.unwrap_or(0));
            let token_usage = TokenUsage {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                reasoning_output_tokens: 0,
                total_tokens: input_tokens.saturating_add(output_tokens),
            };

            events.push(ResponseEvent::Completed {
                response_id: state.response_id.clone(),
                token_usage: Some(token_usage),
                can_append: false,
            });
        }

        AnthropicEventType::Ping => {
            // Ignore ping events
        }

        AnthropicEventType::Error => {
            let payload: ErrorPayload = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse error: {e}")))?;

            if payload.error.message.contains("prompt is too long") {
                return Err(ApiError::ContextWindowExceeded);
            }

            let message = map_user_facing_file_error_from_message(&payload.error.message)
                .map(|mapped| mapped.user_message)
                .unwrap_or(payload.error.message);

            if payload.error.error_type == "invalid_request_error"
                || payload.error.error_type == "not_found_error"
            {
                return Err(ApiError::InvalidRequest { message });
            }

            return Err(ApiError::Stream(format!(
                "Anthropic API error ({}): {message}",
                payload.error.error_type
            )));
        }
    }

    Ok(events)
}

/// Checks if an event type indicates stream completion.
pub fn is_completion_event(event_type: &str) -> bool {
    event_type == "message_stop"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_start() {
        let data = r#"{
            "type": "message_start",
            "message": {
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-20250514",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 0
                }
            }
        }"#;

        let mut state = AnthropicStreamState::new();
        let events = parse_anthropic_event("message_start", data, &mut state).unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ResponseEvent::Created));
        assert_eq!(state.response_id, "msg_123");
    }

    #[test]
    fn test_parse_text_delta() {
        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": "Hello, "
            }
        }"#;

        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_123".to_string();
        state.assistant_message_id = Some(format!("{}:assistant", state.response_id));
        let events = parse_anthropic_event("content_block_delta", data, &mut state).unwrap();

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(
            &events[1],
            ResponseEvent::OutputTextDelta(s) if s == "Hello, "
        ));
        assert_eq!(state.text_content, "Hello, ");
    }

    #[test]
    fn test_parse_tool_use() {
        // First, content_block_start for tool use
        let start_data = r#"{
            "type": "content_block_start",
            "index": 1,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_123",
                "name": "get_weather",
                "input": {}
            }
        }"#;

        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        let _ = parse_anthropic_event("content_block_start", start_data, &mut state).unwrap();

        assert!(state.get_tool(1).is_some());
        assert_eq!(state.get_tool(1).unwrap().0, "toolu_123");
        assert_eq!(state.get_tool(1).unwrap().1, "get_weather");

        // Then, input_json_delta
        let delta_data = r#"{
            "type": "content_block_delta",
            "index": 1,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"location\": \"SF\"}"
            }
        }"#;

        let _ = parse_anthropic_event("content_block_delta", delta_data, &mut state).unwrap();
        assert_eq!(
            state.get_tool_arguments(1).unwrap(),
            "{\"location\": \"SF\"}"
        );

        // Finally, content_block_stop
        let stop_data = r#"{"type": "content_block_stop", "index": 1}"#;
        let events = parse_anthropic_event("content_block_stop", stop_data, &mut state).unwrap();

        assert_eq!(events.len(), 1);
        match &events[0] {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            }) => {
                assert_eq!(call_id, "toolu_123");
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, "{\"location\": \"SF\"}");
            }
            _ => panic!("Expected FunctionCall"),
        }
    }

    #[test]
    fn test_parse_message_stop() {
        let data = r#"{"type": "message_stop"}"#;

        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_123".to_string();
        state.text_content = "Hello, world!".to_string();

        let events = parse_anthropic_event("message_stop", data, &mut state).unwrap();

        assert_eq!(events.len(), 2);
        // First should be the message
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemDone(ResponseItem::Message { .. })
        ));
        // Second should be completed with token usage
        match &events[1] {
            ResponseEvent::Completed {
                response_id,
                token_usage,
                ..
            } => {
                assert_eq!(response_id, "msg_123");
                assert!(token_usage.is_some());
            }
            _ => panic!("Expected Completed event"),
        }
    }

    #[test]
    fn test_parse_error() {
        let data = r#"{
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Invalid API key"
            }
        }"#;

        let mut state = AnthropicStreamState::new();
        let result = parse_anthropic_event("error", data, &mut state);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Invalid API key"));
    }

    #[test]
    fn test_parse_error_prompt_too_long_maps_to_context_window_exceeded() {
        let data = r#"{
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "prompt is too long: 211584 tokens > 200000 maximum"
            }
        }"#;

        let mut state = AnthropicStreamState::new();
        let result = parse_anthropic_event("error", data, &mut state);

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ApiError::ContextWindowExceeded),
            "expected ApiError::ContextWindowExceeded"
        );
    }

    #[test]
    fn test_is_completion_event() {
        assert!(is_completion_event("message_stop"));
        assert!(!is_completion_event("message_start"));
        assert!(!is_completion_event("content_block_delta"));
    }

    #[test]
    fn test_parse_thinking_block() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;

        // Start thinking block
        let start_data = r#"{
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "thinking",
                "thinking": ""
            }
        }"#;
        let events = parse_anthropic_event("content_block_start", start_data, &mut state).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));
        assert!(state.thinking_blocks.contains(&0));

        // Thinking delta
        let delta_data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "thinking_delta",
                "thinking": "Let me think..."
            }
        }"#;
        let events = parse_anthropic_event("content_block_delta", delta_data, &mut state).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryDelta { delta, summary_index: 0 } if delta == "Let me think..."
        ));
        assert_eq!(state.accumulated_thinking_text, "Let me think...");

        // Stop thinking block
        let stop_data = r#"{"type": "content_block_stop", "index": 0}"#;
        let events = parse_anthropic_event("content_block_stop", stop_data, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryPartAdded { summary_index: 0 }
        ));
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning { summary, .. }) => {
                assert_eq!(summary.len(), 1);
                match &summary[0] {
                    ReasoningItemReasoningSummary::SummaryText { text } => {
                        assert_eq!(text, "Let me think...");
                    }
                }
            }
            _ => panic!("Expected Reasoning OutputItemDone"),
        }
        assert_eq!(state.thinking_index, 1);
    }

    #[test]
    fn test_parse_thinking_then_text() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_thinking_then_text".to_string();
        state.assistant_message_id = Some(format!("{}:assistant", state.response_id));

        // Start thinking block
        let _ = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 0, "content_block": {"type": "thinking", "thinking": ""}}"#,
            &mut state,
        )
        .unwrap();

        // Thinking delta
        let _ = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "thinking_delta", "thinking": "reasoning"}}"#,
            &mut state,
        )
        .unwrap();

        // Stop thinking block
        let _ = parse_anthropic_event("content_block_stop", r#"{"index": 0}"#, &mut state).unwrap();

        // Start text block
        let _ = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 1, "content_block": {"type": "text", "text": ""}}"#,
            &mut state,
        )
        .unwrap();

        // Text delta
        let events = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 1, "delta": {"type": "text_delta", "text": "Hello!"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(&events[1], ResponseEvent::OutputTextDelta(s) if s == "Hello!"));

        // Stop text block + message_stop
        let _ = parse_anthropic_event("content_block_stop", r#"{"index": 1}"#, &mut state).unwrap();
        let events = parse_anthropic_event("message_stop", r#"{}"#, &mut state).unwrap();

        // Should have Message + Completed
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemDone(ResponseItem::Message { .. })
        ));
        assert!(matches!(&events[1], ResponseEvent::Completed { .. }));
    }

    #[test]
    fn test_text_delta_after_thinking_without_new_text_start_still_starts_message_item() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_ordering".to_string();
        state.assistant_message_id = Some(format!("{}:assistant", state.response_id));

        // Some streams can start a text block early (empty), then stream thinking, then later
        // emit text deltas without another text block start. We must still emit Message
        // OutputItemAdded before OutputTextDelta so downstream routing has an active item.
        let events = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 0, "content_block": {"type": "text", "text": ""}}"#,
            &mut state,
        )
        .unwrap();
        assert!(events.is_empty());

        let _ = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 1, "content_block": {"type": "thinking", "thinking": ""}}"#,
            &mut state,
        )
        .unwrap();
        let _ = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 1, "delta": {"type": "thinking_delta", "thinking": "reasoning"}}"#,
            &mut state,
        )
        .unwrap();
        let _ = parse_anthropic_event("content_block_stop", r#"{"index": 1}"#, &mut state).unwrap();

        let events = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "text_delta", "text": "Hello!"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(&events[1], ResponseEvent::OutputTextDelta(s) if s == "Hello!"));
    }

    #[test]
    fn test_signature_delta_captured() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.thinking_blocks.insert(0);
        state.in_thinking_section = true;

        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "signature_delta",
                "signature": "abc123sig"
            }
        }"#;
        let events = parse_anthropic_event("content_block_delta", data, &mut state).unwrap();
        assert!(events.is_empty());
        assert_eq!(state.accumulated_signature, "abc123sig");
    }

    #[test]
    fn test_token_usage_populated() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_456".to_string();

        // message_start with usage
        let start_data = r#"{
            "message": {
                "id": "msg_456",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-20250514",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 0,
                    "cache_read_input_tokens": 50,
                    "cache_creation_input_tokens": 10
                }
            }
        }"#;
        let _ = parse_anthropic_event("message_start", start_data, &mut state).unwrap();
        assert_eq!(state.input_tokens, Some(100));
        assert_eq!(state.cache_read_input_tokens, Some(50));

        // message_delta with output_tokens
        let delta_data = r#"{
            "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 42}
        }"#;
        let _ = parse_anthropic_event("message_delta", delta_data, &mut state).unwrap();
        assert_eq!(state.output_tokens, Some(42));

        // message_stop should include token usage
        let events = parse_anthropic_event("message_stop", r#"{}"#, &mut state).unwrap();
        match &events[events.len() - 1] {
            ResponseEvent::Completed { token_usage, .. } => {
                let usage = token_usage.as_ref().expect("expected token usage");
                assert_eq!(usage.input_tokens, 100);
                assert_eq!(usage.output_tokens, 42);
                assert_eq!(usage.cached_input_tokens, 60); // 50 + 10
                assert_eq!(usage.total_tokens, 142);
            }
            _ => panic!("Expected Completed event"),
        }
    }

    #[test]
    fn test_message_stop_flushes_thinking() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_789".to_string();
        state.in_thinking_section = true;
        state.accumulated_thinking_text = "partial thought".to_string();
        state.reasoning_item_started = true;

        let events = parse_anthropic_event("message_stop", r#"{}"#, &mut state).unwrap();

        // Should flush: ReasoningSummaryPartAdded, Reasoning OutputItemDone, Completed
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryPartAdded { summary_index: 0 }
        ));
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning { summary, .. }) => {
                assert_eq!(summary.len(), 1);
                match &summary[0] {
                    ReasoningItemReasoningSummary::SummaryText { text } => {
                        assert_eq!(text, "partial thought");
                    }
                }
            }
            _ => panic!("Expected Reasoning OutputItemDone"),
        }
        match &events[2] {
            ResponseEvent::Completed { token_usage, .. } => {
                assert!(token_usage.is_some());
            }
            _ => panic!("Expected Completed"),
        }
    }

    #[test]
    fn test_redacted_thinking_block() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;

        let start_data = r#"{
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "redacted_thinking",
                "data": "encrypted_data_here"
            }
        }"#;
        let events = parse_anthropic_event("content_block_start", start_data, &mut state).unwrap();

        // Should emit OutputItemAdded for Reasoning
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));
        assert!(state.thinking_blocks.contains(&0));
        assert!(state.is_redacted_thinking);
        assert_eq!(state.redacted_thinking_data, "encrypted_data_here");

        // Stop the block — should produce a Reasoning with encrypted_content
        let stop_data = r#"{"type": "content_block_stop", "index": 0}"#;
        let events = parse_anthropic_event("content_block_stop", stop_data, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                summary,
                encrypted_content,
                ..
            }) => {
                assert!(summary.is_empty());
                assert_eq!(encrypted_content.as_deref(), Some("encrypted_data_here"));
            }
            _ => panic!("Expected Reasoning OutputItemDone with encrypted_content"),
        }
    }

    #[test]
    fn test_signature_captured_in_thinking_block() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;

        // Start thinking block
        let _ = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 0, "content_block": {"type": "thinking", "thinking": ""}}"#,
            &mut state,
        )
        .unwrap();

        // Thinking delta
        let _ = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "thinking_delta", "thinking": "deep thought"}}"#,
            &mut state,
        )
        .unwrap();

        // Signature delta
        let _ = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "signature_delta", "signature": "sig_part1"}}"#,
            &mut state,
        )
        .unwrap();
        let _ = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "signature_delta", "signature": "sig_part2"}}"#,
            &mut state,
        )
        .unwrap();

        // Stop thinking block
        let events =
            parse_anthropic_event("content_block_stop", r#"{"index": 0}"#, &mut state).unwrap();

        assert_eq!(events.len(), 2);
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                summary,
                encrypted_content,
                ..
            }) => {
                assert_eq!(summary.len(), 1);
                match &summary[0] {
                    ReasoningItemReasoningSummary::SummaryText { text } => {
                        assert_eq!(text, "deep thought");
                    }
                }
                assert_eq!(encrypted_content.as_deref(), Some("sig_part1sig_part2"));
            }
            _ => panic!("Expected Reasoning OutputItemDone with signature"),
        }
    }

    // ── Unknown type handling tests ──────────────────

    #[test]
    fn test_unknown_event_type_returns_empty() {
        let mut state = AnthropicStreamState::new();
        let events = parse_anthropic_event("some_new_event", r#"{"foo": "bar"}"#, &mut state)
            .expect("unknown event types should not error");
        assert!(events.is_empty());
    }

    #[test]
    fn test_multiple_thinking_blocks() {
        // Simulates: thinking block (idx 0) → redacted_thinking block (idx 1) → text block (idx 2)
        // This is the crash scenario: without resetting reasoning_item_started,
        // the second thinking block won't emit OutputItemAdded(Reasoning),
        // leaving active_item = None when deltas arrive.
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;
        state.response_id = "msg_multi".to_string();
        state.assistant_message_id = Some(format!("{}:assistant", state.response_id));

        // ── First thinking block (idx 0) ──
        let events = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 0, "content_block": {"type": "thinking", "thinking": ""}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));

        // Thinking delta for block 0
        let events = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 0, "delta": {"type": "thinking_delta", "thinking": "First thought"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryDelta {
                summary_index: 0,
                ..
            }
        ));

        // Stop first thinking block
        let events =
            parse_anthropic_event("content_block_stop", r#"{"index": 0}"#, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryPartAdded { summary_index: 0 }
        ));
        assert!(matches!(
            &events[1],
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning { .. })
        ));
        // reasoning_item_started should be reset
        assert!(!state.reasoning_item_started);

        // ── Second thinking block: redacted_thinking (idx 1) ──
        let events = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 1, "content_block": {"type": "redacted_thinking", "data": "encrypted"}}"#,
            &mut state,
        )
        .unwrap();
        // MUST emit a new OutputItemAdded(Reasoning)
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Reasoning { .. })
        ));

        // Stop second thinking block
        let events =
            parse_anthropic_event("content_block_stop", r#"{"index": 1}"#, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::ReasoningSummaryPartAdded { summary_index: 1 }
        ));
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
                encrypted_content, ..
            }) => {
                assert_eq!(encrypted_content.as_deref(), Some("encrypted"));
            }
            _ => panic!("Expected Reasoning OutputItemDone with encrypted_content"),
        }
        assert!(!state.reasoning_item_started);

        // ── Text block (idx 2) ──
        let events = parse_anthropic_event(
            "content_block_start",
            r#"{"index": 2, "content_block": {"type": "text", "text": ""}}"#,
            &mut state,
        )
        .unwrap();
        assert!(events.is_empty());

        // Text delta
        let events = parse_anthropic_event(
            "content_block_delta",
            r#"{"index": 2, "delta": {"type": "text_delta", "text": "Hello!"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(&events[1], ResponseEvent::OutputTextDelta(s) if s == "Hello!"));

        // Stop text block + message_stop
        let _ = parse_anthropic_event("content_block_stop", r#"{"index": 2}"#, &mut state).unwrap();
        let events = parse_anthropic_event("message_stop", r#"{}"#, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            ResponseEvent::OutputItemDone(ResponseItem::Message { .. })
        ));
        assert!(matches!(&events[1], ResponseEvent::Completed { .. }));
    }

    #[test]
    fn test_unknown_content_block_type_returns_empty() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;

        let data = r#"{
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "server_tool_use",
                "id": "srvtoolu_abc",
                "name": "web_search"
            }
        }"#;
        let events = parse_anthropic_event("content_block_start", data, &mut state)
            .expect("unknown content block types should not error");
        assert!(events.is_empty());
    }

    #[test]
    fn test_unknown_delta_type_returns_empty() {
        let mut state = AnthropicStreamState::new();
        state.created_sent = true;

        let data = r#"{
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "web_search_result_delta",
                "result": "some data"
            }
        }"#;
        let events = parse_anthropic_event("content_block_delta", data, &mut state)
            .expect("unknown delta types should not error");
        assert!(events.is_empty());
    }
}
