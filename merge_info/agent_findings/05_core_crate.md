# Fork-vs-Upstream Divergence Analysis: `codex-rs/core/` Crate

**Baseline**: Upstream tag `rust-v0.129.0`  
**Date**: 2026-05-07  
**Analysis Scope**: Agent runtime architecture (turn manager, codex main loop, agent control, tool dispatch, conversation state, shell/apply_patch tools, MCP integration, sandbox/exec, prompts, environment context, rollout/sessions, compaction)

---

## Executive Summary

The ATA fork has undergone **major architectural refactoring** since the upstream baseline:

1. **Consolidated session architecture**: The upstream's `session/` module (Session, SessionState, handlers) has been partially refactored into `codex.rs` (7324 lines) which contains the main runtime loop.
2. **New subsystems**: Analytics client, authentication manager (multi-provider), memories (phase1/phase2 consolidation), config_loader (multi-layer), MCP connection manager, external agent config, research module.
3. **Removed/restructured agent management**: Upstream had `agent/registry.rs` and `agent/mailbox.rs` which are removed; replaced with `agent/guards.rs` (spawn depth limits, nickname mgmt).
4. **New tool orchestration**: Code mode (JavaScript REPL execution), multi-agents v1 (spawn/wait/send_input/close/resume), research agents, agent_jobs (batch CSV spawning).
5. **Fork-specific defaults**: Agent role configuration, agent names (including "synthesizer" for ATA), fork-specific control flow in `agent/control.rs`.

---

## Features ONLY in Fork (Not in Upstream)

### 1. **Main Codex Runtime Loop** (`codex.rs`)
- **File**: `/codex-rs/core/src/codex.rs` (7324 lines)
- **Description**: Unified agent runtime containing session initialization, turn processing, event streaming, response handling, message management, and agent spawning.
- **Implementation Summary**:
  - Top-level `Codex` struct managing conversation state, threading, task scheduling
  - Turn processing pipeline (`next_turn`, `run_turn_step`)
  - Event streaming via async channels
  - Real-time conversation handling (audio, text)
  - Rollout reconstruction and file attachment handling
  - Integration with memories, compaction, and external agents
- **Status**: **LOCAL-ONLY** — Upstream had distributed session logic across `session/session.rs`, `session/turn.rs`, `session/handlers.rs`, `codex_delegate.rs`, and `codex_thread.rs`.
- **Merge Plan**: Keep ATA's consolidated codex.rs structure. It provides cleaner separation of concerns and is essential to ATA's architecture. Upstream session patterns can be selectively integrated for shared concerns (e.g., mailbox, state transitions) but the main loop is fork-specific.

---

### 2. **Agent Guards & Limits** (`agent/guards.rs`)
- **File**: `/codex-rs/core/src/agent/guards.rs` (226 lines)
- **Description**: Multi-agent spawn depth and count limits per user session; nickname assignment for spawned agents.
- **Implementation Summary**:
  - `Guards` struct: tracks active agents, enforces max thread limits, manages agent nicknames
  - `SpawnReservation` for atomic slot allocation
  - `exceed_thread_spawn_depth_limit()` check
  - Nickname versioning (e.g., "Alice", "Alice the 2nd", "Alice the 3rd")
- **Status**: **LOCAL-ONLY** — Upstream had `agent/registry.rs` (344 lines deleted) but no equivalent spawn guards.
- **Merge Plan**: Keep ATA's guards implementation. It is critical for preventing resource exhaustion in multi-agent scenarios and is not superseded by upstream patterns.

---

### 3. **Analytics Client** (`analytics_client.rs`)
- **Files**: 
  - `/codex-rs/core/src/analytics_client.rs` (766 lines)
  - `/codex-rs/core/src/analytics_client_tests.rs` (289 lines)
- **Description**: Telemetry pipeline for tracking skill invocations, app usage, plugin usage, and agent events.
- **Implementation Summary**:
  - `AnalyticsEventsClient` wraps a queue for async event batching
  - `TrackEventsJob` enum for skill, app, plugin invocations
  - Deduplication of emission by (connector_id, app_name) to avoid spam
  - Integration with auth manager for authenticated event submission
