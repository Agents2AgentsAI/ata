# Fork vs Upstream Divergence Analysis: MCP Integration, App-Server, and Protocol

## Summary

This analysis covers the MCP integration, app-server, app-server-protocol, protocol crate, and related client crates (`lsp-client`, `rmcp-client`, `debug-client`, `codex-client`, `backend-client`) between the ATA fork and upstream `rust-v0.129.0`.

**Key Findings:**
- **Local-only crate**: `lsp-client` (fork-exclusive, not in upstream)
- **Upstream-only crates** (not in local fork): `app-server-transport`, `codex-mcp`, `exec-server`, `builtin-mcps`, `core-api`
- **Major structural difference in app-server**: Local fork has large consolidated message processor with embedded WebSocket support
- **Protocol divergence**: Local has monolithic v2.rs (~7.8k lines), upstream has modularized v2/ directory structure

## Features Analysis

### 1. LSP Client Integration (lsp-client crate)

**Status**: Local-only feature

**Description**  
A standalone crate for Language Server Protocol (LSP) client implementation with language detection, server registry management, and root workspace discovery. Enables Codex to interact with language servers for code intelligence.

**Implementation Summary**
- **Key files**: `lsp-client/src/{client.rs, server_registry.rs, language.rs, root_discovery.rs, config_merge.rs, builtin_servers.rs}`
- Manages LSP server lifecycle, configuration, and workspace discovery
- Handles per-language server routing via phf hash map
- Committed in `12dcaaba21 codex-lsp` (as per CLAUDE.md)

**Status vs Upstream**
- **Local-only**: No equivalent in upstream `rust-v0.129.0`

**Merge Plan**  
This is a fork-exclusive feature. Include in merge as-is since upstream has no conflicting LSP infrastructure. Ensure dev dependencies (lsp-types 0.97) align with workspace constraints.

---

### 2. Embedded WebSocket Server (app-server embedded mode)

**Status**: Local-only enhancement

**Description**  
In-process WebSocket endpoint that shares a pre-existing `ThreadManager` with the host process (e.g., TUI). Enables remote clients (mobile/web) to interact with the same threads via standard app-server JSON-RPC protocol. Minimizes upstream conflict by isolating new code in `embedded.rs`.

**Implementation Summary**
- **Key files**: `app-server/src/embedded.rs` (385 lines), `app-server/src/transport.rs` (1.6k lines)
- `EmbeddedWebSocketConfig` struct for initialization with optional token/JWT auth
- Extends `MessageProcessor` with `new_with_thread_manager()` constructor
- Separate module to avoid modifying `message_processor.rs` (upstream-heavy file)

**Status vs Upstream**
- **Local-only**: Upstream has only stdio and WebSocket server modes, not in-process sharing
- Upstream has `codex-app-server-transport` crate (not in local), providing HTTP/WebSocket abstractions

**Merge Plan**  
Keep local embedded mode as fork-specific feature. When merging upstream changes, ensure `message_processor.rs` updates don't break embedded constructor. Consider whether upstream's `app-server-transport` should be adopted post-merge for better separation of concerns.

---

### 3. Device Registration API

**Status**: Local-only feature

**Description**  
Device registration endpoint for authenticating and registering clients connecting over embedded WebSocket. Part of remote-control mode infrastructure.

**Implementation Summary**
- **Key files**: `app-server/src/device_registration.rs` (432 lines)
- Handles device key generation, validation, and auth token management
- Integrated into codex-message-processor for account/device endpoints

**Status vs Upstream**
- **Local-only**: No equivalent in upstream

**Merge Plan**  
Keep local device registration. This is isolated feature code; no upstream conflicts expected.

---

### 4. Consolidated Message Processor (codex-message-processor)

**Status**: Local-only restructuring (upstream has message_processor.rs)

**Description**  
Large (8.8k lines) unified message processor handling all JSON-RPC request dispatch. Consolidates upstream's scattered request processor modules into a single comprehensive handler for all protocol methods (account, apps, marketplace, config, fs, git, hooks, thread, turn, command-exec, feedback, etc.).

**Implementation Summary**
- **Key files**:
  - `app-server/src/codex_message_processor.rs` (8.8k lines) - main dispatcher
  - `app-server/src/codex_message_processor/{apps_list_helpers.rs, plugin_app_helpers.rs}` - helpers
- Implements `CodexMessageProcessor` struct with `process_request(ClientRequest) -> Result<...>`
- Handles API versioning (v1, v2) via `ApiVersion` enum
- Manages authentication state, rollout parsing, thread state transitions
- Local: Uses this consolidated processor in `embedded.rs` and `in_process.rs`

