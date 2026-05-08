# Fork-vs-Upstream Divergence Analysis: Reading View, Voice Mode, TTS, and Audio

**Tag analyzed:** `rust-v0.129.0`  
**Local branch:** `main` @ `29ff511925` (reading view, TTS, voice mode, auth, figure extraction commit)  
**Analysis scope:** Reading view rendering, document reader, PDF/figure extraction, ElevenLabs TTS/STT, voice mode UI, audio playback/microphone, Supabase auth integration

---

## Summary

The local fork has **five major feature areas that are entirely absent from upstream** and represent significant fork-specific functionality:

1. **Reading View Server** – Browser-based document reader with live streaming
2. **ElevenLabs Integration** – TTS/STT client with voice mode pipeline
3. **Comprehensive Voice Mode** – State machine, karaoke, speed control
4. **Figure Extraction Tool** – PDF rendering and cropping via pdfium
5. **Supabase Authentication** – Alternative auth backend with account UI

Additionally, several **dual-implementation areas** exist where both fork and upstream have code but with significantly different approaches:

- **Audio I/O**: Fork extends local STT/WAV support; upstream uses realtime-webrtc
- **Realtime Audio**: Fork uses ElevenLabs WebSocket; upstream uses libwebrtc
- **Configuration**: Fork centralizes in core; upstream uses modular approach
- **Login**: Fork is minimal; upstream has agent-identity integration

---

## Feature-by-Feature Analysis

### FORK-ONLY FEATURES (do not exist in upstream)

#### 1. Reading View Server Crate (`codex-rs/reading-view-server/`)

**Status:** Local-only (NEW)

**Description:**  
A lightweight async HTTP+WebSocket server that serves a browser-based document reader. Streams document events (section updates, highlights, images) to connected clients via WebSocket. Supports client-to-server messages for follow-up questions and read-aloud requests.

**Implementation:**
- `reading-view-server/Cargo.toml`: Minimal deps (axum, tokio, tower-http, futures)
- `reading-view-server/src/lib.rs` (~238 lines): Server startup, WebSocket handling, broadcast channel, event replay buffer
- `reading-view-server/src/assets/LivingReadingView.html` (2861 lines): Embedded browser template with dynamic HTML rendering, karaoke sync, WebSocket client

**Key capabilities:**
- Dynamic document section streaming
- Event buffering for late-connecting clients
- Optional static asset serving (for figure images)
- Optional incoming message forwarding

**Merge plan:**  
This crate is entirely new and introduces no conflicts. To merge upstream, keep this crate as-is. No upstream changes to this area. If integrating upstream's realtime-webrtc approach later, consider whether to refactor reading view to use it or keep parallel.

---

#### 2. ElevenLabs TTS/STT Crate (`codex-rs/codex-elevenlabs/`)

**Status:** Local-only (NEW)

**Description:**  
Client library for ElevenLabs API (text-to-speech and speech-to-text). Provides streaming TTS via persistent WebSocket connection and HTTP-based STT. Returns PCM audio chunks (24kHz mono i16) suitable for direct playback via cpal.

**Implementation:**
- `codex-elevenlabs/Cargo.toml`: Minimal (reqwest, tokio-tungstenite, serde, thiserror, futures)
- `codex-elevenlabs/src/lib.rs`: Module re-exports
- `codex-elevenlabs/src/tts.rs`: WebSocket TTS client with sentence alignment and karaoke metadata
- `codex-elevenlabs/src/stt.rs`: HTTP STT client (handles WAV upload, returns transcription)
- `codex-elevenlabs/src/types.rs`: Config (API key, voice ID, model), error types, alignment structures

**Key public API:**
- `TtsClient::connect()` – Open persistent WebSocket for streaming TTS
- `TtsClient::push_text()` / `flush()` – Queue text for TTS
- `TtsClient::recv_chunk()` – Receive PCM audio chunks with timing data
- `SttClient::transcribe()` – Upload WAV, get text transcription

**Merge plan:**  
This crate is entirely new and adds no upstream conflicts. To merge, keep it as-is. Upstream has no ElevenLabs support. Consider whether to add upstream's realtime-webrtc as an alternative TTS backend in the future.

---

#### 3. Voice Mode State Machine and Instructions (`codex-rs/tui/src/chatwidget/voice_mode.rs`)

**Status:** Local-only (NEW), 6500+ lines

