# Skills & Hooks System: Fork vs Upstream Divergence Analysis

**Comparison Target:** Local fork vs upstream `rust-v0.129.0`

**Key Areas:** Skill registry, skill packs, skill discovery, skill execution, hooks system (pre/post tool, on-event, compaction hooks), hooks UI.

---

## Overview

The fork has **split the skills and hooks systems** into separate architectural layers compared to upstream:

- **Local fork:** Skills live in `codex-rs/skills/` (crate) + `codex-rs/core/src/skills/` (integration). Hooks in `codex-rs/hooks/` (crate) + session lifecycle in core.
- **Upstream:** Skills integrated into `codex-rs/core-skills/` (dedicated crate) with plugins handled separately in `codex-rs/core-plugins/` (separate crate). Hooks are also in `codex-rs/hooks/` but with more event types and integration.

The **plugin system is significantly different:** upstream has a unified `codex-rs/core-plugins/` managing all plugin marketplaces, remote sync, sharing, and admin controls. The local fork embeds plugin logic into `codex-rs/core/src/plugins/` with less coverage of upstream's marketplace/remote features.

---

## Feature Inventory

### A. Features ONLY in Our Fork

#### 1. **Custom Skill Categories (Research, Workspace, Adapt-Environment)**

- **Name:** Custom embedded skill categories with fingerprinting
- **Description:** The fork extends the upstream's `.system` skill category with three additional categories: research skills, workspace skills, and adapt-environment skills. Each category is compiled into the binary, fingerprinted, and installed to separate `.system-*` directories.
- **Implementation Summary:**
  - Key file: `codex-rs/skills/src/lib.rs` (lines 14-201)
  - Custom category registration via `CUSTOM_SKILL_CATEGORIES_BASE` array (lines 54-70)
  - Support for `ata-plus` feature flag to add private `remote-exec` skills (lines 72-77, 181-183)
  - Backward-compatible API preserved: `install_research_skills()`, `install_workspace_skills()` (lines 161-173)
  - Fingerprinting uses content hashing per category with unique salt
  - Auto-discovered via `custom_skill_cache_root_dirs()` passed to loader
- **Status vs Upstream:** **Local-only.** Upstream only has `.system` category. This fork customization provides ATA-specific skill bundling without modifying core skill loading logic.
- **Merge Plan:** Keep as-is. This is a non-breaking extension that doesn't conflict with upstream's single-category approach. The fork's backward-compatible API (`install_research_skills`, `install_workspace_skills`) wraps the generic category system, so it survives upstream merges. Ensure the `.system` category still installs alongside custom categories on every startup.

---

#### 2. **ATA-Plus Private Remote-Exec Skills**

- **Name:** Private remote execution skill pack (feature-gated)
- **Description:** Conditional compilation of a private `remote-exec` skill directory for enterprise/plus tier users. Guarded by `#[cfg(feature = "ata-plus")]` and stripped on public release branch.
- **Implementation Summary:**
  - `codex-rs/skills/src/lib.rs` lines 37-39, 72-77
  - `codex-rs/skills/build.rs` includes conditional build artifacts
  - Released code skips the directory; Codex home gracefully continues without it
- **Status vs Upstream:** **Local-only private feature.** Upstream doesn't segment skills by tier.
- **Merge Plan:** No conflict. The feature gate ensures upstream builds don't break. Upstream merges proceed normally; `remote-exec` directory remains stripped on public release.

---

#### 3. **Research, Workspace, and Adapt-Environment Skill Assets**

- **Name:** ATA-specific embedded skill packs
- **Description:** Pre-built SKILL.md bundles in `codex-rs/skills/src/assets/{research,workspace,adapt-environment}/` providing domain-specific agent guidance:
  - **Research skills:** Academic paper tools (cross-paper-report, etc.)
  - **Workspace skills:** Project/workspace management (GitHub, PR tracking, file sync)
  - **Adapt-Environment skills:** Environment discovery, dependency management
- **Implementation Summary:**
  - Directories: `codex-rs/skills/src/assets/{research,workspace,adapt-environment}/`
  - Each contains SKILL.md + agents/ + scripts/ + references/
  - Installed via `install_custom_skills()` at startup (manager.rs lines 60-61)
  - Discoverable automatically via loader's skill roots
