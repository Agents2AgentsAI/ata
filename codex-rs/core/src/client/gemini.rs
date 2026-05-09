//! Streams a turn via the Google Gemini `generateContent` API.
//!
//! Same shape as `client/anthropic.rs`: glue between ATA's
//! `ModelClientSession` / `Prompt` API and the codex-api `GeminiAdapter` +
//! `GeminiStreamState` + `parse_gemini_chunk`.

use std::time::Duration;

use codex_api::GeminiAdapter;
use codex_api::GeminiStreamState;
use codex_api::ProviderAdapter;
use codex_api::parse_gemini_chunk;
use codex_login::default_client::build_reqwest_client;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_tools::ToolSpec;
use codex_tools::create_tools_json_for_responses_api;

use super::ModelClientSession;
use super::provider_streaming::ParseSseEventResult;
use super::provider_streaming::build_reasoning_value;
use super::provider_streaming::extract_sse_data_line;
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
use crate::client_common::Prompt;
use crate::client_common::ResponseStream;

const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const GOOGLE_API_KEY_ENV_VAR: &str = "GOOGLE_API_KEY";
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Streams a turn via the Gemini `generateContent` API.
pub(super) async fn stream_gemini_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Result<ResponseStream> {
    let provider_info = session.client().state().provider.info().clone();

    let api_key = std::env::var(GOOGLE_API_KEY_ENV_VAR)
        .ok()
        .or_else(|| {
            provider_info
                .env_key
                .as_deref()
                .and_then(|key| std::env::var(key).ok())
        })
        .ok_or_else(|| {
            CodexErr::Api(format!(
                "Missing {GOOGLE_API_KEY_ENV_VAR} env var (and no fallback provider key)"
            ))
        })?;

    let adapter = GeminiAdapter::new();
    let input = prompt.get_formatted_input();
    let input_values = serialize_input_items(&input)?;
    let tools_array = create_tools_json_for_responses_api(prompt_tools(prompt))
        .map_err(|e| CodexErr::Api(format!("failed to serialize tools: {e}")))?;
    let formatted_tools = adapter
        .format_tools(&tools_array)
        .map_err(|e| CodexErr::Api(e.to_string()))?;
    let reasoning_value = build_reasoning_value(model_info, effort, summary);
    let body = adapter
        .build_request_body(
            &model_info.slug,
            prompt_base_instructions(prompt),
            &input_values,
            &formatted_tools,
            &codex_api::RequestOptions {
                parallel_tool_calls: prompt_parallel_tool_calls(prompt),
                reasoning: reasoning_value,
                ..Default::default()
            },
        )
        .map_err(|e| CodexErr::Api(e.to_string()))?;

    let base_url = provider_info
        .base_url
        .as_deref()
        .unwrap_or(GEMINI_DEFAULT_BASE_URL);
    let endpoint = adapter.streaming_endpoint(&model_info.slug);
    let url = format!(
        "{}{}?alt=sse&key={}",
        base_url.trim_end_matches('/'),
        endpoint,
        urlencoding::encode(&api_key)
    );

    let client = build_reqwest_client();
    let mut request = client.post(&url).json(&body);

    for (name, value) in adapter.extra_headers_for_input(&input_values).iter() {
        if let Ok(value_str) = value.to_str() {
            request = request.header(name.as_str(), value_str);
        }
    }

    Ok(spawn_provider_sse_stream(
        request,
        DEFAULT_IDLE_TIMEOUT,
        "Gemini",
        GeminiStreamState::default(),
        Vec::new(),
        |event_str, state| {
            let data = match extract_sse_data_line(event_str) {
                Some(data) => data,
                None => return ParseSseEventResult::Continue,
            };
            if data.trim().is_empty() {
                return ParseSseEventResult::Continue;
            }
            match parse_gemini_chunk(&data, state) {
                Ok(events) => ParseSseEventResult::Emit(events),
                Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
            }
        },
        |_buffer, _state| ParseSseEventResult::Continue,
    ))
}

fn prompt_tools(prompt: &Prompt) -> &[ToolSpec] {
    &prompt.tools
}

fn prompt_base_instructions(prompt: &Prompt) -> &str {
    &prompt.base_instructions.text
}

fn prompt_parallel_tool_calls(prompt: &Prompt) -> bool {
    prompt.parallel_tool_calls
}
