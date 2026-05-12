## MCP Analysis (Agent 5)

### Scope of changes

```
git diff --stat rust-v0.129.0..HEAD -- codex-rs/{mcp-server,rmcp-client,connectors}
   37 files changed, 1342 insertions(+), 3932 deletions(-)
```

The diff is dominated by **deletions**: upstream split `connectors/` into 4 new modules and added 5 new modules to `rmcp-client/` (we lack all of them). On the core side, upstream extracted MCP logic into two new workspace crates (`codex-mcp`, `codex-builtin-mcps`) we don't have, and refactored `codex-rs/core/src/mcp.rs` from a single file into a 41-line shim.

### Crate / module inventory

| Path | Local | Upstream rust-v0.129.0 |
|---|---|---|
| `codex-rs/mcp-server/` | yes | yes (shared) |
| `codex-rs/rmcp-client/` | yes (7 modules) | yes (12 modules) |
| `codex-rs/connectors/` | yes (single `lib.rs`) | yes (`lib.rs` + `accessible.rs` + `filter.rs` + `merge.rs` + `metadata.rs`) |
| `codex-rs/codex-mcp/` | **missing** | new crate |
| `codex-rs/builtin-mcps/` | **missing** | new crate |
| `codex-rs/memories/mcp/` | **missing** | new crate `codex-memories-mcp` |
| `codex-rs/core/src/mcp/` | local **dir** | upstream is a 41-line `mcp.rs` re-exporting from `codex-mcp` crate |
| `codex-rs/core/src/mcp_openai_file.rs` | **missing locally** | exists upstream (Apps SDK file-upload bridge, 471 lines) |
| `codex-rs/core/src/mcp_tool_exposure.rs` | **missing locally** | exists upstream (deferred-tool exposure threshold) |
| `codex-rs/core/src/mcp_tool_call.rs` | 1292 lines | 2201 lines |
| `codex-rs/core/src/mcp_connection_manager.rs` | 1720 lines (local-owned) | **moved out of core** |
| `shell-tool-mcp/` | yes (TS, ATA-rebranded) | yes |
| `codex-rs/reading-view-server/` | yes | **missing upstream** |

### Features

#### 1. ATA-branded MCP keyring service name
- **Type:** Local-only.
- **Description:** OAuth tokens for MCP servers are stored under `"Ata MCP Credentials"` instead of `"Codex MCP Credentials"`.
- **Implementation:** `codex-rs/rmcp-client/src/oauth.rs:56` — `const KEYRING_SERVICE: &str = "Ata MCP Credentials";`
- **Merge plan:** Keep our string. After every upstream merge, re-assert this constant.

#### 2. Process-global keyring cache for MCP OAuth tokens
- **Type:** Local-only.
- **Description:** Adds a `LazyLock<StdMutex<HashMap<String, Option<StoredOAuthTokens>>>>` (`OAUTH_KEYRING_CACHE`) so repeated MCP connect attempts within a process don't re-prompt the OS keyring.
- **Implementation:** `codex-rs/rmcp-client/src/oauth.rs` — cache populated in `load_oauth_tokens_from_keyring`, invalidated in `save_oauth_tokens_with_keyring` and `delete_oauth_tokens_from_keyring_and_file`.
- **Merge plan:** Re-apply on top of upstream's `oauth.rs`. Consider upstreaming.

#### 3. Local `OAuthCredentialsStoreMode` definition
- **Type:** Local-only divergence.
- **Description:** Upstream defines `OAuthCredentialsStoreMode` in `codex-config::types` and re-imports into `rmcp-client`. Locally it is defined in `rmcp-client/src/oauth.rs` and re-exported.
- **Implementation:** `codex-rs/rmcp-client/src/oauth.rs:78`, `pub use oauth::OAuthCredentialsStoreMode` at `codex-rs/rmcp-client/src/lib.rs:14`.
- **Merge plan:** Move the type into `codex-config::types` to match upstream.

