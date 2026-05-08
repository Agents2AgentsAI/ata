# TUI Module Divergence Analysis: Fork vs Upstream (rust-v0.129.0)

## Executive Summary

The local fork diverges significantly from upstream (rust-v0.129.0) with **194,409 lines of diff** across the TUI module. Major categories of divergence:

- **Features local-only**: Voice mode (TTS/STT with karaoke), reading view (document reader), mobile control daemon, research tools, remote control/discovery, voice-activated speech-to-text.
- **Features in both**: Slash commands (reorganized), chat composer (restructured), theme picker, keymaps, status line, approval/permission flows, model pickers, onboarding (expanded with provider picker).
- **Removed upstream features**: `/keymap` debug picker, `/vim` mode, `/goal`, `/hooks`, `/memories`, `/side` conversations, `/approve`, `/raw` mode, `/title` setup, auto-review denials view.
- **Major structural changes**: Massive app.rs refactor (from 8665+ lines changed), deletion of 246 files, addition of 32 files, reorganization of event dispatch and session management.

---

## Part A: Local-Only Features

### 1. Voice Mode (Speech-to-Text & Karaoke)

**Name:** Voice Mode (TTS Input + Karaoke Playback)

**Description:**
Full speech-to-text and text-to-speech integration for ATA:
- Spacebar hold-to-talk in chat composer (transcription)
- Karaoke playback of agent responses with word-level highlighting
- Voice-specific UI (record/play placeholders, spinner states, pause markers)
- Configurable TTS/STT settings via `/voice-setup` command
- Voice setup form with API key entry, language/speed controls
- Platform-specific: macOS only (Linux gated with `cfg`)

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/voice.rs` (~1300 lines) — core `VoiceCapture` struct, audio device enumeration, WAV encoding, transcription auth
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/chatwidget/voice_mode.rs` — integration hooks for recording, karaoke display
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/voice_setup_view.rs` (~880 lines) — `/voice-setup` form with language/speed pickers, API key input
- `codex-rs/tui/src/bottom_pane/chat_composer.rs` — `VoiceState` struct tracking hold-to-talk, space key repeats, recording placeholders
- Voice tags in text (e.g., `◆ karaoke` prefix, pause markers `◆PAUSE`)

**Status vs Upstream:**
- **Local-only**, no upstream equivalent
- Upstream has minimal `/settings` for realtime audio, but no composition-level voice input

**Merge Plan:**
Feature is entirely new and self-contained. Upstream should retain its lighter realtime audio feature; local voice mode adds a separate input pathway. No conflicts expected if keeping both; implement feature gates to allow upstream to build without voice dependencies.

---

### 2. Reading View (Document Reader & Sectioned Display)

**Name:** Reading View / Document Reader

**Description:**
Integrated document reader for long-form agent outputs:
- Sectioned markdown display with navigation
- Inline composer for follow-up questions within reading view
- Karaoke word highlighting sync with voice playback
- Document persistence across turns
- Dismissal state tracking (closed documents remembered)
- Full keyboard navigation (sections, fold/unfold, search)

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/document_reader/mod.rs` (~8634 lines) — main widget, state machine, event handling, karaoke sync
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/document_reader/render.rs` (~1573 lines) — rendering pipeline for sections, syntax highlighting, word positioning
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/document_reader_ext.rs` (~250 lines) — extensions for document state persistence
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/chatwidget_document_reader.rs` — integration with chat widget lifecycle

**Status vs Upstream:**
- **Local-only**, no upstream implementation

**Merge Plan:**
New feature, standalone module. Integrate as optional bottom-pane view alongside chat widget. Requires:
- App event for `PresentDocument` (marshal reading view state)
- Bottom pane routing to show document_reader on demand
- Karaoke integration (optional, behind voice feature gate)

---

### 3. Mobile Control & Remote Discovery

**Name:** Mobile Daemon & Remote Control

**Description:**
Background WebSocket daemon for remote control of ATA from mobile clients:
- Daemon lifecycle (`~/.ata/mobile-server.pid`)
- Port selection and QR code rendering for pairing
- Remote session bridging through AppServer
- `/mobile` slash command to start/stop daemon
- Authentication and service discovery

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/mobile_daemon.rs` (~200 lines) — spawn detached daemon, manage PID file
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/remote_control.rs` (~200 lines) — client-side WebSocket setup
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/remote_discovery.rs` (~200 lines) — mDNS/discovery
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/qr_render.rs` (~100 lines) — QR code display
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/mobile_setup_view.rs` (~709 lines) — UI for daemon config, QR display, port selection