- **Status vs Upstream:** **Local-only.** Upstream ships `samples/` skills only (imagegen, openai-docs, skill-creator, skill-installer, plugin-creator). Fork adds research/workspace/adapt-environment.
- **Merge Plan:** Separate concern from skill system architecture. Keep research/workspace/adapt-environment in fork's `skills/src/assets/`. Upstream's samples are orthogonal. On merge, add upstream's new `plugin-creator` sample if desired, but don't discard custom categories.

---

### B. Features in BOTH Fork & Upstream (With Implementation Differences)

#### 1. **Skill Loading and Discovery**

- **Name:** Core skill loader and registry
- **Description:** Discover and load skills from filesystem roots, parse SKILL.md metadata, manage skill state in config layers, filter by policy/scope.
- **Implementation Summary - Local Fork:**
  - `codex-rs/core/src/skills/loader.rs` — loads from roots, parses metadata
  - `codex-rs/core/src/skills/manager.rs` — caches outcomes, coordinates with plugins
  - `codex-rs/skills/src/lib.rs` — manages system & custom category installation
  - Skill roots from: user config, project config, plugin skill paths, custom categories
  - Caching by cwd and by effective config state (config_skills_cache_key)
- **Implementation Summary - Upstream:**
  - `codex-rs/core-skills/src/loader.rs` (separate crate)
  - Integration via `codex_core_skills::*` re-exports in `codex-rs/core/src/skills.rs`
  - Upstream uses `PluginSkillRoot` type from `codex_utils_plugins`
  - Config stack filtering: user/session layers only, respects disabled layers
- **Status vs Upstream:** **Both exist; local is embedded, upstream is separated.** Functionally equivalent for basic discovery; upstream has cleaner separation of concerns via `core-skills` crate.
- **Merge Plan:** Upstream's `core-skills` crate is architecturally superior (dedicated, testable, reusable). Consider adopting upstream's structure: extract local skill logic into a `codex-rs/core-skills/` crate, then have `codex-rs/core/src/skills.rs` re-export from it, matching upstream. Keep ATA's custom categories by extending upstream's `system.rs` with category plugin points. This reduces fork divergence long-term.

---

#### 2. **Skill Injection and Rendering**

- **Name:** Skill availability & context rendering
- **Description:** Inject skills into agent prompts, filter by scope/policy, render as context sections, handle implicit invocations.
- **Implementation Summary - Local Fork:**
  - `codex-rs/core/src/skills/injection.rs` — build skill injections, collect mentions
  - `codex-rs/core/src/skills/render.rs` — format skills for agent prompt
  - `codex-rs/core/src/skills/invocation_utils.rs` — detect implicit skill paths, emit invocations
  - Scope filtering: system/user/plugin scopes
  - Policy: skill policies applied per-skill
- **Implementation Summary - Upstream:**
  - `codex-rs/core-skills/src/injection.rs` (same logical structure)
  - `codex-rs/core-skills/src/render.rs`
  - Re-exported in `codex-rs/core/src/skills.rs`
- **Status vs Upstream:** **Both exist; functionally equivalent.** Upstream's are cleaner due to crate isolation.
- **Merge Plan:** If adopting upstream's `core-skills` crate structure, this logic moves with it. No functional change needed; just file location.

---

#### 3. **Skill System Installation & Uninstall**

- **Name:** Embedded skill bootstrap
- **Description:** Install `.system` skills from embedded binary on startup, check fingerprints to avoid repeated writes, clear on config toggle.
- **Implementation Summary - Local Fork:**
  - `codex-rs/skills/src/lib.rs` — system & custom skill installation (lines 88-251)
  - `codex-rs/core/src/skills/system.rs` — re-exports + integration points
  - Marker file: `.codex-system-skills.marker` with fingerprint hash
  - Clears & reinstalls only if fingerprint mismatch
  - Called once at startup via `SkillsManager::new()` (manager.rs lines 57-68)
- **Implementation Summary - Upstream:**
  - Similar structure in `codex-rs/core-skills/src/system.rs`
  - Upstream only has `.system` category (no custom categories)