- **Status**: **LOCAL-ONLY** — Upstream had no equivalent client-side analytics infrastructure.
- **Merge Plan**: Keep as-is. It's a foundational telemetry layer for understanding usage patterns and is not conflicting with upstream.

---

### 4. **Multi-Provider Authentication** (`auth/`)
- **Files**:
  - `/codex-rs/core/src/auth.rs` (1490 lines)
  - `/codex-rs/core/src/auth/{gemini_oauth,gemini_revoke,providers,refresh,storage}.rs` (~2000 lines total)
- **Description**: OAuth and credential management for multiple LLM providers (OpenAI, Anthropic, Gemini, custom).
- **Implementation Summary**:
  - `AuthManager` with multi-provider support
  - OAuth 2.0 flow for Gemini (interactive web flow)
  - Token refresh and revocation
  - Secure credential storage (encrypted on disk)
  - Provider status/capabilities checking
- **Status**: **LOCAL-ONLY** — Upstream had minimal auth; ATA extends to support multiple provider strategies.
- **Merge Plan**: Keep ATA's auth system. It is core to ATA's flexibility in provider selection and not present in upstream.

---

### 5. **Memories Subsystem** (`memories/`)
- **Files**:
  - `/codex-rs/core/src/memories/{mod,phase1,phase2,control,start,storage,citations,prompts,usage}.rs` (~1500 lines)
  - `/codex-rs/core/src/memories/README.md`
- **Description**: Startup memory extraction (Phase 1) and consolidation (Phase 2) for long-running sessions.
- **Implementation Summary**:
  - Phase 1: Extract raw memories from rollouts via fast model (gpt-5.1-codex-mini)
  - Phase 2: Consolidate extracted memories via reasoning model (gpt-5.3-codex)
  - Storage in `~/.codex/memories/` with per-thread rollout summaries
  - Global consolidation lock to serialize phase-2 jobs
  - Metrics tracking (phase1/phase2 job counts, token usage, latency)
  - Citation management for memory sources
- **Status**: **LOCAL-ONLY** — Upstream has no equivalent memory system.
- **Merge Plan**: Keep as-is. It's ATA-specific infrastructure for session summarization and long-term context management.

---

### 6. **Config Loader Multi-Layer System** (`config_loader/`)
- **Files**:
  - `/codex-rs/core/src/config_loader/{mod,layer_io,macos}.rs` (~1300 lines)
  - `/codex-rs/core/src/config_loader/README.md`
- **Description**: Declarative, multi-source configuration layering (cloud, admin, system, user, cwd, tree, repo, runtime).
- **Implementation Summary**:
  - Layer-based merging with trust levels (cloud > admin > system > user > cwd > tree > repo > runtime)
  - macOS-specific managed device profile loading
  - Git repo boundary detection for trust scoping
  - `ConfigLayerStack` abstraction for ordered merging
  - Integrates with `codex_config` crate for shared schema validation
- **Status**: **LOCAL-ONLY** — Upstream had simpler config loading without layer stacking.
- **Merge Plan**: Keep ATA's sophisticated layering. It's essential for managed device support and cross-workspace config inheritance.

---

### 7. **MCP Connection Manager** (`mcp_connection_manager.rs`)
- **File**: `/codex-rs/core/src/mcp_connection_manager.rs` (330+ lines)
- **Description**: Lifecycle and state management for MCP server connections.
- **Implementation Summary**:
  - Connection pooling and reconnection logic
  - Server environment variable resolution
  - Error handling for init/lifecycle failures
  - Integration with skill dependencies and environment context
- **Status**: **LOCAL-ONLY** — Upstream had basic MCP integration but not a dedicated connection manager.
- **Merge Plan**: Keep as-is. It's critical for reliable MCP server management in ATA.

---

### 8. **Research Module** (`research/`)
- **Files**:
  - `/codex-rs/core/src/research/{mod,prompt,output_schema,tool_names,types}.rs` (~70 lines + schemas)
- **Description**: Research task orchestration with specialized prompts and tool definitions.
- **Implementation Summary**:
  - `ResearchRequest` with paper metadata and query
  - `ResearchOutput` with structured answers
  - Specialized prompt templates for literature synthesis
  - Tool names and schema for research-mode agents
