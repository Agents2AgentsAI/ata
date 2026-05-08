# Fork-vs-Upstream Analysis: Multi-Provider Model Support

This document captures the divergence between the ATA fork (`main`) and upstream Codex (`rust-v0.129.0`) in multi-provider model support architecture and implementation.

## Executive Summary

**ATA Fork Approach:** Provider logic is distributed across `core/src/` (model_provider_info.rs, models_manager/*, client/*) and specialized crates (`codex-api/`, `lmstudio/`, `ollama/`). Each provider (Anthropic, Gemini, OpenAI-compatible, local OSS) has distinct request/response adapters and configuration.

**Upstream Approach:** Provider infrastructure is factored into dedicated crates (`model-provider-info/`, `models-manager/`) with cleaner separation of concerns. Upstream has standardized on OpenAI Responses API as the primary wire protocol, with AWS SigV4 auth support and fewer provider variants exposed.

**Key Difference:** ATA supports Anthropic Messages API and Gemini GenerateContent API as first-class wire protocols via `WireApi` enum. Upstream only standardizes on Responses API and pushes provider-specific concerns (AWS) into typed configuration structs (`ModelProviderAwsAuthInfo`).

---

## Feature Analysis

### 1. Model Provider Registry & Configuration

#### Name
Model Provider Info (Provider Registry)

#### Description
Central registry of supported model providers with base URLs, auth environment variables, API keys, wire protocol selection, retry/timeout configuration, and HTTP headers.

#### Implementation Summary (Local)
- **File:** `/Users/huytho_ho/acli/ata/codex-rs/core/src/model_provider_info.rs` (444 lines)
- **Key types:**
  - `WireApi` enum: `Responses`, `AnthropicMessages`, `GeminiGenerate`
  - `ModelProviderInfo` struct: name, base_url, env_key, env_key_instructions, experimental_bearer_token, wire_api, query_params, http_headers, env_http_headers, request_max_retries, stream_max_retries, stream_idle_timeout_ms, requires_openai_auth, supports_websockets
- **Built-in providers:** OpenAI, Anthropic, Google Gemini, Ollama (OSS), LMStudio (OSS)
- **Methods:** `api_key()`, `api_key_with_auth()`, `to_api_provider()`, `name_to_provider_id()` for multi-provider auth storage lookup, `create_openai_provider()`, `create_anthropic_provider()`, `create_gemini_provider()`, `create_oss_provider()`
- **Multi-provider auth:** Maps provider names to provider IDs (PROVIDER_OPENAI, PROVIDER_ANTHROPIC, PROVIDER_GEMINI) for credential lookups in `~/.ata/auth.json`

#### Implementation Summary (Upstream)
- **File:** `codex-rs/model-provider-info/src/lib.rs` (separate crate in upstream workspace)
- **Key types:**
  - `WireApi` enum: Only `Responses` variant (upstream standardizes on OpenAI Responses API)
  - `ModelProviderInfo` struct: Similar fields, plus `aws: Option<ModelProviderAwsAuthInfo>` and `auth: Option<ModelProviderAuthInfo>` for command-backed auth
  - `ModelProviderAwsAuthInfo`: profile, region for AWS SigV4 support
  - `ModelProviderAuthInfo`: command, required_env, cache_duration for command-based auth tokens
- **Built-in providers:** OpenAI, Amazon Bedrock (AWS-backed)
- **Validation:** `validate()` method ensuring auth mode compatibility
- **Note:** Upstream upstream removed provider-specific message format adapters from this layer (Anthropic, Gemini logic is elsewhere)

#### Status vs Upstream
**LOCAL-ONLY features:**
- Multi-protocol support (Anthropic Messages API, Gemini GenerateContent API) as first-class `WireApi` variants
- Built-in Anthropic provider (`create_anthropic_provider()`)
- Built-in Gemini provider (`create_gemini_provider()`)
- Built-in OSS providers (Ollama, LMStudio) with port detection and environment variable overrides (CODEX_OSS_PORT, CODEX_OSS_BASE_URL)
- Multi-provider auth storage lookup via `name_to_provider_id()` and `api_key_with_auth()`

**ALSO-IN-UPSTREAM:**
- Core provider configuration (name, base_url, env_key, headers, retries, timeouts)
- Bearer token auth
- HTTP header injection (static and environment-variable-backed)
- Query parameter injection

**UPSTREAM-ONLY features:**
- AWS SigV4 auth support (`aws` field and validation)
- Command-based bearer token auth (`auth` field for token refreshers)
- Explicit websocket connection timeout configuration (`websocket_connect_timeout_ms`)
- Validation method to enforce auth mode compatibility
- Separation into dedicated `model-provider-info` crate

#### Merge Plan
**Recommendation:** Adopt upstream's `model-provider-info` crate structure and add `WireApi` multi-protocol support back in. Keep upstream's AWS and command-auth infrastructure. ATA's provider-specific adapters (Anthropic, Gemini) should live in `codex-api/src/providers/` and `codex-api/src/sse/`, not in the configuration layer. This requires:
1. Extend upstream's `WireApi` enum to include `AnthropicMessages` and `GeminiGenerate`
2. Move ATA's `ModelProviderInfo` enhancements (Anthropic, Gemini provider factory methods) into the upstream crate
3. Keep provider adapters in `codex-api` for request/response translation
4. Merge provider registry initialization (OpenAI, Anthropic, Gemini, OSS defaults) into upstream's `built_in_model_providers()` function

---

### 2. Multi-Provider Authentication Storage

#### Name
Multi-Provider Credentials Manager

#### Description
Centralized storage and retrieval of API keys and OAuth credentials for multiple providers (OpenAI, Anthropic, Gemini) in `~/.ata/auth.json`, with fallback to environment variables.

#### Implementation Summary (Local)
- **File:** `/Users/huytho_ho/acli/ata/codex-rs/core/src/auth/providers.rs`
- **Key exports:**
  - `PROVIDER_OPENAI`, `PROVIDER_ANTHROPIC`, `PROVIDER_GEMINI` constants
  - `get_provider_api_key(codex_home, provider_id, store_mode)` function
  - AuthDotJson struct with `providers: HashMap<String, ProviderCredentials>` field
  - Methods: `get_provider_api_key()`, `set_provider_api_key()`, `get_provider_oauth_credential()`, `set_provider_oauth_credential()`, `clear_provider_oauth_credential()`, `remove_provider()`
- **OAuth support:** Stores OAuth tokens for Gemini (via GeminiAuthSource)
- **Fallback strategy:** Checks multi-provider storage first, then environment variable
- **Tests:** Extensive tests for migration, mixing API keys and OAuth, clearing credentials

#### Implementation Summary (Upstream)
- **Status:** Upstream likely centralizes auth in `codex-login` or `codex-protocol` crates (not examined in detail due to scope). No equivalent multi-provider registry observed in `model-provider-info` itself.

#### Status vs Upstream
**LOCAL-ONLY feature:** Multi-provider API key and OAuth credential storage in a single auth.json file, with per-provider credential lookups. Allows users to authenticate once per provider and reuse across multiple sessions.

#### Merge Plan
**Recommendation:** Keep ATA's multi-provider auth storage as-is. Upstream should consider adopting this pattern if it doesn't already have equivalent centralized auth storage. Ensure compatibility by maintaining the `get_provider_api_key()` function signature and testing fallback behavior (multi-provider storage → environment variable).

---

### 3. Provider-Specific Request/Response Adapters

#### Name
Provider Adapter Trait & Implementations

#### Description
Abstraction layer for building provider-specific API requests and parsing SSE responses, with implementations for OpenAI Responses API, Anthropic Messages API, and Google Gemini GenerateContent API.

#### Implementation Summary (Local)
- **File:** `/Users/huytho_ho/acli/ata/codex-rs/codex-api/src/provider_adapter.rs` (87 lines)
- **Core trait:** `ProviderAdapter` with methods:
  - `provider_id()` → provider identifier
  - `format_tools()` → convert tools to provider format
  - `build_request_body()` → build streaming request
  - `streaming_endpoint()` → API endpoint path
  - `extra_headers()` → provider-specific headers
  - `extra_headers_for_input()` → conditional headers based on request shape
  - `auth_header_name()` → header name for auth (default "Authorization")
  - `format_auth_header()` → format API key (default Bearer token)
- **Implementations:**
  - `AnthropicAdapter` (`codex-api/src/providers/anthropic.rs`)
  - `GeminiAdapter` (`codex-api/src/providers/gemini.rs`)
  - `OpenAiAdapter` (`codex-api/src/providers/openai.rs`)
- **Factory:** `ProviderFactory::create_adapter(wire_api)` dispatches to correct adapter

#### Implementation Summary (Upstream)
- Upstream not examined for this layer (assumed different structure, possibly `codex-api` crate exists but with different organization)

#### Status vs Upstream
**LOCAL-ONLY features:**
- Unified `ProviderAdapter` trait as abstraction for multi-protocol support
- `GeminiAdapter` implementation for GenerateContent API
- `AnthropicAdapter` implementation for Messages API
- Factory pattern for adapter creation based on `WireApi` enum
- Conditional header injection via `extra_headers_for_input()`
- Customizable auth header name and format

#### Merge Plan
**Recommendation:** This is a core ATA innovation that upstream should adopt (or already has adopted in different form). Keep as-is and ensure upstream integration points to `ProviderFactory` for adapter selection. If upstream has its own provider adapter pattern, reconcile by ensuring Anthropic and Gemini adapters are available via the factory. Flag for integration: the factory depends on `WireApi` enum from `model-provider-info`, which must be extended to support `AnthropicMessages` and `GeminiGenerate`.

---

### 4. Provider-Specific Streaming Response Parsers (SSE Parsing)

#### Name
Provider SSE Event Parsers

#### Description
Protocol-specific parsers for streaming SSE events from Anthropic Messages API, Gemini GenerateContent API, and OpenAI Responses API, with support for tool calls, reasoning, delta text, and error handling.

#### Implementation Summary (Local)
- **Files:**
  - `codex-api/src/sse/anthropic.rs` — Anthropic SSE state and event parsing
  - `codex-api/src/sse/gemini.rs` — Gemini SSE state and event parsing (includes chat history persistence after resume)
  - `codex-api/src/sse/responses.rs` — OpenAI Responses API SSE state and event parsing
  - `codex-api/src/sse/mod.rs` — Common SSE types (`AnthropicStreamState`, `GeminiStreamState`, etc.)
- **Key commit:** `5143789dba` (Mar 1, 2026) "gemini: fix chat history after resuming" — Added `provider_completion_message_persistence.rs` for recovering partial Gemini responses across session resumption
- **Provider streaming utilities:**
  - `core/src/client/provider_streaming.rs` (408 lines) — Common parsing, reasoning value building, SSE extraction, error mapping
  - Helper functions: `build_reasoning_value()`, `extract_sse_data_line()`, `filter_out_created()`, `spawn_provider_sse_stream()`, `serialize_input_items()`
- **Provider-specific client methods:**
  - `core/src/client/anthropic.rs` — `stream_anthropic_api()`
  - `core/src/client/gemini.rs` — `stream_gemini_api()` (with OAuth and API key paths), `stream_gemini_code_assist()` for OAuth credential flow
  - `core/src/client/gemini_code_assist.rs` — OAuth flow handling for Gemini

#### Implementation Summary (Upstream)
- Assumed to exist in `codex-api` or equivalent, but not examined in detail

#### Status vs Upstream
**LOCAL-ONLY features:**
- Full Anthropic Messages API streaming support with structured SSE parsing
- Full Gemini GenerateContent API streaming support with OAuth credential handling
- Chat history persistence for Gemini across session resumption (`provider_completion_message_persistence.rs`)
- Reasoning value formatting per provider (Anthropic, Gemini, OpenAI each handle reasoning differently)
- Specialized code assist flow for Gemini OAuth authentication

#### Merge Plan
**Recommendation:** These are ATA fork-specific provider integrations. Upstream should adopt Anthropic and Gemini adapters if not already present. Test for compatibility by ensuring:
1. SSE parsers match the protocol versions in `model-provider-info` (wire_api: AnthropicMessages, GeminiGenerate)
2. Reasoning effort and summary configurations are properly passed through from models_manager
3. Session resumption and chat history recovery work for all providers (not just Gemini)
4. Error handling maps provider-specific errors (e.g., rate limits, context window exceeded) to `CodexErr` variants

---

### 5. Model Info & Cards (Third-Party Models Metadata)

#### Name
Model Catalog & Cards

#### Description
Metadata registry for supported models (Claude, Gemini, GPT, etc.) with support information (reasoning levels, tool support, input modalities, truncation policies), model cards for display in UI, and picker visibility settings.

#### Implementation Summary (Local)
- **File:** `/Users/huytho_ho/acli/ata/codex-rs/core/third_party_models.json` (6.1K, 218 lines added in commit `a30c082b17`)
- **Structure:** Array of model objects with fields: slug, display_name, description, default_reasoning_level, supported_reasoning_levels, shell_type, visibility, supported_in_api, priority, upgrade, base_instructions, supports_reasoning_summaries, support_verbosity, default_verbosity, apply_patch_tool_type, truncation_policy, supports_parallel_tool_calls, context_window, experimental_supported_tools, input_modalities, prefer_websockets
- **Models included:** Claude (Sonnet 4.6, Opus 4.6), Gemini (Flash 2.0, Pro), GPT (4o, 4o mini), etc.
- **Related code:**
  - `core/src/models_manager/model_info.rs` — Model metadata struct definitions
  - `core/src/models_manager/model_presets.rs` — Preset configurations for UI picker
  - `core/tests/suite/list_models.rs` — Test suite for model listing with card generation
  - Commit `a30c082b17`: "models: create model cards and fix models picker" — Integrated third_party_models.json into model manager

#### Implementation Summary (Upstream)
- **File:** `codex-rs/models-manager/models.json` (upstream equivalent)
- **Structure:** Similar JSON format with model metadata
- **Methods:** `bundled_models_response()` loads bundled catalog; `ModelsManager` trait handles remote model fetching and catalog merging
- **Presets:** `ModelPreset` struct and filtering logic in `ModelsManager::build_available_models()`

#### Status vs Upstream
**BOTH have implementations:**
- Bundled model catalog (JSON file shipped with binary)
- Model metadata structures (slug, display_name, etc.)
- Model presets and filtering

**DIFFERENCES:**
- ATA includes detailed reasoning levels and input modalities per model in third_party_models.json
- Upstream may separate bundled models from remote models differently
- ATA's model picker visibility filtering (show_in_picker field) is integrated into TUI layer

**LOCAL-ONLY:**
- `third_party_models.json` with Anthropic and Gemini model cards
- Integration test for card generation in `list_models` suite

#### Merge Plan
**Recommendation:** Merge ATA's model catalog into upstream's models.json, ensuring:
1. Anthropic and Gemini models are included in the bundled catalog
2. Model priority and visibility settings align with upstream's picker logic
3. Reasoning levels, input modalities, and truncation policies are preserved
4. Test suite for model card generation is adapted to upstream's test structure

---

### 6. Model Manager (Catalog Coordination & Caching)

#### Name
Models Manager (Catalog & Cache Coordination)

#### Description
Coordinates model discovery, caching, and filtering. Fetches remote model catalogs, merges with built-in defaults, caches on disk, handles refresh strategies, and builds picker-ready presets.

#### Implementation Summary (Local)
- **Files:**
  - `core/src/models_manager/mod.rs` — Module exports and client version constants (OPENAI_MODELS_CLIENT_VERSION = "0.105.0")
  - `core/src/models_manager/manager.rs` — Manager struct with list_models(), raw_model_catalog(), refresh_if_new_etag(), get_default_model(), get_model_info()
  - `core/src/models_manager/cache.rs` — Disk caching with TTL
  - `core/src/models_manager/model_info.rs` — ModelInfo struct definitions
  - `core/src/models_manager/model_presets.rs` — ModelPreset and preset filtering
  - `core/src/models_manager/collaboration_mode_presets.rs` — Collaboration mode presets

#### Implementation Summary (Upstream)
- **Crate:** `codex-rs/models-manager/` (dedicated crate in workspace)
- **Files:**
  - `src/lib.rs` — `bundled_models_response()` and version helpers
  - `src/manager.rs` — Trait `ModelsManager` and implementations (OpenAiModelsManager, StaticModelsManager)
  - `src/cache.rs` — Disk caching
  - `src/model_info.rs` — ModelInfo definitions
  - `src/model_presets.rs` — ModelPreset and filtering
  - `src/collaboration_mode_presets.rs` — Collaboration presets
  - `src/config.rs` — ModelsManagerConfig
- **Key trait:** `ModelsManager` with async methods for refresh strategies (Online, Offline, OnlineIfUncached)
- **Endpoint abstraction:** `ModelsEndpointClient` trait for provider-specific fetching (owns auth, handles per-provider model lists)

#### Status vs Upstream
**BOTH implement similar structures:**
- Model caching with TTL
- Preset filtering (by auth mode, visibility)
- Default model selection
- Collaboration mode presets

**UPSTREAM-ONLY:**
- Dedicated `models-manager` crate (separates concerns)
- `ModelsEndpointClient` trait for pluggable remote model fetching (supports provider-specific endpoints)
- `RefreshStrategy` enum (Online, Offline, OnlineIfUncached)
- Structured `ModelsManagerConfig` for configuration
- Async trait-based design for non-blocking operations
- ETag-based cache invalidation

**LOCAL (ATA) concern:**
- Model manager is part of `core` crate, not separated
- May lack pluggable endpoint abstraction

#### Merge Plan
**Recommendation:** Adopt upstream's `models-manager` crate and `ModelsEndpointClient` pattern. ATA's provider-specific model fetching logic (if any) should implement the `ModelsEndpointClient` trait. Ensure:
1. Multi-provider endpoint clients are created for Anthropic, Gemini, etc., if needed
2. Refresh strategies and cache invalidation work correctly for all providers
3. Model picker filtering respects provider-specific model lists
4. Backward compatibility with ATA's model preset configurations

---

### 7. LMStudio & Ollama (OSS Provider Integration)

#### Name
Local OSS Model Providers (LMStudio, Ollama)

#### Description
Crates for discovering and connecting to local open-source models via LMStudio and Ollama, with OpenAI-compatible API endpoint probing and connection management.

#### Implementation Summary (Local)
- **Crates:**
  - `codex-rs/lmstudio/` — LMStudio client for local model discovery and API calls
  - `codex-rs/ollama/` — Ollama client for local model discovery and API calls
- **LMStudio files:** `Cargo.toml` (lists dependencies)
- **Ollama files:**
  - `src/url.rs` — OpenAI-compatible base URL detection via `is_openai_compatible_base_url()`
  - `src/client.rs` — OllamaClient with `probe_server()`, supports both Ollama native API and OpenAI-compatible endpoints, tests for both
- **Integration:** `model_provider_info.rs` defines `LMSTUDIO_OSS_PROVIDER_ID` and `OLLAMA_OSS_PROVIDER_ID` constants, `create_oss_provider()` factory for building provider configs with configurable ports (CODEX_OSS_PORT, CODEX_OSS_BASE_URL environment variables)
- **Default ports:** Ollama port 11434, LMStudio port 1234

#### Implementation Summary (Upstream)
- **Crates:** `codex-rs/lmstudio/` and `codex-rs/ollama/` exist in upstream (members in workspace Cargo.toml)
- **Status:** Not examined in detail, assumed similar structure

#### Status vs Upstream
**BOTH have:** LMStudio and Ollama crates

**LOCAL enhancements:**
- OpenAI-compatible API endpoint detection for Ollama (flexibility to use Ollama's `/v1` endpoint)
- Environment variable configuration for port and base URL overrides
- Probing tests verifying both Ollama-native and OpenAI-compatible endpoints

#### Merge Plan
**Recommendation:** Verify upstream's LMStudio and Ollama implementations include OpenAI-compatible endpoint support. If not, integrate ATA's enhancements. Ensure environment variable configuration (CODEX_OSS_PORT, CODEX_OSS_BASE_URL) is preserved for user flexibility.

---

### 8. Model Picker & UI Integration

#### Name
Model Picker UI & Visibility Filtering

#### Description
UI components for selecting models, filtering hidden models, and displaying model cards with reasoning support indicators in the TUI chat widget and bottom pane.

#### Implementation Summary (Local)
- **Files:**
  - `tui/src/chatwidget.rs` — Model selection popup with hidden model filtering (test: `model_picker_hides_show_in_picker_false_models_from_cache`)
  - `tui/src/bottom_pane/list_selection_view.rs` — Model picker list rendering
  - `tui/src/app.rs` — Model preset visibility checks (`show_in_picker` field), model migration logic
  - `tui/src/model_migration.rs` — Prompts for deprecated/migrated models
- **Commit `a30c082b17`:** "models: create model cards and fix models picker" — Integrated model card generation and picker visibility filtering
- **Commit `f6cc58c45f`:** "openai api: fix model reasoning switching" — Fixed reasoning mode switching in picker

#### Implementation Summary (Upstream)
- Assumed to exist in `tui/` crate, structure not examined

#### Status vs Upstream
**LOCAL features:**
- Model card generation from metadata
- Visibility filtering (show_in_picker) in picker UI
- Model migration prompts for deprecated models
- Reasoning effort selection in model picker
- Filtering per auth mode

#### Merge Plan
**Recommendation:** Ensure model picker UI integrates with upstream's `ModelPreset` filtering logic. Test that:
1. Visibility filtering respects `show_in_picker` field
2. Model cards are generated from merged catalog (built-in + remote)
3. Reasoning effort selection works for Anthropic and Gemini models
4. Model migration prompts guide users away from deprecated models

---

### 9. Provider-Specific Configuration & Transport Capabilities

#### Name
Provider Transport Capabilities & WebSocket Support

#### Description
Configuration flags for provider-specific transport options (WebSocket support, streaming capabilities) and transport-level capabilities negotiation.

#### Implementation Summary (Local)
- **File:** `core/src/provider_transport_capabilities.rs`
- **Key fields in ModelProviderInfo:**
  - `supports_websockets: bool` — Whether provider supports Responses API WebSocket transport
  - `require_openai_auth: bool` — Whether provider requires ChatGPT/OpenAI login (vs API key in env var)
- **Logic:** Restricts WebSocket transport to OpenAI provider for now, with comments noting OpenAI-compatible providers need testing

#### Implementation Summary (Upstream)
- **Upstream has:** `websocket_connect_timeout_ms` field for WebSocket timeouts
- **Upstream validation:** Ensures AWS providers don't enable WebSocket support (TODO for AWS SigV4 WebSocket signing)

#### Status vs Upstream
**BOTH implement:** WebSocket support flags and transport capability checks

**UPSTREAM enhancements:**
- Explicit websocket_connect_timeout_ms configuration
- Validation preventing incompatible auth/transport combinations (e.g., AWS + WebSocket)

**LOCAL concern:**
- WebSocket support currently hardcoded to OpenAI provider only

#### Merge Plan
**Recommendation:** Adopt upstream's websocket_connect_timeout_ms field. Extend WebSocket support to other providers once tested (Anthropic, Gemini). Ensure validation logic prevents unsupported combinations.

---

## Cross-Cutting Concerns

### OpenAI-Compatible API Support
**Status:** ATA supports OpenAI-compatible base URLs for local providers (Ollama, LMStudio) via configurable base_url in ModelProviderInfo and `is_openai_compatible_base_url()` detection in Ollama client.

**Upstream:** Upstream likely supports OpenAI-compatible via base URL configuration in model-provider-info.

**Merge consideration:** Ensure OpenAI-compatible providers (e.g., local instances, vLLM, text-generation-webui) can be configured via `base_url` in provider config.

### Provider Auth Modes
**Local:** `AuthMode` enum (Chatgpt, Ata, etc.) and per-provider auth resolution (Gemini OAuth, OpenAI bearer token, Anthropic env var).

**Upstream:** Similar auth mode handling with additional modes (ChatgptAuthTokens, AgentIdentity).

**Merge consideration:** Ensure all auth modes are supported and auth resolution logic properly dispatches to provider-specific handlers.

### Reasoning Effort & Summaries
**Local:** Reasoning configuration passed through provider adapters via `RequestOptions::reasoning` field. Per-provider handling (Anthropic, Gemini have custom reasoning parsing).

**Upstream:** Likely centralized reasoning handling in models_manager or client layer.

**Merge consideration:** Ensure reasoning effort and summary settings are properly translated to each provider's API format.

---

## Upstream Innovations Not in Local Fork

1. **AWS SigV4 Authentication** — ModelProviderAwsAuthInfo struct for Amazon Bedrock provider
2. **Command-Based Bearer Token Auth** — ModelProviderAuthInfo for token refreshers (e.g., gcloud auth application-default print-access-token)
3. **Dedicated model-provider-info Crate** — Clean separation of provider configuration concerns
4. **ModelsEndpointClient Trait** — Pluggable remote model fetching per provider
5. **RefreshStrategy Enum** — Explicit cache/network refresh strategies (Online, Offline, OnlineIfUncached)
6. **ETag-Based Cache Invalidation** — Version-aware cache refresh

---

## Recommendations for Merge

### Priority 1: Adopt Upstream Crate Structure
- Move `model_provider_info` logic to dedicated crate (or adopt upstream's crate)
- Move `models_manager` to dedicated crate (or adopt upstream's crate)
- Reduces circular dependencies and improves modularity

### Priority 2: Extend Upstream's WireApi Enum
- Add `AnthropicMessages` and `GeminiGenerate` variants
- Update provider factory to dispatch to Anthropic and Gemini adapters
- Ensures upstream can support ATA's multi-provider architecture

### Priority 3: Integrate Multi-Provider Auth Storage
- Adopt ATA's `get_provider_api_key()` and credential lookup logic
- Ensures credentials for Anthropic, Gemini, etc. are stored and retrieved consistently

### Priority 4: Merge Model Catalogs
- Add Anthropic and Gemini model cards to bundled models.json
- Preserve ATA's reasoning levels and input modality metadata
- Ensure model picker visibility and presets work across all providers

### Priority 5: Verify Provider Adapter Patterns
- Ensure upstream's SSE parsing and request building handles Anthropic and Gemini
- Test Gemini chat history recovery across session resumption
- Add tests for OAuth flows (Gemini)

### Priority 6: Adopt Upstream's AWS & Command Auth
- Implement support for Amazon Bedrock provider
- Add command-based token refresher support for any provider

---

## Files to Review & Integrate

### Local Fork (ATA)
- `/Users/huytho_ho/acli/ata/codex-rs/core/src/model_provider_info.rs` (444 lines)
- `/Users/huytho_ho/acli/ata/codex-rs/core/src/model_provider_info_tests.rs`
- `/Users/huytho_ho/acli/ata/codex-rs/core/src/models_manager/` (manager.rs, cache.rs, model_info.rs, model_presets.rs, collaboration_mode_presets.rs)
- `/Users/huytho_ho/acli/ata/codex-rs/core/src/auth/providers.rs` (multi-provider auth storage)
- `/Users/huytho_ho/acli/ata/codex-rs/codex-api/src/provider_adapter.rs` (87 lines)
- `/Users/huytho_ho/acli/ata/codex-rs/codex-api/src/provider_factory.rs`
- `/Users/huytho_ho/acli/ata/codex-rs/codex-api/src/providers/` (anthropic.rs, gemini.rs, openai.rs)
- `/Users/huytho_ho/acli/ata/codex-rs/codex-api/src/sse/` (anthropic.rs, gemini.rs, responses.rs)
- `/Users/huytho_ho/acli/ata/codex-rs/core/src/client/` (anthropic.rs, gemini.rs, gemini_code_assist.rs, provider_streaming.rs)
- `/Users/huytho_ho/acli/ata/codex-rs/core/third_party_models.json`
- `/Users/huytho_ho/acli/ata/codex-rs/lmstudio/` (LMStudio crate)
- `/Users/huytho_ho/acli/ata/codex-rs/ollama/` (Ollama crate with OpenAI-compat support)
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/chatwidget.rs` (model picker)

### Upstream (rust-v0.129.0)
- `codex-rs/model-provider-info/src/lib.rs`
- `codex-rs/models-manager/src/` (manager.rs, cache.rs, model_info.rs, etc.)
- `codex-rs/models-manager/models.json`
- `codex-rs/lmstudio/`
- `codex-rs/ollama/`
- `codex-rs/tui/` (model picker integration)

