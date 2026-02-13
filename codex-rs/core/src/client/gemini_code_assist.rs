use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use super::ModelClientSession;
use super::provider_streaming::ParseSseEventResult;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::extract_sse_data_line;
use super::provider_streaming::filter_out_created;
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
use crate::auth::ProviderOauthCredential;
use crate::auth::code_assist_method_url;
use crate::auth::ensure_gemini_oauth_context;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::default_client::get_codex_user_agent;
use crate::error::CodexErr;
use crate::error::Result;
use crate::tools::spec::create_tools_json_for_responses_api;

const CODE_ASSIST_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

pub(super) async fn stream_gemini_code_assist(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
    oauth_credential: ProviderOauthCredential,
) -> Result<ResponseStream> {
    let input = prompt.get_formatted_input();
    let instructions = &prompt.base_instructions.text;
    let tools = create_tools_json_for_responses_api(&prompt.tools)?;

    let adapter = GeminiAdapter::new();
    let input_values = serialize_input_items(&input)?;
    let reasoning_value = build_reasoning_value(model_info, effort, summary);

    let raw_request_body = adapter
        .build_request_body(
            &model_info.slug,
            instructions,
            &input_values,
            &tools,
            &codex_api::RequestOptions {
                parallel_tool_calls: prompt.parallel_tool_calls,
                reasoning: reasoning_value,
                ..Default::default()
            },
        )
        .map_err(|err| crate::error::CodexErr::Api(err.to_string()))?;
    let request_body = normalize_code_assist_request_body(raw_request_body);

    let context = ensure_gemini_oauth_context(
        &session.client.state.codex_home,
        session.client.state.cli_auth_credentials_store_mode,
        oauth_credential,
    )
    .await?;
    let request_body =
        wrap_code_assist_request(&model_info.slug, &context.project_id, request_body);
    let access_token = context.access_token;

    let url = code_assist_method_url("streamGenerateContent");
    let client = build_reqwest_client();
    let request = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {access_token}"))
        .header("X-Goog-Api-Client", build_x_goog_api_client_header())
        .header("User-Agent", get_codex_user_agent())
        .json(&request_body);

    let idle_timeout = session.client.state.provider.stream_idle_timeout();
    let mut stream_state = GeminiStreamState::new();
    stream_state.created_sent = true;

    Ok(spawn_provider_sse_stream(
        request,
        idle_timeout,
        "Gemini Code Assist",
        stream_state,
        vec![ResponseEvent::Created],
        |event_str, state| {
            let Some(data) = extract_sse_data_line(event_str) else {
                return ParseSseEventResult::Continue;
            };
            parse_code_assist_sse_data(data, state, false)
        },
        |buffer, state| {
            let Some(data) = extract_sse_data_line(buffer) else {
                return ParseSseEventResult::Continue;
            };
            parse_code_assist_sse_data(data, state, true)
        },
    ))
}

fn build_x_goog_api_client_header() -> String {
    let codex_version = env!("CARGO_PKG_VERSION");
    format!("gl-rust/unknown codex-core/{codex_version}")
}

fn parse_code_assist_sse_data(
    data: &str,
    state: &mut GeminiStreamState,
    ignore_parse_errors: bool,
) -> ParseSseEventResult {
    if data.trim() == "[DONE]" {
        return ParseSseEventResult::Emit(vec![ResponseEvent::Completed {
            response_id: String::new(),
            token_usage: None,
            can_append: false,
        }]);
    }

    if let Some(error_message) = extract_code_assist_stream_error_message(data) {
        return ParseSseEventResult::Fatal(CodexErr::Api(error_message));
    }

    let normalized = unwrap_code_assist_response_envelope(data);
    if let Some(error_message) = extract_code_assist_stream_error_message(&normalized) {
        return ParseSseEventResult::Fatal(CodexErr::Api(error_message));
    }

    match codex_api::sse::gemini::parse_gemini_chunk(&normalized, state) {
        Ok(events) => {
            let events = filter_out_created(events);
            if events.is_empty() {
                ParseSseEventResult::Continue
            } else {
                ParseSseEventResult::Emit(events)
            }
        }
        Err(err) if ignore_parse_errors => ParseSseEventResult::Continue,
        Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
    }
}

