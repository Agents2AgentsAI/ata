# Upstream New Features in rust-v0.129.0

## Overview

This document catalogs all **new upstream features** introduced in OpenAI Codex `rust-v0.129.0` that do NOT exist or exist only in primitive form in the ATA fork. It serves as the "what we'll inherit by merging upstream" report for planning the merge from upstream to the fork.

The analysis covers:
1. **39 new crates** added upstream that don't exist locally
2. **Significant feature changes** in shared crates (tui, core, mcp-server, app-server, protocol)
3. **TUI enhancements** from the release notes
4. **Architecture refactorings** that affect merge strategy
5. **Features explicitly REVERTED** at the tip that we need to know about

---

## Part 1: New Crates Not in Fork

### Agent & Identity Infrastructure

#### **agent-graph-store** (⚠️ REVERTED - see Part 4)
- **Crate**: `codex-rs/agent-graph-store/`
- **Purpose**: Storage-neutral parent/child topology for thread-spawned sub-agents
- **What it does**: Provides a distributed agent graph store interface for tracking agent lineage and descendant relationships in multi-agent sessions
- **User-visible impact**: Enables robust sub-agent coordination and session tree visualization
- **Why it matters for merge**: 
  - **CRITICAL**: This crate was injected as a mandatory dependency into `codex-core` in PR #20689, but was **reverted in commit a8488fec5e** (final commit before release tag) because it created a hard dependency on the state DB
  - When merging, we should **not attempt to add this back** unless we have a clear path to make state DB optional
  - The revert keeps descendant lookup using the optional state DB when available (fallback mechanism)
- **Adoption plan**: Skip adding this crate. If your fork has added it, note that upstream reverted it and use the optional state DB pattern instead.

#### **agent-identity**
- **Crate**: `codex-rs/agent-identity/`
- **Purpose**: Shared infrastructure for agent identification and resolution across the runtime
- **What it does**: Provides types and utilities for uniquely identifying and resolving agent/sub-agent identities
- **User-visible impact**: Enables multi-agent sessions with clear identity tracking and inter-agent communication
- **Why it matters for merge**: Complements the agent-graph-store revert by providing identity resolution without hard DB dependencies
- **Adoption plan**: Add as a new crate in the tree; it has minimal dependencies (protocol, config, serde). Should be straightforward to port.

---

### Core API & Plugin Infrastructure

#### **core-api**
- **Crate**: `codex-rs/core-api/`
- **Purpose**: Public facade for thread management APIs built on `codex-core`
- **What it does**: 
  - Aggregates and re-exports key types from core (ThreadManager, CodexThread, StateDbHandle, etc.)
  - Exposes configuration types, analytics, plugins, and auth
  - Centralizes the public surface for embedded Codex usage
- **User-visible impact**: Simplifies imports for app-server and other consumers; reduces coupling to internal core modules
- **Why it matters for merge**:
  - If your fork directly imports from `codex-core`, this crate may be a breaking change or require refactoring
  - The fork's app-server and TUI may need to be updated to use the new public API surface
  - Cleaner separation of internal vs. public core APIs
- **Adoption plan**: After merging core changes, extract a core-api crate in the fork by identifying and re-exporting public types. Check for import paths that need updating.

#### **core-plugins**
- **Crate**: `codex-rs/core-plugins/`
- **Purpose**: Plugin loading, marketplace management, and installation orchestration
- **What it does**:
  - Manages installed marketplaces (OpenAI curated, bundled, custom)
  - Handles plugin installation/uninstallation/upgrades
  - Syncs remote plugin bundles
  - Enforces admin-disabled status
  - Provides tool-suggest discoverability filters
- **Key features**:
  - Workspace plugin sharing with access controls
  - Source filtering in marketplace discovery
  - Local share path tracking for shared plugins
  - Remote bundle sync for installed plugins
  - Removed marketplace flows (via `/plugins` commands)
- **User-visible impact**: Full plugin lifecycle management with workspace sharing, source control, and granular access policies
- **Why it matters for merge**:
  - If fork has plugin code scattered in core, this consolidation is a significant refactor
  - The fork may need plugin manager updates to support the new sharing/filtering features
  - Analytics for plugin skills now flow through this crate
- **Adoption plan**: Migrate existing plugin code from core into core-plugins following upstream structure. Add tests for marketplace sync and share access control workflows.

#### **core-skills**
- **Crate**: `codex-rs/core-skills/`
- **Purpose**: Skills discovery, loading, rendering, and dependency resolution
- **What it does**:
  - Loads skills from multiple roots (system, workspace, shared plugins)
  - Builds implicit skill invocation detectors (command aliases)
  - Renders available-skills markdown for the composer
  - Tracks skill mention counts and dependencies
  - Injects skill metadata into agent prompts
  - Enforces skill policy (allow/deny/require-approval)