#### 4. `codex-rs/core/src/mcp/` directory module
- **Type:** Shared concept, divergent structure.
- **Description:** Both forks expose `McpManager`, `with_codex_apps_mcp`, `collect_mcp_snapshot`, `split_qualified_tool_name`, `group_tools_by_server`, `ToolPluginProvenance`, `CODEX_APPS_MCP_SERVER_NAME`. Upstream extracted them into a separate `codex-mcp` crate.
- **Implementation:**
  - Local: `codex-rs/core/src/mcp/{mod.rs, auth.rs, skill_dependencies.rs, mod_tests.rs, skill_dependencies_tests.rs}`, `core/src/mcp_connection_manager.rs`, `core/src/mcp_tool_call.rs`, `core/src/mcp_tool_approval_templates.rs`.
  - Upstream: 2-line `core/src/mcp.rs` plus a `codex-mcp` workspace crate.
- **Merge plan:** Adopt upstream's crate split next merge.

#### 5. Local MCP OAuth scopes resolution helpers (`mcp::auth`)
- **Type:** Shared (both have it; lives in different places).
- **Implementation:** Local `core/src/mcp/auth.rs` (288 lines). Upstream version in `codex-mcp/src/auth_elicitation.rs`.
- **Merge plan:** Move into the new local `codex-mcp` crate.

#### 6. Skill MCP-dependency auto-install prompt (`maybe_prompt_and_install_mcp_dependencies`)
- **Type:** Shared (both forks have it).
- **Description:** When a triggered skill declares MCP server dependencies that are not yet configured, prompts the user via `RequestUserInput` and edits config to install them.
- **Implementation:** Local `codex-rs/core/src/mcp/skill_dependencies.rs` (464 lines). Upstream `codex-rs/core/src/mcp_skill_dependencies.rs` (467 lines).
- **Merge plan:** When adopting upstream's crate split, move our `skill_dependencies.rs` to the local `codex-mcp` crate and pull in upstream's new `ElicitationReviewerHandle`.

#### 7. Built-in MCP servers (`codex-builtin-mcps` + `codex-memories-mcp`) — UPSTREAM-NEW
- **Type:** Upstream-new — we need to adopt it.
- **Description:** Upstream introduced a `codex-builtin-mcps` crate that auto-registers product-shipped MCP servers (currently `memories`).
- **Implementation:** `codex-rs/builtin-mcps/src/lib.rs` (140 lines); `codex-rs/memories/mcp/`.
- **Merge plan:** Adopt verbatim — small surface, useful feature.

#### 8. New `rmcp-client` modules removed locally — UPSTREAM-NEW (regression in our fork)
- **Type:** Upstream-new — we need to re-adopt.
- **Description:** Upstream has 12 modules; local has 7. Missing:
  - `elicitation_client_service.rs` — `ElicitationClientService` wrapping `LoggingClientHandler`.
  - `executor_process_transport.rs` — adapter that routes stdio MCP server I/O through `codex-exec-server::HttpClient`.
  - `http_client_adapter.rs` — `StreamableHttpClientAdapter`.
  - `stdio_server_launcher.rs` — `StdioServerLauncher` trait with `LocalStdioServerLauncher` and `ExecutorStdioServerLauncher` implementations.
- **Implementation gap:** Local `RmcpClient::new_stdio_client` (`rmcp_client.rs:478`) has 600+ lines of inline transport code that upstream factored out.
- **Merge plan:** Schedule a dedicated merge step. **Biggest single conflict-reduction win.**

#### 9. `perform_oauth_login_silent` — UPSTREAM-NEW
- **Type:** Upstream-new — adopt.
- **Implementation gap:** `codex-rs/rmcp-client/src/perform_oauth_login.rs` line 100 (upstream).
- **Merge plan:** Port the function and re-export.

#### 10. Connectors module split (`accessible`, `filter`, `merge`, `metadata`) — UPSTREAM-NEW
- **Type:** Upstream-new — adopt for hygiene.
- **Description:** Upstream split `connectors/src/lib.rs` into a `lib.rs` plus four submodules. Our fork merged everything back into a single 534-line `lib.rs`.
- **Merge plan:** Take upstream's file split verbatim. Note: local URL builder uses `tier=categorized` query param — keep that.

