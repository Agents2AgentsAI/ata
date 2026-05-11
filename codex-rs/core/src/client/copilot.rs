//! Streams a turn via GitHub Copilot's Chat Completions or Responses
//! endpoint.
//!
//! Glue layer between ATA's `ModelClientSession` / `Prompt` API and
//! `codex-api`. Two wire paths live here:
//!
//! - [`stream_copilot_chat_completions`] — the historic path. Uses
//!   `ChatCompletionsClient` + `CopilotAdapter` to translate Responses-API
//!   input into Chat Completions messages. Works for Claude, Gemini, Grok,
//!   gpt-4.x, gpt-5-mini, etc.
//! - [`stream_copilot_responses_api`] — added so frontier OpenAI models
//!   that Copilot exposes only via `/responses` (gpt-5.x except
//!   `gpt-5-mini`) actually stream. Without this they returned
//!   `unsupported_api_for_model` from `/chat/completions`.
//!
//! Both paths share the same bearer-token resolution: a fresh Copilot
//! bearer is obtained from `copilot_oauth::get_or_refresh_copilot_token`
//! per request, then a clone of `ModelProviderInfo` is patched with that
//! bearer so the standard codex-api auth chain picks it up.
//!
//! Dispatch lives in [`super::ModelClientSession::stream`]'s
//! `WireApi::CopilotInline` arm; it calls [`requires_responses_api`] on
//! the model slug to pick the path.

use std::sync::Arc;

use codex_api::ChatCompletionsClient;
use codex_api::Compression;
use codex_api::CopilotAdapter;
use codex_api::ProviderAdapter;
use codex_api::ReqwestTransport;
use codex_api::ResponsesClient as ApiResponsesClient;
use codex_api::map_api_error;
use codex_login::default_client::build_reqwest_client;
use codex_model_provider::BearerAuthProvider;
use codex_otel::SessionTelemetry;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_rollout_trace::InferenceTraceAttempt;
use codex_tools::create_tools_json_for_responses_api;

use super::ModelClientSession;
use super::provider_streaming::serialize_input_items;
use crate::client_common::Prompt;
use crate::client_common::ResponseStream;

/// Returns `true` when `model_slug` is reachable only via Copilot's
/// `/responses` endpoint. Currently any `gpt-N` with N≥5 except
/// `gpt-5-mini` (which still ships on Chat Completions). Mirrors
/// opencode's `shouldUseResponsesApi` so the two clients stay in sync.
///
/// Empirically (2026-05): Copilot's `/responses` rejects `claude-*` with
/// `unsupported_api_for_model` at the model level, and the allow-list
/// returned for the integrators we can reach (openai-codex-cli,
/// copilot-cli) does not include any Gemini model on /responses. So
/// Claude and Gemini both stay on `/chat/completions`. Use the standalone
/// Anthropic / Gemini provider if you need native PDF for those models.
pub(crate) fn requires_responses_api(model_slug: &str) -> bool {
    let Some(rest) = model_slug.strip_prefix("gpt-") else {
        return false;
    };
    // Leading version digits (no minor / suffix yet).
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let Ok(major) = digits.parse::<u32>() else {
        return false;
    };
    if major < 5 {
        return false;
    }
    // gpt-5-mini keeps Chat Completions; the rest of gpt-5.x is /responses.
    !model_slug.starts_with("gpt-5-mini")
}

/// Streams a turn via the GitHub Copilot Chat Completions API.
pub(super) async fn stream_copilot_chat_completions(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    session_telemetry: &SessionTelemetry,
) -> Result<ResponseStream> {
    let state = session.client().state();

    // Resolve a fresh Copilot bearer token (refreshing from the long-lived
    // GitHub OAuth token if needed). This calls into copilot_oauth directly
    // because AuthManager lives in codex-login and cannot depend on
    // codex-core's auth submodules.
    let token = crate::auth::copilot_oauth::get_or_refresh_copilot_token(
        &state.codex_home,
        state.cli_auth_credentials_store_mode,
    )
    .await
    .map_err(|err| {
        CodexErr::Api(format!(
            "GitHub Copilot login required (run `ata login` and choose GitHub Copilot): {err}"
        ))
    })?;

    // Patch a clone of the provider info with the Copilot bearer token so
    // the standard codex-api auth chain attaches it to the request.
    let mut patched_info = state.provider.info().clone();
    patched_info.experimental_bearer_token = Some(token.clone());
    let api_provider = patched_info
        .to_api_provider(/*auth_mode*/ None)
        .map_err(|err| CodexErr::Api(err.to_string()))?;
    let api_auth: codex_api::SharedAuthProvider = Arc::new(BearerAuthProvider::new(token));

    // Build request body via the adapter.
    let adapter = CopilotAdapter::new();
    let input = prompt.get_formatted_input();
    let input_values = serialize_input_items(&input)?;
    let tools_array = create_tools_json_for_responses_api(&prompt.tools)
        .map_err(|e| CodexErr::Api(format!("failed to serialize tools: {e}")))?;
    let formatted_tools = adapter
        .format_tools(&tools_array)
        .map_err(|e| CodexErr::Api(e.to_string()))?;
    let body = adapter
        .build_request_body(
            &model_info.slug,
            &prompt.base_instructions.text,
            &input_values,
            &formatted_tools,
            &codex_api::RequestOptions {
                parallel_tool_calls: prompt.parallel_tool_calls,
                ..Default::default()
            },
        )
        .map_err(|e| CodexErr::Api(e.to_string()))?;

    let extra_headers = adapter.extra_headers_for_input(&input_values);

    let transport = ReqwestTransport::new(build_reqwest_client());
    let client = ChatCompletionsClient::new(transport, api_provider, api_auth);
    let stream = client
        .stream(body, extra_headers)
        .await
        .map_err(map_api_error)?;

    let (stream, _last_request_rx) = super::map_response_stream(
        stream,
        session_telemetry.clone(),
        InferenceTraceAttempt::disabled(),
    );
    Ok(stream)
}

