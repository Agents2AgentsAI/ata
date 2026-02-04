//! Anthropic SSE event parser.
//!
//! Anthropic's Messages API uses the following SSE event types:
//! - `message_start`: Initial message metadata
//! - `content_block_start`: Start of a content block (text or tool_use)
//! - `content_block_delta`: Incremental content updates
//! - `content_block_stop`: End of a content block
//! - `message_delta`: Final message metadata (stop_reason, usage)
//! - `message_stop`: End of message

use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::common::ResponseEvent;
use crate::error::ApiError;

/// Parses an Anthropic SSE event into a ResponseEvent.
pub fn parse_anthropic_event(
    event_type: &str,
    data: &str,
) -> Result<Option<ResponseEvent>, ApiError> {
    match event_type {
        "message_start" => {
            // Initial message - emit Created event
            Ok(Some(ResponseEvent::Created))
        }

        "content_block_start" => {
            let parsed: ContentBlockStart = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse content_block_start: {e}")))?;

            match parsed.content_block {
                ContentBlock::Text { .. } => {
                    // Text block starting - no event needed
                    Ok(None)
                }
                ContentBlock::ToolUse { id, name, .. } => {
                    // Tool use starting - we'll emit the full item when it's done
                    debug!("Tool use started: {} ({})", name, id);
                    Ok(None)
                }
            }
        }

        "content_block_delta" => {
            let parsed: ContentBlockDelta = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse content_block_delta: {e}")))?;

            match parsed.delta {
                ContentDelta::TextDelta { text } => Ok(Some(ResponseEvent::OutputTextDelta(text))),
                ContentDelta::InputJsonDelta { .. } => {
                    // Tool input is streamed as JSON - we accumulate this server-side
                    // and emit the full tool call on content_block_stop
                    Ok(None)
                }
            }
        }

        "content_block_stop" => {
            // Content block finished - might need to emit OutputItemDone for tool calls
            // For now, we handle this in the streaming logic
            Ok(None)
        }

        "message_delta" => {
            // Contains stop_reason and final usage
            let parsed: MessageDelta = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse message_delta: {e}")))?;

            // We don't emit an event here - usage will be captured at message_stop
            debug!(
                "Message delta: stop_reason={:?}",
                parsed.delta.stop_reason
            );
            Ok(None)
        }

        "message_stop" => {
            // Message complete
            Ok(Some(ResponseEvent::Completed {
                response_id: String::new(),
                token_usage: None,
            }))
        }

        "error" => {
            let error: AnthropicError = serde_json::from_str(data)
                .map_err(|e| ApiError::Stream(format!("Failed to parse error: {e}")))?;

            let error_type = error.error.error_type.as_deref().unwrap_or("unknown");
            let message = error
                .error
                .message
                .unwrap_or_else(|| "Unknown error".to_string());

            match error_type {
                "rate_limit_error" => Err(ApiError::Retryable {
                    message,
                    delay: None,
                }),
                "overloaded_error" => Err(ApiError::Retryable {
                    message,
                    delay: None,
                }),
                "invalid_request_error" => {
                    if message.contains("context") || message.contains("tokens") {
                        Err(ApiError::ContextWindowExceeded)
                    } else {
                        Err(ApiError::InvalidRequest { message })
                    }
                }
                _ => Err(ApiError::Stream(format!("{}: {}", error_type, message))),
            }
        }

        "ping" => {
            // Keepalive - ignore
            Ok(None)
        }

        _ => {
            debug!("Unhandled Anthropic event type: {}", event_type);
            Ok(None)
        }
    }
}

/// Converts an Anthropic content block into a ResponseItem.
pub fn convert_anthropic_content_to_response_item(
    content_blocks: &[Value],
    message_id: &str,
) -> Option<ResponseItem> {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in content_blocks {
        let block_type = block.get("type")?.as_str()?;
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(ContentItem::OutputText {
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let id = block.get("id")?.as_str()?;
                let name = block.get("name")?.as_str()?;
                let input = block.get("input")?;

                tool_calls.push((id.to_string(), name.to_string(), input.clone()));
            }
            _ => {}
        }
    }

    // If we have text content, return a message
    if !text_parts.is_empty() {
        return Some(ResponseItem::Message {
            id: Some(message_id.to_string()),
            role: "assistant".to_string(),
            content: text_parts,
            end_turn: None,
            phase: None,
        });
    }

    // If we have a tool call, return the first one
    // (Multiple tool calls would need separate ResponseItems)
    if let Some((call_id, name, input)) = tool_calls.into_iter().next() {
        let arguments = serde_json::to_string(&input).unwrap_or_default();
        return Some(ResponseItem::FunctionCall {
            id: None,
            call_id,
            name,
            arguments,
        });
    }

    None
}

// Anthropic SSE event types
// These structs contain fields required for JSON deserialization that may not be directly used.

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ContentBlockStart {
    index: i64,
    content_block: ContentBlock,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String, input: Value },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ContentBlockDelta {
    index: i64,
    delta: ContentDelta,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MessageDelta {
    delta: MessageDeltaContent,
    usage: Option<MessageUsage>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MessageDeltaContent {
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct MessageUsage {
    output_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    error_type: Option<String>,
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_message_start() {
        let data = r#"{"type":"message_start","message":{"id":"msg_123","role":"assistant"}}"#;
        let result = parse_anthropic_event("message_start", data);
        assert!(matches!(result, Ok(Some(ResponseEvent::Created))));
    }

    #[test]
    fn test_parse_text_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let result = parse_anthropic_event("content_block_delta", data);
        assert!(matches!(
            result,
            Ok(Some(ResponseEvent::OutputTextDelta(text))) if text == "Hello"
        ));
    }

    #[test]
    fn test_parse_message_stop() {
        let data = r#"{"type":"message_stop"}"#;
        let result = parse_anthropic_event("message_stop", data);
        assert!(matches!(result, Ok(Some(ResponseEvent::Completed { .. }))));
    }

    #[test]
    fn test_parse_error() {
        let data = r#"{"type":"error","error":{"type":"rate_limit_error","message":"Rate limit exceeded"}}"#;
        let result = parse_anthropic_event("error", data);
        assert!(matches!(result, Err(ApiError::Retryable { .. })));
    }
}