- **Status**: **LOCAL-ONLY** — Upstream has no research-specific tooling.
- **Merge Plan**: Keep as-is. It's a specialized capability for ATA research mode.

---

### 9. **External Agent Config Detection** (`external_agent_config.rs`)
- **File**: `/codex-rs/core/src/external_agent_config.rs` (200+ lines)
- **Description**: Detection and import of legacy Claude config/skills from Claude.app.
- **Implementation Summary**:
  - Scan for `.claude/config.toml`, `.claude/skills/`, `agents.md`, and MCP config
  - Migration item detection (Config, Skills, AgentsMd, McpServerConfig)
  - Home directory and CWD scoping
- **Status**: **LOCAL-ONLY** — ATA-specific migration path from Claude.app.
- **Merge Plan**: Keep as-is. It provides user migration convenience.

---

### 10. **Code Mode (JavaScript Execution)** (`tools/code_mode/`)
- **Files**:
  - `/codex-rs/core/src/tools/code_mode/{mod,protocol,service,execute_handler,wait_handler,worker}.rs` (~1000 lines)
  - `/codex-rs/core/src/tools/code_mode/{bridge.js,runner.cjs,description.md,*.md,*.rs}`
- **Description**: Isolated JavaScript REPL for inline computation and tool composition.
- **Implementation Summary**:
  - `CodeModeRuntime` spawns isolated Node process
  - Bridge.js exposes global `tools`, `image`, `load`, `store`, `text`, `yield_control`
  - Pragma parsing for `yield_time_ms` and `max_output_tokens`
  - Process lifecycle management (start, send stdin, wait, kill)
  - Integration with tool registry via `ALL_TOOLS`
- **Status**: **LOCAL-ONLY** — Upstream has no code mode capability.
- **Merge Plan**: Keep as-is. It's a powerful feature for agent composition and scripting.

---

### 11. **Agent Roles Configuration** (`config/agent_roles.rs`)
- **File**: `/codex-rs/core/src/config/agent_roles.rs` (205 lines)
- **Description**: Role-based agent configuration and capability definitions.
- **Implementation Summary**:
  - `AgentRole` with capabilities, model overrides, permissions
  - `AgentRoleConfig` serialization from TOML
  - Default roles (user, developer, analyst, etc.)
- **Status**: **LOCAL-ONLY** — Upstream did not have declarative agent roles.
- **Merge Plan**: Keep as-is. It's essential for role-based agent spawning in ATA.

---

### 12. **Agent Guards Tests** (`agent/guards_tests.rs`)
- **File**: `/codex-rs/core/src/agent/guards_tests.rs` (243 lines)
- **Description**: Test suite for spawn limits, nickname generation, depth checking.
- **Status**: **LOCAL-ONLY** (tests only).
- **Merge Plan**: Keep as-is.

---

### 13. **State Database & Bridge** (`state_db.rs`, `state_db_bridge.rs`)
- **Files**: 
  - `/codex-rs/core/src/state_db.rs` (300+ lines)
  - `/codex-rs/core/src/state_db_bridge.rs` (new bridge module)
- **Description**: Query and update interface for session/turn state persistence.
- **Status**: **LOCAL-ONLY** — Upstream had simpler state management.
- **Merge Plan**: Keep as-is.

---

### 14. **Subagent Notification & Context** (`session_prefix.rs`, `context/subagent_notification.rs`)
- **Files**:
  - `/codex-rs/core/src/session_prefix.rs` (150+ lines)
  - `/codex-rs/core/src/context/subagent_notification.rs`
- **Description**: Formatting and injection of subagent context hints.
- **Status**: **LOCAL-ONLY** — ATA-specific context framing for spawned agents.
- **Merge Plan**: Keep as-is.

---

### 15. **Custom Prompts** (`custom_prompts.rs`)
- **File**: `/codex-rs/core/src/custom_prompts.rs` (149 lines)
- **Description**: User-provided prompt overrides for system/user/developer instructions.
- **Status**: **LOCAL-ONLY** — ATA feature for customization.
- **Merge Plan**: Keep as-is.

---

