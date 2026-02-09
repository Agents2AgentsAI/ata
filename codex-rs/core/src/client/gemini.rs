use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::debug;

use super::ModelClientSession;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::serialize_input_items;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;

/// Streams a turn via the Gemini GenerateContent API.
pub(super) async fn stream_gemini_api(
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
        .ok_or_else(|| CodexErr::Api("Missing GOOGLE_API_KEY".to_string()))?;

    let adapter = GeminiAdapter::new();
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
        .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
    let endpoint = adapter.streaming_endpoint(&model_info.slug);
    let url = format!("{base_url}{endpoint}?alt=sse");

    let client = build_reqwest_client();
    let request = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header(
            adapter.auth_header_name(),
            adapter.format_auth_header(&api_key),
        )
        .json(&body);

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
                            "Gemini API error {status}: {body}"
                        ))))
                        .await;
                    return;
                }

                if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
                    return;
                }

                let mut state = GeminiStreamState::new();
                state.created_sent = true;

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

                                let data = if let Some(data_line) =
                                    event_str.lines().find(|line| line.starts_with("data: "))
                                {
                                    &data_line[6..]
                                } else if event_str.starts_with("data:") {
                                    event_str[5..].trim()
                                } else {
                                    continue;
                                };

                                if data.trim() == "[DONE]" {
                                    let _ = tx_event
                                        .send(Ok(ResponseEvent::Completed {
                                            response_id: String::new(),
                                            token_usage: None,
                                        }))
                                        .await;
                                    return;
                                }

                                match codex_api::sse::gemini::parse_gemini_chunk(data, &mut state) {
                                    Ok(evts) => {
                                        for evt in evts {
                                            if matches!(evt, ResponseEvent::Created) {
                                                continue;
                                            }
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
                                        debug!("Gemini parse error (continuing): {e}");
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
                            if !buffer.trim().is_empty()
                                && let Some(data_line) =
                                    buffer.lines().find(|line| line.starts_with("data: "))
                            {
                                let data = &data_line[6..];
                                if data.trim() != "[DONE]"
                                    && let Ok(evts) =
                                        codex_api::sse::gemini::parse_gemini_chunk(data, &mut state)
                                {
                                    for evt in evts {
                                        if matches!(evt, ResponseEvent::Created) {
                                            continue;
                                        }
                                        let _ = tx_event.send(Ok(evt)).await;
                                    }
                                }
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
            Err(e) => {
                let _ = tx_event
                    .send(Err(CodexErr::Api(format!("Request failed: {e}"))))
                    .await;
            }
        }
    });

    Ok(ResponseStream { rx_event })
}
