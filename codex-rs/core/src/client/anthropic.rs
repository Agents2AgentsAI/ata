use codex_api::AnthropicAdapter;
use codex_api::AnthropicStreamState;
use codex_api::ProviderAdapter;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use futures::StreamExt;
use tokio::sync::mpsc;

use super::ModelClientSession;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::serialize_input_items;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;

/// Streams a turn via the Anthropic Messages API.
pub(super) async fn stream_anthropic_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Result<ResponseStream> {
    let api_prompt = ModelClientSession::build_responses_request(prompt)?;

    let api_key = session
        .client
        .state
        .provider
        .api_key_with_auth(
            &session.client.state.codex_home,
            session.client.state.cli_auth_credentials_store_mode,
        )?
        .ok_or_else(|| CodexErr::Api("Missing ANTHROPIC_API_KEY".to_string()))?;

    let adapter = AnthropicAdapter::new();
    let input_values = serialize_input_items(&api_prompt.input)?;
    let reasoning_value = build_reasoning_value(model_info, effort, summary);

    let body = adapter
        .build_request_body(
            &model_info.slug,
            &api_prompt.instructions,
            &input_values,
            &api_prompt.tools,
            &codex_api::RequestOptions {
                parallel_tool_calls: api_prompt.parallel_tool_calls,
                reasoning: reasoning_value,
                ..Default::default()
            },
        )
        .map_err(|e| CodexErr::Api(e.to_string()))?;

    let base_url = session
        .client
        .state
        .provider
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com/v1");
    let endpoint = adapter.streaming_endpoint(&model_info.slug);
    let url = format!("{base_url}{endpoint}");

    let client = build_reqwest_client();
    let mut request = client
        .post(&url)
        .header(
            adapter.auth_header_name(),
            adapter.format_auth_header(&api_key),
        )
        .json(&body);

    for (name, value) in adapter.extra_headers().iter() {
        if let Ok(value_str) = value.to_str() {
            request = request.header(name.as_str(), value_str);
        }
    }

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);

    let idle_timeout = session.client.state.provider.stream_idle_timeout();

    tokio::spawn(async move {
        match request.send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    let _ = tx_event
                        .send(Err(CodexErr::Api(format!(
                            "Anthropic API error {status}: {body}"
                        ))))
                        .await;
                    return;
                }

                let mut state = AnthropicStreamState::new();
                let mut stream = response.bytes_stream();
                let mut buffer = String::new();

                loop {
                    let chunk_result = tokio::time::timeout(idle_timeout, stream.next()).await;

                    match chunk_result {
                        Ok(Some(Ok(chunk))) => {
                            if let Ok(text) = std::str::from_utf8(&chunk) {
                                buffer.push_str(text);
                            }

                            while let Some(end_pos) =
                                buffer.find("\n\n").or_else(|| buffer.find("\r\n\r\n"))
                            {
                                let event_end = if buffer[end_pos..].starts_with("\r\n\r\n") {
                                    end_pos + 4
                                } else {
                                    end_pos + 2
                                };

                                let event_str = buffer[..end_pos].to_string();
                                buffer = buffer[event_end..].to_string();

                                if event_str.trim().is_empty() {
                                    continue;
                                }

                                let mut event_type = String::new();
                                let mut data = String::new();

                                for line in event_str.lines() {
                                    if let Some(stripped) = line.strip_prefix("event: ") {
                                        event_type = stripped.to_string();
                                    } else if let Some(stripped) = line.strip_prefix("event:") {
                                        event_type = stripped.trim().to_string();
                                    } else if let Some(stripped) = line.strip_prefix("data: ") {
                                        data = stripped.to_string();
                                    } else if let Some(stripped) = line.strip_prefix("data:") {
                                        data = stripped.trim().to_string();
                                    }
                                }

                                if event_type.is_empty() || data.is_empty() {
                                    continue;
                                }

                                match codex_api::sse::anthropic::parse_anthropic_event(
                                    &event_type,
                                    &data,
                                    &mut state,
                                ) {
                                    Ok(evts) => {
                                        for evt in evts {
                                            let is_completed =
                                                matches!(evt, ResponseEvent::Completed { .. });
                                            if tx_event.send(Ok(evt)).await.is_err() {
                                                return;
                                            }
                                            if is_completed {
                                                return;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ =
                                            tx_event.send(Err(CodexErr::Api(e.to_string()))).await;
                                        return;
                                    }
                                }
                            }
                        }
                        Ok(Some(Err(e))) => {
                            let _ = tx_event
                                .send(Err(CodexErr::Api(format!("Stream error: {e}"))))
                                .await;
                            return;
                        }
                        Ok(None) => {
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
            Err(e) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api(format!("Request failed: {e}"))))
                    .await;
            }
        }
    });

    Ok(ResponseStream { rx_event })
}