**Status vs Upstream:**
- **Local-only**, no upstream equivalent

**Merge Plan:**
ATA-specific feature for remote control. Integrate under `/mobile` slash command. Requires coordination server integration (behind private feature gate if needed). Can be fully isolated to mobile-specific modules.

---

### 4. Voice-Activated Audio Detection (VAD)

**Name:** Voice Activity Detection

**Description:**
Detects speech in audio streams to optimize spacebar hold-to-talk recording:
- Trim silence from beginning/end of recordings
- Real-time VAD scoring during recording
- Platform-specific implementation

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/vad.rs` (~200 lines) — VAD wrapper and scoring

**Status vs Upstream:**
- **Local-only**

**Merge Plan:**
Small, self-contained utility. Can remain as part of voice mode feature. No conflicts.

---

### 5. Research Tools UI

**Name:** Research Tools Settings & Toggle

**Description:**
Settings view for enabling/disabling research tool integrations (replaces `/memories`):
- Toggle switch for paper search, synthesis, and other research capabilities
- Settings persistence
- `/research` slash command

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/research_tools_view.rs` (~400 lines)

**Status vs Upstream:**
- **Local-only** (upstream has `/memories` view which is removed here)

**Merge Plan:**
Replaces upstream's `/memories` feature. Consider keeping upstream's `/memories` and adding `/research` as a separate new command, or map one to the other in slash command dispatch.

---

### 6. Account Management View

**Name:** Account View