- **Status vs Upstream:** **Both exist; local adds custom category variants.**
- **Merge Plan:** Keep as-is; custom categories are additive and don't conflict with upstream's `.system` install.

---

#### 4. **Hooks System - Core Types & Registry**

- **Name:** Hook event types, payload structure, registry, dispatcher
- **Description:** Define hook lifecycle (HookEvent enum), payload schema, hook execution, registry of configured handlers.
- **Implementation Summary - Local Fork:**
  - `codex-rs/hooks/src/types.rs` — HookEvent, HookPayload, HookResult enums
  - `codex-rs/hooks/src/registry.rs` — Hooks config, command dispatch
  - `codex-rs/hooks/src/engine/mod.rs` — dispatcher & handler selection
  - `codex-rs/hooks/src/lib.rs` — public API exports
  - Events supported: `AfterAgent`, `AfterToolUse` (in types.rs lines 149-158)
- **Implementation Summary - Upstream:**
  - Same core types in `codex-rs/hooks/src/types.rs`
  - Upstream expands HookEvent enum with: `PreToolUse`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`, `PermissionRequest`
  - Additional files: `codex-rs/hooks/src/config_rules.rs` — hook state from config layers
  - `codex-rs/hooks/src/output_spill.rs` — truncate large hook outputs, spill to temp
  - Engine extensions for matching/dispatching to all event types
- **Status vs Upstream:** **Local is **subset** of upstream.** Local has basic types & registry; upstream adds 4+ event types and output management.
- **Merge Plan:** **Adopt upstream's full event set.** Merge the missing event types (PreToolUse, PostToolUse, PreCompact, PostCompact, UserPromptSubmit) into local types.rs. Integrate output_spill.rs and config_rules.rs into hooks crate. This is additive and doesn't break existing AfterAgent/AfterToolUse usage.

---

#### 5. **Hooks Events - Session Start & Stop**

- **Name:** Lifecycle hook events (session start, session stop)
- **Description:** Trigger hooks when a session starts or stops, passing session context.
- **Implementation Summary - Local Fork:**
  - `codex-rs/hooks/src/events/session_start.rs` — SessionStartRequest, SessionStartOutcome
  - `codex-rs/hooks/src/events/stop.rs` — StopRequest, StopOutcome
  - Both handled via shell command dispatch, output parsing
- **Implementation Summary - Upstream:**
  - Same files and structure
  - Identical logic for session_start and stop
- **Status vs Upstream:** **Identical.**
- **Merge Plan:** No action needed; these merge cleanly.

---

#### 6. **Plugin System - Manager, Marketplace, Manifest**

- **Name:** Plugin discovery, installation, marketplace, manifest handling
- **Description:** Load plugins from manifests, install from marketplaces, manage enabled/disabled state, render plugin capabilities.
- **Implementation Summary - Local Fork:**
  - `codex-rs/core/src/plugins/manager.rs` — plugin loading & state
  - `codex-rs/core/src/plugins/marketplace.rs` — marketplace operations
  - `codex-rs/core/src/plugins/manifest.rs` — manifest parsing
  - `codex-rs/core/src/plugins/curated_repo.rs` — OpenAI curated repo sync
  - `codex-rs/core/src/plugins/store.rs` — plugin ID/persistence
  - `codex-rs/core/src/plugins/render.rs` — plugin instructions
  - ~14 files total in `codex-rs/core/src/plugins/`
  - Features: basic install, disable/enable toggles, manifest reading
- **Implementation Summary - Upstream:**
  - `codex-rs/core-plugins/` — dedicated crate (separate from core)
  - ~23 .rs files covering:
    - `manager.rs` — expanded plugin state, `ConfiguredMarketplacePlugin`, rich details
    - `marketplace.rs`, `marketplace_add.rs`, `marketplace_remove.rs`, `marketplace_upgrade.rs`
    - `remote.rs`, `remote_bundle.rs`, `startup_remote_sync.rs` — workspace sharing, remote sync, bundle updates
    - `store.rs`, `toggles.rs`, `installed_marketplaces.rs` — enhanced state mgmt
    - **Not in local:** `marketplace_remove()`, `marketplace_upgrade()`, `remote_bundle` sync, workspace share, admin-disabled status
- **Status vs Upstream:** **Local is a **subset** of upstream.** Upstream has marketplace removal/upgrade, remote bundle sync, workspace plugin sharing, admin-disabled status.
- **Merge Plan:** **Upstream's `core-plugins` crate is significantly more capable.** Local fork should adopt it:
  1. Replace `codex-rs/core/src/plugins/` with reference to `codex-rs/core-plugins/` re-exports
  2. Add marketplace_remove, marketplace_upgrade, remote_bundle, and workspace sharing features
  3. Maintain any ATA-specific plugin discovery customization (e.g., curated_repo local overrides) via plugins module extension points
  4. This is a substantial refactor but necessary to match upstream's plugin capabilities and reduce divergence.

---

#### 7. **Plugin Crate (Metadata & Namespacing)**

- **Name:** Plugin type definitions, plugin ID, plugin namespace, load outcome types
- **Description:** Define plugin metadata structures, plugin ID type, namespace (marketplace@namespace), plugin load results.
- **Implementation Summary - Local Fork:**
  - **Not in local.** Local doesn't have a dedicated `plugin/` crate.
  - Plugin info embedded in `codex-rs/core/src/plugins/` types
- **Implementation Summary - Upstream:**
  - `codex-rs/plugin/src/lib.rs` — LoadedPlugin<T>, PluginLoadOutcome<T>, LoadOutcomeByPlugin, load_outcome
  - `codex-rs/plugin/src/plugin_id.rs` — PluginId type
  - `codex-rs/plugin/src/plugin_namespace.rs` — Namespace type & parsing
  - `codex-rs/plugin/src/load_outcome.rs` — Detailed outcome enum
  - Generic <T> for config type allows reuse across contexts (MCP, skills, etc.)
- **Status vs Upstream:** **Local doesn't have it; upstream does.** Upstream's `plugin/` crate is a clean abstraction layer.
- **Merge Plan:** Adopt upstream's `codex-rs/plugin/` crate to provide type definitions. Update local plugin manager to use these types. This improves type safety and allows sharing plugin metadata across skills and plugin contexts.

---

#### 8. **Hook Events - Pre/Post Tool Use** (MISSING in LOCAL)

- **Name:** Pre and post tool use hooks
- **Description:** Hooks that run before/after a tool is executed, allowing interception, logging, or modification of tool inputs/outputs.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/events/pre_tool_use.rs` — PreToolUseRequest, PreToolUseOutcome; blocks or allows tool; can inject additional context
  - `codex-rs/hooks/src/events/post_tool_use.rs` — PostToolUseRequest, PostToolUseOutcome; logs results
  - Handlers selected by tool name matching (matcher_aliases)
  - PreToolUse can block tool execution and provide additional_contexts to model
  - PostToolUse provides tool output for logging/processing