**Description:**  
Comprehensive voice mode implementation with a state machine that manages push-to-talk recording, STT streaming, agent response listening, and TTS playback. Includes karaoke highlighting (sync'ing TTS word-level timing with visual display) and adjustable playback speed. Two verbosity levels (Verbose/Concise) with different instruction prefixes to guide agent behavior.

**Implementation:**
- Voice mode instructions (const strings): VOICE_MODE_INSTRUCTION_VERBOSE, VOICE_MODE_INSTRUCTION_CONCISE
- Phase enum: Off → Idle → Recording → Waiting → Listening → Speaking
- `VoiceModeState` struct: Tracks phase, timers, TTS state, transcript, karaoke state
- Event handlers: space press/release, agent deltas, TTS chunks, user cancellation
- Sentence buffering: Collects voice tags from agent output and queues to TTS
- Karaoke highlighting: Word-level sync from TTS alignment data to TUI text
- Equation handling: `<eq>` tags for math with LaTeX and spoken reading

**Key public API:**
- `voice_mode_instruction()` – Get instruction for given verbosity level
- `voice_mode_instruction_prefixes()` – For prefix stripping when turning off voice mode
- `VoiceModeState::new()` – Initialize state machine
- `VoiceModeState::handle_*()` – Methods for each event type

**Merge plan:**  
This file is entirely new and has no upstream equivalent. Keep it as-is when merging. No conflicts. If considering upstream's realtime approach, would need to refactor voice mode to work with webrtc events instead of ElevenLabs chunks.

---

#### 4. Document Reader Tool and TUI Module

**Status:** Local-only (NEW)

**Description:**  
A two-part system for streaming and rendering documents in the TUI:
- **Tool (`core/src/tools/handlers/document_reader.rs`)**: Handles agent calls to present, update, and stream document sections. Manages document caching, streaming state, markdown/HTML rendering.
- **TUI Module (`tui/src/bottom_pane/document_reader/`)**: Renders documents with section navigation, syntax highlighting, embedded images, reading progress, and integration with reading-view-server.

**Implementation:**

*Core tool (~1459 lines):*
- `PresentDocumentEvent`: Schema for presenting new document
- `AddDocumentSectionEvent`: Add a new section to document
- `AppendDocumentSectionEvent`: Stream content into a section
- `UpdateDocumentSectionEvent`: Modify existing section (title/content)
- `PatchDocumentSectionEvent`: Line-by-line patching of section content
- Citation marker stripping (removes `citeturn\dview\d` artifacts)
- Section caching and markdown reconstruction
- Event streaming to reading-view-server

*TUI module:*
- `document_reader/mod.rs` (~254KB source): Full rendering, navigation, image loading, reading progress
- `document_reader/render.rs` (~56KB): Markdown-to-ratatui conversion with syntax highlighting
- Displays title, sections, figures, scroll position
- Keyboard navigation (arrows, page up/down, search)
- WebSocket integration with reading-view-server
- Karaoke highlight support (animated text highlighting as TTS plays)

**Merge plan:**  
Both files are entirely new. No upstream conflicts. Keep as-is when merging. These provide a core feature not present in upstream. If upstream later adds document reading, would need to decide whether to adopt its approach or keep the current fork implementation.

---

#### 5. Figure Extraction Tool (`codex-rs/core/src/tools/handlers/crop_figure.rs`)

**Status:** Local-only (NEW), ~304 lines

**Description:**  
Extracts and crops figures from PDF documents. Renders a specified page from a cached PDF using pdfium, crops a region of interest, and returns the image. Supports figure captions and descriptions for accessibility.

**Implementation:**
- `CropFigureArgs`: Defines pdf_url, page, x, y, w, h, caption, description
- `render_pdf_page()`: Load PDF from cache, render at 150 DPI, extract page image
- Image cropping: Extract region [x, y, x+w, y+h] from rendered page
- Image encoding: Convert to WebP for efficient storage
- SHA256 hashing: Generate unique filename from PDF URL and figure region
- Return: Base64-encoded image data with metadata

**Key dependencies:**
- `pdfium-render`: PDF rendering library
- `image`: Image manipulation (crop, encode)
- `sha2`: Hash functions
- `pdfium_downloader` module: Ensures pdfium library is available locally

**Merge plan:**  
This tool is entirely new. Keep as-is when merging upstream. No conflicts. Upstream has no figure extraction capability.

---

#### 6. PDF Library Downloader (`codex-rs/core/src/tools/pdfium_downloader.rs`)

**Status:** Local-only (NEW), ~148 lines

**Description:**  
Ensures the pdfium native library is available for PDF rendering. Downloads prebuilt binaries for the platform if not already cached locally in `~/.ata/lib/`.

**Implementation:**
- Platform detection: macOS, Linux, Windows handling
- Version-specific binary URLs (GitHub release artifacts)
- Download and extract to `~/.ata/lib/libpdfium.*`
- Cache check: Skip if already present
- Symlink resolution for system library paths

**Merge plan:**  
Entirely new utility module. Keep as-is. No upstream equivalent.

---

#### 7. Voice-Mode Extended Audio Support in `voice.rs`

**Status:** Local expansion (both have `voice.rs`, but LOCAL significantly extended)

**Description:**  
The local version extends audio capture beyond realtime-to-model streaming. Adds:
- WAV file writing (`hound` crate) for recorded audio
- Transcription authentication context (bearer token, ChatGPT account ID, base URL)
- STT integration via ElevenLabs
- Recorded audio struct (data, sample rate, channels)
- Conversion between device sample rates and model sample rates (with channel mixing)

**Local `voice.rs` specifics:**
- `TranscriptionAuthContext`: Manages OAuth token, account ID, base URL for STT
- `RecordedAudio`: Struct with data, sample_rate, channels
- `VoiceCapture::start()`: Records full audio to buffer, not streaming to model
- `VoiceCapture::finalize_recording()` (implied): Extract WAV and send to STT
- `RealtimeAudioPlayer`: Playback of TTS chunks from ElevenLabs

**Upstream `voice.rs` specifics:**
- More minimal: Realtime streaming only
- Uses `legacy_core::config::Config` (local uses `codex_core::config::Config`)
- `VoiceCapture::start_realtime()` only
- Streams PCM directly to model via `ThreadRealtimeAudioChunk`
- No transcription context

**Merge plan:**  
This is a significant divergence. Both versions coexist. To merge upstream:
1. Keep local's transcription features (RecordedAudio, TranscriptionAuthContext, WAV writing)
2. Adopt upstream's realtime streaming for model input if merging webrtc approach
3. Ensure both paths (STT via ElevenLabs and realtime model audio) work together

---

#### 8. Supabase Authentication (`codex-rs/core/src/supabase/`)

**Status:** Local-only (NEW), 6 files

**Description:**  
Supabase client library for authentication. Manages sessions, JWT refresh, user state, and OAuth integration with Supabase backend. Alternative to upstream's agent-identity system.

**Implementation:**
- `auth.rs`: Supabase auth client, OAuth flow initiation, JWT token refresh
- `client.rs`: HTTP client wrapper, request signing
- `error.rs`: Error types
- `session.rs`: Session storage, token management
- `types.rs`: Auth types (User, Session, OAuth URLs, etc.)
- `mod.rs`: Module re-exports

**Integration points:**
- Used by `login::supabase_auth.rs` (new file in local login crate)
- Integrated into auth flow in `core/src/auth.rs`
- Account status displayed in TUI `bottom_pane/account_view.rs`

**Merge plan:**  
This module is entirely new and replaces upstream's agent-identity approach for Codex authentication. To merge upstream:
1. Keep local supabase module as-is (no upstream equivalent to conflict)
2. Upstream's agent-identity would coexist in parallel
3. Need to decide which auth system to use as primary for the merged codebase

---

#### 9. Account View UI (`codex-rs/tui/src/bottom_pane/account_view.rs`)

**Status:** Local-only (NEW), ~500 lines

**Description:**  
TUI widget displaying account status, login/logout, and OAuth connection. Shows user info, subscription status, token refresh status, and provides interactive login/logout controls.

**Implementation:**
- Account status display (logged in / logged out)
- Subscription tier and token usage metrics
- Refresh token button
- Login/logout handlers
- Interactive popup for OAuth flow

**Merge plan:**  
Entirely new. Keep as-is. Upstream has no account UI in TUI.

---

#### 10. Voice Setup View UI (`codex-rs/tui/src/bottom_pane/voice_setup_view.rs`)

**Status:** Local-only (NEW), ~28 lines

**Description:**  
TUI popup for configuring voice mode settings:
- Voice mode on/off toggle
- Microphone device selection
- Speaker device selection
- Playback speed adjustment
- Verbosity level (Verbose/Concise)

**Implementation:**
- Uses ratatui widgets for selection popups
- Integrates with `audio_device.rs` device enumeration
- Updates config on device/speed/verbosity changes

**Merge plan:**  
Entirely new. Keep as-is. Upstream has no voice setup UI.

---

#### 11. Config Types Module (`codex-rs/core/src/config/types.rs`)

**Status:** Local-only (NEW), 1194 lines

**Description:**  
Comprehensive configuration types for the entire system, including voice mode, reading view, HTML rendering, and other features. Replaces upstream's more modular approach with a centralized schema.

**Key types added:**
- `VoiceModeToml`: Voice mode settings (on/off, device selection, speed, verbosity)
- `VoiceOutput`: Device selection (auto-detect, specific device names)
- `VoiceVerbosity`: Enum (Verbose, Concise)
- `HtmlRenderingConfig`: Settings for browser rendering
- Audio device configuration
- Reading view display preferences

**Impact:**
- `core/src/config/mod.rs`: Updated to use new types
- `core/src/config/edit.rs`: New editor functions
- Centralizes all config in one place instead of spread across crates

**Merge plan:**  
This file is entirely new locally. Upstream has a different modular approach with `codex-config` crate. To merge upstream:
1. May need to adopt upstream's structure and distribute types across crates
2. Or keep local's centralized approach (simpler for reading view/voice features)
3. Ensure all voice/reading config is preserved

---

#### 12. Text Formatting Module (`codex-rs/tui/src/text_formatting.rs`)

**Status:** Local-only (NEW/EXPANDED), 1102 lines

**Description:**  
HTML and markdown parsing for rendering in the reading view. Converts HTML/markdown to styled ratatui text with syntax highlighting, code blocks, math equations, and voice tags.

**Key functions:**
- `parse_html_to_styled_text()`: Convert HTML to styled ratatui lines
- `parse_markdown_to_styled_text()`: Convert markdown to styled text
- Equation handling: `<eq>` tag parsing with LaTeX and spoken reading
- Voice tag handling: `<voice>` tag extraction for TTS
- Code block syntax highlighting via syntect
- Link and image reference handling

**Merge plan:**  
Entirely new and supporting reading view. Keep as-is. No upstream equivalent.

---

#### 13. Reading View HTML Template (`reading-view-server/src/assets/LivingReadingView.html`)

**Status:** Local-only (NEW), 2861 lines

**Description:**  
Embedded browser-based reading view template. Single-page HTML/CSS/JavaScript app that:
- Connects to reading-view-server WebSocket
- Renders document sections dynamically
- Supports karaoke highlighting (animated text sync with TTS)
- Handles figure/image display
- Provides search and navigation

**Features:**
- WebSocket client for live event streaming
- Dynamic DOM updates for sections
- Karaoke timing sync (highlight words as TTS plays)
- Responsive layout
- MathML rendering for equations
- Copy-to-clipboard for text selection

**Merge plan:**  
Entirely new asset file. Keep as-is when merging. No upstream equivalent.

---

### DUAL-IMPLEMENTATION FEATURES (both fork and upstream, significant divergence)

#### A. Realtime Audio Architecture

**Upstream approach:**  
- `codex-realtime-webrtc` crate using OpenAI's realtime API
- libwebrtc on macOS for audio handling
- Integrated into TUI via realtime event streams

**Local approach:**  
- `codex-elevenlabs` crate with ElevenLabs WebSocket TTS + HTTP STT
- cpal for audio I/O (no WebRTC)
- Integrated into chatwidget voice mode state machine
- More control over TTS timing (karaoke) and sentence boundary detection

**Comparison table:**

| Aspect | Upstream (webrtc) | Local (elevenlabs) |
|--------|------|------|
| TTS Provider | OpenAI | ElevenLabs |
| Transport | WebRTC | WebSocket + HTTP |
| Control | Model-managed | App-managed |
| Karaoke | Not present | Full support |
| Speed control | Not present | Supported |
| STT Provider | OpenAI | ElevenLabs |
| Audio I/O | libwebrtc | cpal |
| Device selection | Basic | Comprehensive (UI popup) |

**Merge strategy:**  
These are fundamentally different approaches with different TTS providers and user experiences. To merge:
- Option 1: Keep local ElevenLabs (simpler, more control, karaoke support)
- Option 2: Adopt upstream WebRTC (OpenAI integration, realtime protocol support)
- Option 3: Parallel implementation with feature flags
- Recommend: Option 1 for now due to richer UX features; consider adding Option 2 as alternative in future

---

#### B. Audio Device Selection and Configuration

**Upstream (`audio_device.rs`):**
```rust
use crate::legacy_core::config::Config;
pub fn select_configured_input_device_and_config(config: &Config) -> ...
pub fn select_configured_output_device_and_config(config: &Config) -> ...
```

**Local (`audio_device.rs`):**
```rust
use codex_core::config::Config;
// Same function signatures and logic
```

**Difference:**  
Only the import path differs. Local uses `codex_core::config`, upstream uses `legacy_core::config`. Functionality is similar (device selection, config matching, sample rate negotiation).

**Merge plan:**  
Minor import path change. When merging, update to use correct config import path. No functional changes needed.

---

#### C. Authentication Backend

**Upstream:**
- `codex-agent-identity` crate for agent-based auth
- `codex-login` integrates agent identity flows
- OAuth via browser popup with various provider support

**Local:**
- Supabase client (new `core/src/supabase/`)
- ChatGPT fallback auth
- Direct HTTP to Supabase backend
- Account view in TUI

**Merge strategy:**  
Keep local Supabase as-is. If upstream auth is critical, both can coexist behind feature flags or config options. No code conflicts; just different backends.

---

#### D. Configuration System

**Upstream:**
- Modular: `codex-config`, separate feature crates (features, plugin, terminal-detection)
- Distributed type definitions across crates
- Lazy-loaded modules

**Local:**
- Centralized: Core config/types.rs with all configuration in one place
- Simpler for reading view/voice features which have many config options
- But less modular

**Merge plan:**  
To merge upstream's modular approach, would need to:
1. Distribute local types across appropriate crates
2. Create new config crates for voice/reading features if needed
3. Or keep local's centralized approach (simpler, but diverges more from upstream)

Recommend: Keep local's centralized approach for now; can refactor to match upstream later if needed.

---

### Removed Features in Local

The local fork **removed coordination system** (per commit message):
- Deleted: `coordination/`, `coordination-relay/` crates
- Deleted: `core/src/coordination_context.rs`
- Removed: `core/src/tools/handlers/team_post.rs`
- Removed: Coordination templates

This is intentional (stated in commit) and not a merge concern. Upstream still has these; merging will not restore them (they're gone locally for a reason).

---

## Merge Impact Summary

### Merge Upstream into Local: LOW RISK

**Conflicts to expect:**
1. **Cargo.toml workspace members**: Upstream has many more crates (agent-graph-store, memories, code-mode, realtime-webrtc, etc.). Will need to add them back or disable.
2. **TUI dependencies**: Upstream has many codex-* deps; local has been pruned. Git merge will restore upstream's deps; may need manual curation.
3. **Audio device config**: Path import differences in `audio_device.rs` (legacy_core vs codex_core).
4. **Voice features**: `tui/Cargo.toml` has feature flags in local; upstream does not. Will need to preserve or decide on approach.

**Feature gaps after merge (if not addressed):**
- Reading view will NOT exist (new feature in local, not in upstream)
- Voice mode will NOT exist (new feature in local, not in upstream)
- Document reader will NOT exist
- Figure extraction will NOT exist
- Supabase auth will NOT exist

To keep these after merging upstream, **do not let git merge delete the new local crates and modules**. Explicitly keep them.

### Preserve Local Features During Merge

```bash
# After resolving git conflicts, ensure these are kept:
- codex-rs/reading-view-server/ (new crate)
- codex-rs/codex-elevenlabs/ (new crate)
- codex-rs/tui/src/chatwidget/voice_mode.rs
- codex-rs/tui/src/bottom_pane/document_reader/
- codex-rs/tui/src/bottom_pane/account_view.rs
- codex-rs/tui/src/bottom_pane/voice_setup_view.rs
- codex-rs/tui/src/text_formatting.rs (new additions)
- codex-rs/tui/src/voice.rs (extended)
- codex-rs/core/src/tools/handlers/document_reader.rs
- codex-rs/core/src/tools/handlers/crop_figure.rs
- codex-rs/core/src/tools/pdfium_downloader.rs
- codex-rs/core/src/supabase/
- codex-rs/core/src/config/types.rs
- codex-rs/login/src/supabase_auth.rs
```

---

## File Inventory

### Fork-only Crates

| Crate | Status | Key files | Size |
|-------|--------|-----------|------|
| reading-view-server | NEW | lib.rs, LivingReadingView.html | ~3KB code + 2.8KB HTML |
| codex-elevenlabs | NEW | tts.rs, stt.rs, types.rs | ~450 lines |

### Fork-only Modules (in existing crates)

| Module | Crate | Status | Size | Purpose |
|--------|-------|--------|------|---------|
| chatwidget/voice_mode.rs | tui | NEW | 6500+ lines | Voice state machine |
| bottom_pane/document_reader/ | tui | NEW | 280+ KB | Document rendering |
| bottom_pane/account_view.rs | tui | NEW | 500 lines | Auth UI |
| bottom_pane/voice_setup_view.rs | tui | NEW | 28 KB | Voice config UI |
| text_formatting.rs | tui | EXPANDED | 1100+ lines | HTML/markdown parsing |
| voice.rs | tui | EXTENDED | 1200 lines | Audio I/O + STT |
| document_reader.rs | core/tools/handlers | NEW | 1459 lines | Document tool |
| crop_figure.rs | core/tools/handlers | NEW | 304 lines | PDF figure extraction |
| pdfium_downloader.rs | core/tools | NEW | 148 lines | Library management |
| config/types.rs | core | NEW | 1194 lines | Configuration schema |
| supabase/ | core | NEW | 6 files, ~800 lines | Auth backend |
| login/supabase_auth.rs | login | NEW | 580+ lines | Supabase client |

### Upstream-only Crates (not in local)

| Crate | Purpose | Impact |
|-------|---------|--------|
| realtime-webrtc | WebRTC audio (OpenAI) | Conflicts with ElevenLabs approach |
| agent-identity | Auth identity system | Conflicts with Supabase approach |
| (30+ other crates) | Various features | Will need to re-integrate or disable |

---

## Recommendations

### For Merging Upstream

1. **Use `git merge` with careful conflict resolution:**
   - Accept ALL local changes for reading view, voice mode, elevenlabs, supabase
   - Resolve Cargo.toml by keeping local's reading-view-server and elevenlabs in members
   - Handle realtime-webrtc in upstream as optional (feature flag or separate path)

2. **Test audio I/O compatibility:**
   - Both use cpal; ensure version compatibility
   - Test audio device selection with upstream's config path if unified
   - Verify ElevenLabs TTS still works after import path changes

3. **Configuration strategy:**
   - Decide on keeping local's centralized config/types.rs vs adopting upstream's modular approach
   - If keeping local: No merge conflict, just add new upstream config options
   - If adopting upstream: Significant refactor needed to distribute types

4. **Authentication:**
   - Decide primary auth backend (Supabase or agent-identity)
   - Can keep both behind feature flags for now
   - TUI account view only works with Supabase; upstream has different flow

5. **After merge, verify:**
   - Reading view server starts and serves HTML
   - Voice mode state machine works with ElevenLabs
   - Document reader renders sections
   - Figure extraction finds cached PDFs
   - Account view shows login status

### For Minimal Divergence Going Forward

1. Avoid modifying upstream's realtime-webrtc even if not using it
2. Keep reading view and elevenlabs as self-contained crates
3. Wrap voice mode in feature gate if upstream doesn't support it
4. Document config strategy (centralized vs modular) and stick to it

---

## Conclusion

The fork introduces **substantial new functionality** (reading view, voice mode with ElevenLabs, figure extraction, Supabase auth) that does not exist in upstream. These features represent new product capabilities, not modifications of existing upstream code.

**Merge conflict risk: LOW** (mostly new files, no overwrites)  
**Integration risk: MEDIUM** (realtime-webrtc and agent-identity exist upstream; need to decide on architecture)  
**Feature preservation risk: HIGH** (must explicitly keep local crates/modules during merge or they may be lost)

Recommended approach: **Merge upstream into local**, preserving all local features, then selectively integrate upstream features as needed (e.g., add realtime-webrtc as optional alternative to ElevenLabs).