#### 11. `mcp_openai_file.rs` (Apps SDK file-upload bridge) — UPSTREAM-NEW
- **Type:** Upstream-new — adopt.
- **Description:** New 471-line module that inspects `_meta["openai/fileParams"]` on MCP tool definitions and rewrites local file paths in the tool args into uploaded-file payloads via `codex_api::upload_local_file`.
- **Merge plan:** Adopt as-is.

#### 12. `mcp_tool_exposure.rs` (deferred tool threshold) — UPSTREAM-NEW
- **Type:** Upstream-new — adopt.
- **Description:** New module with `DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD = 100`. When the model has the search tool and either the `ToolSearchAlwaysDeferMcpTools` feature is on OR there are >=100 MCP tools, the manager defers most MCP tools behind the search tool.
- **Implementation gap:** Local `mcp_tool_call.rs` has no such filtering.
- **Merge plan:** Adopt as-is.

#### 13. MCP guardian / approval elicitation (`build_guardian_mcp_tool_review_request`) — Shared but local-richer
- **Type:** Shared.
- **Description:** Translates pending MCP tool invocations into guardian review requests with structured metadata (`MCP_TOOL_APPROVAL_*` keys). Supports "Allow", "Allow for this session", "Allow and don't ask me again", "Cancel", and a synthetic decline path.
- **Implementation:** `codex-rs/core/src/mcp_tool_call.rs:419-446`, `mcp_tool_approval_templates.rs` (371 lines), wired from `core/src/codex_delegate.rs:45-49`.
- **Merge plan:** Local-richer feature — keep.

#### 14. `McpServerConfig` extended fields — UPSTREAM-NEW
- **Type:** Upstream-new — adopt for config compatibility.
- **Description:** Upstream `McpServerConfig` (`codex-rs/config/src/types.rs`) gained four fields we don't have:
  - `experimental_environment: Option<...>` — selects the experimental sandbox/environment.
  - `supports_parallel_tool_calls: bool` (default true).
  - `default_tools_approval_mode: Option<AppToolApproval>`.
  - `tools: HashMap<String, McpServerToolConfig>` — per-tool overrides.
- **Implementation gap:** Our `codex-rs/core/src/config/types.rs:68-111` only has `transport`, `enabled`, `required`, `disabled_reason`, `startup_timeout_sec`, `tool_timeout_sec`, `enabled_tools`, `disabled_tools`, `scopes`, `oauth_resource`.
- **Merge plan:** Adopt all four fields.

#### 15. `connectors/src/lib.rs` — `tier=categorized` query parameter
- **Type:** Local-only ATA tweak.
- **Description:** Our `list_all_connectors_with_options` calls `/connectors/directory/list?tier=categorized&...`.
- **Merge plan:** Preserve our addition.

#### 16. ATA-rebranded `shell-tool-mcp` package
- **Type:** Local-only (rebranding).
- **Implementation:** `shell-tool-mcp/{package.json,src/index.ts}`, `codex-rs/app-server/tests/suite/bash:25-70`.
- **Merge plan:** Brand re-assertion only.

#### 17. `reading-view-server` Rust crate
- **Type:** Local-only — listed for completeness.
- **Implementation:** `codex-rs/reading-view-server/`.
- **Merge plan:** No upstream interaction.

### Summary recommendation

The merge plan should prioritize, in order:
1. **Adopt upstream `codex-mcp` + `codex-builtin-mcps` + `codex-memories-mcp` crates** (items 4, 7) — biggest divergence reduction.
2. **Restore the four `rmcp-client` modules** (item 8) — large block of upstream code currently inlined locally.
3. **Adopt new `McpServerConfig` fields** (item 14) — needed for upstream-config compatibility.
4. **Adopt `mcp_openai_file.rs` and `mcp_tool_exposure.rs`** (items 11, 12).
5. **Adopt connectors split + `perform_oauth_login_silent`** (items 9, 10).
6. **Preserve our local-only items**: ATA branding (1, 16), keyring cache (2), MCP guardian approval elicitation (13), connectors `tier=categorized` (15).
