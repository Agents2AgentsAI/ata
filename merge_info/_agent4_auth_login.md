## Auth/Login Analysis (Agent 4)

### Overview

Local has **massively restructured** the auth subsystem vs upstream `rust-v0.129.0`:

- **Upstream** keeps the legacy layout: `codex-rs/login/src/auth/{manager,storage,external_bearer,agent_identity,revoke,...}.rs`.
- **Local** moved virtually all that logic out of `codex-rs/login/` and into `codex-rs/core/src/auth.rs` + `codex-rs/core/src/auth/{providers,gemini_oauth,gemini_revoke,refresh,storage,test_utils}` and a fully new `codex-rs/core/src/supabase/`. The `login` crate became a thin shim that re-exports `codex_core::auth::*` and adds the extra interactive flows (`gemini_server`, `supabase_auth`).
- The diff is ~+1.5k / -7.7k LOC under `codex-rs/login/`.

Conflict surface is large in: `codex-rs/login/src/lib.rs`, `codex-rs/login/Cargo.toml`, `codex-rs/login/src/server.rs`, `codex-rs/cli/src/login.rs`, `codex-rs/cli/src/main.rs`, `codex-rs/tui/src/onboarding/auth.rs`, and the new `codex-rs/core/src/auth/` tree.

### Local-only features

#### 1. ATA account (Supabase) auth
- **Type**: Local-only.
- **Description**: First-class "ATA account" auth via Supabase email + 6-digit OTP, persisted as a separate `ata_session.json` independent from `auth.json`. Surfaces as a fourth `AuthMode::Ata` enum variant.
- **Implementation**:
  - `codex-rs/core/src/supabase/{mod,auth,client,session,types,error}.rs` (~825 LOC). Types: `SupabaseClient`, `SupabaseAuth`, `SupabaseSession`, `SupabaseUser`, `Profile`, `DeviceRegistration`, `AuthTokenResponse`, `OtpRequest`, `VerifyOtpRequest`, `RefreshTokenRequest`.
  - Session helpers: `load_ata_session`, `save_ata_session`, `delete_ata_session`, `is_session_expired` (file `~/.ata/ata_session.json`, mode 0600).
  - `codex-rs/login/src/supabase_auth.rs` (600 LOC) with `send_ata_otp`, `verify_ata_otp`, `supabase_magic_link_login`, `supabase_device_code_login`, `request_supabase_device_code`, `complete_supabase_device_code_login`. Listens on port 1455 `/auth/callback`.
  - `AuthMode::Ata` added to `app-server-protocol/src/protocol/common.rs:44` and `otel/src/lib.rs:42`.
  - `AtaAuth` variant on `CodexAuth` enum (`core/src/auth.rs:101-114`), `refresh_ata_session`, `load_ata_session_as_auth`, `load_ata_session_info`.
  - `AtaAccountConfig` in `core/src/config/types.rs:1151`.
- **Merge plan**: Preserve & reapply.

#### 2. `ata login --a2a` and `ata login` Subcommand (CLI)
- **Type**: Local-only.
- **Description**: New CLI flag `--a2a` to drive the Supabase OTP flow; `--ata-only` flag on logout.
- **Implementation**: `codex-rs/cli/src/main.rs:311 (--a2a)`, `337 (--ata-only)`; `codex-rs/cli/src/login.rs:423 (run_login_with_a2a)`, `480 (run_login_status)`, `534 (ATA session status check)`.
- **Merge plan**: Preserve & reapply.

#### 3. ATA OTP onboarding flow (TUI)
- **Type**: Local-only.
- **Description**: Onboarding screen integrates email-OTP "Sign in with A2A account" path with multi-step state machine.
- **Implementation**:
  - `codex-rs/tui/src/onboarding/auth.rs` (~+1200 LOC additions): `AtaOtpInputState`, `spawn_ata_send_otp`, `spawn_ata_verify_otp`, `render_ata_sending_otp`, `render_ata_otp_input`, `render_ata_verifying_otp`.
  - `codex-rs/tui/src/bottom_pane/account_view.rs` (517 LOC, fully new): `AtaLoginStep` enum.
- **Merge plan**: Preserve & reapply. Heavy conflicts expected.

