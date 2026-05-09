//! Chat Completions SSE parser for GitHub Copilot.
//!
//! Translates streamed `chat/completions` chunks into the same `ResponseEvent`
//! shape that the rest of Ata consumes from the Responses API. Only handles
//! the subset Copilot actually emits today: text deltas, tool calls, and
//! `finish_reason`.

use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::trace;

#[derive(Default, Debug, Deserialize)]
struct ChatCompletionsChunk {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Default, Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    delta: ChatDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Debug, Deserialize)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCallDelta>,
}

#[derive(Default, Debug, Deserialize)]
struct ChatToolCallDelta {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChatFunctionDelta>,
}

#[derive(Default, Debug, Deserialize)]
struct ChatFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Default, Debug, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: i64,
    #[serde(default)]
    completion_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

pub fn spawn_chat_completions_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) -> ResponseStream {
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_chat_sse(
        stream_response.bytes,
        tx_event,
        idle_timeout,
        telemetry,
    ));
    ResponseStream { rx_event }
}

async fn process_chat_sse(
    bytes: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    _telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = bytes
        .map_err(|err: TransportError| std::io::Error::other(err.to_string()))
        .eventsource();

    if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
        return;
    }

    let mut response_id: Option<String> = None;
    let mut emitted_message_added = false;
    let mut accumulated_text = String::new();
    let mut tool_calls: BTreeMap<u32, ToolCallAccumulator> = BTreeMap::new();
    let mut token_usage: Option<TokenUsage> = None;
    let mut finish_reason: Option<String> = None;

    loop {
        let next = match timeout(idle_timeout, stream.next()).await {
            Ok(Some(item)) => item,
            Ok(None) => break,
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!(
                        "stream idle for {idle_timeout:?} without an event"
                    ))))
                    .await;
                return;
            }
        };

        let event = match next {
            Ok(event) => event,
            Err(err) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(format!("SSE error: {err}"))))
                    .await;
                return;
            }
        };

        let data = event.data;
        if data.trim() == "[DONE]" {
            break;
        }

        let chunk: ChatCompletionsChunk = match serde_json::from_str(&data) {
            Ok(chunk) => chunk,
            Err(err) => {
                trace!("skipping unparsable chat-completions chunk: {err}");
                continue;
            }
        };

        if response_id.is_none() {
            response_id = chunk.id.clone();
        }

        if let Some(usage) = chunk.usage {
            token_usage = Some(TokenUsage {
                input_tokens: usage.prompt_tokens,
                cached_input_tokens: 0,
                output_tokens: usage.completion_tokens,
                reasoning_output_tokens: 0,
                total_tokens: usage.total_tokens,
            });
        }

        for choice in chunk.choices {
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                if !emitted_message_added {
                    emitted_message_added = true;
                    let item = ResponseItem::Message {
                        id: response_id.clone(),
                        role: "assistant".to_string(),
                        content: Vec::new(),
                        end_turn: None,
                        phase: None,
                    };
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }

                accumulated_text.push_str(&text);
                if tx_event
                    .send(Ok(ResponseEvent::OutputTextDelta(text)))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            for tool_delta in choice.delta.tool_calls {
                let entry = tool_calls.entry(tool_delta.index).or_default();
                if let Some(id) = tool_delta.id
                    && !id.is_empty()
                {
                    entry.id = id;
                }
                if let Some(func) = tool_delta.function {
                    if let Some(name) = func.name
                        && !name.is_empty()
                    {
                        entry.name = name;
                    }
                    if let Some(args) = func.arguments {
                        entry.arguments.push_str(&args);
                    }
                }
            }

            if let Some(reason) = choice.finish_reason {
                finish_reason = Some(reason);
            }
        }
    }

    if emitted_message_added {
        let final_item = ResponseItem::Message {
            id: response_id.clone(),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: accumulated_text.clone(),
            }],
            end_turn: matches!(finish_reason.as_deref(), Some("stop")).then_some(true),
            phase: None,
        };
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(final_item)))
            .await
            .is_err()
        {
            return;
        }
    }

    for (_, call) in tool_calls {
        let item = ResponseItem::FunctionCall {
            id: None,
            name: call.name,
            namespace: None,
            arguments: call.arguments,
            call_id: call.id,
            thought_signature: None,
        };
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return;
        }
    }

    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: response_id.unwrap_or_default(),
            token_usage,
        }))
        .await;
}
