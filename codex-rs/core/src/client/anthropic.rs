use codex_api::AnthropicAdapter;
use codex_api::AnthropicStreamState;
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
use crate::client_common::ResponseStream;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::tools::spec::create_tools_json_for_responses_api;

/// Streams a turn via the Anthropic Messages API.
pub(super) async fn stream_anthropic_api(
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
        .ok_or_else(|| CodexErr::Api("Missing ANTHROPIC_API_KEY".to_string()))?;

    let adapter = AnthropicAdapter::new();
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

    for (name, value) in adapter.extra_headers_for_input(&input_values).iter() {
        if let Ok(value_str) = value.to_str() {
            request = request.header(name.as_str(), value_str);
        }
    }

    let idle_timeout = session.client.state.provider.stream_idle_timeout();

    Ok(spawn_provider_sse_stream(
        request,
        idle_timeout,
        "Anthropic",
        AnthropicStreamState::new(),
        Vec::new(),
        |event_str, state| {
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
                return ParseSseEventResult::Continue;
            }

            match codex_api::sse::anthropic::parse_anthropic_event(&event_type, &data, state) {
                Ok(events) => ParseSseEventResult::Emit(events),
                Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
            }
        },
        |_buffer, _state| ParseSseEventResult::Continue,
    ))
}