#### 4. Multi-provider auth (Anthropic + Gemini, alongside OpenAI)
- **Type**: Local-only.
- **Description**: Generalizes auth from OpenAI-only into a provider-keyed credential map (`providers: HashMap<String, ProviderCredential>`) supporting OpenAI, Anthropic, Gemini.
- **Implementation**:
  - `codex-rs/core/src/auth/providers/types.rs` (243 LOC): `ProviderCredential` enum (`Api{key}` / `Oauth{credential}` / `ApiAndOauth{key, credential}`), `ProviderAuthSource`, `ProviderAuthMethod`, `ProviderAuthStatus`, constants `PROVIDER_OPENAI/ANTHROPIC/GEMINI`, `ANTHROPIC_API_KEY_ENV_VAR`, `GOOGLE_API_KEY_ENV_VAR`.
  - `codex-rs/core/src/auth/providers/storage_ops.rs` (147 LOC).
  - `codex-rs/core/src/auth/providers/status.rs` (97 LOC) and `env.rs` (25 LOC).
  - `codex-rs/core/src/auth/providers.rs` (807 LOC).
  - CLI `--with-api-key` and `--provider {openai|anthropic|gemini}`, `validate_provider_id` (login.rs:282).
  - TUI `provider_picker.rs` (369 LOC, new).
- **Merge plan**: Preserve & reapply.

#### 5. Gemini OAuth (Google Cloud Code Assist)
- **Type**: Local-only.
- **Description**: Full Google OAuth 2.0 (PKCE) flow for Gemini using Code Assist.
- **Implementation**:
  - `codex-rs/login/src/gemini_server.rs` (518 LOC, new): `GeminiServerOptions`, `run_gemini_login_server`. Scopes `cloud-platform + userinfo.email + userinfo.profile`.
  - `codex-rs/core/src/auth/gemini_oauth.rs` (987 LOC): `GeminiOauthRuntimeContext`, `ensure_gemini_oauth_context`, `force_refresh_gemini_oauth_context`, `code_assist_method_url`. Refresh skew 300s. Onboarding-poll initial 1s, max 8s, timeout 60s.
  - `codex-rs/core/src/auth/gemini_revoke.rs` (88 LOC).
  - CLI `run_login_with_provider_oauth` (login.rs:226).
- **Merge plan**: Preserve & reapply.

#### 6. Anthropic API-key auth
- **Type**: Local-only.
- **Implementation**: Constants in `core/auth/providers/types.rs`; routed via `login_with_provider_api_key("anthropic", …)`. Env var via `provider_env_var(PROVIDER_ANTHROPIC)`.
- **Merge plan**: Preserve.

#### 7. ATA-branded keyring service
- **Type**: Local-only.
- **Description**: Keyring service identifier renamed `codex` → `ata`.
- **Implementation**: `codex-rs/secrets/src/lib.rs:21` (`KEYRING_SERVICE = "ata"`).
- **Merge plan**: Preserve & reapply.

#### 8. Inlined `get_git_repo_root` in `secrets`
- **Type**: Local-only.
- **Description**: Local removed `codex-git-utils` dep and inlined a tiny `.git` walker.
- **Implementation**: `codex-rs/secrets/Cargo.toml`, `codex-rs/secrets/src/lib.rs:165-178`.
- **Merge plan**: Reapply.

#### 9. ATA branding in device-code login messages
- **Type**: Local-only.
- **Implementation**: `codex-rs/login/src/device_code_auth.rs:85,151`.
- **Merge plan**: Reapply.

#### 10. `chatgpt_token` global cache
- **Type**: Local-only (refactor).
- **Description**: New `codex-rs/chatgpt/src/chatgpt_token.rs` (36 LOC) with a global `RwLock<Option<TokenData>>` and `init_chatgpt_token_from_auth`, used by `connectors.rs`.
- **Merge plan**: Likely **adopt upstream's design** for connectors.rs.

#### 11. Bottom-pane account view
- **Type**: Local-only.
- **Implementation**: `codex-rs/tui/src/bottom_pane/account_view.rs` (517 LOC, new).
- **Merge plan**: Preserve & reapply.

### Features both have (shared) — but local moved them to a different crate

#### 12. `AuthManager`, `CodexAuth`, `AuthDotJson`, `TokenData`, `ChatgptAuth`
- **Type**: Shared (both implemented).
- **Implementation**:
  - Upstream: `codex-rs/login/src/auth/manager.rs` (1885 LOC), `…/storage.rs` (361), `…/error.rs`, `…/util.rs`, `token_data.rs` (180), `default_client.rs` (256).
  - Local: `codex-rs/core/src/auth.rs` (1490), `core/src/auth/storage.rs` (468), `core/src/auth/refresh.rs` (128), `core/src/token_data.rs`, `core/src/default_client.rs`. `codex-rs/login/src/lib.rs` re-exports everything from `codex_core::auth::*`.
