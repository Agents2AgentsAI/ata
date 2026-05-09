//! Streams a turn via the Anthropic Messages API.
//!
//! Glue layer between ATA's `ModelClientSession` / `Prompt` API and the
//! `codex-api` adapter + SSE state machine. The adapter does request shaping
//! and response parsing; this module just wires HTTP transport, auth, and
//! the SSE pipe.

use std::time::Duration;

use codex_api::AnthropicAdapter;
use codex_api::AnthropicStreamState;
use codex_api::ProviderAdapter;
use codex_api::parse_anthropic_event;
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
use super::provider_streaming::map_api_err_to_codex_err;
use super::provider_streaming::serialize_input_items;
use super::provider_streaming::spawn_provider_sse_stream;
use crate::client_common::Prompt;
use crate::client_common::ResponseStream;

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
const ANTHROPIC_API_KEY_ENV_VAR: &str = "ANTHROPIC_API_KEY";
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Streams a turn via the Anthropic Messages API.
pub(super) async fn stream_anthropic_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Result<ResponseStream> {
    let provider_info = session.client().state().provider.info().clone();

    // API key resolution: env var first, then provider's configured env_key, then provider api_key.
    let api_key = std::env::var(ANTHROPIC_API_KEY_ENV_VAR)
        .ok()
        .or_else(|| {
            provider_info
                .env_key
                .as_deref()
                .and_then(|key| std::env::var(key).ok())
        })
        .ok_or_else(|| {
            CodexErr::Api(format!(
                "Missing {ANTHROPIC_API_KEY_ENV_VAR} env var (and no fallback provider key)"
            ))
        })?;

    // Build request body via the provider adapter.
    let adapter = AnthropicAdapter::new();
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

    // URL = base + endpoint (e.g. "/messages").
    let base_url = provider_info
        .base_url
        .as_deref()
        .unwrap_or(ANTHROPIC_DEFAULT_BASE_URL);
    let endpoint = adapter.streaming_endpoint(&model_info.slug);
    let url = format!("{}{}", base_url.trim_end_matches('/'), endpoint);

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

    Ok(spawn_provider_sse_stream(
        request,
        DEFAULT_IDLE_TIMEOUT,
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

            match parse_anthropic_event(&event_type, &data, state) {
                Ok(events) => ParseSseEventResult::Emit(events),
                Err(err) => ParseSseEventResult::Fatal(map_api_err_to_codex_err(err)),
            }
        },
        |_buffer, _state| ParseSseEventResult::Continue,
    ))
}

// Field accessors with `pub(super)` visibility so we can stay tidy when the
// Prompt fields stop being `pub(crate)` in the future.
fn prompt_tools(prompt: &Prompt) -> &[ToolSpec] {
    &prompt.tools
}

fn prompt_base_instructions(prompt: &Prompt) -> &str {
    &prompt.base_instructions.text
}

fn prompt_parallel_tool_calls(prompt: &Prompt) -> bool {
    prompt.parallel_tool_calls
}
