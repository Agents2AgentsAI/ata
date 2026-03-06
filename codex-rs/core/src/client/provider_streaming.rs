use codex_api::common::Reasoning;
use codex_api::error::ApiError;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use futures::StreamExt;
use reqwest::RequestBuilder;
use reqwest::StatusCode;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::error::CodexErr;
use crate::error::Result;
use crate::util::redact_error_body;

/// Maps an [`ApiError`] to the corresponding [`CodexErr`] variant,
/// preserving structured error kinds (e.g. `ContextWindowExceeded`)
/// instead of flattening them to a generic string.
pub(super) fn map_api_err_to_codex_err(err: ApiError) -> CodexErr {
    match err {
        ApiError::ContextWindowExceeded => CodexErr::ContextWindowExceeded,
        other => CodexErr::Api(other.to_string()),
    }
}

/// Serializes input items with proper error handling.
///
/// Unlike `filter_map(...ok())`, this returns an error if any item fails to serialize,
/// preventing incomplete prompts from being sent silently.
pub(super) fn serialize_input_items(input: &[ResponseItem]) -> Result<Vec<Value>> {
    input
        .iter()
        .map(|item| {
            serde_json::to_value(item)
                .map_err(|e| CodexErr::Api(format!("Failed to serialize input item: {e}")))
        })
        .collect()
}

/// Builds provider reasoning payload for non-Responses streaming adapters.
pub(super) fn build_reasoning_value(
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Option<Value> {
    if !model_info.supports_reasoning_summaries {
        return None;
    }

    let reasoning = Reasoning {
        effort: effort.or(model_info.default_reasoning_level),
        summary: if summary == ReasoningSummaryConfig::None {
            None
        } else {
            Some(summary)
        },
    };

    serde_json::to_value(reasoning).ok()
}

pub(super) fn extract_sse_data_line(event_str: &str) -> Option<String> {
    let data_lines: Vec<&str> = event_str
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:").map(str::trim_start))
        })
        .collect();
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

pub(super) fn filter_out_created(events: Vec<ResponseEvent>) -> Vec<ResponseEvent> {
    events
        .into_iter()
        .filter(|event| !matches!(event, ResponseEvent::Created))
        .collect()
}

pub(super) enum ParseSseEventResult {
    Continue,
    Emit(Vec<ResponseEvent>),
    Fatal(CodexErr),
}

pub(super) fn spawn_provider_sse_stream<State, ParseEvent, ParseTrailing>(
    request: RequestBuilder,
    idle_timeout: Duration,
    status_error_prefix: &'static str,
    mut state: State,
    initial_events: Vec<ResponseEvent>,
    mut parse_event: ParseEvent,
    mut parse_trailing: ParseTrailing,
) -> ResponseStream
where
    State: Send + 'static,
    ParseEvent: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
    ParseTrailing: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
{
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

    tokio::spawn(async move {
        match request.send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let err = map_status_error(status_error_prefix, status, &body);
                    let _ = tx_event.send(Err(err)).await;
                    return;
                }

                stream_provider_response(
                    response,
                    idle_timeout,
                    &tx_event,
                    &mut state,
                    initial_events,
                    &mut parse_event,
                    &mut parse_trailing,
                )
                .await;
            }
            Err(err) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api(format!("Request failed: {err}"))))
                    .await;
            }
        }
    });

    ResponseStream { rx_event }
}

pub(super) fn spawn_provider_sse_stream_from_response<State, ParseEvent, ParseTrailing>(
    response: reqwest::Response,
    idle_timeout: Duration,
    mut state: State,
    initial_events: Vec<ResponseEvent>,
    mut parse_event: ParseEvent,
    mut parse_trailing: ParseTrailing,
) -> ResponseStream
where
    State: Send + 'static,
    ParseEvent: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
    ParseTrailing: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
{
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);
    tokio::spawn(async move {
        stream_provider_response(
            response,
            idle_timeout,
            &tx_event,
            &mut state,
            initial_events,
            &mut parse_event,
            &mut parse_trailing,
        )
        .await;
    });
    ResponseStream { rx_event }
}

async fn stream_provider_response<State, ParseEvent, ParseTrailing>(
    response: reqwest::Response,
    idle_timeout: Duration,
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    state: &mut State,
    initial_events: Vec<ResponseEvent>,
    parse_event: &mut ParseEvent,
    parse_trailing: &mut ParseTrailing,
) where
    ParseEvent: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
    ParseTrailing: FnMut(&str, &mut State) -> ParseSseEventResult + Send + 'static,
{
    if emit_events(tx_event, initial_events).await {
        return;
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    loop {
        let chunk_result = tokio::time::timeout(idle_timeout, stream.next()).await;

        match chunk_result {
            Ok(Some(Ok(chunk))) => {
                if let Ok(text) = std::str::from_utf8(&chunk) {
                    buffer.push_str(text);
                }

                while let Some(event_str) = take_next_sse_event(&mut buffer) {
                    if event_str.trim().is_empty() {
                        continue;
                    }

                    if handle_parse_result(parse_event(&event_str, state), tx_event).await {
                        return;
                    }
                }
            }
            Ok(Some(Err(err))) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api(format!("Stream error: {err}"))))
                    .await;
                return;
            }
            Ok(None) => {
                if !buffer.trim().is_empty()
                    && handle_parse_result(parse_trailing(&buffer, state), tx_event).await
                {
                    return;
                }

                let _ = tx_event
                    .send(Ok(ResponseEvent::Completed {
                        response_id: String::new(),
                        token_usage: None,
                    }))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api("Stream timeout".to_string())))
                    .await;
                return;
            }
        }
    }
}