- **Merge plan**: **Adopt upstream's structure where possible** — let upstream own `AuthManager`/storage in `login/src/auth/`, then add `AtaAuth` variant + Supabase glue on top. The current local layout increases conflict surface.

#### 13. ChatGPT OAuth login server (`server.rs`)
- **Type**: Shared.
- **Implementation**: `codex-rs/login/src/server.rs` — local 1198 LOC, upstream 1188 LOC. Local merely changes the imports.
- **Merge plan**: Trivial reapply.

#### 14. PKCE helpers
- **Type**: Shared.
- **Implementation**: `codex-rs/login/src/pkce.rs` — unchanged.

#### 15. Device code login (ChatGPT)
- **Type**: Shared.
- **Implementation**: `codex-rs/login/src/device_code_auth.rs` — only branding differences.
- **Merge plan**: Adopt upstream test changes, reapply branding.

#### 16. Headless ChatGPT login UI (TUI)
- **Type**: Shared.
- **Implementation**: `codex-rs/tui/src/onboarding/auth/headless_chatgpt_login.rs` (375 LOC modified).
- **Merge plan**: Three-way merge.

#### 17. Local ChatGPT auth helper (`local_chatgpt_auth.rs`)
- **Type**: Upstream-only (was deleted in local).
- **Description**: Upstream `codex-rs/tui/src/local_chatgpt_auth.rs` (214 LOC).
- **Merge plan**: Re-evaluate at merge time.

### Features upstream has that local dropped

#### 18. Agent Identity / JWT auth (`agent_identity.rs`)
- **Type**: Upstream-only.
- **Description**: Upstream supports authenticating agent processes via short-lived JWTs. `CodexAuth::from_agent_identity_jwt` constructor.
- **Implementation (upstream)**: `codex-rs/login/src/auth/agent_identity.rs` (140 LOC).
- **Merge plan**: **Adopt from upstream** if we want feature parity.

#### 19. External bearer-token refresher (`external_bearer.rs`)
- **Type**: Upstream-only.
- **Description**: Pluggable subprocess-based token refresher for custom enterprise bearer auth.
- **Implementation (upstream)**: `codex-rs/login/src/auth/external_bearer.rs` (174 LOC).
- **Note**: Local does keep `ExternalAuthTokens`, `ExternalAuthRefreshContext`, `ExternalAuthRefreshReason`, and the `ExternalAuthRefresher` trait, but **not** the bearer refresher implementation itself.
- **Merge plan**: Adopt upstream.

#### 20. Auth env telemetry (`auth_env_telemetry.rs`)
- **Type**: Upstream-only.
- **Implementation (upstream)**: `codex-rs/login/src/auth_env_telemetry.rs` (89 LOC).
- **Merge plan**: Adopt from upstream; rename env vars list to include ATA equivalents.

#### 21. `success_legacy.html` callback page
- **Type**: Upstream-only (deleted in local).
- **Merge plan**: Leave deleted unless a flow regresses.

#### 22. Logout test suite & auth_refresh test suite
- **Type**: Upstream-only (deleted).
- **Description**: `codex-rs/login/tests/suite/logout.rs` (235 LOC) and `auth_refresh.rs` (1092 LOC) removed in local because the underlying logic moved to `core::auth`. Local has new tests in `core/src/auth_tests.rs` (551), `core/src/auth/storage_tests.rs` (608).
- **Merge plan**: When relocating auth back into `login/auth/`, reinstate these test suites.

### Top-level merge strategy recommendation

1. **Land the multi-provider + Gemini OAuth + Anthropic + ATA-OTP work as preserved-and-reapplied**.
2. **Strongly consider pulling auth code back into `codex-rs/login/src/auth/`** matching upstream layout. This will substantially reduce future merge cost.
3. **Adopt upstream**: `external_bearer.rs`, `auth_env_telemetry.rs`, possibly `agent_identity.rs`.
4. **Re-evaluate**: `local_chatgpt_auth.rs` removal, the `chatgpt_token` global vs upstream's per-call AuthManager pattern.
5. Watch for upstream changes to `AuthDotJson` v2 schema.