**Status vs Upstream**
- **Upstream has**: Modular request processors under `request_processors/` with individual handler modules:
  - `account_processor.rs`, `apps_processor.rs`, `catalog_processor.rs`, etc. (now deleted in local)
  - These are deleted in local fork, replaced by consolidated processor

**Key Difference**:
- Upstream: Distributed request handler architecture (easier to parallelize development)
- Local: Monolithic processor (harder to merge, but simplified single-point dispatch)

**Merge Plan**  
**Critical merge conflict area**. Options:
1. **Keep local unified processor** and port upstream request handlers into consolidated form (complex but reduces duplication)
2. **Adopt upstream modular approach** and refactor local features into distributed handlers (aligns with upstream, more maintainable)
3. **Hybrid**: Keep local processor as dispatcher, delegate to upstream-style modules for implementation

Recommend option 2 (adopt upstream modular approach) for easier future upstreaming. Requires refactoring `codex_message_processor.rs` into `request_processors/` modules.

---

### 5. Bespoke Event Handling (turn/thread event mapping)

**Status**: Local-only feature

**Description**  
Complex event translation layer mapping internal Codex protocol events (from codex-protocol) to app-server JSON-RPC notifications (codex-app-server-protocol). Handles turn transitions, approval requests, tool calls, reasoning output, etc.

**Implementation Summary**
- **Key files**: `app-server/src/bespoke_event_handling.rs` (3.8k lines)
- Enum `CommandExecutionApprovalPresentation` for network vs command approval contexts
- Functions for mapping `codex_protocol::protocol::Event` → `codex_app_server_protocol::ServerNotification`
- Handles delta streaming (text, tokens, reasoning summary)
- Permission profile intersection logic
- Guardian approval review workflows

**Status vs Upstream**
- **Both have event handling**, but local and upstream may diverge on:
  - Notification schema shape (v1 vs v2 protocol versions)
  - Approval workflow representation
  - Streaming token delta encoding

**Merge Plan**  
Keep local event handler. During merge, verify that upstream's app-server-protocol v2 notification schema matches local's expected shapes. If protocol v2 changed upstream, update event mappings to match new notification types.

---

### 6. App-Server Protocol Restructuring (Protocol v2 Design)

**Status**: Both have implementations, significant divergence

**Description**  
Local fork has a monolithic `protocol/v2.rs` (7.8k lines) defining all v2 API types (requests, responses, notifications, data structures). Upstream modularizes this into a `protocol/v2/` directory with 30+ sub-modules.

**Implementation Summary**

**Local fork structure**:
- `app-server-protocol/src/protocol/v2.rs` (single file, 7.8k lines)
- Contains: Account types, Apps, Collaboration, CommandExec, Config, Device keys, Experimental features, FS, Hooks, Items, Models, Plugins, Processes, Realtime, Reviews, Shared defs, Thread data, Turns, Windows sandbox
- Uses codex_experimental_api_macros for versioning

**Upstream structure**:
- `app-server-protocol/src/protocol/v2/` (directory)
- Modularized files: `account.rs`, `apps.rs`, `collaboration_mode.rs`, `command_exec.rs`, `config.rs`, `device_key.rs`, `experimental_feature.rs`, `feedback.rs`, `fs.rs`, `hook.rs`, `item.rs`, `mcp.rs`, `model.rs`, `notification.rs`, `permissions.rs`, `plugin.rs`, `process.rs`, `realtime.rs`, `review.rs`, `shared.rs`, `tests.rs`, `thread_data.rs`, `thread.rs`, `turn.rs`, `windows_sandbox.rs`

**Status vs Upstream**
- **Both have v2 API**: But local is monolithic, upstream is modular
- **Upstream advantage**: Easier to locate specific type definitions, smaller compilation units
- **Local advantage**: Single file = explicit type dependency graph

**Merge Plan**  
**Significant refactoring needed**. Either:
1. **Adopt upstream modular v2 structure** (recommended): Split `v2.rs` into `v2/{account,apps,collaboration,etc}.rs`. Align with upstream for maintainability.
2. **Keep local monolithic v2.rs**: Merge upstream v2 changes into single file (complex conflict resolution).

Recommend option 1. Plan 2-3 day refactoring to modularize v2.rs post-merge.

---

### 7. MCP Support in Protocol and App-Server

**Status**: Both have implementations, different organization

**Description**  
Model Context Protocol (MCP) integration for tool calling and server management.

**Implementation Summary**

**Local fork**:
- `protocol/src/mcp.rs` - Core MCP types (Tool, Resource, ResourceTemplate)
- `app-server/src/` - MCP request handling in consolidated message processor
- `codex-rmcp-client` - MCP client library
- Dependencies: `rmcp` crate with features `[auth, base64, client, macros, schemars, server, transport-*]`