- **Status vs Upstream:** **Upstream only.** Local doesn't have these events.
- **Merge Plan:** **Add to local.** Implement `codex-rs/hooks/src/events/pre_tool_use.rs` and `post_tool_use.rs` following upstream's structure. Update HookEvent enum to include these variants. Integrate dispatch logic into engine/dispatcher.rs. This enables hook-based tool authorization and monitoring in ATA.

---

#### 9. **Hook Events - Pre/Post Compaction** (MISSING in LOCAL)

- **Name:** Pre and post compaction hooks
- **Description:** Hooks that run before/after context compaction (history summarization), allowing inspection or modification of compaction triggers.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/events/compact.rs` — PreCompactRequest, PostCompactRequest, PreCompactOutcome, PostCompactOutcome
  - PreCompact can block compaction; PostCompact logs result
  - Called from `codex-rs/core/src/compact.rs` via `run_pre_compact_hooks()`, `run_post_compact_hooks()`
- **Status vs Upstream:** **Upstream only.** Local doesn't have these events.
- **Merge Plan:** **Add to local.** Implement `codex-rs/hooks/src/events/compact.rs` and integrate with local compaction logic. This requires:
  1. Create events/compact.rs module
  2. Add PreCompact, PostCompact variants to HookEvent
  3. Call hooks from codex-rs/core/src/compact.rs before/after compaction runs
  4. This enables observability and control over compaction lifecycle.

---

#### 10. **Hook Events - User Prompt Submit** (MISSING in LOCAL)

- **Name:** User prompt submit hook
- **Description:** Hook that runs when user submits a prompt, allowing inspection, modification, or rejection of the prompt before agent processes it.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/events/user_prompt_submit.rs` — UserPromptSubmitRequest, UserPromptSubmitOutcome
  - Can block submission or inject additional_contexts
