# Auth, Login, Secrets & Network Proxy: Fork vs Upstream Analysis

## Overview
The fork introduces **Supabase account authentication** (ATA-specific) while upstream (rust-v0.129.0) has a more comprehensive **auth manager system**, **device key management**, and **agent identity auth**. Both systems exist in parallel and need careful integration.

---

## Feature-by-Feature Comparison

### 1. Supabase Account Auth (Magic Link + Device Code)
- **Description:** ATA account authentication with magic link email flow (port 1455) and device code flow; token storage in `~/.codex/ata_session.json`.
- **Implementation:**
  - `codex-rs/login/src/supabase_auth.rs` (~580 lines)
  - `codex-rs/core/src/supabase/` (client, auth, session, types modules)
  - `codex-rs/core/src/auth.rs` includes Supabase session loading/saving
- **Status:** Local-only (fork-specific)
- **Merge Plan:** Preserve as-is; integrate with upstream auth manager. Supabase handles user-account auth; upstream auth handles agent/API auth. Layer Supabase flows on top of upstream auth architecture.

### 2. Comprehensive Auth Manager & Storage (upstream)
- **Description:** Token lifecycle management, lenient/strict storage modes, revocation, token refresh, external bearer auth.
- **Implementation:**
  - Upstream: `codex-rs/login/src/auth/` (manager.rs, storage.rs, external_bearer.rs, revoke.rs, agent_identity.rs)
  - `codex-rs/login/src/token_data.rs` (JWT parsing + claims)
  - `codex-rs/login/src/auth_env_telemetry.rs` (auth event tracking)
- **Status:** Upstream-only (not in fork)
- **Merge Plan:** Adopt upstream's auth module structure. Fork currently has simplified auth in `core/src/auth.rs`; replace with upstream's `login/src/auth/`. Re-shape Supabase flows to fit the new structure.

### 3. Agent Identity & Device Key Auth
- **Description:** Hardware/TPM device key generation and signing for agent proof-of-identity; prevents unauthorized token use.
- **Implementation:**
  - Upstream: `codex-rs/login/src/auth/agent_identity.rs`
  - Upstream new crate: `codex-rs/device-key/src/` (lib.rs, platform.rs with macOS/Linux/Windows support)
- **Status:** Upstream-only
- **Merge Plan:** Add `codex-rs/device-key/` crate to fork. Integrate device-key into auth manager. Foundational prerequisite for upstream's agent auth.

### 4. Network Proxy: Admin Debug API (local)
- **Description:** HTTP admin endpoint for proxy health, config inspection, pattern listing, block list, mode toggle, reload.
- **Implementation:** `codex-rs/network-proxy/src/admin.rs` (~72 lines, serves health/config/patterns/blocked endpoints)
- **Status:** Local-only
- **Merge Plan:** Preserve. Upstream has `connect_policy.rs` (TCP connector with policy checks); fork adds `admin.rs` (debug API). Merge both — upstream's connect_policy as core enforcement, fork's admin.rs as the debug surface.

### 5. Network Proxy: TCP Connect Policy (upstream)
- **Description:** Enforces network policy on outbound TCP connections (blocks non-public IPs unless `allow_local_binding` is set).
- **Implementation:** `codex-rs/network-proxy/src/connect_policy.rs` (~76 lines, `TargetCheckedTcpConnector` + policy enforcement)
- **Status:** Upstream-only
- **Merge Plan:** Add upstream's `connect_policy.rs` to fork. Low-risk addition; complements fork's admin.rs.

### 6. Connectors: Directory + Workspace Listing
- **Description:** Lists all available connectors from ChatGPT directory and workspace; caches results; merges duplicate entries.
- **Implementation:**
  - Fork: All logic in `codex-rs/connectors/src/lib.rs` (monolithic, ~535 lines)
  - Upstream: Same logic, modularized into:
    - `lib.rs` (~350 lines, cache + high-level API)
    - `merge.rs` (DirectoryApp merging logic)
    - `filter.rs` (app filtering)
    - `accessible.rs` (accessibility checks)
    - `metadata.rs` (metadata normalization)
- **Status:** Both-exist (fork is monolithic, upstream is modular)
- **Merge Plan:** Refactor fork's `lib.rs` to extract `merge`, `filter`, `accessible`, and `metadata` submodules. Behavior unchanged; improves maintainability.

### 7. Workspace Settings Cache (upstream)
- **Description:** Caches workspace beta settings (e.g., `enable_plugins`) for ~15 minutes; used to gate plugin availability.
- **Implementation:** `codex-rs/chatgpt/src/workspace_settings.rs` (~160 lines, `WorkspaceSettingsCache` + `codex_plugins_enabled_for_workspace()`)
- **Status:** Upstream-only
- **Merge Plan:** Add upstream's `workspace_settings.rs` module. Pure feature addition; no dependencies on other upstream changes.

