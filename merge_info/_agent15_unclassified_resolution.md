## Wave-2 Unclassified Resolution + Core Cross-cuts (Agent 15)

Working tree: `/Users/huytho_ho/acli/ata`, branch `merge_upstream_0.129.0`, comparing local against tag `rust-v0.129.0`.

### 1. `codex-rs/tui/src/chatwidget/agent.rs` (60-line bootstrapper)
- **Classification:** Local-only (legacy bootstrap pattern; upstream uses different abstraction)
- **Local file:** 150 lines, 3 fns: `spawn_agent`, `spawn_agent_from_existing`, `spawn_op_forwarder`.
- **Upstream equivalent:** Does NOT exist as a separate file. Upstream's `chatwidget.rs` (11 210 lines vs local 10 518) replaced the `ThreadManager.start_thread(config) -> NewThread { thread, session_configured }` API with the `codex_app_server_protocol` flow. Upstream introduces enum `CodexOpTarget { Direct(UnboundedSender<AppCommand>), AppEvent }` (line 1041) and stores `codex_op_target: CodexOpTarget` on `ChatWidget`.
- **Merge plan:** Keep `agent.rs` for now. Wholesale migration to the app-server protocol is a Wave-3+ task.

### 2. `codex-rs/tui/src/chatwidget/interrupts.rs`
- **Classification:** Shared (upstream rewrote)
- **Local:** 105 lines. **Upstream:** 245 lines.
- **Difference:**
  - Local enum variants: `ExecApproval`, `ApplyPatchApproval`, `Elicitation`, `RequestPermissions`, `RequestUserInput`, `ExecBegin`, `ExecEnd`, `McpBegin`, `McpEnd`, `PatchEnd`.
  - Upstream collapsed `ExecBegin/ExecEnd/McpBegin/McpEnd/PatchEnd` (5 events) into 2 generic `ItemStarted/ItemCompleted` carrying `ThreadItem` from `codex_app_server_protocol`.
- **Merge plan:** Defer rewrite — coupled to AppServer migration.

### 3. `codex-rs/tui/src/bottom_pane/footer.rs`
- **Classification:** Shared (extended locally + upstream rewrote)
- **Diff:** 849 lines of churn
- **Local-only additions:** `FooterProps::context_window_percent`, `context_window_used_tokens`, `voice_mode_available`, `scheduler_enabled`, `mobile_available`, `research_enabled`. Shortcut overlay branches.
- **Upstream-only additions:** New struct `FooterKeyHints` (toggle_shortcuts, queue, insert_newline, external_editor, edit_previous, show_transcript, history_search, reasoning_down, reasoning_up). New enum `GoalStatusIndicator`. New `FooterMode::HistorySearch` variant.
- **Merge plan:** Rebase on upstream's new shape. Re-add local fields onto upstream's `FooterProps`.

### 4. `codex-rs/tui/src/bottom_pane/chat_composer.rs`
- **Classification:** Shared (extended locally; upstream did NOT split into subdir, only added `chat_composer/history_search.rs`)
- **Local file:** 9878 lines, no `chat_composer/` subdir
- **Upstream:** 10 468 lines + sibling `bottom_pane/chat_composer/history_search.rs`
- **Local-only:** `SkillPopup` import (line 180-181), `voice_mode_available: bool` field (line 413), `reverse_search` field (line 418), `voice_transcription_enabled()`, `handle_key_event_with_skill_popup`, `try_reverse_search_key`/`is_reverse_search_active`/`try_render_reverse_search_footer`, `voice_mode_available` setter, `include!("chat_composer_reverse_search.rs");` (line 4178).
- **Merge plan:** Pull upstream's `chat_composer/history_search.rs` as a new sibling. Keep local `chat_composer_reverse_search.rs` and `voice_*`/`SkillPopup` integration unchanged.

### 5. `codex-rs/tui/src/render/highlight.rs`, `render/renderable.rs`, `markdown_render.rs`
- **Classification:** All three are **Shared (upstream rewrote)** — purely additive upstream changes; no fork-specific features.
- **`render/highlight.rs`** (96-line diff): Upstream ADDED `pub(crate) fn foreground_style_for_scopes`.
- **`render/renderable.rs`** (101-line diff): Upstream added `cursor_style(&self, area: Rect) -> SetCursorStyle` method to the `Renderable` trait. Local has 2 dead-code helpers `push_ref` that upstream REMOVED.
- **`markdown_render.rs`** (75-line diff): Upstream added list/code-block blank-line interaction fix and URL-decodes path text in markdown links.
- **Merge plan:** Take upstream wholesale; drop the 2 `push_ref` dead-code helpers.