- **Status vs Upstream:** **Upstream only.** Local doesn't have this event.
- **Merge Plan:** **Add to local.** Implement user_prompt_submit.rs and integrate with turn submission logic. This enables prompt validation hooks.

---

#### 11. **Hook Events - Permission Request** (MISSING in LOCAL)

- **Name:** Permission request hook
- **Description:** Hook triggered when sandbox permission is requested, allowing custom approval/denial logic.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/events/permission_request.rs` — PermissionRequestEvent, outcome handling
- **Status vs Upstream:** **Upstream only.**
- **Merge Plan:** **Add to local if ATA needs custom permission logic.** Less critical than tool/compact hooks for merge priority.

---

#### 12. **Hooks Output Spill Management** (MISSING in LOCAL)

- **Name:** Large hook output truncation and spillover
- **Description:** When hook output exceeds token budget (2,500 tokens), write full text to temp dir and replace with truncated preview + path reference. Prevents unbounded hook output growth in prompt.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/output_spill.rs` — HookOutputSpiller struct, maybe_spill_text(), maybe_spill_texts(), maybe_spill_prompt_fragments()
  - Spills to `/tmp/hook_outputs/<thread_id>/` on filesystem
  - Used by hook outcome rendering to cap model-visible hook output
  - Related: `codex-rs/hooks/src/output_spill_tests.rs`
- **Status vs Upstream:** **Upstream only.**
- **Merge Plan:** **Add to local.** Implement output_spill.rs to prevent large hook outputs from bloating context. This is defensive and avoids regressions if users write verbose hooks.

---

#### 13. **Hooks Configuration Rules** (MISSING in LOCAL)

- **Name:** Hook state persistence and config layer integration
- **Description:** Read hook enablement/trust state from config layers (user config, session flags), merge field-by-field to allow granular overrides without full state replacement.
- **Implementation Summary - Upstream Only:**
  - `codex-rs/hooks/src/config_rules.rs` — hook_states_from_stack(), respects ConfigLayerStack precedence
  - Only user/session layers can set hook state; project/managed layers discover but don't override
  - Field-by-field merging: enabled, trusted_hash
- **Status vs Upstream:** **Upstream only.**
- **Merge Plan:** **Add to local.** Implement config_rules.rs to allow users to enable/disable/trust hooks via config. This integrates hooks with the existing config precedence system.

---

#### 14. **Hooks TUI Browser** (MISSING in LOCAL)

- **Name:** TUI `/hooks` command for browsing and toggling hooks
- **Description:** Interactive terminal UI to list configured hooks, toggle enabled/disabled state, view hook details.
- **Implementation Summary - Upstream Only:**
  - Mentioned in commit 2808a4deb1 as "Hooks can be browsed and toggled from `/hooks`"
  - Likely in `codex-rs/tui/src/hook*.rs` files (not yet examined)
- **Status vs Upstream:** **Upstream only.** Local TUI doesn't have `/hooks` view.
- **Merge Plan:** **Add to local TUI if ATA needs hook management UI.** Lower priority for core merge; can follow in TUI-specific PR.

---

## Upstream Features Not Yet in Local (Summary Table)

