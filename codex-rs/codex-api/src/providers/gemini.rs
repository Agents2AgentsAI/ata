//! Gemini provider adapter implementation.
//!
//! This adapter handles Gemini API-specific request building and response parsing,
//! including the critical schema transformation to remove unsupported properties.

use http::HeaderMap;
use serde_json::Value;
use serde_json::json;

use crate::error::ApiError;
use crate::provider_adapter::ProviderAdapter;
use crate::provider_adapter::RequestOptions;
use crate::tools::ToolFormatter;
use crate::tools::gemini::GeminiToolFormatter;

/// Gemini GenerateContent API adapter.
pub struct GeminiAdapter {
    tool_formatter: GeminiToolFormatter,
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self {
            tool_formatter: GeminiToolFormatter::new(),
        }
    }
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAdapter for GeminiAdapter {
    fn provider_id(&self) -> &str {
        "gemini"
    }

    fn format_tools(&self, tools: &[Value]) -> Result<Vec<Value>, ApiError> {
        self.tool_formatter.format_tools(tools)
    }

    fn build_request_body(
        &self,
        _model: &str,
        instructions: &str,
        input: &[Value],
        tools: &[Value],
        options: &RequestOptions,
    ) -> Result<Value, ApiError> {
        // Build contents array from input
        let contents = build_gemini_contents(input)?;

        // Build the generation config with optional thinking config
        let mut generation_config = json!({
            "temperature": 0.7
        });

        // Add thinking config if reasoning options are specified
        if let Some(reasoning) = &options.reasoning {
            if let Some(effort) = reasoning.get("effort").and_then(|e| e.as_str()) {
                match effort {
                    "none" => {
                        // No thinking requested — skip thinkingConfig entirely
                    }
                    "minimal" | "low" => {
                        generation_config["thinkingConfig"] = json!({
                            "thinkingLevel": "low",
                            "includeThoughts": true
                        });
                    }
                    "medium" => {
                        generation_config["thinkingConfig"] = json!({
                            "thinkingLevel": "medium",
                            "includeThoughts": true
                        });
                    }
                    // "high", "xhigh", or any unknown value
                    _ => {
                        generation_config["thinkingConfig"] = json!({
                            "thinkingLevel": "high",
                            "includeThoughts": true
                        });
                    }
                }
            }
        }

        // Build the request body
        let mut body = json!({
            "contents": contents,
            "generationConfig": generation_config
        });

        // Add system instruction if provided, with Gemini-specific guidance
        if !instructions.is_empty() {
            // Append Gemini-specific guidance for shell commands to prevent
            // the model from incorrectly quoting shell metacharacters
            let gemini_guidance = "\n\nIMPORTANT for shell commands: Use standard shell syntax. \
                Do NOT quote shell metacharacters like >, |, <, &, etc. \
                Correct: cat > file.txt\n\
                Incorrect: cat '>' file.txt\n\n\
                IMPORTANT for MCP tools: When using read_mcp_resource, NEVER guess server names. \
                Common mistakes include guessing 'filesystem' for file:// URIs. \
                ALWAYS call list_mcp_resources first to discover the actual server names and URIs.";
            let full_instructions = format!("{}{}", instructions, gemini_guidance);
            body["systemInstruction"] = json!({
                "parts": [{"text": full_instructions}]
            });
        }

        // Add tools if any
        if !tools.is_empty() {
            let formatted_tools = self.format_tools(tools)?;
            body["tools"] = json!([{
                "functionDeclarations": formatted_tools
            }]);

            // Tool configuration
            body["toolConfig"] = json!({
                "functionCallingConfig": {
                    "mode": "AUTO"
                }
            });
        }

        // Handle parallel tool calls (Gemini doesn't have direct equivalent)
        let _ = options.parallel_tool_calls;

        Ok(body)
    }

    fn streaming_endpoint(&self, model: &str) -> String {
        // Gemini streaming endpoint format
        // Note: The model parameter needs URL encoding for model names with special chars
        format!("/models/{}:streamGenerateContent", model)
    }

    fn extra_headers(&self) -> HeaderMap {
        // Gemini doesn't require extra headers beyond auth
        HeaderMap::new()
    }

    fn auth_header_name(&self) -> &str {
        // Gemini uses x-goog-api-key header
        "x-goog-api-key"
    }

    fn format_auth_header(&self, api_key: &str) -> String {
        // Gemini API key is passed directly, not as Bearer token
        api_key.to_string()
    }
}