### 16. **Data & Tool Names** (`data/mod.rs`, `data/tool_names.rs`)
- **Files**: `/codex-rs/core/src/data/{mod,tool_names}.rs`
- **Description**: Centralized tool name constants and data module.
- **Status**: **LOCAL-ONLY** (small utility modules).
- **Merge Plan**: Keep as-is.

---

### 17. **API Bridge** (`api_bridge.rs`)
- **File**: `/codex-rs/core/src/api_bridge.rs` (274 lines)
- **Description**: HTTP API client for coordinating with app server / cloud backend.
- **Status**: **LOCAL-ONLY** — For cloud integration.
- **Merge Plan**: Keep as-is.

---

### 18. **Skills/Plugins Management Enhancements** (`skills/`, `plugins/`)
- **Key expansions**:
  - `/codex-rs/core/src/skills/{loader,manager,model,permissions,remote,render,system,env_var_dependencies,invocation_utils}.rs` (new detailed modules)
  - `/codex-rs/core/src/plugins/{curated_repo,marketplace,discoverable,store,toggles,test_support}.rs` (expanded capabilities)
- **Description**: Extended skills and plugins ecosystem with remote loading, permissions, discovery.
- **Status**: **LOCAL-ONLY** enhancements over upstream.
- **Merge Plan**: Keep as-is. These are crucial for the ATA plugin marketplace.

---

### 19. **Tool Handlers Expansion** (`tools/handlers/`)
- **New handlers**:
  - `agent_jobs.rs`, `agent_jobs_spec.rs` — Batch spawn agents from CSV
  - `goal.rs`, `goal_spec.rs` — Goal management (create, get, update)
  - `mcp_resource.rs`, `mcp_resource_spec.rs` — MCP resource listing and reading
  - `multi_agents.rs`, `multi_agents_v2.rs` — Agent orchestration (spawn, send, wait, close)
  - `plan.rs`, `plan_spec.rs` — Goal/plan generation
  - `request_plugin_install.rs`, `request_plugin_install_spec.rs` — Plugin installation approval
  - `unavailable_tool.rs` — Placeholder for disabled tools
  - `unified_exec.rs` — Unified shell/code execution
  - `test_sync.rs`, `test_sync_spec.rs` — Test synchronization
- **Status**: **LOCAL-ONLY** expansions.
- **Merge Plan**: Keep as-is. These enable ATA's advanced agentic capabilities.

---

### 20. **Unified Exec Module** (`unified_exec/`)
- **Files**: `/codex-rs/core/src/unified_exec/{mod,errors,async_watcher,head_tail_buffer,process_manager,process,process_state}.rs` (~1000 lines)
- **Description**: Unified command execution with async streaming, process management, and output buffering.
- **Status**: **LOCAL-ONLY** enhancement over upstream `exec.rs`.
- **Merge Plan**: Keep as-is. This is critical for reliable agent execution.

---

### 21. **Hook Runtime** (`hook_runtime.rs`)
- **File**: `/codex-rs/core/src/hook_runtime.rs` (100+ lines)
- **Description**: Execution environment for user-defined hooks (before/after/on-event).
- **Status**: **LOCAL-ONLY** — ATA extensibility feature.
- **Merge Plan**: Keep as-is.

---

### 22. **Turn/Session Metadata & Tracking** 
- **Files**:
  - `/codex-rs/core/src/turn_metadata.rs` — Turn-scoped metadata (task count, timing)
  - `/codex-rs/core/src/turn_diff_tracker.rs` — Tracks changes within a turn
  - `/codex-rs/core/src/thread_rollout_truncation.rs` — Fork-specific truncation policy
  - `/codex-rs/core/src/state/turn.rs` — Turn state machine
- **Status**: **LOCAL-ONLY** or enhanced implementations.
- **Merge Plan**: Keep as-is. These are core to ATA's turn and fork management.

---

### 23. **Seatbelt Sandbox Policies** (`seatbelt*.sbpl`, `seatbelt*.rs`)
- **Files**:
  - `/codex-rs/core/src/seatbelt_base_policy.sbpl`
  - `/codex-rs/core/src/seatbelt_network_policy.sbpl`
  - `/codex-rs/core/src/seatbelt_platform_defaults.sbpl`
  - `/codex-rs/core/src/seatbelt.rs`, `seatbelt_permissions.rs`
  - `/codex-rs/core/src/sandboxing/macos_permissions.rs`