pub(super) fn map_status_error(
    status_error_prefix: &str,
    status: StatusCode,
    body: &str,
) -> CodexErr {
    if status == StatusCode::BAD_REQUEST && body.contains("prompt is too long") {
        return CodexErr::ContextWindowExceeded;
    }

    let redacted_body = redact_error_body(body);
    if status_error_prefix == "Gemini Code Assist"
        && let Some(message) = map_gemini_code_assist_status_error(status, body, &redacted_body)
    {
        return CodexErr::Api(message);
    }

    CodexErr::Api(format!(
        "{status_error_prefix} API error {status}: {redacted_body}"
    ))
}

fn map_gemini_code_assist_status_error(
    status: StatusCode,
    body: &str,
    redacted_body: &str,
) -> Option<String> {
    if status != StatusCode::FORBIDDEN && status != StatusCode::UNAUTHORIZED {
        return None;
    }

    let lower = body.to_ascii_lowercase();
    if lower.contains("vpcsc")
        || (lower.contains("vpc") && lower.contains("service perimeter"))
        || (lower.contains("service") && lower.contains("perimeter"))
    {
        return Some(format!(
            "Gemini Code Assist request was blocked by VPC Service Controls. Use a project outside the restricted perimeter or update VPC-SC policy. Raw error: {status}: {redacted_body}"
        ));
    }

    if lower.contains("org policy") || lower.contains("organization policy") {
        return Some(format!(
            "Gemini Code Assist request was blocked by organization policy. Ask your admin to allow Gemini Code Assist / Cloud Code access for this project. Raw error: {status}: {redacted_body}"
        ));
    }

    if lower.contains("preview")
        || lower.contains("allowlist")
        || lower.contains("not enabled")
        || lower.contains("permission denied")
    {
        return Some(format!(
            "Gemini Code Assist access appears unavailable for this account or project. Confirm preview/allowlist access and required API enablement, then retry. Raw error: {status}: {redacted_body}"
        ));
    }

    None
}

async fn handle_parse_result(
    parse_result: ParseSseEventResult,
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
) -> bool {
    match parse_result {
        ParseSseEventResult::Continue => false,
        ParseSseEventResult::Emit(events) => emit_events(tx_event, events).await,
        ParseSseEventResult::Fatal(err) => {
            let _ = tx_event.send(Err(err)).await;
            true
        }
    }
}

async fn emit_events(
    tx_event: &mpsc::Sender<Result<ResponseEvent>>,
    events: Vec<ResponseEvent>,
) -> bool {
    for event in events {
        let is_completed = matches!(event, ResponseEvent::Completed { .. });
        if tx_event.send(Ok(event)).await.is_err() {
            return true;
        }
        if is_completed {
            return true;
        }
    }
    false
}

fn take_next_sse_event(buffer: &mut String) -> Option<String> {
    let end_pos = buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))?;
    let event_end = if buffer[end_pos..].starts_with("\r\n\r\n") {
        end_pos + 4
    } else {
        end_pos + 2
    };
    let event = buffer[..end_pos].to_string();
    buffer.drain(..event_end);
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extract_sse_data_line_reads_single_data_line() {
        assert_eq!(
            extract_sse_data_line("event: message\ndata: {\"ok\":true}"),
            Some("{\"ok\":true}".to_string())
        );
    }

    #[test]
    fn extract_sse_data_line_joins_multiple_data_lines() {
        assert_eq!(
            extract_sse_data_line("event: message\ndata: {\"items\":[\ndata: 1,2,3]}"),
            Some("{\"items\":[\n1,2,3]}".to_string())
        );
    }

    #[test]
    fn map_status_error_detects_context_window() {
        let err = map_status_error(
            "Gemini",
            StatusCode::BAD_REQUEST,
            "prompt is too long: 100000 tokens",
        );
        assert!(matches!(err, CodexErr::ContextWindowExceeded));
    }

    #[test]
    fn map_status_error_enhances_gemini_code_assist_policy_errors() {
        let err = map_status_error(
            "Gemini Code Assist",
            StatusCode::FORBIDDEN,
            "Request blocked by VPCSC service perimeter.",
        );
        let CodexErr::Api(message) = err else {
            panic!("expected API error");
        };
        assert!(message.contains("VPC Service Controls"), "{message}");
    }

    #[test]
    fn map_status_error_keeps_default_for_other_prefixes() {
        let err = map_status_error("Gemini", StatusCode::FORBIDDEN, "forbidden");
        let CodexErr::Api(message) = err else {
            panic!("expected API error");
        };
        assert_eq!(message, "Gemini API error 403 Forbidden: forbidden");
    }

    #[test]
    fn map_status_error_redacts_sensitive_body_fields() {
        let err = map_status_error(
            "Gemini",
            StatusCode::FORBIDDEN,
            r#"{"access_token":"secret-access-token-123","message":"forbidden"}"#,
        );
        let CodexErr::Api(message) = err else {
            panic!("expected API error");
        };
        assert!(message.contains("[REDACTED_SECRET]"), "{message}");
        assert!(!message.contains("secret-access-token-123"), "{message}");
    }
}