fn wrap_code_assist_request(model: &str, project: &str, request_body: Value) -> Value {
    let mapped_model = map_code_assist_model(model);
    let mut request_body = request_body;

    if let Some(envelope) = request_body.as_object_mut()
        && envelope.contains_key("request")
        && envelope.contains_key("model")
    {
        if !project.is_empty() && !envelope.contains_key("project") {
            envelope.insert("project".to_string(), Value::String(project.to_string()));
        }
        if let Some(model_value) = envelope.get_mut("model")
            && let Some(existing_model) = model_value.as_str()
        {
            *model_value = Value::String(map_code_assist_model(existing_model).to_string());
        }
        return request_body;
    }

    if project.is_empty() {
        json!({
            "model": mapped_model,
            "request": request_body,
        })
    } else {
        json!({
            "project": project,
            "model": mapped_model,
            "request": request_body,
        })
    }
}

fn map_code_assist_model(model: &str) -> &str {
    match model {
        // Code Assist currently rejects image-focused aliases.
        "gemini-2.5-flash-image-preview" => "gemini-2.5-flash",
        "gemini-2.0-flash-exp-image-generation" => "gemini-2.0-flash",
        // Code Assist currently does not accept Gemini 3 preview aliases.
        "gemini-3-flash-preview" | "gemini-3-flash" => "gemini-2.5-flash",
        "gemini-3-pro-preview" | "gemini-3-pro" => "gemini-2.5-pro",
        _ => model,
    }
}

fn extract_code_assist_stream_error_message(payload: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(payload).ok()?;

    if let Some(error) = parsed.get("error") {
        return Some(format_code_assist_error(error));
    }

    if let Some(error) = parsed
        .get("response")
        .and_then(|response| response.get("error"))
    {
        return Some(format_code_assist_error(error));
    }

    let response_internal = parsed
        .pointer("/response/sdkHttpResponse/responseInternal")
        .or_else(|| parsed.pointer("/sdkHttpResponse/responseInternal"))?;
    if response_internal.get("ok").and_then(Value::as_bool) == Some(false) {
        let status = response_internal.get("status").and_then(Value::as_i64);
        let status_text = response_internal
            .get("statusText")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let status_suffix = match (status, status_text) {
            (Some(status), Some(status_text)) => format!(" ({status} {status_text})"),
            (Some(status), None) => format!(" ({status})"),
            (None, Some(status_text)) => format!(" ({status_text})"),
            (None, None) => String::new(),
        };
        return Some(format!(
            "Gemini Code Assist API stream returned a non-OK response{status_suffix}."
        ));
    }

    None
}

fn format_code_assist_error(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let status = error
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let status_suffix = match (code, status) {
        (Some(code), Some(status)) => format!(" ({code} {status})"),
        (Some(code), None) => format!(" ({code})"),
        (None, Some(status)) => format!(" ({status})"),
        (None, None) => String::new(),
    };
    let message = message
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string());
    format!("Gemini Code Assist API error{status_suffix}: {message}")
}

fn normalize_code_assist_request_body(body: Value) -> Value {
    let mut body = body;
    let Some(object) = body.as_object_mut() else {
        return body;
    };

    rename_field(object, "system_instruction", "systemInstruction");
    rename_field(object, "generation_config", "generationConfig");
    rename_field(object, "cached_content", "cachedContent");

    if let Some(generation_config) = object.get_mut("generationConfig")
        && let Some(generation_config) = generation_config.as_object_mut()
    {
        rename_field(generation_config, "thinking_config", "thinkingConfig");
    }

    if let Some(contents) = object.get_mut("contents").and_then(Value::as_array_mut) {
        for content in contents {
            normalize_content_part(content);
        }
    }

    body
}

fn normalize_content_part(content: &mut Value) {
    let Some(content_obj) = content.as_object_mut() else {
        return;
    };

    if let Some(parts) = content_obj.get_mut("parts").and_then(Value::as_array_mut) {
        for part in parts {
            let Some(part_obj) = part.as_object_mut() else {
                continue;
            };

            rename_field(part_obj, "function_call", "functionCall");
            rename_field(part_obj, "thought_signature", "thoughtSignature");

            if part_obj.contains_key("functionCall") && !part_obj.contains_key("thoughtSignature") {
                part_obj.insert(
                    "thoughtSignature".to_string(),
                    Value::String(CODE_ASSIST_THOUGHT_SIGNATURE.to_string()),
                );
            }
        }
    }
}

