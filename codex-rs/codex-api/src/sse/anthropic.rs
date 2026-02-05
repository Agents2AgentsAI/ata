//! Anthropic SSE streaming response parser.
//!
//! Anthropic uses standard SSE with specific event types for content blocks.
//! This module handles the interleaved streaming of text and tool calls.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::common::ResponseEvent;
use crate::error::ApiError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

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

impl AnthropicEventType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "message_start" => Some(Self::MessageStart),
            "content_block_start" => Some(Self::ContentBlockStart),
            "content_block_delta" => Some(Self::ContentBlockDelta),
            "content_block_stop" => Some(Self::ContentBlockStop),
            "message_delta" => Some(Self::MessageDelta),
            "message_stop" => Some(Self::MessageStop),
            "ping" => Some(Self::Ping),
            "error" => Some(Self::Error),
            _ => None,
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
}

/// Content block delta payload.
#[derive(Debug, Deserialize)]
pub struct ContentBlockDeltaPayload {
    pub index: u32,
    pub delta: ContentDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
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
    let event_type = AnthropicEventType::from_str(event_type)
        .ok_or_else(|| ApiError::Stream(format!("Unknown Anthropic event type: {event_type}")))?;

    let mut events = Vec::new();

    match event_type {
        AnthropicEventType::MessageStart => {
            let payload: MessageStartPayload = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse message_start: {e}")))?;

            state.response_id = payload.message.id;

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
                ContentBlock::Text { .. } => {
                    // Text blocks are handled via deltas
                }
            }
        }

        AnthropicEventType::ContentBlockDelta => {
            let payload: ContentBlockDeltaPayload = serde_json::from_str(data).map_err(|e| {
                ApiError::Stream(format!("Failed to parse content_block_delta: {e}"))
            })?;

            match payload.delta {
                ContentDelta::TextDelta { text } => {
                    if !text.is_empty() {
                        state.text_content.push_str(&text);
                        events.push(ResponseEvent::OutputTextDelta(text));
                    }
                }
                ContentDelta::InputJsonDelta { partial_json } => {
                    state.append_tool_arguments(payload.index, &partial_json);
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

            // If this was a tool call block, emit the completed tool call
            if let Some((id, name)) = state.get_tool(payload.index).cloned() {
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

            // We'll use this info in MessageStop
            let _ = payload;
        }

        AnthropicEventType::MessageStop => {
            // Build the final message if we have text content
            if !state.text_content.is_empty() {
                events.push(ResponseEvent::OutputItemDone(ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: std::mem::take(&mut state.text_content),
                    }],
                    end_turn: Some(true),
                    phase: None,
                }));
            }

            events.push(ResponseEvent::Completed {
                response_id: state.response_id.clone(),
                token_usage: None, // Usage comes from message_delta
            });
        }

        AnthropicEventType::Ping => {
            // Ignore ping events
        }

        AnthropicEventType::Error => {
            let payload: ErrorPayload = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse error: {e}")))?;

            return Err(ApiError::Stream(format!(
                "Anthropic API error ({}): {}",
                payload.error.error_type, payload.error.message
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
        let events = parse_anthropic_event("content_block_delta", data, &mut state).unwrap();

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ResponseEvent::OutputTextDelta(ref s) if s == "Hello, "));
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
        // Second should be completed
        assert!(matches!(
            &events[1],
            ResponseEvent::Completed { response_id, .. } if response_id == "msg_123"
        ));
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
    fn test_is_completion_event() {
        assert!(is_completion_event("message_stop"));
        assert!(!is_completion_event("message_start"));
        assert!(!is_completion_event("content_block_delta"));
    }
}