- **Description**: macOS Seatbelt sandbox policy definitions and management.
- **Status**: **LOCAL-ONLY** or enhanced.
- **Merge Plan**: Keep as-is. macOS-specific hardening critical for security.

---

### 24. **Environment Selection & Context** 
- **Files**:
  - `/codex-rs/core/src/environment_selection.rs` (from upstream, but fork-enhanced)
  - `/codex-rs/core/src/environment_context.rs` (300+ lines, fork-specific enhancements)
  - `/codex-rs/core/src/contextual_user_message.rs` (108 lines)
- **Description**: Dynamic environment variable and context injection.
- **Status**: **LOCAL-ONLY** enhancements.
- **Merge Plan**: Keep as-is.

---

### 25. **Prompt-Related Modules** (`review_prompts.rs`, `review_format.rs`, `prompt_snapshot.rs`, `prompt_debug.rs`)
- **Description**: Specialized prompts for review flows, compaction, and debugging.
- **Status**: **LOCAL-ONLY** or enhanced.
- **Merge Plan**: Keep as-is.

---

### 26. **Memories-Related Context Fragments** (`context/`)
- **Deleted from upstream**:
  - `context/contextual_user_message.rs` (moved to top-level `contextual_user_message.rs`)
  - `context/environment_context.rs` (moved to top-level `environment_context.rs`)
  - `context/fragment.rs` (removed; logic consolidated)
- **Status**: **REFACTORED** — Not a new feature, but reorganized.
- **Merge Plan**: Understand upstream's current context structure and integrate carefully.

---

## Features in BOTH Fork and Upstream (With Divergences)

### 1. **Agent Control & Spawning** (`agent/control.rs`)
- **Local**: 1152 lines (heavily refactored)
- **Upstream**: 1074 lines (different structure)
- **Divergences**:
  - **Local**: Uses `Guards` for spawn limits, simpler control flow, subagent output message injection
  - **Upstream**: Uses `AgentRegistry` for agent tracking, complex fork mode handling (`SpawnAgentForkMode::FullHistory` vs `LastNTurns`), inter-agent communication mailbox
  - **Local removed**: `AgentRegistry`, `LiveAgent`, `ListedAgent`, `keep_forked_rollout_item()` filtering, `SpawnAgentForkMode`
  - **Local added**: `Guards`, `fork_parent_spawn_call_id` tracking, simplified options
- **Status**: **BOTH-PRESENT, SIGNIFICANTLY DIVERGED**
- **Merge Plan**: Keep ATA's simplified control flow and Guards-based approach. Upstream's fork mode and registry features were superseded by ATA's architecture. If needed for compatibility, add lightweight aliases/adapters.

---

### 2. **Agent Role Configuration** (`agent/role.rs`)
- **Local**: 56 lines (+ tests)
- **Upstream**: Similar role enum/types
- **Divergences**:
  - **Local**: `DEFAULT_ROLE_NAME`, minimal role metadata
  - **Upstream**: More extensive role definitions and capabilities
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Cross-reference upstream's role capabilities and merge if beneficial. ATA's simplified approach may suffice.

---

### 3. **Agent Status Tracking** (`agent/status.rs`)
- **Local**: Minor changes
- **Upstream**: Similar status enum
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Align status definitions between fork and upstream. No conflicts expected.

---

### 4. **Apply Patch Tool** (`apply_patch.rs`, `tools/handlers/apply_patch.rs`, `tools/runtimes/apply_patch.rs`)
- **Local**: 
  - Core: 23 lines (minimal wrapper)
  - Handler: Full implementation with patching logic
  - Runtime: Patch application via unified_exec
- **Upstream**: 
  - Core: 23 lines (similar)
  - Handler: Similar logic
- **Divergences**: 
  - **Local**: Uses unified_exec backend, enhanced error handling
  - **Upstream**: May use different exec strategy
- **Status**: **BOTH-PRESENT, SLIGHTLY DIVERGED**
- **Merge Plan**: Compare upstream's apply_patch strategy. If compatible, unify backends. ATA's unified_exec approach is preferable.

