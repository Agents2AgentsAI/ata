//! Gemini streaming response parser.
//!
//! Gemini's GenerateContent API streams responses as JSON lines, not standard SSE.
//! Each line is a complete JSON object with candidates and usage metadata.

use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::common::ResponseEvent;
use crate::error::ApiError;

/// Parses a Gemini streaming response line into a ResponseEvent.
pub fn parse_gemini_event(data: &str) -> Result<Option<ResponseEvent>, ApiError> {
    // Gemini streams JSON lines, not standard SSE events
    let response: GeminiStreamResponse = serde_json::from_str(data)
        .map_err(|e| ApiError::Stream(format!("Failed to parse Gemini response: {e}")))?;

    // Check for errors first
    if let Some(error) = response.error {
        return Err(map_gemini_error(&error));
    }

    // Process candidates
    if let Some(candidates) = response.candidates {
        if let Some(candidate) = candidates.first() {
            // Check for finish reason (completion)
            if let Some(finish_reason) = &candidate.finish_reason {
                let token_usage = response.usage_metadata.map(|u| TokenUsage {
                    input_tokens: u.prompt_token_count.unwrap_or(0),
                    output_tokens: u.candidates_token_count.unwrap_or(0),
                    total_tokens: u.total_token_count.unwrap_or(0),
                    cached_input_tokens: u.cached_content_token_count.unwrap_or(0),
                    reasoning_output_tokens: 0,
                });

                match finish_reason.as_str() {
                    "STOP" | "END_TURN" => {
                        return Ok(Some(ResponseEvent::Completed {
                            response_id: String::new(),
                            token_usage,
                        }));
                    }
                    "MAX_TOKENS" => {
                        return Err(ApiError::ContextWindowExceeded);
                    }
                    "SAFETY" => {
                        return Err(ApiError::InvalidRequest {
                            message: "Response blocked due to safety settings".to_string(),
                        });
                    }
                    "RECITATION" => {
                        return Err(ApiError::InvalidRequest {
                            message: "Response blocked due to recitation".to_string(),
                        });
                    }
                    _ => {
                        debug!("Unknown finish reason: {}", finish_reason);
                    }
                }
            }

            // Extract content parts
            if let Some(content) = &candidate.content {
                for part in &content.parts {
                    match part {
                        GeminiPart::Text { text } => {
                            return Ok(Some(ResponseEvent::OutputTextDelta(text.clone())));
                        }
                        GeminiPart::FunctionCall { function_call } => {
                            // Function call - we need to emit this as OutputItemDone
                            debug!(
                                "Gemini function call: {} with args {:?}",
                                function_call.name, function_call.args
                            );
                            // For now, just acknowledge - full handling requires stateful parsing
                            return Ok(None);
                        }
                        GeminiPart::FunctionResponse { .. } => {
                            // Function response from user - shouldn't appear in streaming
                            return Ok(None);
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Converts a Gemini response to ResponseItem.
pub fn convert_gemini_to_response_item(candidate: &GeminiCandidate) -> Option<ResponseItem> {
    let content = candidate.content.as_ref()?;

    let mut text_parts = Vec::new();
    let mut function_calls = Vec::new();

    for part in &content.parts {
        match part {
            GeminiPart::Text { text } => {
                text_parts.push(ContentItem::OutputText {
                    text: text.clone(),
                });
            }
            GeminiPart::FunctionCall { function_call } => {
                function_calls.push(function_call.clone());
            }
            GeminiPart::FunctionResponse { .. } => {
                // Skip function responses
            }
        }
    }

    // Return text message if we have text
    if !text_parts.is_empty() {
        return Some(ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: text_parts,
            end_turn: None,
            phase: None,
        });
    }

    // Return function call if we have one
    if let Some(fc) = function_calls.into_iter().next() {
        let arguments = serde_json::to_string(&fc.args).unwrap_or_default();
        // Generate a unique call_id since Gemini doesn't provide one
        let call_id = format!("gemini_call_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0));
        return Some(ResponseItem::FunctionCall {
            id: None,
            call_id,
            name: fc.name,
            arguments,
        });
    }

    None
}

fn map_gemini_error(error: &GeminiError) -> ApiError {
    let code = error.code.unwrap_or(0);
    let message = error
        .message
        .clone()
        .unwrap_or_else(|| "Unknown error".to_string());
    let status = error.status.as_deref().unwrap_or("UNKNOWN");

    match status {
        "RESOURCE_EXHAUSTED" | "RATE_LIMIT_EXCEEDED" => ApiError::Retryable {
            message,
            delay: None,
        },
        "INVALID_ARGUMENT" => {
            if message.contains("token") || message.contains("context") {
                ApiError::ContextWindowExceeded
            } else {
                ApiError::InvalidRequest { message }
            }
        }
        "PERMISSION_DENIED" => ApiError::InvalidRequest {
            message: format!("Permission denied: {}", message),
        },
        _ => ApiError::Stream(format!("Gemini error {}: {} ({})", code, message, status)),
    }
}

// Gemini response types

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiStreamResponse {
    pub candidates: Option<Vec<GeminiCandidate>>,
    pub usage_metadata: Option<GeminiUsageMetadata>,
    pub error: Option<GeminiError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiCandidate {
    pub content: Option<GeminiContent>,
    pub finish_reason: Option<String>,
    pub index: Option<i64>,
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiContent {
    pub parts: Vec<GeminiPart>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeminiFunctionResponse {
    pub name: String,
    pub response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiUsageMetadata {
    pub prompt_token_count: Option<i64>,
    pub candidates_token_count: Option<i64>,
    pub total_token_count: Option<i64>,
    pub cached_content_token_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiSafetyRating {
    pub category: Option<String>,
    pub probability: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiError {
    pub code: Option<i32>,
    pub message: Option<String>,
    pub status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let data = r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"}}]}"#;
        let result = parse_gemini_event(data);
        assert!(matches!(
            result,
            Ok(Some(ResponseEvent::OutputTextDelta(text))) if text == "Hello"
        ));
    }

    #[test]
    fn test_parse_completion() {
        let data = r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"Done"}]}}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5,"totalTokenCount":15}}"#;
        let result = parse_gemini_event(data);
        assert!(matches!(result, Ok(Some(ResponseEvent::Completed { .. }))));
    }

    #[test]
    fn test_parse_error() {
        let data = r#"{"error":{"code":429,"message":"Rate limit exceeded","status":"RESOURCE_EXHAUSTED"}}"#;
        let result = parse_gemini_event(data);
        assert!(matches!(result, Err(ApiError::Retryable { .. })));
    }
}
