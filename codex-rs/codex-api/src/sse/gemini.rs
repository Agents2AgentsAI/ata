//! Gemini SSE/streaming response parser.
//!
//! Gemini uses a JSON lines streaming format rather than standard SSE.
//! Each response chunk contains candidates with content parts.

use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::common::ResponseEvent;
use crate::error::ApiError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;

/// Gemini streaming response chunk structure.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiStreamChunk {
    pub candidates: Option<Vec<GeminiCandidate>>,
    pub usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    pub prompt_feedback: Option<GeminiPromptFeedback>,
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
    pub parts: Option<Vec<GeminiPart>>,
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPart {
    pub text: Option<String>,
    pub function_call: Option<GeminiFunctionCall>,
    pub function_response: Option<GeminiFunctionResponse>,
    pub thought_signature: Option<String>,
    /// Indicates this part is thinking/reasoning content (Gemini thinking mode).
    #[serde(default)]
    pub thought: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionResponse {
    pub name: String,
    pub response: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiUsageMetadata {
    pub prompt_token_count: Option<i64>,
    pub candidates_token_count: Option<i64>,
    pub total_token_count: Option<i64>,
    pub cached_content_token_count: Option<i64>,
    /// Token count for thinking/reasoning content (Gemini thinking mode).
    pub thoughts_token_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiPromptFeedback {
    pub block_reason: Option<String>,
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiSafetyRating {
    pub category: String,
    pub probability: String,
    #[serde(default)]
    pub blocked: bool,
}

/// State tracker for Gemini streaming.
pub struct GeminiStreamState {
    /// Counter for generating unique call IDs for function calls.
    call_id_counter: u64,
    /// Whether we've sent the Created event.
    pub created_sent: bool,
    /// Whether we've emitted OutputItemAdded for the message.
    message_started: bool,
    /// Counter for tracking current thinking/reasoning section index.
    thought_index: i64,
    /// Whether we're currently in a thinking section.
    in_thought_section: bool,
    /// Accumulated thinking text for the current reasoning item.
    accumulated_thought_text: String,
    /// Whether we've emitted OutputItemAdded for a Reasoning item.
    reasoning_item_started: bool,
}

impl Default for GeminiStreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiStreamState {
    pub fn new() -> Self {
        Self {
            call_id_counter: 0,
            created_sent: false,
            message_started: false,
            thought_index: 0,
            in_thought_section: false,
            accumulated_thought_text: String::new(),
            reasoning_item_started: false,
        }
    }

    fn next_call_id(&mut self) -> String {
        self.call_id_counter += 1;
        format!("call_{}", self.call_id_counter)
    }

    fn flush_thinking_section(&mut self, events: &mut Vec<ResponseEvent>) {
        if !self.in_thought_section {
            return;
        }

        events.push(ResponseEvent::ReasoningSummaryPartAdded {
            summary_index: self.thought_index,
        });
        events.push(ResponseEvent::OutputItemDone(ResponseItem::Reasoning {
            id: String::new(),
            summary: vec![ReasoningItemReasoningSummary::SummaryText {
                text: std::mem::take(&mut self.accumulated_thought_text),
            }],
            content: None,
            encrypted_content: None,
        }));
        self.thought_index += 1;
        self.in_thought_section = false;
        self.reasoning_item_started = false;
    }
}

/// Fix Gemini's tendency to add quotes around commands and shell metacharacters.
///
/// Gemini models sometimes:
/// - Wrap the entire `cmd` string in quotes
/// - Quote shell metacharacters like `>`, `|`, `<` within the command
///
/// For example, Gemini might generate `cat '>' file.py` instead of `cat > file.py`.
/// This function fixes these issues by:
/// - Stripping leading/trailing quotes from the entire `cmd` string (only if the entire string is quoted)
/// - Converting patterns like `'>'` back to `>` for proper shell redirection
///   only when all single-quoted segments are standalone shell metachar tokens.
fn is_shell_metachar_token(token: &str) -> bool {
    matches!(
        token,
        "2>&1" | ">&2" | ">>" | "<<" | "2>" | ">&" | ">" | "<" | "|" | "&"
    )
}

fn quoted_segments_are_only_shell_metachar_tokens(cmd: &str) -> bool {
    let mut saw_quoted_segment = false;
    let mut quote_start: Option<usize> = None;

    for (idx, ch) in cmd.char_indices() {
        if ch != '\'' {
            continue;
        }
        if let Some(start) = quote_start {
            saw_quoted_segment = true;
            if !is_shell_metachar_token(&cmd[start..idx]) {
                return false;
            }
            quote_start = None;
        } else {
            quote_start = Some(idx + 1);
        }
    }

    saw_quoted_segment && quote_start.is_none()
}

fn unquote_shell_metachar_tokens(cmd: &str) -> String {
    // Order matters: process longer/more specific patterns first to avoid partial matches.
    cmd.replace("'2>&1'", "2>&1")
        .replace("'>&2'", ">&2")
        .replace("'>>'", ">>")
        .replace("'<<'", "<<")
        .replace("'2>'", "2>")
        .replace("'>&'", ">&")
        .replace("'>'", ">")
        .replace("'<'", "<")
        .replace("'|'", "|")
        .replace("'&'", "&")
}

fn fix_gemini_command_quoting(args: &Value) -> Value {
    if let Some(obj) = args.as_object()
        && let Some(cmd) = obj
            .get("cmd")
            .or_else(|| obj.get("command"))
            .and_then(|v| v.as_str())
    {
        // Strip leading/trailing quotes only if the ENTIRE command is wrapped in matching quotes
        let cmd = if (cmd.starts_with('\'') && cmd.ends_with('\''))
            || (cmd.starts_with('"') && cmd.ends_with('"'))
        {
            &cmd[1..cmd.len() - 1]
        } else {
            cmd
        };

        // Only rewrite quoted metacharacters when every single-quoted segment is one
        // of those tokens. This avoids altering commands that intentionally use quoted
        // literal values like `'x>y'` or `'>'`.
        let cmd = if quoted_segments_are_only_shell_metachar_tokens(cmd) {
            unquote_shell_metachar_tokens(cmd)
        } else {
            cmd.to_string()
        };

        // Rebuild the object with the fixed command
        let mut new_obj = obj.clone();
        if obj.contains_key("cmd") {
            new_obj.insert("cmd".to_string(), json!(cmd));
        } else if obj.contains_key("command") {
            new_obj.insert("command".to_string(), json!(cmd));
        }
        return Value::Object(new_obj);
    }
    args.clone()
}

/// Parses a Gemini streaming response chunk into ResponseEvents.
///
/// Gemini streams JSON objects (not SSE), so this function parses
/// a single JSON chunk and returns any events it contains.
///
/// # Returns
/// A vector of ResponseEvents extracted from the chunk.
pub fn parse_gemini_chunk(
    data: &str,
    state: &mut GeminiStreamState,
) -> Result<Vec<ResponseEvent>, ApiError> {
    let chunk: GeminiStreamChunk = serde_json::from_str(data)
        .map_err(|e| ApiError::Stream(format!("Failed to parse Gemini response: {e}")))?;

    let mut events = Vec::new();

    // Send Created event on first chunk
    if !state.created_sent {
        events.push(ResponseEvent::Created);
        state.created_sent = true;
    }

    // Check for prompt feedback blocking
    if let Some(feedback) = &chunk.prompt_feedback
        && let Some(reason) = &feedback.block_reason
    {
        return Err(ApiError::Stream(format!("Prompt blocked: {reason}")));
    }

    // Process candidates
    if let Some(ref candidates) = chunk.candidates {
        for candidate in candidates {
            // Check for blocked content
            if let Some(ratings) = &candidate.safety_ratings {
                for rating in ratings {
                    if rating.blocked {
                        return Err(ApiError::Stream(format!(
                            "Content blocked by safety filter: {}",
                            rating.category
                        )));
                    }
                }
            }

            if let Some(ref content) = candidate.content
                && let Some(ref parts) = content.parts
            {
                for part in parts {
                    // Handle thinking/reasoning content (Gemini thinking mode)
                    if part.thought {
                        if let Some(ref text) = part.text
                            && !text.is_empty()
                        {
                            // Emit OutputItemAdded for Reasoning on first thinking part
                            if !state.reasoning_item_started {
                                state.reasoning_item_started = true;
                                events.push(ResponseEvent::OutputItemAdded(
                                    ResponseItem::Reasoning {
                                        id: String::new(),
                                        summary: vec![],
                                        content: None,
                                        encrypted_content: None,
                                    },
                                ));
                            }
                            events.push(ResponseEvent::ReasoningSummaryDelta {
                                delta: text.clone(),
                                summary_index: state.thought_index,
                            });
                            state.accumulated_thought_text.push_str(text);
                            if !state.in_thought_section {
                                state.in_thought_section = true;
                            }
                        }
                        continue; // Don't process as regular text
                    }

                    // Emit section break and Reasoning item done when transitioning out of thought
                    if state.in_thought_section {
                        state.flush_thinking_section(&mut events);
                    }

                    // Handle text content
                    if let Some(ref text) = part.text
                        && !text.is_empty()
                    {
                        // Emit OutputItemAdded for the message if this is the first text
                        if !state.message_started {
                            state.message_started = true;
                            events.push(ResponseEvent::OutputItemAdded(ResponseItem::Message {
                                id: None,
                                role: "assistant".to_string(),
                                content: vec![],
                                end_turn: None,
                                phase: None,
                            }));
                        }
                        events.push(ResponseEvent::OutputTextDelta(text.clone()));
                    }

                    // Handle function calls
                    if let Some(ref function_call) = part.function_call {
                        let call_id = state.next_call_id();
                        // Fix Gemini's tendency to quote shell metacharacters
                        let fixed_args = function_call
                            .args
                            .as_ref()
                            .map(fix_gemini_command_quoting)
                            .unwrap_or_else(|| json!({}));
                        let arguments = fixed_args.to_string();

                        // Emit the function call as an OutputItemDone
                        events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                            id: None,
                            call_id,
                            name: function_call.name.clone(),
                            arguments,
                            thought_signature: part.thought_signature.clone(),
                        }));
                    }
                }
            }

            // Check for completion
            if let Some(finish_reason) = &candidate.finish_reason
                && (finish_reason == "STOP"
                    || finish_reason == "MAX_TOKENS"
                    || finish_reason == "SAFETY"
                    || finish_reason == "RECITATION"
                    || finish_reason == "OTHER")
            {
                // Flush any outstanding thinking section before completing
                if state.in_thought_section {
                    state.flush_thinking_section(&mut events);
                }

                // Get usage from the chunk if available
                let token_usage = chunk.usage_metadata.as_ref().map(|usage| TokenUsage {
                    input_tokens: usage.prompt_token_count.unwrap_or(0),
                    output_tokens: usage.candidates_token_count.unwrap_or(0),
                    cached_input_tokens: usage.cached_content_token_count.unwrap_or(0),
                    reasoning_output_tokens: usage.thoughts_token_count.unwrap_or(0),
                    total_tokens: usage.total_token_count.unwrap_or(0),
                });

                events.push(ResponseEvent::Completed {
                    response_id: String::new(),
                    token_usage,
                });
            }
        }
    }

    // If no candidates but we have usage metadata, this might be a final chunk
    if chunk.candidates.is_none()
        && let Some(usage) = chunk.usage_metadata
    {
        // Flush any outstanding thinking section before completing
        if state.in_thought_section {
            state.flush_thinking_section(&mut events);
        }

        let token_usage = Some(TokenUsage {
            input_tokens: usage.prompt_token_count.unwrap_or(0),
            output_tokens: usage.candidates_token_count.unwrap_or(0),
            cached_input_tokens: usage.cached_content_token_count.unwrap_or(0),
            reasoning_output_tokens: usage.thoughts_token_count.unwrap_or(0),
            total_tokens: usage.total_token_count.unwrap_or(0),
        });

        events.push(ResponseEvent::Completed {
            response_id: String::new(),
            token_usage,
        });
    }

    Ok(events)
}

/// Converts accumulated text and function calls into a final message ResponseItem.
pub fn build_message_item(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        end_turn: Some(true),
        phase: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_chunk() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello, "}],
                    "role": "model"
                },
                "index": 0
            }]
        }"#;

        let mut state = GeminiStreamState::new();
        let events = parse_gemini_chunk(data, &mut state).unwrap();

        assert_eq!(events.len(), 3); // Created + OutputItemAdded + TextDelta
        assert!(matches!(events[0], ResponseEvent::Created));
        assert!(matches!(
            events[1],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(events[2], ResponseEvent::OutputTextDelta(ref s) if s == "Hello, "));
    }

    #[test]
    fn test_parse_function_call() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "San Francisco"}
                        }
                    }],
                    "role": "model"
                },
                "index": 0
            }]
        }"#;

        let mut state = GeminiStreamState::new();
        let events = parse_gemini_chunk(data, &mut state).unwrap();

        assert_eq!(events.len(), 2); // Created + FunctionCall
        assert!(matches!(events[0], ResponseEvent::Created));
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { name, .. }) => {
                assert_eq!(name, "get_weather");
            }
            _ => panic!("Expected FunctionCall"),
        }
    }

    #[test]
    fn test_parse_completion() {
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Done!"}],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 5,
                "totalTokenCount": 15
            }
        }"#;

        let mut state = GeminiStreamState::new();
        let events = parse_gemini_chunk(data, &mut state).unwrap();

        assert_eq!(events.len(), 4); // Created + OutputItemAdded + TextDelta + Completed
        assert!(matches!(events[0], ResponseEvent::Created));
        assert!(matches!(
            events[1],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(events[2], ResponseEvent::OutputTextDelta(_)));
        match &events[3] {
            ResponseEvent::Completed { token_usage, .. } => {
                let usage = token_usage.as_ref().unwrap();
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
                assert_eq!(usage.total_tokens, 15);
            }
            _ => panic!("Expected Completed"),
        }
    }

    #[test]
    fn test_created_only_sent_once() {
        let data = r#"{"candidates": [{"content": {"parts": [{"text": "a"}]}}]}"#;

        let mut state = GeminiStreamState::new();

        let events1 = parse_gemini_chunk(data, &mut state).unwrap();
        assert!(matches!(events1[0], ResponseEvent::Created));

        let events2 = parse_gemini_chunk(data, &mut state).unwrap();
        // Second call should not have Created or OutputItemAdded
        assert!(!events2.iter().any(|e| matches!(e, ResponseEvent::Created)));
        assert!(
            !events2
                .iter()
                .any(|e| matches!(e, ResponseEvent::OutputItemAdded(_)))
        );
    }

    #[test]
    fn test_fix_gemini_command_quoting_strips_outer_quotes() {
        // Only strip outer quotes if the ENTIRE command is wrapped
        let args = json!({"cmd": "'echo hello'"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "echo hello");

        let args = json!({"cmd": "\"echo hello\""});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "echo hello");

        // Should NOT strip if quotes don't wrap the entire command
        let args = json!({"cmd": "echo 'hello world'"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "echo 'hello world'");
    }

    #[test]
    fn test_fix_gemini_command_quoting_fixes_quoted_metacharacters() {
        // Test the main issue: cat '>' file.py should become cat > file.py
        let args = json!({"cmd": "cat '>' fibonacci.py"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "cat > fibonacci.py");

        // Test pipe
        let args = json!({"cmd": "ls '|' grep foo"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "ls | grep foo");

        // Test input redirection
        let args = json!({"cmd": "sort '<' input.txt"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "sort < input.txt");

        // Test append redirection
        let args = json!({"cmd": "echo hello '>>' output.txt"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "echo hello >> output.txt");

        // Test stderr redirection
        let args = json!({"cmd": "cmd '2>' errors.log"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "cmd 2> errors.log");

        // Test stderr to stdout redirect
        let args = json!({"cmd": "cmd '2>&1'"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "cmd 2>&1");
    }

    #[test]
    fn test_fix_gemini_command_quoting_preserves_other_args() {
        let args = json!({"cmd": "cat '>' file.py", "workdir": "/tmp", "timeout_ms": 5000});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "cat > file.py");
        assert_eq!(fixed["workdir"], "/tmp");
        assert_eq!(fixed["timeout_ms"], 5000);
    }

    #[test]
    fn test_fix_gemini_command_quoting_handles_command_key() {
        // Test with "command" key instead of "cmd"
        let args = json!({"command": "cat '>' file.py"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["command"], "cat > file.py");
    }

    #[test]
    fn test_fix_gemini_command_quoting_preserves_valid_quotes() {
        // Quotes inside file paths should be preserved (they're not the patterns we're fixing)
        let args = json!({"cmd": "cat 'file with spaces.txt'"});
        let fixed = fix_gemini_command_quoting(&args);
        // This should be unchanged since 'file with spaces.txt' isn't a metacharacter pattern
        assert_eq!(fixed["cmd"], "cat 'file with spaces.txt'");
    }

    #[test]
    fn test_fix_gemini_command_quoting_does_not_rewrite_literal_quoted_metachar() {
        let args = json!({"cmd": "test 'x>y' '>' z"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["cmd"], "test 'x>y' '>' z");
    }

    #[test]
    fn test_fix_gemini_command_quoting_non_cmd_args_unchanged() {
        // Non-command arguments should not be modified
        let args = json!({"path": "/some/path", "content": "some > content"});
        let fixed = fix_gemini_command_quoting(&args);
        assert_eq!(fixed["path"], "/some/path");
        assert_eq!(fixed["content"], "some > content");
    }

    #[test]
    fn test_parse_function_call_with_quoted_redirect() {
        // Test the full flow: parsing a function call with a quoted redirect
        let data = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "shell",
                            "args": {"cmd": "cat '>' fibonacci.py"}
                        }
                    }],
                    "role": "model"
                },
                "index": 0
            }]
        }"#;

        let mut state = GeminiStreamState::new();
        let events = parse_gemini_chunk(data, &mut state).unwrap();

        assert_eq!(events.len(), 2); // Created + FunctionCall
        match &events[1] {
            ResponseEvent::OutputItemDone(ResponseItem::FunctionCall { arguments, .. }) => {
                // The arguments should have the fixed command
                let args: Value = serde_json::from_str(arguments).unwrap();
                assert_eq!(args["cmd"], "cat > fibonacci.py");
            }
            _ => panic!("Expected FunctionCall"),
        }
    }
}