---

### 5. **Shell Tool** (`shell.rs`, `tools/handlers/shell.rs`, `tools/runtimes/shell.rs`)
- **Local**:
  - Core shell: 330+ lines (shell detection, snapshot mgmt)
  - Handler: 600+ lines with subprocess management
  - Runtime: Complex zsh_fork_backend, unix_escalation
- **Upstream**: Similar structure
- **Divergences**:
  - **Local**: Enhanced zsh fork backend, privilege escalation handling, shell snapshot management
  - **Upstream**: May have different escalation or fork strategies
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED**
- **Merge Plan**: Merge upstream's shell improvements carefully. ATA's zsh fork backend and escalation logic are critical; preserve them unless upstream has superior alternatives.

---

### 6. **Rollout Management** (`rollout/`)
- **Local**:
  - `rollout.rs`: Simplified main entry point
  - Submodules: `error.rs`, `list.rs`, `metadata.rs`, `mod.rs`, `policy.rs`, `recorder.rs`, `session_index.rs`, `truncation.rs`
- **Upstream**: 
  - `rollout.rs`: 800+ lines (more monolithic)
  - Fewer submodules
- **Divergences**:
  - **Local**: Modularized, cleaner separation of concerns
  - **Upstream**: More monolithic, potentially different truncation/recording logic
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED (STRUCTURE)**
- **Merge Plan**: Merge upstream's rollout logic into ATA's modularized structure. No fundamental conflicts; mostly refactoring alignment.

---

### 7. **Compaction** (`compact.rs`, `compact_remote.rs`)
- **Local**:
  - `compact.rs`: 233 lines (core compaction logic)
  - `compact_remote.rs`: 155 lines (remote compaction)
  - `compact_remote_v2.rs`: Removed from fork
- **Upstream**:
  - `compact.rs`: 290+ lines
  - `compact_remote.rs`: 155+ lines
  - `compact_remote_v2.rs`: Exists
- **Divergences**:
  - **Local**: Removed `compact_remote_v2.rs`; unknown if consolidated into v1 or removed
  - **Upstream**: Has v2
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED**
- **Merge Plan**: Understand what was in `compact_remote_v2.rs` upstream and whether ATA intentionally removed it or if it should be reintegrated.

---

### 8. **Config System** (`config/mod.rs`, `config/*.rs`)
- **Local**: Extensively refactored (~3143 lines changed in mod.rs alone)
- **Upstream**: Similar scope but different layout
- **Divergences**:
  - **Local**: New submodules: `mcp_config.rs`, `profile.rs`, `project.rs`, `service.rs`, `types.rs`, `web_search.rs`
  - **Upstream**: Older structure
  - **Local removed**: Some upstream config types likely consolidated or renamed
- **Status**: **BOTH-PRESENT, SIGNIFICANTLY DIVERGED**
- **Merge Plan**: Review upstream's config innovations (if any) and selectively integrate. ATA's modularized approach is better; avoid reverting to upstream's monolithic style.

---