### 6. `chat_composer/` subdir layout
- Upstream files: just `history_search.rs`.
- Local: No subdir. Has `chat_composer_history.rs`, `chat_composer_reverse_search.rs` (`include!`'d), `reverse_search.rs`, `skill_popup.rs`.
- **Merge plan:** Add new file `bottom_pane/chat_composer/history_search.rs` from upstream. Do NOT relocate local files.

### 7. `codex-rs/core/src/codex/` (directory)
- **Classification:** Local-only / Shared (upstream rewrote heavily) — directory exists in both trees but with completely different filenames.
- **Local files:** `code_intel.rs` (677), `file_attachments.rs` (1308), `file_attachments/`, `mcp_startup.rs` (78), `response_events.rs` (711), `rollout_reconstruction.rs` (372), `rollout_reconstruction_tests.rs` (1291), `tests.rs` (3802), `url_file_recovery.rs` (198).
- **Upstream `core/src/session/` directory** (semantic equivalent): `config_lock.rs`, `handlers.rs`, `mcp.rs`, `mcp_tests.rs`, `mod.rs`, `multi_agents.rs`, `review.rs`, `rollout_reconstruction.rs`, `session.rs`, `tests.rs`, `tests/guardian_tests.rs`, `turn.rs`, `turn_context.rs`, `snapshots/`.
- **Difference:** Upstream renamed `codex` → `session` and split orchestrator across multiple files. Local kept the older flat `codex.rs` + adjacent `codex/` submodule pattern.
- **Local-only files:** `code_intel.rs`, `file_attachments.rs`, `mcp_startup.rs`, `response_events.rs`, `url_file_recovery.rs`.
- **Merge plan:** Keep local `codex/` as-is. The directory rename `codex/` → `session/` is the largest fork-divergence point — separate Wave (Wave-3 core-rename).

### 8. `codex-rs/core/src/codex.rs` (top-level orchestrator)
- **Classification:** Shared (upstream rewrote — file removed entirely)
- **Local file:** 7324 lines. **Upstream:** Does NOT exist. Functionality split across `core/src/session/{session,mod,turn_context,handlers}.rs`, `core/src/codex_thread.rs`, `core/src/codex_delegate.rs`.
- **Merge plan:** Major divergence — defer the full rename. Until then, manually translate import paths.

### 9. `codex-rs/core/src/state/`
- **Classification:** Shared (extended locally + upstream rewrote)
- **Local-only:** `state/multi_root.rs` — gated `#[cfg(any(feature = "lsp", feature = "treesitter"))]`.
- **Upstream-only additions in `state/turn.rs`:** `MailboxDeliveryPhase` enum, `RemovedTask` struct, `remove_task` returns `Option<RemovedTask>` instead of `bool`. Trait switched: `Arc<dyn SessionTask>` → `Arc<dyn AnySessionTask>`. `pending_request_permissions` value type changed. Renamed `granted_permissions: Option<PermissionProfile>` → `Option<AdditionalPermissionProfile>`. `strict_auto_review_enabled: bool` field added. `mailbox_delivery_phase: MailboxDeliveryPhase` field added. Local field `url_attachments_injected: usize` removed in upstream.
- **Merge plan:** Keep `multi_root.rs`. Port upstream's `MailboxDeliveryPhase`, `RemovedTask`, etc.

### 10. `codex-rs/core/src/thread_manager.rs`
- **Classification:** Shared (upstream rewrote heavily)
- **Local:** 833 lines. **Upstream:** 1449 lines. **Diff size:** 1281 lines.
- **Local-only:** retains older flat module imports plus `crate::data::SharedDataToolkit`, `crate::research::SharedResearchToolkit`.
- **Upstream:** Imports moved to extracted crates. New types: `ThreadHistoryBuilder`, `RefreshStrategy`, `SubAgentSource`, `ThreadSource`, `TurnAbortReason`, `TurnAbortedEvent`, `TurnEnvironmentSelection`, `InterruptedTurnHistoryMarker`, `SkillsWatcher`/`SkillsWatcherEvent`.
- **Merge plan:** Defer to crate-split wave. Cherry-pick `TurnAbortReason`, `TurnAbortedEvent`, `SubAgentSource`, `ThreadSource`.

### 11. `codex-rs/core/src/tools/spec.rs` and `tools/spec/`
- **Classification:** Local-only subdir + Shared (upstream rewrote)
- **Local file:** 2777 lines.
- **Local subdir `tools/spec/`** (4 files, fork-only): `agent_jobs.rs` (131), `integrations.rs` (156), `javascript.rs` (74), `workspace.rs` (420).
- **Upstream `tools/spec.rs`:** 169 lines only — extracted into `tools/spec_plan.rs`, `spec_plan_types.rs`, `spec_plan_tests.rs`, `hosted_spec.rs`.
- **Local-only fork features:** research toolkit integration, `is_research_tool_enabled`, `build_specs_with_toolkits_and_external`.
- **Merge plan:** Keep `tools/spec/` subdir. Do NOT migrate to upstream's split now.

### 12. `codex-rs/core/src/tools/router.rs`
- **Classification:** Shared (extended locally + upstream rewrote)
- **Local:** 434 lines. **Upstream:** 334 lines. **Diff size:** 439 lines.
- **Local-only injections:** Functions accept `research_toolkit: Option<&Arc<SharedResearchToolkit>>` and `data_toolkit: Option<&Arc<SharedDataToolkit>>` parameters.
- **Upstream additions:** `parallel_mcp_server_names`, `unavailable_called_tools`, `deferred_mcp_tools`, `ToolName` (replacing `String`/`tool_namespace`).
- **Merge plan:** Keep local research/data toolkit injection. Port upstream's `parallel_mcp_server_names` and `unavailable_called_tools` plus `ToolName`.

### 13. `codex-rs/core/src/openai_tools.rs`
- **Classification:** N/A — file does not exist in either tree.

### 14. Local-only additions in `chatwidget/` not yet covered
- `voice_mode.rs` (6572 lines) — entirely fork-only.
- `skills.rs` (454 lines) — fork-specific skill UX surface.
- `session_header.rs` (15 lines) — local stub; upstream is much larger.

### 15. Upstream-only `chatwidget/` files that should be pulled
- `goal_menu.rs`, `goal_status.rs`, `goal_validation.rs`, `hooks.rs`, `ide_context.rs`, `keymap_picker.rs`, `mcp_startup.rs` (TUI-side), `plan_implementation.rs`, `plugins.rs`, `reasoning_shortcuts.rs`, `side.rs`, `slash_dispatch.rs`, `status_surfaces.rs`, `user_messages.rs`, `warnings.rs`.
- **Merge plan:** Pull these new modules wholesale. Some may not compile cleanly until item 1 (app-server migration) lands. Prioritize `slash_dispatch.rs`, `user_messages.rs`, `warnings.rs` (lower coupling) first.

### 16. `chat_composer_history.rs` divergence
- **Local:** 435 lines. **Upstream:** 1334 lines.
- **Merge plan:** Big upstream rewrite; needs its own pass.

### 17. Lib.rs module shape divergence
- **Local:** `pub mod codex;`, `mod state;`, `mod thread_manager;`, etc.
- **Upstream:** `pub(crate) mod session;`, plus new modules: `compact_remote_v2`, `session_startup_prewarm`, `session_rollout_init_error`.
- **Merge plan:** Add upstream's new modules. Defer the `pub mod codex` → `pub(crate) mod session` rename.

### 18. Tools dir — local-only files not in upstream
- Local-only: `tools/code_mode_description.rs`, `tools/discoverable.rs`, `tools/file_injection.rs`, `tools/js_repl/`, `tools/pdfium_downloader.rs`, `tools/url_downloader.rs`, `tools/url_validation.rs`, `tools/spec/` subdir.
- Upstream-only: `tools/code_mode/{execute_handler,execute_spec,response_adapter,wait_handler,wait_spec,mod}.rs`, `tools/hook_names.rs`, `tools/hosted_spec.rs`, `tools/runtimes/`, `tools/spec_plan.rs`, `tools/spec_plan_types.rs`, `tools/tool_dispatch_trace.rs`.

### Summary of merge priorities

| Priority | Item | Action |
|---|---|---|
| P0 (take wholesale) | render/highlight, render/renderable, markdown_render | Pure upstream improvements |
| P1 (rebase) | bottom_pane/footer.rs | Take upstream's `FooterKeyHints`/`GoalStatusIndicator`/`HistorySearch`; re-add local feature flags |
| P1 (additive) | chatwidget/{slash_dispatch, user_messages, warnings}.rs and chat_composer/history_search.rs | Pull new files |
| P2 (defer) | agent.rs, interrupts.rs, thread_manager.rs, codex.rs, state/turn.rs, tools/spec.rs, tools/router.rs | Coupled to AppServer migration / `codex` → `session` rename |
| Keep fork-only | tools/spec/{agent_jobs,integrations,javascript,workspace}.rs, state/multi_root.rs, codex/{code_intel,file_attachments,mcp_startup,response_events,url_file_recovery}.rs, voice_mode.rs | Local-only features |
