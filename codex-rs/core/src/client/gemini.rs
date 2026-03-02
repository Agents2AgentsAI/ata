use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

use super::ModelClientSession;
use super::gemini_code_assist::stream_gemini_code_assist;
use super::provider_streaming::ParseSseEventResult;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::extract_sse_data_line;
use super::provider_streaming::filter_out_created;
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
use crate::auth::GeminiAuthSource;
use crate::auth::PROVIDER_GEMINI;
use crate::auth::resolve_gemini_auth_source;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::tools::spec::create_tools_json_for_responses_api;

/// Streams a turn via the Gemini GenerateContent API.
pub(super) async fn stream_gemini_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Result<ResponseStream> {
    let auth_source =
        if session.client.state.provider.name_to_provider_id() == Some(PROVIDER_GEMINI) {
            resolve_gemini_auth_source(
                &session.client.state.codex_home,
                session.client.state.cli_auth_credentials_store_mode,
            )
        } else {
            match session.client.state.provider.api_key_with_auth(
                &session.client.state.codex_home,
                session.client.state.cli_auth_credentials_store_mode,
            )? {
                Some(api_key) => GeminiAuthSource::ApiKey(api_key),
                None => GeminiAuthSource::Missing,
            }
        };

    match auth_source {
        GeminiAuthSource::ApiKey(api_key) => {
            stream_gemini_with_api_key(session, prompt, model_info, effort, summary, api_key).await
        }
        GeminiAuthSource::Oauth(oauth_credential) => {
            stream_gemini_code_assist(
                session,
                prompt,
                model_info,
                effort,
                summary,
                oauth_credential,
            )
            .await
        }
        GeminiAuthSource::Missing => Err(CodexErr::Api(
            "Missing Gemini credentials. Set GOOGLE_API_KEY or run `ata login --provider gemini --with-oauth`."
                .to_string(),
        )),
    }
}

async fn stream_gemini_with_api_key(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
    api_key: String,
) -> Result<ResponseStream> {
    let input = prompt.get_formatted_input();
    let instructions = &prompt.base_instructions.text;
    let tools = create_tools_json_for_responses_api(&prompt.tools)?;

    let adapter = GeminiAdapter::new();
    let input_values = serialize_input_items(&input)?;
    let reasoning_value = build_reasoning_value(model_info, effort, summary);

    let body = adapter
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

    let idle_timeout = session.client.state.provider.stream_idle_timeout();
    let mut stream_state = GeminiStreamState::new();
    stream_state.created_sent = true;

    Ok(spawn_provider_sse_stream(
        request,
        idle_timeout,
        "Gemini",
        stream_state,
        vec![ResponseEvent::Created],
        |event_str, state| {
            let Some(data) = extract_sse_data_line(event_str) else {
                return ParseSseEventResult::Continue;
            };
            parse_gemini_sse_data(&data, state, false)
        },
        |buffer, state| {
            let Some(data) = extract_sse_data_line(buffer) else {
                return ParseSseEventResult::Continue;
            };
            parse_gemini_sse_data(&data, state, true)
        },
    ))
}

fn parse_gemini_sse_data(
    data: &str,
    state: &mut GeminiStreamState,
    ignore_parse_errors: bool,
) -> ParseSseEventResult {
    if data.trim() == "[DONE]" {
        return ParseSseEventResult::Emit(codex_api::sse::gemini::finalize_gemini_stream(
            state, None,
        ));
    }

    match codex_api::sse::gemini::parse_gemini_chunk(data, state) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::ResponseItem;

    #[test]
    fn parse_done_flushes_pending_message_item() {
        let mut state = GeminiStreamState::new();
        let first_chunk = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "hello"}],
                    "role": "model"
                },
                "index": 0
            }]
        }"#;

        let first_result = parse_gemini_sse_data(first_chunk, &mut state, false);
        let ParseSseEventResult::Emit(initial_events) = first_result else {
            panic!("expected parse to emit streaming events");
        };
        assert!(
            initial_events
                .iter()
                .any(|event| matches!(event, ResponseEvent::OutputTextDelta(_)))
        );

        let done_result = parse_gemini_sse_data("[DONE]", &mut state, false);
        let ParseSseEventResult::Emit(events) = done_result else {
            panic!("expected [DONE] to emit completion events");
        };

        assert_eq!(events.len(), 2);
        match &events[0] {
            ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. }) => {
                assert_eq!(
                    content,
                    &vec![ContentItem::OutputText {
                        text: "hello".to_string(),
                    }]
                );
            }
            _ => panic!("expected OutputItemDone(Message) before Completed"),
        }
        assert!(matches!(
            events[1],
            ResponseEvent::Completed {
                token_usage: None,
                ..
            }
        ));
    }
}