| Feature | Type | Status | Local Impact |
|---------|------|--------|--------------|
| core-plugins crate | Plugin system | Upstream only | Significant missing functionality (marketplace removal, upgrade, remote sync, sharing) |
| plugin crate | Type definitions | Upstream only | Infrastructure; enables type-safe plugin metadata |
| Pre/Post Tool Use hooks | Hooks event | Upstream only | Missing; enables tool authorization hooks |
| Pre/Post Compaction hooks | Hooks event | Upstream only | Missing; enables compaction lifecycle observability |
| User Prompt Submit hook | Hooks event | Upstream only | Missing; enables prompt validation hooks |
| Permission Request hook | Hooks event | Upstream only | Missing; enables custom permission logic |
| Hook output spill | Hooks output mgmt | Upstream only | Missing; prevents unbounded output growth |
| Hook config rules | Hooks config | Upstream only | Missing; enables hook state persistence |
| Hooks TUI browser | Hooks UI | Upstream only | Missing; enables interactive hook management |
| Marketplace removal/upgrade | Plugin mgmt | Upstream only | Missing; critical for plugin lifecycle |
| Remote plugin bundle sync | Plugin mgmt | Upstream only | Missing; enables workspace plugin sharing |
| Admin-disabled plugin status | Plugin mgmt | Upstream only | Missing; enables admin controls |

---

## Local Features Not in Upstream (Summary Table)

| Feature | Type | Status | Upstream Impact |
|---------|------|--------|-----------------|
| Research skills | Skill pack | Local only | ATA-specific; not upstream concern |
| Workspace skills | Skill pack | Local only | ATA-specific; not upstream concern |
| Adapt-environment skills | Skill pack | Local only | ATA-specific; not upstream concern |
| Custom skill categories system | Architecture | Local only | Non-breaking extension; clean API |
| ATA-Plus private remote-exec skills | Feature | Local only | Gated by feature flag; not upstream concern |

---

## Merge Strategy Recommendations

### High Priority (Core Functionality)
1. **Add missing hook events:** Pre/Post Tool Use, Pre/Post Compaction, User Prompt Submit
   - **Why:** Enables hook-based tool authorization and compaction observability
   - **Effort:** Moderate (~500 LOC per event, follow upstream structure)
   - **Risk:** Low; additive changes, no breaking changes

2. **Adopt upstream's core-plugins crate:**
   - **Why:** Marketplace removal, upgrade, remote sync, workspace sharing are significant gaps
   - **Effort:** High; major refactor of `codex-rs/core/src/plugins/` to use upstream's structure
   - **Risk:** Medium; requires integration testing; plugin tests must pass
   - **Approach:** Adopt upstream's crate structure; extend with ATA-specific plugin discovery

3. **Add hook output_spill and config_rules:**
   - **Why:** Defensive; prevents unbounded hook output growth and enables hook state persistence
   - **Effort:** Low (~200 LOC total)
   - **Risk:** Low; well-contained

### Medium Priority (Enhanced Features)
4. **Adopt upstream's plugin crate:**
   - **Why:** Type-safe plugin metadata; enables code reuse across contexts
   - **Effort:** Low; mostly type updates
   - **Risk:** Low; improves type safety

5. **Add Hooks TUI browser:**
   - **Why:** Enables interactive hook management
   - **Effort:** Medium; TUI-specific work
   - **Risk:** Low; UI-only feature
   - **Timing:** Can follow main merge as separate TUI PR

### Lower Priority (Local Enhancements)
6. **Keep custom skill categories:**
   - **Why:** Non-breaking; ATA-specific value
   - **Status:** Already compatible; no changes needed
   - **Long-term:** Consider upstreaming if other projects benefit

---

## Architecture Observations

### Crate Structure Divergence

**Local fork:**
- Skills logic in `codex-rs/skills/` (system/custom installer) + `codex-rs/core/src/skills/` (integration)
- Plugin logic in `codex-rs/core/src/plugins/` (embedded in core)
- Hooks in `codex-rs/hooks/` (events only; dispatch minimal)

**Upstream:**
- Skills logic in `codex-rs/core-skills/` (dedicated, reusable crate)
- Plugin logic in `codex-rs/core-plugins/` (dedicated crate) + `codex-rs/plugin/` (type layer)
- Hooks in `codex-rs/hooks/` (full event system)

**Recommendation:** Upstream's separation is cleaner and more maintainable. On merge, extract local skill logic into `codex-rs/core-skills/` and plugin logic into `codex-rs/core-plugins/` to match. This reduces long-term divergence.

---

## Implementation Checklist for Merge