**Description:**
Settings panel for account info, subscription, authentication status:
- Display authenticated user info
- Linked providers
- Subscription tier
- `/account` slash command

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/account_view.rs` (~517 lines)

**Status vs Upstream:**
- **Local-only**

**Merge Plan:**
New feature. Map `/account` to account_view in bottom pane routing.

---

### 7. Reverse Search / History Search

**Name:** Reverse Search (Ctrl+R History)

**Description:**
Ctrl+R history search in composer (replacement for history_search):
- Incremental search through command history
- Match highlighting
- Reverse scrolling through matches

**Implementation Summary:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/reverse_search.rs` (~300 lines)
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/chat_composer_reverse_search.rs` (~136 lines) — composer integration
- Deleted upstream file: `chat_composer/history_search.rs` (956 lines)

**Status vs Upstream:**
- **Local refactoring**: upstream's history_search moved/restructured into reverse_search

**Merge Plan:**
Upstream's history search is refactored in local fork. Merge upstream version first, then re-apply local improvements on top.

---

## Part B: Features in Both Versions (With Divergence)

### 1. Slash Commands

**Name:** Slash Command Enum & Handlers

**Description:**
Core command palette for TUI interaction. Both versions implement slash commands but with different sets.

**Local Commands (Fork):**
- Model, Fast, Approvals, Permissions, ElevateSandbox, SandboxReadRoot, Experimental, Skills, Review, Rename, New, Resume, Research, Fork, Init, Compact, Plan, Collab, Agent, Jobs, Mobile, Diff, Copy, Mention, Status, DebugConfig, Statusline, Theme, Mcp, Apps, Account, Logout, Quit, Exit, Feedback, Rollout, Ps, Team, Stop, Clear, Personality, Realtime, Settings, Voice, VoiceSetup, TestApproval, MultiAgents

**Upstream Commands (rust-v0.129.0):**
- Model, Fast, Ide, Permissions, Keymap, Vim, ElevateSandbox, SandboxReadRoot, Experimental, AutoReview, Memories, Skills, Hooks, Review, Rename, New, Resume, Fork, Init, Compact, Plan, Goal, Collab, Agent, Side, Copy, Raw, Diff, Mention, Status, DebugConfig, Title, Statusline, Theme, Mcp, Apps, Plugins, Logout, Quit, Exit, Feedback, Rollout, Ps, Stop, Clear, Personality, Realtime, Settings, TestApproval, MultiAgents

**Key Differences:**
| Removed (in fork) | Added (in fork) | Modified (in fork) |
|---|---|---|
| `/ide` | `/research` | Fast mode description |
| `/keymap` | `/voice` & `/voice-setup` | |
| `/vim` | `/account` | |
| `/goal` | `/jobs` | |
| `/approvals` (was `/approve`) | `/mobile` | |
| `/memories` | `/team` | |
| `/hooks` | | |
| `/side` | | |
| `/raw` | | |
| `/title` | | |
| `/plugins` | | |

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/slash_command.rs` — enum and descriptions
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/slash_commands.rs` — dispatch and routing

**Status vs Upstream:**
- **Both have implementations, local has removed/added commands**
- Significant enum reordering (presentation order)

**Merge Plan:**
**CRITICAL DECISION**: Upstream's `/ide`, `/keymap`, `/vim`, `/goal`, `/hooks`, `/side`, `/raw`, `/title`, `/plugins` were intentionally removed in local fork. Before merging upstream:
1. Determine if these should return to fork or stay removed
2. If staying removed, curate upstream's slash_command.rs before merge
3. Map upstream features like `/ide` (IDE context), `/keymap` (rebinding), `/vim` (modal editing) to new local feature equivalents if needed
4. Local's new `/voice`, `/mobile`, `/research`, `/jobs`, `/team`, `/account` should be added to upstream version

**Recommendation**: Accept upstream's command set as baseline, then cherry-pick additions (voice, mobile, research, account, jobs, team).

---

### 2. Chat Composer

**Name:** Chat Composer Widget (Textarea Input + History)

**Description:**
Main input area for user messages, with history, completions, and keybindings.

**Scale of Change:**
- Local: 6694 line changes (net -3257 lines added, +4610 removed)
- Restructured event handling, added voice state, reverse search integration
- Removed history_search submodule, added reverse_search

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/chat_composer.rs` (~6000+ lines)
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/chat_composer_history.rs` (~1000+ lines) — history model
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/chat_composer_reverse_search.rs` (~136 lines, new)
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/reverse_search.rs` (~300 lines, new)

**Status vs Upstream:**
- **Both have chat_composer**, but with structural refactors in fork
- Upstream's history_search (956 lines in bottom_pane/chat_composer/) moved to reverse_search
- Fork adds voice state management (VoiceState struct, spacebar hold-to-talk)
- Fork removes zellij detection, some status line hyperlinks, side conversation context

**Merge Plan:**
Upstream version is baseline. Local fork's changes are primarily:
1. Addition of voice state (feature-gated)
2. Reverse search refactor (can be merged incrementally)
3. Removal of some features (zellij, side context) — verify intent before removing

**Action**: Start with upstream's chat_composer.rs, then layer voice/reverse-search changes on top with feature gates.

---

### 3. Keymaps & Keymap Picker

**Name:** Keybinding Configuration & Debug

**Description:**
Remap TUI shortcuts, browse actions, inspect live key events.

**Local Deletions:**
- `/keymap` slash command removed (was in upstream)
- `/vim` slash command removed
- Deleted: `chatwidget/keymap_picker.rs` (181 lines)

**Upstream Implementation:**
- `keymap.rs` — RuntimeKeymap parsing
- `keymap_setup.rs` — picker model and capture logic
- `keymap_setup/picker.rs` — selection UI
- `keymap_setup/debug.rs` — `/keymap debug` inspector
- `keymap_setup/actions.rs` — action list

**Status vs Upstream:**
- **Upstream has full implementation, local removed `/keymap` command from dispatcher**
- Keymap files still exist in local (not deleted), but command entry point removed

**Merge Plan:**
Upstream's keymap system is complete and should remain. Local fork chose to hide `/keymap` and `/vim` from user-facing commands (likely for MVP scope). 
- **Option A**: Re-enable `/keymap` if upstream is baseline
- **Option B**: Keep removed, accept upstream has it but don't expose it

**Recommendation**: If merging upstream, verify that keymap infrastructure is still present in local's codebase. If files exist but command is hidden, just re-add `/keymap` to slash_command enum. If you want to keep it removed, delete the keymap_setup modules from upstream during merge.

---

### 4. Theme Picker

**Name:** Theme Picker (Syntax Highlighting)

**Description:**
Interactive selector for code syntax highlighting theme (Dracula, Nord, Catppuccin, etc.).

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/theme_picker.rs` — both versions have this