/// Streams a turn via the GitHub Copilot Responses API (`/responses`).
///
/// Required for `gpt-N` (N≥5) models that Copilot refuses to serve via
/// `/chat/completions` with `unsupported_api_for_model`. Reuses
/// `ModelClient::build_responses_request` so request shape stays in
/// lockstep with the OpenAI path; only auth + transport are swapped to
/// the Copilot bearer.
#[expect(
    clippy::too_many_arguments,
    reason = "Mirrors the OpenAI Responses streaming arity. Bundling args into a struct would diverge from the existing per-wire-API call sites in client.rs without buying any clarity at the dispatch point."
)]
pub(super) async fn stream_copilot_responses_api(
    session: &ModelClientSession,
    prompt: &Prompt,
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
    service_tier: Option<String>,
    turn_metadata_header: Option<&str>,
    session_telemetry: &SessionTelemetry,
) -> Result<ResponseStream> {
    let state = session.client().state();

    // Same per-request bearer resolution as the Chat Completions path.
    let token = crate::auth::copilot_oauth::get_or_refresh_copilot_token(
        &state.codex_home,
        state.cli_auth_credentials_store_mode,
    )
    .await
    .map_err(|err| {
        CodexErr::Api(format!(
            "GitHub Copilot login required (run `ata login` and choose GitHub Copilot): {err}"
        ))
    })?;

    // Patch a clone of the provider info so codex-api's auth chain picks up
    // the Copilot bearer instead of looking for a CodexAuth.
    let mut patched_info = state.provider.info().clone();
    patched_info.experimental_bearer_token = Some(token.clone());
    let api_provider = patched_info
        .to_api_provider(/*auth_mode*/ None)
        .map_err(|err| CodexErr::Api(err.to_string()))?;
    let api_auth: codex_api::SharedAuthProvider = Arc::new(BearerAuthProvider::new(token));

    // Reuse the canonical Responses request + options builders so the
    // request body matches the OpenAI Responses path exactly. Compression
    // is left off — Copilot's gateway is not known to negotiate it and
    // mirroring opencode keeps the wire output stable.
    let request = session.client().build_responses_request(
        &api_provider,
        prompt,
        model_info,
        effort,
        summary,
        service_tier,
    )?;
    let options = session.build_responses_options(turn_metadata_header, Compression::None);

    let transport = ReqwestTransport::new(build_reqwest_client());
    let client = ApiResponsesClient::new(transport, api_provider, api_auth);
    let stream = client
        .stream_request(request, options)
        .await
        .map_err(map_api_error)?;
    let (stream, _) = super::map_response_stream(
        stream,
        session_telemetry.clone(),
        InferenceTraceAttempt::disabled(),
    );
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::requires_responses_api;

    #[test]
    fn responses_api_required_for_gpt5_family() {
        // Mirrors opencode's `shouldUseResponsesApi`: gpt-N for N>=5 ⇒
        // /responses, except gpt-5-mini which stays on /chat/completions.
        assert!(requires_responses_api("gpt-5.5"));
        assert!(requires_responses_api("gpt-5.4"));
        assert!(requires_responses_api("gpt-5.4-mini"));
        assert!(requires_responses_api("gpt-5.4-nano"));
        assert!(requires_responses_api("gpt-5.3-codex"));
        assert!(requires_responses_api("gpt-5.2"));
        assert!(requires_responses_api("gpt-5.2-codex"));
        assert!(requires_responses_api("gpt-6"));
        assert!(requires_responses_api("gpt-10"));
    }

    #[test]
    fn responses_api_not_required_for_chat_only_models() {
        // gpt-5-mini is the documented Chat Completions hold-out.
        assert!(!requires_responses_api("gpt-5-mini"));
        // Older OpenAI families stay on chat completions.
        assert!(!requires_responses_api("gpt-4.1"));
        assert!(!requires_responses_api("gpt-4o"));
        assert!(!requires_responses_api("gpt-3.5-turbo"));
        // Claude on Copilot stays on chat completions: /responses returns
        // `unsupported_api_for_model` at the model level.
        assert!(!requires_responses_api("claude-opus-4.7"));
        assert!(!requires_responses_api("claude-sonnet-4.6"));
        assert!(!requires_responses_api("claude-haiku-4.5"));
        // Gemini on Copilot stays on chat completions: the reachable
        // integrator allow-lists do not include Gemini on /responses.
        assert!(!requires_responses_api("gemini-3.1-pro-preview"));
        assert!(!requires_responses_api("gemini-3-flash-preview"));
        assert!(!requires_responses_api("gemini-2.5-pro"));
        assert!(!requires_responses_api("grok-code-fast-1"));
        // Bare or malformed slugs short-circuit to false.
        assert!(!requires_responses_api(""));
        assert!(!requires_responses_api("gpt-"));
        assert!(!requires_responses_api("gpt"));
        assert!(!requires_responses_api("not-a-model"));
    }
}