**Upstream additions**:
- Dedicated `codex-mcp` crate (~new in upstream) - MCP server/client orchestration
- Dedicated `codex-builtin-mcps` crate - Built-in MCP server implementations
- `app-server-protocol/src/protocol/v2/mcp.rs` - MCP request/response types
- `codex-core-api` crate - Core API abstractions for MCP

**Status vs Upstream**
- **Local**: Integrated into core protocol, centralized handling
- **Upstream**: Modular MCP infrastructure with dedicated crates + builtin servers

**Merge Plan**  
**Moderate refactoring**: 
1. Import upstream `codex-mcp` crate as dependency
2. Adopt `codex-builtin-mcps` for built-in server implementations
3. Update app-server message processor to delegate MCP requests to upstream codex-mcp
4. Verify local MCP handling integrates cleanly with upstream's modular approach
5. May enable removal of MCP-specific code from app-server if upstream handles orchestration

Timeline: 3-5 days post-merge.

---

### 8. Protocol Library (codex-protocol) Refactoring

**Status**: Both have implementations, significant content changes

**Description**  
Core protocol types shared by app-server, codex-client, and internal execution engine.

**Implementation Summary**

**Removed in local (vs upstream)**:
- `account.rs` (moved to app-server-protocol or codex-api?)
- `agent_path.rs`, `session_id.rs`, `tool_name.rs` (scope changes)
- `auth.rs` (auth moved elsewhere)
- `error.rs`, `error_tests.rs` (8.2k lines) - error type definitions
- `exec_output.rs`, `exec_output_tests.rs` - execution output model
- `mcp_approval_meta.rs`, `memory_citation.rs` - scope changes
- `network_policy.rs` (moved to approvals?)
- `shell_environment.rs` (moved to config_types?)

**Added in local**:
- `custom_prompts.rs` (20 lines)
- `document_reader.rs` (280 lines) - document reading integration
- `message_history.rs` (11 lines)

**Status vs Upstream**
- **Upstream maintains all original modules**, local has pruned significantly
- **Upstream may have expanded**: error handling, new approval types, etc.

**Merge Plan**  
**Review required**: 
1. Understand why local removed modules (consolidation? scope change?)
2. Verify removed types aren't still imported in app-server (will break build)
3. Adopt upstream's preserved modules if local refactoring was premature
4. For new local additions (`document_reader.rs`), ensure they don't conflict with upstream

Estimated effort: 1-2 days of careful review and potential module restoration.

---

### 9. Tools Crate Restructuring

**Status**: Both have implementations, different structure

**Description**  
Utilities and binaries for code generation, rollout analysis, and protocol introspection.

**Implementation Summary**

**Local fork**:
- `tools/` is now a workspace directory containing:
  - `tools/prompt-inspector/` - Analyzes agent-facing prompts in codebase
  - `tools/rollout-analyzer/` - Analyzes rollout configurations

**Upstream**:
- `tools/` is a single crate with `src/` containing tool binaries
- Appears to have different tool set (original OpenAI structure)

**Status vs Upstream**
- **Local refactored**: Into workspace with purpose-driven subdirectories
- **Upstream**: More integrated structure

**Merge Plan**  
Keep local tools workspace structure. This is isolated tooling; unlikely to conflict with upstream. Verify Cargo.toml workspace includes both directories.

---

### 10. App-Server Client and Protocol Crates (app-server-client, app-server-protocol)

**Status**: Both have implementations, moderate changes

**Description**  
Client library and protocol definitions for communicating with app-server from other processes.

**Implementation Summary**

**Local fork**:
- `app-server-client/` - Client for app-server JSON-RPC
- `app-server-protocol/` - Protocol types (v1, v2, experimental API)
- Dependencies: Both depend on `codex-protocol`, `codex-app-server-protocol`

**Upstream differences**:
- Upstream added `app-server-transport` crate (not in local)
- Upstream may have different experimental API structure

**Status vs Upstream**
- **Both have similar scope**, but transport may be split differently

**Merge Plan**  
Straightforward merge. Import upstream `app-server-transport` crate as dependency if it provides useful abstractions. Otherwise, keep local implementation.

---

### 11. Other Client Crates (rmcp-client, debug-client, codex-client, backend-client)

**Status**: In both, may have divergent implementations

**Description**  
Client libraries for various internal services.

**Implementation Summary**
- `rmcp-client` (codex-rmcp-client): MCP client with OAuth2, keyring storage, HTTP/child-process transports
- `debug-client`: Debugging/introspection client
- `codex-client`: Main Codex engine client
- `backend-client`: Backend service client

**Status vs Upstream**
- All crates exist in both fork and upstream
- Local and upstream likely have diverged on features (new auth modes, transports, etc.)