**Status vs Upstream:**
- **Both have implementation** (same upstream file remains in fork)

**Merge Plan:**
No action needed; feature is stable in both.

---

### 5. Resume/Fork Picker

**Name:** Resume/Fork Picker (Thread Selection)

**Description:**
Modal dialog to select previous conversations to resume or fork from.

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/resume_picker.rs` (~600 lines)
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/resume_picker/transcript.rs` (new in upstream v0.129.0)

**Status vs Upstream:**
- **Upstream added `transcript.rs` submodule** for transcript rendering in picker
- Local fork's resume_picker.rs remains largely compatible

**Merge Plan:**
Upstream's transcript submodule should be integrated. Local fork may have UI tweaks; verify compatibility and merge upstream's changes on top.

---

### 6. Status Line & Footer

**Name:** Status Line / Footer Indicators

**Description:**
Line at bottom of screen showing:
- Current model, reasoning effort, mode (fast, plan, etc.)
- PR/branch info, token usage, session status
- Active agent label

**Scale of Change:**
- Footer: 651 line changes
- Status line setup: 378 line changes
- Deleted: `status_line_style.rs` (296 lines), `status_surface_preview.rs` (184 lines), `title_setup.rs` (543 lines)
- Deleted: `action_required_title.rs` (25 lines)

**Upstream Features (rust-v0.129.0):**
- PR/branch status display (new in v0.129.0)
- Raw scrollback mode toggle state
- Keymap status line

**Local Fork:**
- Removed title customization (`/title` command deleted)
- Streamlined status line setup
- Removed status surface preview

**Status vs Upstream:**
- **Both have status line**, but upstream added PR/branch info in v0.129.0
- Local removed title configuration (deleted `title_setup.rs`)

**Merge Plan:**
Upstream's PR/branch info is a valuable addition. When merging:
1. Accept upstream's footer.rs and status_line_setup.rs as baseline
2. Re-apply local modifications (removal of title setup is intentional)
3. Integrate upstream's PR/branch rendering

---

### 7. Approval/Permission Flows

**Name:** Approval Overlay & Permission Settings

**Description:**
Request user confirmation for unsafe operations (exec, file writes, network access). Choose permission level (deny, prompt, auto-approve).

**Scale of Change:**
- `approval_overlay.rs`: 1483 lines changed (net reduction)
- Deleted: `auto_review_denials.rs` (131 lines)

**Local Changes:**
- Removed auto-review denials feature
- Approval modal refactored
- Renamed `/approve` to `/approvals`

**Upstream:**
- Has AutoReviewMode, auto_review_denials handling

**Status vs Upstream:**
- **Both have approval system**, but local removed auto-review denials

**Merge Plan:**
Local's simplification is intentional (MVP). If merging upstream, accept upstream's auto-review feature or explicitly document removal.

---

### 8. Bottom Pane Views (List Selection, Popups)

**Name:** List Selection & Picker Views (Model, Agent, Skills, etc.)

**Description:**
Generic picker widget for displaying and selecting from lists:
- Model picker (filter hidden models)
- Agent/thread picker
- Skills list
- Approvals picker
- Permission/approval profile selector

**Scale of Change:**
- `list_selection_view.rs`: 1042 line changes
- Various picker snapshots updated

**Status vs Upstream:**
- **Both have list_selection_view**, but local has UI refinements
- Snapshots show visual alignment/sizing changes

**Merge Plan:**
Upstream's version is baseline. Local has UX improvements (column width, visibility). Merge upstream's logic, preserve local UI tweaks.

---

### 9. Onboarding Flow

**Name:** Onboarding & Authentication