### Phase 1: Hook Events (Immediate)
- [ ] Add `pre_tool_use.rs` event module (follow upstream structure)
- [ ] Add `post_tool_use.rs` event module
- [ ] Add `compact.rs` event module (pre/post)
- [ ] Add `user_prompt_submit.rs` event module
- [ ] Update HookEvent enum to include all variants
- [ ] Update engine/dispatcher.rs to handle all event types
- [ ] Add `output_spill.rs` for large output truncation
- [ ] Add `config_rules.rs` for hook state from config layers
- [ ] Tests: verify all events dispatch and execute correctly

### Phase 2: Hook Integration (Weeks 1-2)
- [ ] Integrate pre/post tool use hooks into core tool execution (likely in session/turn logic)
- [ ] Integrate pre/post compaction hooks into compact.rs
- [ ] Integrate user prompt submit hooks into turn submission
- [ ] Add hook event previews to TUI live rows (if planned)
- [ ] Smoke test: hooks fire and execute correctly in real sessions

### Phase 3: Plugin System (Weeks 2-4)
- [ ] Adopt upstream's `core-plugins` crate structure
- [ ] Implement marketplace_remove, marketplace_upgrade
- [ ] Implement remote_bundle sync and workspace sharing
- [ ] Implement admin-disabled plugin status handling
- [ ] Update PluginsManager API to match upstream
- [ ] Tests: marketplace operations, remote sync, sharing flows

### Phase 4: Plugin Crate (Week 4)
- [ ] Adopt upstream's `plugin/` crate (PluginId, Namespace, LoadOutcome types)
- [ ] Update plugin manager to use plugin crate types
- [ ] Verify type safety improvements

### Phase 5: Skill System (Week 5, if needed)
- [ ] Extract local skill logic into `codex-rs/core-skills/` crate (optional; aligns with upstream but not blocking)
- [ ] Preserve custom skill category system via extension points
- [ ] Update re-exports in `codex-rs/core/src/skills.rs`

### Phase 6: TUI Hooks Browser (Week 6+)
- [ ] Add `/hooks` command to TUI (lower priority; can follow main merge)
- [ ] List configured hooks, toggle enabled/disabled
- [ ] Show hook details and run summaries

---

## Risk Mitigation

1. **Plugin system refactor:** High-risk due to scope. Mitigate by:
   - Adopt upstream's structure in a feature branch first
   - Run existing plugin tests against new structure
   - Test marketplace install/remove/upgrade flows end-to-end
   - Verify workspace sharing (if ATA uses it) works

2. **Hook events dispatch:** Medium-risk if handlers are wired incorrectly. Mitigate by:
   - Add unit tests for each event's dispatcher matching
   - Run sessions with hooks enabled and verify event payloads

3. **Custom skill categories:** Low-risk; non-breaking. Just ensure they install alongside `.system` on startup.

---

## File Structure Summary

### Key Local Files
- `codex-rs/skills/src/lib.rs` — custom category system
- `codex-rs/core/src/skills/` — loader, manager, injection, render
- `codex-rs/hooks/src/` — types, registry, events (session_start, stop)
- `codex-rs/core/src/plugins/` — plugin manager (incomplete vs upstream)

### Key Upstream Files to Adopt
- `codex-rs/core-plugins/src/` — marketplace ops, remote sync, sharing
- `codex-rs/plugin/src/` — type definitions
- `codex-rs/hooks/src/events/{pre_tool_use,post_tool_use,compact,user_prompt_submit,permission_request}.rs`
- `codex-rs/hooks/src/{config_rules,output_spill}.rs`

---

## Conclusion

The fork and upstream have **diverged significantly** in the plugin system (core-plugins crate missing in local) and hooks implementation (4+ event types missing). The skill system is functionally equivalent but architecturally different.

**Merge path:**
1. Add missing hook events and output management (high priority, low risk)
2. Adopt upstream's core-plugins and plugin crates (high priority, medium risk)
3. Preserve ATA's custom skill categories (already compatible, low risk)
4. Optionally extract skill logic into core-skills crate (medium priority, aligns with upstream)

These changes reduce fork divergence and unlock upstream's advanced plugin marketplace and hook lifecycle features for ATA.
