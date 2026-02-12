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
                    let err = if status == StatusCode::BAD_REQUEST
                        && body.contains("prompt is too long")
                    {
                        CodexErr::ContextWindowExceeded
                    } else {
                        CodexErr::Api(format!("{status_error_prefix} API error {status}: {body}"))
                    };
                    let _ = tx_event.send(Err(err)).await;
                    return;
                }

                if emit_events(&tx_event, initial_events).await {
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

                                if handle_parse_result(
                                    parse_event(&event_str, &mut state),
                                    &tx_event,
                                )
                                .await
                                {
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
                                && handle_parse_result(
                                    parse_trailing(&buffer, &mut state),
                                    &tx_event,
                                )
                                .await
                            {
                                return;
                            }

                            let _ = tx_event
                                .send(Ok(ResponseEvent::Completed {
                                    response_id: String::new(),
                                    token_usage: None,
                                    can_append: false,
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
            Err(err) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api(format!("Request failed: {err}"))))
                    .await;
            }
        }
    });

    ResponseStream { rx_event }
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