**Description:**
First-run setup:
- Welcome screen
- Authentication mode selection (API key, OAuth, headless login)
- Directory trust prompt
- Provider selection (OpenAI, Anthropic, Gemini, etc.)

**Local Additions:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/onboarding/provider_picker.rs` (new)
- Expanded OAuth provider list (Gemini, Anthropic, custom configure option)

**Upstream:**
- Has auth.rs with OpenAI and headless login
- Has provider-level API key entry

**Status vs Upstream:**
- **Upstream has core onboarding, local expanded provider picker**
- Local added more provider options and provider picker UI

**Merge Plan:**
Accept upstream's auth.rs as baseline, integrate local's provider_picker.rs as new feature to enable multi-provider setup.

---

### 10. App.rs (Core State Machine)

**Name:** App State & Event Dispatch

**Description:**
Main application state machine, event routing, thread management.

**Scale of Change:**
- `app.rs`: 8665 line changes (massive refactor)
- Deleted ~25 related files (app server session, approval conversion, auto-review denials, event dispatch, session lifecycle, etc.)
- Added `app_server_adapter.rs` (72 lines, new)
- Restructured event handling, thread routing

**Key Structural Changes:**
| Deleted (Upstream) | Notes |
|---|---|
| app/app_server_event_targets.rs | Event targeting system |
| app/app_server_events.rs | Event definitions |
| app/app_server_requests.rs | Request queuing |
| app/background_requests.rs | Background task management |
| app/config_persistence.rs | Config auto-save |
| app/event_dispatch.rs | Event routing |
| app/history_ui.rs | History UI integration |
| app/input.rs | Input handling |
| app/loaded_threads.rs | Thread caching |
| app/platform_actions.rs | Platform-specific actions |
| app/session_lifecycle.rs | Session creation/cleanup |
| app/thread_routing.rs | Thread routing logic |
| app_server_session.rs | Server session bridge |
| app_server_approval_conversions.rs | Approval protocol conversion |
| auto_review_denials.rs | Auto-review handling |
| app_command.rs | Command types |
| approval_events.rs | Approval event types |

**Status vs Upstream:**
- **Massive local refactor**, essentially restructured event dispatch architecture
- Upstream's modular separation replaced with more integrated approach
- Likely a significant merge conflict point

**Merge Plan:**
**HIGH PRIORITY FOR MERGE PLANNING**:
1. Understand why these modules were deleted (consolidation vs. feature removal)
2. If consolidation: check if functionality is moved into main app.rs or other locations
3. If feature removal: document which features were intentionally removed
4. Likely need to:
   - Start with upstream's app.rs as baseline
   - Manually integrate local's event handling improvements
   - OR: accept local's refactored approach and cherry-pick upstream features into it

**Recommendation**: Before merging, run both versions' test suites independently to identify which features are actually removed vs. refactored.

---

### 11. Text Formatting & Text Area

**Name:** Text Area & Input Handling

**Description:**
Textarea widget for composer input, with paste handling, syntax highlighting, word wrapping.

**Scale of Change:**
- `textarea.rs`: 1750 line changes (significant refactor)

**Status vs Upstream:**
- **Both have textarea**, local has restructuring
- Added large paste detection, spinner controls

**Merge Plan:**
Upstream's version is baseline. Local changes are refinements; apply on top if compatible.

---

### 12. Mobile Setup View

**Name:** Mobile Server Configuration UI

**Description:**
UI for starting/stopping mobile daemon, QR code display, port config.

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/mobile_setup_view.rs` (~709 lines, local-only)
- Paired with `/mobile` slash command

**Status vs Upstream:**
- **Local-only**, no upstream equivalent (mobile feature is new)

**Merge Plan:**
Feature-gated, self-contained. No conflicts with upstream.

---

### 13. Voice Setup View

**Name:** Voice Configuration Panel

**Description:**
Settings for TTS (language, speed, voice) and STT API key.

**Implementation:**
- `/Users/huytho_ho/acli/ata/codex-rs/tui/src/bottom_pane/voice_setup_view.rs` (~882 lines, local-only)

**Status vs Upstream:**
- **Local-only**, paired with `/voice-setup` command