### 9. **Client (Provider Abstraction)** (`client.rs`, `client/anthropic.rs`, `client/gemini.rs`, `client/gemini_code_assist.rs`, `client/provider_streaming.rs`)
- **Local**: 
  - Main: 1221 lines (reduced from upstream's 1440+)
  - Anthropic provider: 121 lines
  - Gemini: 228 lines + Code Assist: 696 lines
  - Provider streaming: 408 lines
- **Upstream**:
  - Main: 1440+ lines (monolithic)
  - No separate provider modules in this structure
- **Divergences**:
  - **Local**: Modularized per-provider implementations
  - **Upstream**: More monolithic client
  - **Local added**: `gemini_code_assist.rs` (specific to Gemini's Code Assist API)
- **Status**: **BOTH-PRESENT, SIGNIFICANTLY DIVERGED (STRUCTURE)**
- **Merge Plan**: Keep ATA's per-provider modularization. It's cleaner and more maintainable. Upstream's monolithic approach is a step backward.

---

### 10. **MCP Tool Integration** (`mcp.rs`, `mcp_tool_*.rs`, `tools/handlers/mcp.rs`)
- **Local**:
  - Core: `mcp.rs` (simplified)
  - Tool call: `mcp_tool_call.rs` (200+ lines)
  - Tool approval: `mcp_tool_approval_templates.rs`
  - Tool exposure: `mcp_tool_exposure.rs` (with test)
  - Handler: `tools/handlers/mcp.rs`
- **Upstream**: Similar structure
- **Divergences**:
  - **Local**: Enhanced approval templates, exposure checks
  - **Upstream**: May have different approval or exposure logic
- **Status**: **BOTH-PRESENT, MINIMALLY TO MODERATELY DIVERGED**
- **Merge Plan**: Review upstream's MCP improvements. Likely compatible; merge carefully.

---

### 11. **Turn Management** (`state/turn.rs`, `turn_*.rs`)
- **Local**: 
  - State: `state/turn.rs` (100+ lines, ActiveTurn + TurnState)
  - Metadata: `turn_metadata.rs`
  - Timing: `turn_timing.rs`
  - Diff tracking: `turn_diff_tracker.rs`
- **Upstream**: 
  - Similar turn state, but may be in `session/turn.rs`
- **Divergences**:
  - **Local**: Modularized turn concerns
  - **Upstream**: May be more integrated with Session
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED (STRUCTURE)**
- **Merge Plan**: Keep ATA's modularized turn management. Upstream's integration may conflict with ATA's codex.rs architecture.

---

### 12. **Prompts and System Instructions** 
- **Local**: 
  - Review prompts, formats, snapshots, debug
  - Context-based instructions (skills, plugins, realtime, etc.)
- **Upstream**: Similar but different organization
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Cross-reference for improvements. Merge upstream's innovations where applicable.

---

### 13. **Sandboxing & Permissions** (`sandboxing/`, `seatbelt.rs`, `safety.rs`, `exec_policy.rs`, etc.)
- **Local**: Extensive macOS-specific hardening (seatbelt, landlock, windows sandbox)
- **Upstream**: Similar sandbox architecture
- **Divergences**:
  - **Local**: Enhanced platform-specific policies
  - **Upstream**: May have different policy approaches
- **Status**: **BOTH-PRESENT, MINIMALLY TO MODERATELY DIVERGED**
- **Merge Plan**: Review upstream's sandbox innovations. Merge where compatible, keep ATA's platform-specific hardening.

---

### 14. **Models Manager** (`models_manager/`)
- **Local**: 
  - Manager, cache, model_info, presets, collaboration_mode_presets
  - ~1000 lines total
- **Upstream**: Similar structure
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Merge upstream's model innovations carefully. ATA's structure is likely compatible.

---

### 15. **Skills System** (`skills.rs`, `skills/`)
- **Local**: 
  - Core: `skills.rs` (simplified)
  - Manager: `skills/manager.rs` (600+ lines)
  - Loader: `skills/loader.rs`
  - Renderer: `skills/render.rs`
  - Multiple support modules
- **Upstream**: 
  - Core: `skills.rs` (simpler)
  - Fewer submodules
- **Divergences**:
  - **Local**: Modularized, enhanced management
  - **Upstream**: More monolithic
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED (STRUCTURE)**
- **Merge Plan**: Keep ATA's modularized skills system. It's better designed for extensibility.

---

### 16. **Plugins System** (`plugins/`)
- **Local**: Expanded with marketplace, curated_repo, toggles, store, discoverable
- **Upstream**: Similar core but fewer modules
- **Status**: **BOTH-PRESENT, MODERATELY DIVERGED (STRUCTURE)**
- **Merge Plan**: Keep ATA's plugin ecosystem. Merge upstream's innovations where applicable.

---

### 17. **Context & Instructions** (`context/`)
- **Local**: Extensive submodules for different instruction types
- **Upstream**: Similar but different organization
- **Divergences**:
  - **Local**: Comprehensive coverage of all instruction types
  - **Upstream**: May have different prompt organization
- **Status**: **BOTH-PRESENT, MINIMALLY TO MODERATELY DIVERGED**
- **Merge Plan**: Merge upstream's instruction improvements where beneficial. ATA's organization is comprehensive.

---

### 18. **Thread Manager & Session Services** (`thread_manager.rs`, `state/service.rs`)
- **Local**: 
  - Thread manager: 430+ lines (session lifecycle, thread spawning)
  - Service: Session service abstraction
- **Upstream**: Similar structure
- **Divergences**:
  - **Local**: Integration with ATA's architecture
  - **Upstream**: May have different service patterns
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Merge upstream's improvements carefully. No major conflicts expected.

---

### 19. **Realtime Conversation** (`realtime_conversation.rs`, `realtime_context.rs`, `realtime_prompt.rs`)
- **Local**: Comprehensive realtime audio/text handling
- **Upstream**: Similar real-time support
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Merge upstream's realtime improvements where applicable.

---

### 20. **File Attachments & URL Handling** (`codex/file_attachments.rs`, `codex/url_file_recovery.rs`, `codex/code_intel.rs`)
- **Local**: Robust attachment caching, URL recovery, code intelligence
- **Upstream**: Similar features
- **Status**: **BOTH-PRESENT, MINIMALLY DIVERGED**
- **Merge Plan**: Merge upstream improvements. No major conflicts.

---

## Summary Table

| **Category** | **Fork-Only** | **Both (Diverged)** | **Count** |
|---|---|---|---|
| Architecture | codex.rs, guards, memories, config_loader, MCP manager, research | agent/control, compact, config, client, MCP, turn, rollout | ~40 |
| Tools | code_mode, agent_jobs, goals, multi_agents_v2, unified_exec | apply_patch, shell, tool orchestration | ~25 |
| Systems | analytics, auth, skills/plugins expansion, external_agent_config | sandboxing, models_manager, prompts, realtime | ~20 |
| **Total Modules** | **~26** | **~34** | **~60** |

---

## Merge Strategy Recommendations

### High Priority (Keep All)
1. **codex.rs** — Central runtime loop; cannot be superseded
2. **Agent Guards** — Resource limits critical for safety
3. **Memories** — ATA-specific long-term context feature
4. **Config Loader Multi-Layer** — Managed device support depends on it
5. **Analytics Client** — Usage tracking infrastructure
6. **Code Mode** — Powerful execution capability
7. **MCP Connection Manager** — Stable MCP lifecycle
8. **Skills/Plugins Ecosystem** — ATA competitive advantage

### Medium Priority (Merge Selectively)
1. **Agent Control** — Keep ATA's simplified approach; adapt upstream's registry if needed
2. **Client Modules** — Keep per-provider modularization; adopt upstream optimizations
3. **Rollout Management** — Merge upstream's logic into ATA's structure
4. **Config System** — Keep ATA's modularization; adopt upstream innovations
5. **Turn Management** — Keep ATA's modularized turn concerns
6. **Sandboxing** — Merge upstream's improvements; keep ATA's platform-specific hardening

### Low Priority (Integrate Carefully)
1. **Realtime Conversation** — Mostly compatible; merge optimizations
2. **Prompts/Instructions** — Merge upstream's improvements where applicable
3. **Thread Manager** — Minor divergences; merge for feature parity

### Do Not Merge (Keep Local Implementations)
1. Upstream's agent registry (superseded by Guards)
2. Upstream's mailbox system (if exists; ATA may use different IPC)
3. Upstream's monolithic session.rs (ATA's codex.rs is superior)

---

## Post-Merge Tasks

1. **Test Coverage**: Run full test suite against merged codebase
2. **Performance**: Benchmark memory/CPU usage of consolidated codex.rs
3. **Integration**: Verify MCP, skills, plugins, and multi-agent orchestration
4. **Platform-Specific**: Test macOS seatbelt, Windows sandbox, Linux landlock
5. **Analytics**: Ensure telemetry pipeline works end-to-end
6. **Config**: Validate multi-layer config loading with cloud/admin/system/user layers

---

## Conclusion

The ATA fork has evolved substantially from the upstream baseline with major architectural improvements (consolidated runtime, modularized subsystems, multi-layer config) and significant new capabilities (memories, analytics, code mode, research, external config migration). The merge should prioritize **keeping ATA's core innovations** while selectively adopting upstream's optimizations and bug fixes in compatible areas. The fork is not a superset of upstream; it is a **different architecture** designed for ATA's specific needs.