/// Converts input ResponseItems to Gemini contents format.
fn build_gemini_contents(input: &[Value]) -> Result<Vec<Value>, ApiError> {
    let mut contents = Vec::new();
    // Track function call_id -> function name mapping
    let mut call_id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for item in input {
        let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match item_type {
            "message" => {
                let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                let gemini_role = match role {
                    "user" => "user",
                    "assistant" => "model",
                    _ => "user",
                };

                let mut parts = Vec::new();
                if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                    for block in content {
                        let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match block_type {
                            "input_text" | "output_text" => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    parts.push(json!({"text": text}));
                                }
                            }
                            "input_image" => {
                                // Handle image content
                                if let Some(url) = block.get("image_url").and_then(|u| u.as_str()) {
                                    if url.starts_with("data:") {
                                        // Base64 data URL
                                        if let Some((mime, data)) = parse_data_url(url) {
                                            parts.push(json!({
                                                "inlineData": {
                                                    "mimeType": mime,
                                                    "data": data
                                                }
                                            }));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if !parts.is_empty() {
                    contents.push(json!({
                        "role": gemini_role,
                        "parts": parts
                    }));
                }
            }

            "function_call" => {
                // Gemini function calls are part of model content
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let arguments = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let thought_signature = item.get("thought_signature").and_then(|t| t.as_str());

                // Track call_id -> name mapping for later function_call_output
                if !call_id.is_empty() && !name.is_empty() {
                    call_id_to_name.insert(call_id.to_string(), name.to_string());
                }

                let args: Value = serde_json::from_str(arguments).unwrap_or(json!({}));

                let mut part = json!({
                    "functionCall": {
                        "name": name,
                        "args": args
                    }
                });

                // Include thought signature if present (required for Gemini 3 models)
                if let Some(sig) = thought_signature {
                    part["thoughtSignature"] = json!(sig);
                }

                contents.push(json!({
                    "role": "model",
                    "parts": [part]
                }));
            }

            "function_call_output" => {
                // Gemini function responses
                let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                let output = item.get("output").cloned().unwrap_or(json!({}));

                // Get the function name from our mapping, or use call_id as fallback
                let name = call_id_to_name
                    .get(call_id)
                    .cloned()
                    .unwrap_or_else(|| call_id.to_string());

                // Gemini requires response to be a JSON object (Struct), not a string
                // If output is a string or other primitive, wrap it in an object
                let response_obj = match &output {
                    Value::Object(_) => output,
                    Value::String(s) => json!({"result": s}),
                    Value::Array(_) => json!({"result": output}),
                    Value::Number(n) => json!({"result": n}),
                    Value::Bool(b) => json!({"result": b}),
                    Value::Null => json!({"result": null}),
                };

                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": response_obj
                        }
                    }]
                }));
            }

            _ => {
                // Skip unsupported item types
            }
        }
    }

    Ok(contents)
}

/// Parses a data URL into (mime_type, base64_data).
fn parse_data_url(url: &str) -> Option<(String, String)> {
    // Format: data:mime/type;base64,<data>
    let url = url.strip_prefix("data:")?;
    let (header, data) = url.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    Some((mime.to_string(), data.to_string()))
}

/// Maps tool choice to Gemini function calling mode.
///
/// Gemini supports:
/// - AUTO: Let the model decide
/// - ANY: Force function calling (at least one must be called)
/// - NONE: Disable function calling
///
/// Note: Gemini's AUTO mode doesn't support allowedFunctionNames,
/// so if specific functions are required, use ANY mode instead.
pub fn map_tool_choice(choice: Option<&str>, has_allowed_tools: bool) -> Value {
    match choice {
        None | Some("auto") => {
            // Quirk: Gemini Auto doesn't support allowed_function_names
            // If we have allowed tools, use ANY mode instead
            if has_allowed_tools {
                json!({ "mode": "ANY" })
            } else {
                json!({ "mode": "AUTO" })
            }
        }
        Some("required") => json!({ "mode": "ANY" }),
        Some("none") => json!({ "mode": "NONE" }),
        Some(specific) => json!({
            "mode": "ANY",
            "allowedFunctionNames": [specific]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_endpoint() {
        let adapter = GeminiAdapter::new();
        assert_eq!(
            adapter.streaming_endpoint("gemini-2.0-flash"),
            "/models/gemini-2.0-flash:streamGenerateContent"
        );
    }

    #[test]
    fn test_auth_header() {
        let adapter = GeminiAdapter::new();
        assert_eq!(adapter.auth_header_name(), "x-goog-api-key");
        assert_eq!(adapter.format_auth_header("my_key"), "my_key");
    }

    #[test]
    fn test_build_request_body() {
        let adapter = GeminiAdapter::new();

        let input = vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "Hello"}]
        })];

        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "test",
                "description": "A test function",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })];

        let options = RequestOptions::default();

        let body = adapter
            .build_request_body(
                "gemini-2.0-flash",
                "You are helpful",
                &input,
                &tools,
                &options,
            )
            .unwrap();

        assert!(body.get("contents").is_some());
        assert!(body.get("systemInstruction").is_some());
        assert!(body.get("tools").is_some());
        assert!(body.get("toolConfig").is_some());
    }

    #[test]
    fn test_map_tool_choice() {
        assert_eq!(map_tool_choice(None, false), json!({"mode": "AUTO"}));
        assert_eq!(
            map_tool_choice(Some("auto"), false),
            json!({"mode": "AUTO"})
        );
        assert_eq!(map_tool_choice(Some("auto"), true), json!({"mode": "ANY"}));
        assert_eq!(
            map_tool_choice(Some("required"), false),
            json!({"mode": "ANY"})
        );
        assert_eq!(
            map_tool_choice(Some("none"), false),
            json!({"mode": "NONE"})
        );
        assert_eq!(
            map_tool_choice(Some("get_weather"), false),
            json!({"mode": "ANY", "allowedFunctionNames": ["get_weather"]})
        );
    }

    #[test]
    fn test_parse_data_url() {
        let url = "data:image/png;base64,iVBORw0KGgo=";
        let result = parse_data_url(url);
        assert!(result.is_some());
        let (mime, data) = result.unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(data, "iVBORw0KGgo=");
    }

    #[test]
    fn test_build_gemini_contents() {
        let input = vec![
            json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "Hi"}]
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }),
        ];

        let contents = build_gemini_contents(&input).unwrap();

        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "Hi");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "Hello!");
    }
}
