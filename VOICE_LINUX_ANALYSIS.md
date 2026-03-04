# Voice Mode on Linux — Analysis

## Status: Disabled by design

Voice mode is **intentionally excluded on Linux** at the dependency level.

## Root Cause

In `codex-rs/tui/Cargo.toml` (line 110), three critical voice dependencies are
compiled only for non-Linux targets:

```toml
[target.'cfg(not(target_os = "linux"))'.dependencies]
codex-elevenlabs = { workspace = true }
cpal = { version = "0.15", optional = true }
hound = { version = "3.5", optional = true }
```

This means on Linux:
- **cpal** (cross-platform audio I/O via ALSA) is not compiled
- **hound** (WAV encoding/decoding) is not compiled
- **codex-elevenlabs** (ElevenLabs TTS/STT integration) is not compiled

## Scope of cfg gating

The entire voice feature is then cfg-gated out across ~150+ locations in ~15
source files using `#[cfg(not(target_os = "linux"))]` or
`#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]`.

Key files affected:
- `tui/src/chatwidget.rs` (~50 cfg gates)
- `tui/src/app_event.rs` (~16 cfg gates)
- `tui/src/app.rs` (~15 cfg gates)
- `tui/src/bottom_pane/chat_composer.rs` (~25 cfg gates)
- `tui/src/bottom_pane/document_reader/mod.rs` (~12 cfg gates)
- `tui/src/bottom_pane/mod.rs` (~8 cfg gates)
- `tui/src/chatwidget/realtime.rs` (~10 cfg gates)
- `tui/src/chatwidget/voice_mode.rs` (~6 cfg gates)
- `tui/src/lib.rs` (~4 cfg gates)
- `tui/src/chatwidget/tests.rs` (~8 cfg gates)
- `tui/src/bottom_pane/bottom_pane_view.rs` (~6 cfg gates)
- `tui/src/bottom_pane/document_reader_ext.rs` (~6 cfg gates)
- `tui/src/bottom_pane/footer.rs` (~2 cfg gates)
- `tui/src/bottom_pane/textarea.rs` (~1 cfg gate)

## Can it work on Linux?

**Yes, technically.** The `cpal` library supports Linux via ALSA backend.
Required system dependency: `libasound2-dev` (or equivalent).

## What would be needed to enable it

1. Move `codex-elevenlabs`, `cpal`, and `hound` from the
   `cfg(not(target_os = "linux"))` section to general `[dependencies]` in
   `codex-rs/tui/Cargo.toml`

2. Replace all ~150 cfg gate patterns across the source:
   - `#[cfg(all(not(target_os = "linux"), feature = "voice-input"))]` → `#[cfg(feature = "voice-input")]`
   - `#[cfg(not(target_os = "linux"))]` → remove (for voice code)
   - `#[cfg(target_os = "linux")]` → remove (for voice stubs/fallbacks)
   - `#[cfg(any(target_os = "linux", not(feature = "voice-input")))]` → `#[cfg(not(feature = "voice-input"))]`
   - `cfg!(not(target_os = "linux"))` → `true` or remove condition (in runtime checks)

3. Ensure `libasound2-dev` is available at build time on Linux

## History

Commits `7aa3f15`, `bd420e9`, and `87dc6c6` (March 2, 2026) added the Linux
cfg gates to fix compilation — the approach was to exclude voice rather than
make it work on Linux.