**Merge Plan:**
Feature-gated, self-contained.

---

## Summary of Merge Strategy

### Files to Preserve from Upstream
- `keymap.rs`, `keymap_setup/*` (even if `/keymap` command is hidden)
- `theme_picker.rs`
- `resume_picker.rs` + new `transcript.rs` submodule
- `slash_commands.rs` (dispatch logic)
- Core bottom_pane views (list_selection, approval, feedback, etc.)
- Status line and footer (with PR/branch info from v0.129.0)
- Onboarding core (auth, welcome, trust)

### Files to Merge Carefully (Conflicts Expected)
- `app.rs` — massive refactor, requires detailed conflict resolution
- `chat_composer.rs` — local has voice state and reverse search, upstream may have new features
- `slash_command.rs` — enum reordering, command set differences
- `footer.rs` — status line changes
- `approval_overlay.rs` — auto-review feature removed in local

### Files to Add from Local
- `voice.rs`, `vad.rs` (voice mode, feature-gated)
- `mobile_daemon.rs`, `remote_control.rs`, `remote_discovery.rs`, `qr_render.rs` (mobile)
- `document_reader/*` (reading view)
- `chatwidget_document_reader.rs` (integration)
- `bottom_pane/mobile_setup_view.rs`, `voice_setup_view.rs` (UI)
- `chatwidget/voice_mode.rs` (integration)
- `bottom_pane/reverse_search.rs`, `chat_composer_reverse_search.rs` (search)
- `bottom_pane/account_view.rs`, `research_tools_view.rs` (settings)
- `onboarding/provider_picker.rs` (provider multi-selection)
- `clipboard_text.rs` (utility)

### Recommended Merge Order
1. **Start with upstream's TUI as baseline** (fresh start)
2. **Apply structural changes from local** (app refactor, event dispatch)
3. **Layer local features on top** (voice, reading view, mobile, research)
4. **Re-expose/hide commands** as needed (decide on `/keymap`, `/vim`, `/ide`, etc.)
5. **Test feature flags** — ensure voice mode and mobile build correctly with/without feature gates

### Critical Decision Points
1. **Slash command set** — Which of upstream's removed commands should stay removed?
2. **Auto-review denials** — Was this intentionally removed or just deprioritized?
3. **App.rs refactor** — Is local's approach fundamentally better or should upstream's modular structure be preserved?
4. **Keymap/Vim/IDE context** — Do these features belong in merged version?

---

## File Inventory

### Deleted from Upstream (in local)
- 246 files total (mostly tests and snapshots)
- Key modules: app server session, event dispatch, approval conversions, auto-review denials, title setup, status surface preview

### Added in Local (not in upstream)
- 32 files total
- Voice: `voice.rs`, `vad.rs`, `chatwidget/voice_mode.rs`, `bottom_pane/voice_setup_view.rs`
- Mobile: `mobile_daemon.rs`, `remote_control.rs`, `remote_discovery.rs`, `qr_render.rs`, `bottom_pane/mobile_setup_view.rs`
- Reading: `bottom_pane/document_reader/*`, `chatwidget_document_reader.rs`, `bottom_pane/document_reader_ext.rs`
- Settings: `bottom_pane/account_view.rs`, `bottom_pane/research_tools_view.rs`
- Search: `bottom_pane/reverse_search.rs`, `bottom_pane/chat_composer_reverse_search.rs`
- Onboarding: `onboarding/provider_picker.rs`
- Utilities: `clipboard_text.rs`
- Snapshots: Various TUI/onboarding/agent snapshots (new tests)

---

## Notes

- **VAD integration**: Voice Activity Detection (`vad.rs`) is small utility, low-risk merge
- **Karaoke word sync**: Implemented in document_reader with offset tracking; doesn't conflict with upstream
- **Feature gates**: All voice features are behind `#[cfg(not(target_os = "linux"))]` or feature `"voice-input"`; mobile features likely behind feature gate too
- **Session management**: Local's refactoring of event dispatch suggests fundamental architectural preference; should be addressed early in merge
- **Test coverage**: Snapshot test count has changed significantly; be prepared to regenerate snapshots after merge

