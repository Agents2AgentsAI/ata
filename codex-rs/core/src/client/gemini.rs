use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;

use super::ModelClientSession;
use super::provider_streaming::ParseSseEventResult;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
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
    let input = prompt.get_formatted_input();
    let instructions = &prompt.base_instructions.text;
    let tools = create_tools_json_for_responses_api(&prompt.tools)?;

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
            let data = if let Some(data_line) = event_str
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
            {
                data_line
            } else if let Some(data_line) = event_str.strip_prefix("data:") {
                data_line.trim()
            } else {
                return ParseSseEventResult::Continue;
            };

            if data.trim() == "[DONE]" {
                return ParseSseEventResult::Emit(vec![ResponseEvent::Completed {
                    response_id: String::new(),
                    token_usage: None,
                }]);
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
                Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
            }
        },
        |buffer, state| {
            if let Some(data_line) = buffer.lines().find(|line| line.starts_with("data: ")) {
                let data = &data_line[6..];
                if data.trim() == "[DONE]" {
                    return ParseSseEventResult::Continue;
                }

                if let Ok(events) = codex_api::sse::gemini::parse_gemini_chunk(data, state) {
                    let events = filter_out_created(events);
                    if !events.is_empty() {
                        return ParseSseEventResult::Emit(events);
                    }
                }
            }

            ParseSseEventResult::Continue
        },
    ))
}

fn filter_out_created(events: Vec<ResponseEvent>) -> Vec<ResponseEvent> {
    events
        .into_iter()
        .filter(|event| !matches!(event, ResponseEvent::Created))
        .collect()
}
