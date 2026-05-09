//! Streams a turn via GitHub Copilot's Chat Completions endpoint.
//!
//! Glue layer between ATA's `ModelClientSession` / `Prompt` API and
//! `codex-api`'s `ChatCompletionsClient` + `CopilotAdapter`. The adapter
//! handles request shaping (Responses-API input -> Chat Completions
//! messages, tool spec translation, VS Code impersonation headers); this
//! module wires HTTP transport, OAuth bearer-token resolution, and the
//! mapping back into `ResponseStream`.

use std::sync::Arc;

use codex_api::ChatCompletionsClient;
use codex_api::CopilotAdapter;
use codex_api::ProviderAdapter;
use codex_api::ReqwestTransport;
use codex_api::map_api_error;
use codex_login::default_client::build_reqwest_client;
use codex_model_provider::BearerAuthProvider;
use codex_otel::SessionTelemetry;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::openai_models::ModelInfo;
use codex_rollout_trace::InferenceTraceAttempt;
use codex_tools::create_tools_json_for_responses_api;

use super::ModelClientSession;
use super::provider_streaming::serialize_input_items;
use crate::client_common::Prompt;
use crate::client_common::ResponseStream;

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