### 8. AWS Authentication (SigV4)
- **Description:** Signs HTTP requests with AWS credentials (IAM/STS) using SigV4 protocol; enables authenticated calls to AWS services.
- **Implementation:** Upstream crate `codex-rs/aws-auth/src/` (lib.rs, config.rs, signing.rs)
- **Status:** Upstream-only
- **Merge Plan:** Add if ATA needs AWS backend integration. Otherwise optional/defer. No dependencies on other auth changes.

### 9. Device Key Management (upstream)
- **Description:** Generates and stores device identity keys; supports hardware secure enclave (macOS), TPM (Linux/Windows), and OS-protected storage fallback; generates proof-of-identity signatures.
- **Implementation:** Upstream crate `codex-rs/device-key/src/` (lib.rs ~200 lines, platform.rs with per-OS implementations)
- **Status:** Upstream-only
- **Merge Plan:** Required before upstream auth manager fully works. Device keys are referenced by `agent_identity.rs`. Moderate complexity; platform-specific code. Prioritize after adopting upstream auth module.

### 10. External Agent Session Migration (upstream)
- **Description:** Detects, exports, and imports session histories from external agent tools (e.g., Claude CLI, IDE plugins); prevents duplicate imports via ledger.
- **Implementation:**
  - `codex-rs/external-agent-sessions/src/` (lib.rs, detect.rs, export.rs, ledger.rs, records.rs)
  - `codex-rs/external-agent-migration/src/` (lib.rs)
- **Status:** Upstream-only
- **Merge Plan:** Add if ATA needs to import external-agent sessions. Optional; defer unless required.

### 11. Auth Module Organization
- **Description:**
  - Fork: Simplified auth in `codex-rs/core/src/auth.rs` with embedded providers; Supabase session calls scattered.
  - Upstream: Comprehensive auth system in `codex-rs/login/src/auth/` (manager, storage, revoke, identity modules); core/src/auth purely for types and providers.
- **Status:** Both-exist (fork simplified, upstream comprehensive)
- **Merge Plan:** Adopt upstream's auth architecture. Move auth logic to login crate. Refactor fork's `core/src/auth.rs` to import from `login/src/auth`; add Supabase session support to login-side auth manager.

### 12. ChatGPT Crate Differences
- **Description:** Fork has chatgpt_client, chatgpt_token, connectors, apply_command, get_task. Upstream adds `workspace_settings.rs`.
- **Status:** Local-missing-upstream-feature (workspace_settings)
- **Merge Plan:** Add `workspace_settings.rs` from upstream. Low risk; pure addition.

### 13. Responses-API-Proxy
- **Description:** Fork has lib.rs, main.rs, read_api_key.rs. Upstream adds `dump.rs`.
- **Status:** Local-missing-upstream-feature
- **Merge Plan:** Add `dump.rs` from upstream. Likely a debug utility; low-risk addition.

### 14. Secrets & Keyring
- **Description:** Local storage of secrets; masking/sanitization in logs.
- **Implementation:**
  - `codex-rs/secrets/src/` (lib.rs, local.rs, sanitizer.rs)
  - `codex-rs/keyring-store/src/` (lib.rs)
- **Status:** Both-identical
- **Merge Plan:** No action; trivial merge.

### 15. CLI Login
- **Description:** Fork extends `codex-rs/cli/src/login.rs` with ATA Supabase auth flows (`send_ata_otp`, `verify_ata_otp`, device-code flows). Upstream is standard Codex.
- **Status:** Local-extended
- **Merge Plan:** Merge upstream's CLI login first, then layer ATA-specific Supabase flows on top (flag-gated if needed).

---

## Merge Sequencing Recommendation

1. **Foundation:** Add upstream's `device-key` crate (prerequisite for agent identity).
2. **Auth System:** Adopt upstream's `login/src/auth/` (manager, storage, revoke, agent_identity); refactor fork's `core/src/auth.rs` to use it.
3. **Supabase:** Integrate Supabase auth into upstream auth manager; preserve all ATA-specific flows.
4. **Network:** Add upstream's `network-proxy/src/connect_policy.rs`; keep fork's admin.rs.
5. **Features:** Add `workspace_settings.rs`, `responses-api-proxy/dump.rs`; refactor connectors to modular structure.
6. **Optional:** Add `aws-auth` and `external-agent-*` crates if needed.

---

## Risk Assessment

**Low Risk:** secrets, keyring, workspace_settings, responses-api-proxy/dump.rs, connectors refactoring.

**Medium Risk:** network proxy (merge admin.rs + connect_policy.rs; test TCP enforcement), chatgpt crate (add workspace_settings; verify integration).

**High Risk:** auth manager adoption (large refactor; Supabase integration must not break), device key (platform-specific; test on all platforms).

**Mitigation:** test Supabase flows after auth manager integration; run network proxy tests after connect_policy merge; test device key on macOS, Linux, Windows.