**Merge Plan**  
Perform detailed diff of each crate's `Cargo.toml` and `src/lib.rs` to identify divergences. Likely safe merges with careful testing. Plan 1-2 days for testing all client integrations.

---

### 12. In-Process Mode (in_process.rs)

**Status**: Both have implementations, local may have enhanced

**Description**  
In-process app-server mode for embedding Codex library directly in Rust applications, avoiding subprocess overhead.

**Implementation Summary**
- **Key files**: `app-server/src/in_process.rs` (899 lines)
- Provides `run_app_server_in_process()` for library users
- Shares thread manager with host process
- Uses same message processor as subprocess mode

**Status vs Upstream**
- Both likely have similar capability, but local may have enhanced with embedded WebSocket support

**Merge Plan**  
Merge straightforward. Ensure in-process and embedded modes don't conflict (both share ThreadManager).

---

## Crate Dependency Changes

### New upstream dependencies (not in local)
- `codex-app-server-transport` - HTTP/WebSocket transport abstraction
- `codex-mcp` - MCP orchestration and server management
- `codex-builtin-mcps` - Built-in MCP implementations
- `codex-core-api` - Core API types
- `codex-exec-server` - Execution server (likely split from app-server)

### Removed from local (vs upstream)
- None in the target crates of this analysis

### Modified dependencies in local app-server
**Local removes** from upstream's Cargo.toml:
- `codex-analytics`
- `codex-config`
- `codex-core-plugins`
- `codex-device-key`
- `codex-exec-server`
- `codex-external-agent-migration`, `codex-external-agent-sessions`
- `codex-features`
- `codex-git-utils`
- `codex-hooks`
- `codex-mcp`
- `codex-model-provider`, `codex-models-manager`
- `codex-app-server-transport`
- `codex-memories-write`
- `codex-models`, `codex-rollout`, `codex-sandboxing`, `codex-thread-store`, `codex-tools`
- `owo-colors`
- `reqwest`
- `sha2`, `thiserror`, `toml-edit`, `url`

**Merge strategy**: Restore these dependencies from upstream during merge. Many are likely essential for proper functionality.

---

## Merge Priority and Effort Estimation

| Feature/Crate | Priority | Effort | Notes |
|---|---|---|---|
| LSP Client | HIGH | 1 day | Fork-only, no conflicts |
| Embedded WebSocket | MEDIUM | 1-2 days | Isolate in dedicated module |
| Device Registration | MEDIUM | 0.5 days | Fork-only, isolated |
| Message Processor Consolidation | **CRITICAL** | 3-5 days | Major refactoring vs upstream modular approach |
| Bespoke Event Handling | HIGH | 1-2 days | Verify protocol v2 compatibility |
| Protocol v2 Modularization | **CRITICAL** | 2-3 days | Split monolithic v2.rs into modules |
| MCP Integration | HIGH | 2-3 days | Adopt upstream codex-mcp crate |
| Protocol Library Pruning | HIGH | 1-2 days | Understand removed modules |
| Tools Workspace | LOW | 0.5 days | Keep as-is |
| Client Crates | MEDIUM | 1-2 days | Diff and test each |
| Dependency Restoration | **CRITICAL** | 1-2 days | Restore upstream dependencies |

**Total estimated effort**: 15-25 days

---

## Recommended Merge Sequence

1. **Phase 1 (Days 1-3)**: Restore upstream dependencies, adopt modular v2 structure, restore removed protocol modules
2. **Phase 2 (Days 4-6)**: Refactor consolidated message processor into upstream's distributed handler pattern
3. **Phase 3 (Days 7-10)**: Integrate upstream codex-mcp, codex-exec-server, codex-app-server-transport
4. **Phase 4 (Days 11-15)**: Layer fork-specific features (embedded WebSocket, device registration, lsp-client) on top
5. **Phase 5 (Days 16-25)**: Integration testing, conflict resolution, performance validation

---

## Risk Areas

1. **Message processor refactoring**: Large consolidated file requires careful modularization
2. **Protocol v2 schema divergence**: Upstream may have changed notification schemas incompatibly
3. **Dependency graph**: Removing dependencies may break app-server compilation
4. **Event handler assumptions**: Bespoke event handling may assume v2 schema shape changes

## Success Criteria

- [ ] All upstream dependencies restored and building
- [ ] App-server compiles and passes tests
- [ ] Embedded WebSocket and device registration remain functional
- [ ] LSP client integrates without issues
- [ ] MCP integration aligned with upstream codex-mcp
- [ ] Protocol v2 modularized into upstream structure
- [ ] No performance regression in message processing
- [ ] All fork-specific features documented and tested