- **User-visible impact**: Skills are now first-class configuration entities with clearer dependency tracking and cross-plugin support
- **Why it matters for merge**:
  - Upstream moved skills loading from the core path into app-server (PR #21287) then reverted it (PR #21460) due to concurrency issues
  - The fork needs to be aware of this ongoing refactor; skills watcher positioning is still in flux
  - Parallelized cwd loading for skills list (PR #21441) improves startup performance
- **Adoption plan**: Ensure fork's skills manager is compatible with the new loader/manager API. Verify that workspace and plugin-bundled skills are both discovered. Add tests for implicit invocation detection.

#### **plugin**
- **Crate**: `codex-rs/plugin/`
- **Purpose**: Shared plugin identifiers and telemetry-facing summaries
- **What it does**:
  - Defines PluginId and PluginLoadOutcome types
  - Provides plugin-to-telemetry mappings (including remote IDs for analytics)
  - Exports plugin mention syntax and discovery metadata
- **User-visible impact**: Lightweight; mostly internal infrastructure for consistent plugin naming and analytics
- **Why it matters for merge**: Upstream refactored plugin identity into a separate crate to reduce dependency bloat in core
- **Adoption plan**: Small change; add as a new dependency for plugin and core-plugins. No user-facing changes needed.

---

### Model & Provider Infrastructure

#### **model-provider**
- **Crate**: `codex-rs/model-provider/`
- **Purpose**: Model provider abstraction and auth integration
- **What it does**:
  - Implements provider-agnostic model provider interface
  - Handles AWS Bedrock provider setup
  - Provides bearer-token auth for custom providers
  - Bridges to login/auth system for credential management
- **User-visible impact**: Enables users to configure custom models through provider plugins (e.g., Bedrock, local LMs)
- **Why it matters for merge**:
  - Upstream refactored model auth into this crate from core
  - Bedrock runtime endpoint reporting was fixed (PR #20275)
  - If fork has inline provider auth, this is a structural improvement
- **Adoption plan**: Migrate custom provider logic into this crate. Update model manager to use the new provider interface.

#### **model-provider-info**
- **Crate**: `codex-rs/model-provider-info/`
- **Purpose**: Registry of model providers supported by Codex
- **What it does**:
  - Built-in defaults (OpenAI, Anthropic, etc.) compiled into the binary
  - User-defined entries loaded from `~/.codex/config.toml` under `model_providers` key
  - Provides metadata (service tiers, capabilities) for each provider
- **User-visible impact**: Clear, declarative provider configuration; easy to add custom providers
- **Why it matters for merge**:
  - Model service tiers were added to protocol (PR #20971, #20969)
  - Service tier metadata now propagates through compact operations
  - Fork may need to update config parsing to accept model_providers in TOML
- **Adoption plan**: Extract built-in providers into model-provider-info registry. Update config loader to merge user overrides. Add tests for provider metadata resolution.

#### **models-manager**
- **Crate**: `codex-rs/models-manager/`
- **Purpose**: Aggregates available models from multiple providers and manages refresh strategy
- **What it does**:
  - Collects models from built-in and user-configured providers
  - Exposes refresh strategy (lazy, on-demand, cached)
  - Handles provider fallback when models are unavailable
- **User-visible impact**: Unified model list across providers; transparent handling of provider outages
- **Why it matters for merge**:
  - Upstream deduped fallback model metadata warnings to reduce noise
  - Models manager is the entry point for session model selection
  - Fork needs this if it offers multi-provider model switching
- **Adoption plan**: Port models manager to the fork using the new model-provider-info registry. Test fallback behavior when a provider is down.

---

### File System & Sandbox Infrastructure

#### **file-system**
- **Crate**: `codex-rs/file-system/`
- **Purpose**: Abstract filesystem interface with permission and sandbox context
- **What it does**:
  - Defines trait-based read/write/delete operations
  - Provides FileMetadata, ReadDirectoryEntry types
  - Integrates with Windows sandbox enforcement (ConPTY, named pipes)
  - Supports legacy landlock on Linux
- **User-visible impact**: Transparent sandboxing of file access; works in Windows, Linux, and macOS with platform-specific policies
- **Why it matters for merge**:
  - Windows sandbox now handles named pipes and ConPTY teardown more reliably (PR #20270, #20685)
  - Symlink-protected paths and shared /tmp setups on Linux are now handled
  - Safe.directory for git worktrees is enforced on Windows (PR #21409)
- **Adoption plan**: If fork has inline sandbox logic, extract into file-system crate. Ensure Windows ConPTY and named-pipe handling matches upstream.

#### **sandboxing**
- **Crate**: `codex-rs/sandboxing/`
- **Purpose**: Cross-platform sandbox policy enforcement and command wrapping
- **What it does**:
  - Wraps commands in platform-specific sandboxes (bwrap on Linux, Seatbelt on macOS)
  - Transforms sandbox policies based on permission profiles
  - Provides SandboxManager for orchestrating exec-time enforcement
- **User-visible impact**: Commands are automatically sandboxed according to permission and workspace policies
- **Why it matters for merge**:
  - Standalone bundled bwrap fallback (PR #21255) reduces dependency on system bwrap; improves npm and DotSlash installs
  - Bubblewrap vendored at 0.11.2 with upstream security updates
  - Execpolicy now handles PowerShell -Command wrappers and heredoc redirects correctly
- **Adoption plan**: Consolidate any inline sandbox logic from core/exec. Test against the standalone bwrap fallback in release builds.

---

### Git & Dev Utilities

#### **git-utils**
- **Crate**: `codex-rs/git-utils/`
- **Purpose**: Git operations, diff generation, patch application
- **What it does**:
  - Collect git info (branches, commits, remotes, diffs)
  - Apply git patches with path extraction
  - Compute git baselines for workspace reset
  - Handle pagination flags by position (PR #21381)
  - Create symlinks with platform-specific handling
- **User-visible impact**: Transparent git integration for file workflows, apply-patch, and /diff
- **Why it matters for merge**:
  - Apply-patch file changes are now emitted as turn items (PR #20540) for better history
  - Git pagination is more robust; handles edge cases with flag ordering
  - Fork's git integration can be centralized here
- **Adoption plan**: Port git operations from core/tools into git-utils. Add tests for edge cases (shallow clones, large diffs, pagination).

#### **install-context**
- **Crate**: `codex-rs/install-context/`
- **Purpose**: Detect and abstract over installation context (standalone, npm, bun, brew, dev)
- **What it does**:
  - Determines how Codex was installed (managed releases, package manager, dev)
  - Provides paths for resources and bundled dependencies
  - Handles platform-specific resource locations
- **User-visible impact**: Codex automatically adapts behavior based on installation method (e.g., uses bundled bwrap for npm installs)
- **Why it matters for merge**:
  - Enables standalone bwrap fallback for npm releases
  - Simplifies release packaging and distribution logic
  - Fork may benefit from this abstraction if it supports multiple install channels
- **Adoption plan**: Add install-context detection to fork. Update release builds to use it for resource path resolution.

---

### Execution & Environment

#### **exec-server** (significant upgrade)
- **Crate**: `codex-rs/exec-server/`
- **Purpose**: Remote and local process/file-system execution server
- **What it does**:
  - Provides RPC interfaces for subprocess execution, file I/O, and HTTP requests
  - Supports both local and remote (HTTP-based) executor backends
  - Manages sandboxed filesystems with permission enforcement
  - Integrates with codex-file-system for cross-platform FS policies
- **Key features**:
  - Process spawning with environment capture/injection
  - Streaming HTTP request/response handling
  - Local process ID tracking
  - Async file operations with sandbox context
- **User-visible impact**: Exec operations are now unified through a server interface (local or remote) with consistent sandboxing
- **Why it matters for merge**:
  - Fork may already have this in a different form; understand that upstream consolidated it
  - Async environment manager reduces startup latency
  - If fork supports remote execution (e.g., via container or another machine), this is the integration point
- **Adoption plan**: Review fork's current exec implementation. If it's inline in core, consider extracting into exec-server. Test both local and remote execution paths.

#### **app-server-transport**
- **Crate**: `codex-rs/app-server-transport/`
- **Purpose**: Transport layer abstraction for app-server communication (WebSocket, HTTP, stdio)
- **What it does**:
  - Defines protocol-agnostic transport interfaces
  - Handles connection lifecycle (accept, send, receive)
  - Manages transport shutdown and error propagation
- **User-visible impact**: App-server can be accessed over multiple transport types transparently
- **Why it matters for merge**:
  - Upstream extracted transport into a dedicated crate (PR #20545) to slim down app-server
  - Simplifies testing of app-server logic independent of transport concerns
  - Fork's transport code can now be organized here
- **Adoption plan**: After merging, extract transport abstraction from app-server into app-server-transport. Update tests to use the new transport interfaces.

---

### External Agent Migration

#### **external-agent-migration**
- **Crate**: `codex-rs/external-agent-migration/`
- **Purpose**: Migration helpers for importing external-agent (Claude desktop app) configuration into Codex
- **What it does**:
  - Parses external-agent skill and MCP configurations
  - Converts frontmatter and command definitions to Codex format
  - Builds MCP config from external `.mcp.json`
  - Maps hooks from external agent metadata
- **User-visible impact**: Users can import their external-agent setup (skills, MCPs, hooks) into Codex via a migration flow
- **Why it matters for merge**:
  - Enables smooth onboarding for external-agent users
  - If fork targets external-agent users, this crate is essential
  - Could be a differentiation point if fork has additional migration paths
- **Adoption plan**: Add external-agent-migration crate. Ensure MCP server name mappings and skill path conversions are tested.

#### **external-agent-sessions**
- **Crate**: `codex-rs/external-agent-sessions/`
- **Purpose**: Parsing and export helpers for external-agent session histories
- **What it does**:
  - Detects recent external-agent sessions
  - Loads and imports session histories into Codex format
  - Provides session summaries and metadata
  - Tracks imported sessions to avoid re-importing
- **User-visible impact**: Users can import their external-agent conversation histories into Codex
- **Why it matters for merge**:
  - Complements external-agent-migration for full configuration + history import
  - State is tracked in a ledger to prevent duplicate imports
- **Adoption plan**: Add external-agent-sessions crate alongside migration. Test round-trip import of multi-turn sessions with various tool outputs.

---

### MCP & Plugin Integration

#### **builtin-mcps**
- **Crate**: `codex-rs/builtin-mcps/`
- **Purpose**: Built-in MCP servers shipped with Codex
- **What it does**:
  - Currently ships the `memories` MCP server
  - Declares builtin servers separately from user-configured MCPs
  - Spawns built-in MCPs as stdio subprocesses
  - Configures them without needing external definitions
- **User-visible impact**: Users can use the memories MCP out-of-the-box without additional setup
- **Why it matters for merge**: 
  - Memories is a core feature (see `memories` crate below)
  - Built-in MCPs follow the same stdio protocol as user MCPs (consistency)
  - Fork may want to ship additional built-in MCPs (e.g., GitHub, Slack)
- **Adoption plan**: Add builtin-mcps crate. If fork has memories or other built-in MCPs, register them here. Test stdio spawning and config generation.

#### **codex-mcp**
- **Crate**: `codex-rs/codex-mcp/`
- **Purpose**: Comprehensive MCP server and client coordination
- **What it does**:
  - Manages MCP server lifecycle (connection, discovery, auth)
  - Provides tool integration and authorization (Codex Apps auth elicitations)
  - Handles MCP OAuth login and scope discovery
  - Integrates with Guardian for elicitation review flows
  - Provides MCP resource reading and status snapshots
- **Key features**:
  - Codex Apps connector auth with Guardian routing (PR #19431)
  - MCP permission prompt auto-approval logic
  - Tool provenance tracking (Codex Apps vs. user MCPs)
  - Status snapshot with detailed server info
- **User-visible impact**: MCP servers are seamlessly integrated with auth flows, Guardian approvals, and tool suggestions
- **Why it matters for merge**:
  - This is a major upstream refactor; the fork's MCP integration is now a large, sophisticated system
  - Codex Apps auth elicitations route through Guardian (new workflow)
  - Auto-review now bypasses review for always-allow MCP tools
  - MCP tool output truncation prevents unbounded growth in rollouts
- **Adoption plan**: The fork likely has some MCP code in core; codex-mcp is the upstream destination. Carefully review the elicitation and auth integration; ensure Guardian flows are compatible with fork's auth system.

---

### Analytics & Diagnostics

#### **analytics**
- **Crate**: `codex-rs/analytics/`
- **Purpose**: Centralized analytics events client and telemetry emission
- **What it does**:
  - Provides an async AnalyticsEventsClient for emitting telemetry
  - Tracks tool lifecycle events (start, end, output delta)
  - Emits goal lifecycle metrics (create, start, complete)
  - Reports plugin skill usage and thread sources
  - Tracks service tier metadata across compact operations
- **User-visible impact**: Better visibility into Codex usage patterns and performance (not directly visible to users, but improves product decisions)
- **Why it matters for merge**:
  - Analytics was expanded significantly (PR #17089, #17090, #20799, #20923, #20949, #20969, #20893)
  - Tool item lifecycle events are now emitted (PR #17090)
  - Thread sources are tracked for analytics (PR #20949)
  - If fork omits analytics, this can be skipped; otherwise, ensure you're emitting the same events
- **Adoption plan**: Add analytics crate. Update tool handlers, goals, and plugin managers to emit events. Test telemetry correctness with a mock events collector.

---

### Identity & Session Infrastructure

#### **device-key**
- **Crate**: `codex-rs/device-key/`
- **Purpose**: Device-specific cryptographic key management
- **What it does**:
  - Generates and stores device keys (HSE, TPM, OS-native)
  - Provides key IDs with format validation (dk_hse_, dk_tpm_, dk_osn_)
  - Integrates with native OS key storage (Keychain on macOS, etc.)
- **User-visible impact**: Enables end-to-end encryption for sensitive operations without user intervention
- **Why it matters for merge**: 
  - Not strictly required for core Codex functionality; used for advanced security features
  - If fork targets high-security environments, this is valuable
  - The fork's app-server now has device-key request handling (processor renamed in revert commit)
- **Adoption plan**: Add device-key crate. Update app-server device key processor to use the new crate. Test key generation on each platform.

#### **memories**
- **Crate**: `codex-rs/memories/` (directory with subcrates: read, write, mcp)
- **Purpose**: Long-term memory system for Codex sessions
- **What it does**:
  - **Phase 1**: Extracts structured memories from recent rollouts asynchronously
  - **Phase 2**: Consolidates memories into workspace artifacts (raw_memories.md, rollout_summaries/)
  - **Read path**: Injects developer instructions, handles memory citations, tracks usage telemetry
  - **Write path**: Renders memory prompts, manages workspace diffs, runs consolidation agent
  - **MCP surface**: Exposes memories as a built-in MCP server
- **Key features**:
  - Asynchronous phase-1/phase-2 memory generation
  - Per-rollout summarization with optional slug generation
  - Memory usage tracking and stale cleanup
  - Git baseline for memories workspace to compute diffs
  - Consolidation agent runs with no approvals, no network, local-write-only
  - Extension resource pruning for memory-related extensions
- **User-visible impact**: Codex learns from previous sessions and surfaces relevant context automatically
- **Why it matters for merge**:
  - This is a complex, multi-component feature; one of the biggest upstream additions
  - Memories spawn an internal sub-agent (Phase 2 consolidation), which requires careful thread isolation
  - State DB is required for memory job scheduling and claim tracking
  - Fork may not have memories at all; adding it is a significant undertaking
- **Adoption plan**: 
  - CRITICAL: This feature requires a state DB and background job orchestration
  - Bring in `codex-memories-read` for the read path first (simpler)
  - Bring in `codex-memories-mcp` to expose it as an MCP
  - Bring in `codex-memories-write` and job scheduling if fork can support state DB and background tasks
  - Plan 2-3 weeks of integration and testing for a full memories system

#### **message-history**
- **Crate**: `codex-rs/message-history/`
- **Purpose**: Persistence layer for the global, append-only message history file
- **What it does**:
  - Stores messages in `~/.codex/history.jsonl` (one JSON object per line)
  - Records session_id, timestamp, and message text
  - Implements atomic writes using `O_APPEND` and POSIX guarantees
  - Provides async read/write APIs with retry logic
  - Enforces soft/hard caps on file size with trimming
- **User-visible impact**: Users can search and review their entire Codex conversation history across all sessions
- **Why it matters for merge**:
  - Upstream moved message history out of core (PR #21278) to simplify core
  - The history file is critical for recall and debugging
  - If fork has inline history, extracting it is a good refactor
- **Adoption plan**: Add message-history crate. Update core to use the new API. Test concurrent writes from multiple processes and size trimming on large files.

---

### Feature Management & Configuration

#### **features**
- **Crate**: `codex-rs/features/`
- **Purpose**: Feature flag registry and evaluation
- **What it does**:
  - Defines Feature enum for each gated feature
  - Provides Features struct with enabled() checks
  - Integrates with config.toml for user-facing toggles
  - Supports feature aliases (e.g., `codex_hooks` → `hooks`)
- **User-visible impact**: Users can enable/disable experimental features via config
- **Why it matters for merge**: 
  - Remote control is now gated by feature flag AND state DB availability (PR #670)
  - Features like goals, multi-agent-v2, browser-use are all gated here
  - Fork may have ad-hoc feature checks; consolidating into a crate improves auditability
- **Adoption plan**: Extract feature definitions from config into features crate. Update all feature checks to use the new API. Test feature combinations and deprecation flows.

---

### Realtime & Communications

#### **realtime-webrtc**
- **Crate**: `codex-rs/realtime-webrtc/`
- **Purpose**: WebRTC integration for realtime audio communication
- **What it does**:
  - Manages WebRTC peer connections and SDP negotiation
  - Provides audio level tracking (local microphone)
  - Handles connection events (connected, failed, closed)
  - Platform-specific native support on macOS
- **User-visible impact**: Voice calling and realtime audio interactions (advanced feature)
- **Why it matters for merge**: 
  - This is specialized infrastructure; not all forks may need it
  - If fork targets voice interactions, this is the integration point
  - Otherwise, can be safely skipped during merge
- **Adoption plan**: Optional. Only bring in if fork supports realtime voice. Test WebRTC offer/answer flow and audio level tracking on each platform.

#### **response-debug-context**
- **Crate**: `codex-rs/response-debug-context/`
- **Purpose**: Extract debug metadata from API responses (headers, request IDs, auth errors)
- **What it does**:
  - Parses x-request-id, x-oai-request-id, cf-ray, auth error headers
  - Provides ResponseDebugContext for error reporting
- **User-visible impact**: Better error messages with trace IDs for debugging
- **Why it matters for merge**: 
  - Lightweight utility; unlikely to conflict
  - Improved error messages aid in support and debugging
- **Adoption plan**: Add response-debug-context. Update API error reporting to include debug context in error messages.

---

### Infrastructure & Utilities

#### **rollout** & **rollout-trace**
- **Crates**: `codex-rs/rollout/`, `codex-rs/rollout-trace/`
- **Purpose**: Rollout persistence, trace recording, and state management
- **What it does**:
  - Manages session/rollout metadata and indexing
  - Provides state DB interface for storing thread history
  - Records trace bundles with checkpoints and reduced-state caches
  - Handles trace compaction and writer APIs
  - Supports code-cell trace contexts for code-mode execution
- **Key features**:
  - Rollout extraction per-thread with DB persistence
  - Trace bundle format with efficient compression
  - Reduced-state cache for fast trace replays
  - Code-mode cell instrumentation
- **User-visible impact**: Sessions are durably stored and can be resumed/replayed
- **Why it matters for merge**:
  - Upstream split rollout and rollout-trace to separate concerns (persistence vs. tracing)
  - If fork has inline rollout logic, extraction is a good refactor
  - State DB integration is foundational; ensure optional handling is correct
- **Adoption plan**: Review fork's current rollout implementation. Port persistence logic into rollout crate. Add trace recording in rollout-trace. Test resume and replay flows.

#### **terminal-detection**
- **Crate**: `codex-rs/terminal-detection/`
- **Purpose**: Terminal identification and metadata extraction
- **What it does**:
  - Detects terminal name (Terminal.app, iTerm2, Ghostty, kitty, etc.)
  - Extracts version, TERM, and multiplexer info (tmux, screen)
  - Provides structured TerminalInfo for TUI features
- **User-visible impact**: TUI can adapt behavior based on detected terminal (e.g., key handling, colors)
- **Why it matters for merge**: 
  - TUI features like /copy in tmux now work better (PR #20207)
  - Alt+Enter and modified Delete/Backspace keys behave correctly
  - Terminal-specific keybindings and color support depend on this
- **Adoption plan**: Add terminal-detection. Update TUI to use it for keybinding and color logic. Test on multiple terminal emulators.

#### **test-binary-support**, **thread-manager-sample**, **thread-store**, **uds**
- **Crates**: Various test and infrastructure utilities
- **Purpose**: 
  - `test-binary-support`: Test harness utilities
  - `thread-manager-sample`: Example code for ThreadManager usage (also a test fixture)
  - `thread-store`: Storage-neutral thread persistence (as mentioned in Part 1)
  - `uds`: Cross-platform async Unix domain socket helpers
- **User-visible impact**: Minimal for end users; infrastructure for development and testing
- **Why it matters for merge**: 
  - thread-store is essential; all thread operations funnel through it
  - uds provides the communication backbone for in-process/remote app-server
  - test-binary-support improves test reliability
- **Adoption plan**: thread-store and uds are must-haves. test-binary-support and thread-manager-sample are nice-to-have. Ensure thread-store is fully ported before testing other features.

---

## Part 2: Significant Changes in Shared Crates

### TUI Enhancements (codex-rs/tui/)

**Vim Composer Mode** (PR #18595)
- Modal Vim editing in the composer with `/vim` command
- Vim-specific keymap contexts and motion support
- Default-mode configuration for persistent editor choice
- User impact: Power users get a familiar editor environment

**Redesigned Resume/Fork Picker** (PR #20065)
- Improved UX for choosing which session to resume
- Faster filtering and preview
- Integrated into the session startup flow

**Raw Scrollback Mode** (PR #20819)
- View raw message history without formatting
- Useful for debugging and transcript export

**/ide Context Injection** (PR #20294)
- New `/ide` command to inject IDE context (code, diagnostics)
- Workspace-aware context gathering

**Workspace-Aware /diff** (PR #21001)
- `/diff` now understands workspace structure
- Better diff output formatting and navigation

**Status Line Enhancements** (PR #19631, #20892, #20794)
- Theme-aware colors in status line
- Optional PR summary display
- Branch change notifications
- `/keymap debug` command for terminal key event inspection

**Ctrl-C & Paste Handling** (PR #21091, #21190, #21351, #21397)
- Large paste placeholders survive clear/editor workflows
- Ctrl-C-stashed drafts persist correctly across operations
- Draft history no longer corrupts from paste cleanup

**Startup & Accessibility** (PR #20654, #21450, #20564)
- Bounded terminal probes to reduce startup latency
- Clear first inline viewport render to prevent stale text
- Enforce `animations = false` for screen readers

---

### Core Enhancements (codex-rs/core/)

**Tool Handler Refactoring** (PR #21395, #21416, #21427)
- Split tool handlers into separate files for clarity
- Moved tool specs into core handlers (less indirection)
- Deleted tool handler plan indirection (simplified flow)

**Thread Naming Migration** (PR #21260)
- Moved thread naming from core to app-server
- Enables naming even when state DB is absent (thread-store based)

**Lifecycle Hooks** (PR #19905, #19882)
- Hooks can run before/after compaction
- `/hooks` browser command to list and toggle hooks
- PreToolUse context for hooks to inject context before tool execution
- Hooks can now discover and discover themselves via plugin bundles (PR #19705)

**Compact Enhancements**
- Service tier propagation through compact (PR #21249)
- Cache key consistency across compact operations
- Memory-based goals now display multi-day duration output (PR #20558)

**MCP & Guardian Integration** (PR #19431, #19193, #19905)
- Codex Apps auth elicitations route through Guardian
- Eligible MCP servers surface auth needs through TUI flows
- Guardian approval workflow for sensitive operations

---

### Protocol & App-Server Refactoring

**Protocol Decomposition** (PR #20324, #20325, #20348, #20545)
- Extracted app-server-transport into dedicated crate
- Split protocol module into smaller, focused pieces
- Removed core protocol dependency (simplified imports)
- Item event mapping moved into app-server-protocol

**Thread & Identity Changes** (PR #20437, #21336, #21329, #21332)
- Session ID now part of protocol and return values
- Thread ID included in MCP turn metadata
- Session ID returned from thread/fork operations
- Installation ID resolution moved out of core startup (PR #21182)

**Turn Items & History** (PR #20540, #21063, #21278)
- Apply-patch file changes emitted as turn items
- Turn items view available in app-server
- Message history moved out of core (PR #21278)

**Model & Reasoning Metadata** (PR #20971, #20969, #20971, #21219)
- Model service tiers added to protocol (OpenAI, Claude, etc.)
- Reasoning effort metadata in compact/MCP turn data
- Service tier consistency across calls

---

## Part 3: TUI & User-Facing Features from Release Notes

### Slash Command Additions

- `/vim` — Enter modal Vim editing mode in composer
- `/ide` — Inject IDE context (open files, diagnostics)
- `/diff` — Workspace-aware diff display
- `/hooks` — Browse and toggle lifecycle hooks
- `/keymap debug` — Inspect terminal key events
- `/clear` — Now properly preserves Ctrl-C-stashed drafts

### Configuration Additions

- `default_mode = "vim"` — Persist Vim editor choice
- `animations = false` — Disable animations for accessibility/screen readers
- `model_providers` — User-defined model provider entries in config.toml

### Goal Enhancements

- Goals marked as experimental feature
- Paused goals stay paused across resume unless user opts back in
- Multi-day goal durations display clearly
- Goal validation improved (objective length constraints, PR #20746)

---

## Part 4: Upstream REVERTED Features (Critical for Merge)

### **agent-graph-store Injection Revert** (Commit a8488fec5e)

**What was reverted**: 
- Mandatory state DB injection into core ThreadManager
- agent-graph-store as a hard dependency for descendant lookup

**Why it was reverted**:
- Breaking change: made state DB mandatory when it should be optional
- Affected many consumers (app-server, MCP server, prompt debug, tests)
- Incompatible with process-scoped thread store (broke in-process client)

**What was kept**:
- Installation ID forwarding (newer feature kept)
- Session/thread identity changes (newer feature kept)
- Optional state DB handle fallback mechanism (restored)

**Impact for fork**:
- **DO NOT** add agent-graph-store as a hard dependency on core
- **IF** your fork attempted to add agent-graph-store, you must revert it using the same pattern (a8488fec5e)
- Descendant lookups should use optional state DB when available, not panic if absent
- Thread naming now works without state DB (thread-store based)

**Lines changed**: ~54 files, ~781 insertions, ~834 deletions

---

### **Skills Watcher Motion** (Reverts PR #21287, then commits PR #21460)

**What was attempted**: Move skills watcher from core to app-server for better concurrency

**Why it was reverted**: Caused integration issues with concurrent skills list loading

**What's in place now**: 
- Skills watcher stays in core (for now)
- Skills list CWD loading is parallelized (PR #21441) to improve performance
- App-server can read skills config, but core coordinates the watcher

**Impact for fork**: 
- Skills watcher remains in core; do not attempt to move it upstream
- Parallelized cwd loading is an optimization; can be applied if fork has similar code

---

## Part 5: Architecture & Breaking Changes

### Mandatory State DB → Optional State DB
- **Change**: ThreadManager, app-server, and other consumers now accept `Option<StateDbHandle>`
- **Why**: Enables in-process usage without SQLite dependency
- **Fork impact**: If fork hardcodes state DB, refactor to make it optional

### Message History Out of Core
- **Change**: Message history moved to separate crate
- **Why**: Simplifies core; history is a runtime feature, not a core concern
- **Fork impact**: Update imports from `codex-core::history` to `codex-message-history`

### Plugin Moved Out of Core
- **Change**: Plugin management into core-plugins crate
- **Why**: Reduces core coupling to plugin infrastructure
- **Fork impact**: Plugin code may need rearchitecting to follow upstream structure

### Transport Abstraction
- **Change**: App-server transport extracted into app-server-transport
- **Why**: Enables multiple transport implementations; simplifies testing
- **Fork impact**: If fork has custom transports, align with the new interface

---

## Part 6: New Top-Level Directories

Upstream added these top-level directories not present in fork:
- `.codex/` — Codex metadata (hooks, scripts)
- `.devcontainer/` — Dev container configuration
- `.github/` — GitHub Actions workflows
- `.vscode/` — VS Code workspace settings

These are typically configuration and metadata; bringing them in is optional but recommended for consistency.

---

## Summary: Merge Strategy

### Must-Have Crates (Bring in Full)
1. **core-api** — Needed for app-server refactoring
2. **model-provider**, **model-provider-info**, **models-manager** — Model infrastructure
3. **git-utils** — Git operations consolidation
4. **file-system**, **sandboxing** — Sandbox abstraction
5. **rollout**, **rollout-trace** — State persistence
6. **thread-store**, **uds** — Thread and socket infrastructure
7. **analytics** — Telemetry (can skip if you don't emit events)
8. **message-history** — History file management

### Should-Have Crates (Bring in If Fork Uses These Features)
9. **memories** (read, write, mcp) — Long-term memory system
10. **core-skills**, **core-plugins** — Plugin/skill centralization
11. **codex-mcp** — MCP coordination (likely necessary)
12. **builtin-mcps** — Built-in MCP servers
13. **external-agent-migration**, **external-agent-sessions** — Migration tooling
14. **exec-server**, **app-server-transport** — Execution abstraction
15. **device-key**, **terminal-detection** — Platform integration

### Can-Skip Crates (Advanced/Optional)
16. **realtime-webrtc** — Only if fork supports voice
17. **agent-graph-store** — Do NOT add; upstream reverted it
18. **response-debug-context** — Nice to have; not critical
19. **test-binary-support**, **thread-manager-sample** — Development utilities

### Architecture Changes to Adopt
- Make state DB optional everywhere (revert if you attempted mandatory injection)
- Extract message history out of core
- Extract plugin management into core-plugins
- Extract transport into app-server-transport
- Move thread naming to app-server

---

## Key Merge Checklist

- [ ] Remove agent-graph-store hard dependency (if fork has it)
- [ ] Make state DB optional in ThreadManager and app-server
- [ ] Extract message-history into separate crate
- [ ] Add core-api for cleaner public surface
- [ ] Bring in git-utils, file-system, sandboxing
- [ ] Port model provider infrastructure (3 crates)
- [ ] Update TUI with Vim mode, /hooks, /ide, /diff
- [ ] Refactor protocol and app-server transport
- [ ] Add memories (if fork supports background jobs + state DB)
- [ ] Test state DB optional behavior thoroughly
- [ ] Run full integration test suite (especially thread store + core)
- [ ] Verify Linux/Windows/macOS sandbox behavior

---

## Notes for Future Work

1. **Memories is large**: Plan 2-3 weeks to fully integrate if fork needs it
2. **State DB is subtle**: Making it truly optional requires careful testing across all consumers
3. **Plugin/Skill refactor ongoing**: Upstream still optimizing (skills watcher moved, then moved back); monitor for further changes
4. **Windows sandbox reliability**: Multiple fixes in PR #20270, #20685, #20336, #21409; test thoroughly on Windows
5. **Service tiers are new**: All model operations now propagate service tier metadata; ensure consistency in compact
6. **Standalone bwrap**: Improves Linux reliability; consider adopting standalone bwrap approach for npm releases

---