fn rename_field(map: &mut Map<String, Value>, from: &str, to: &str) {
    if map.contains_key(to) {
        let _ = map.remove(from);
        return;
    }

    if let Some(value) = map.remove(from) {
        map.insert(to.to_string(), value);
    }
}

fn unwrap_code_assist_response_envelope(data: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<Value>(data) else {
        return data.to_string();
    };

    if let Some(inner_response) = parsed.get("response") {
        match serde_json::to_string(inner_response) {
            Ok(serialized) => serialized,
            Err(_) => data.to_string(),
        }
    } else {
        data.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn wrap_code_assist_request_wraps_bare_requests() {
        let wrapped = wrap_code_assist_request(
            "gemini-2.5-flash-image-preview",
            "project-123",
            json!({"contents": []}),
        );

        assert_eq!(
            wrapped,
            json!({
                "project": "project-123",
                "model": "gemini-2.5-flash",
                "request": {
                    "contents": []
                }
            })
        );
    }

    #[test]
    fn wrap_code_assist_request_does_not_double_wrap() {
        let wrapped = wrap_code_assist_request(
            "gemini-2.5-flash",
            "project-123",
            json!({
                "project": "existing-project",
                "model": "gemini-2.0-flash-exp-image-generation",
                "request": {"contents": []}
            }),
        );

        assert_eq!(
            wrapped,
            json!({
                "project": "existing-project",
                "model": "gemini-2.0-flash",
                "request": {
                    "contents": []
                }
            })
        );
    }

    #[test]
    fn normalize_code_assist_request_body_normalizes_casing_and_signatures() {
        let body = normalize_code_assist_request_body(json!({
            "system_instruction": {"parts": [{"text": "test"}]},
            "generation_config": {"thinking_config": {"includeThoughts": true}},
            "cached_content": "cache-1",
            "contents": [{
                "parts": [{
                    "function_call": {"name": "tool", "args": {}}
                }]
            }]
        }));

        assert_eq!(
            body,
            json!({
                "systemInstruction": {"parts": [{"text": "test"}]},
                "generationConfig": {"thinkingConfig": {"includeThoughts": true}},
                "cachedContent": "cache-1",
                "contents": [{
                    "parts": [{
                        "functionCall": {"name": "tool", "args": {}},
                        "thoughtSignature": "skip_thought_signature_validator"
                    }]
                }]
            })
        );
    }

    #[test]
    fn unwrap_code_assist_response_envelope_extracts_inner_response() {
        let unwrapped = unwrap_code_assist_response_envelope(
            r#"{"response":{"candidates":[{"finishReason":"STOP"}]}}"#,
        );
        assert_eq!(unwrapped, r#"{"candidates":[{"finishReason":"STOP"}]}"#);
    }

    #[test]
    fn map_code_assist_model_remaps_gemini_3_aliases() {
        assert_eq!(
            map_code_assist_model("gemini-3-flash-preview"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            map_code_assist_model("gemini-3-pro-preview"),
            "gemini-2.5-pro"
        );
    }

    #[test]
    fn parse_code_assist_sse_data_returns_fatal_for_error_payload() {
        let mut state = GeminiStreamState::new();
        let result = parse_code_assist_sse_data(
            r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","message":"Unsupported model 'gemini-3-flash-preview'"}}"#,
            &mut state,
            false,
        );

        let ParseSseEventResult::Fatal(CodexErr::Api(message)) = result else {
            panic!("expected fatal API error");
        };
        assert!(message.contains("Unsupported model"), "{message}");
        assert!(message.contains("INVALID_ARGUMENT"), "{message}");
    }

    #[test]
    fn parse_code_assist_sse_data_returns_fatal_for_non_ok_response_internal() {
        let mut state = GeminiStreamState::new();
        let result = parse_code_assist_sse_data(
            r#"{"response":{"sdkHttpResponse":{"responseInternal":{"ok":false,"status":400,"statusText":"Bad Request"}}}}"#,
            &mut state,
            false,
        );

        let ParseSseEventResult::Fatal(CodexErr::Api(message)) = result else {
            panic!("expected fatal API error");
        };
        assert!(message.contains("non-OK response"), "{message}");
        assert!(message.contains("400 Bad Request"), "{message}");
    }
}
