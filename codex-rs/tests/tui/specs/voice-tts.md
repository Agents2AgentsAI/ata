# Voice mode + TTS / karaoke — behavioral spec

This spec describes what voice mode and the speech pipeline do for
users, and the outcomes that must hold. It stays at capability level:
the testing agent discovers concrete commands, key bindings, backend
names, and wording at run time via the slash menu, `/voice-setup`, the
features menu, and the session JSONL. If a behavior here changes, that
is a product decision, not a refactor.

## Platform reality — read this first

The voice-entry surface (`/voice`, `/voice-setup`) is compiled out on
Linux. On a Linux host these commands do not exist in the binary at
all: they are absent from the slash menu and behave like any unknown
command. **This is correct behavior, not a bug.** A run on Linux must
SKIP every voice-entry probe in this spec and say so plainly in the
report's coverage notes — never report the missing command as an
issue. TTS-in-reader probes (the narration sections below) may still
apply on Linux if a speech backend is available; attempt them, and
record honestly which ones ran and which were skipped for platform or
backend reasons.

On platforms where voice mode exists, it is also gated behind an
experimental feature flag that defaults off. Disabled, the entry
command refuses with a clear message rather than half-entering.
Enable the flag (features menu or config) before probing.

## What voice mode is for

1. **Hands-free input**: enter voice mode, hold a key to speak, release
   to transcribe; the transcription lands in the composer like typed
   text.
2. **Spoken output (TTS)**: the assistant's responses and reading-view
   sections can be narrated aloud through a configurable speech
   backend, with pause/resume, stop, and speed controls.
3. **Karaoke tracking**: while TTS plays, the word currently being
   spoken is highlighted in the rendered text, tracking the audio.
4. **Configuration**: a setup flow chooses TTS/STT backends, API key,
   language, and whether the choices are session-scoped or saved as
   defaults.

Voice mode cross-cuts the chat composer, the reading view, and session
lifecycle (/clear, resume) — most of its historical bugs live at those
seams, not in the happy path.

## Capabilities and required behavior

### Entering and leaving voice mode

- The entry command toggles voice mode for the current session. On
  entry the user gets an explicit announcement and the composer
  switches to a push-to-talk prompt with a visible voice indicator. On
  exit (the same toggle), an exit confirmation prints and the voice
  composer is fully gone — no voice indicator lingering anywhere on
  the pane.
- **Escape does NOT exit voice mode.** Escape keeps its normal binding
  (edit previous message when idle); only the toggle command leaves
  voice mode. A run where Escape silently drops voice mode is a
  regression — this exact rebinding has happened before and the
  guard exists because of it.
- With the feature flag off, the entry command refuses with a clear
  "disabled" message and changes nothing.
- Hold-to-speak recording needs a real keyboard hold and microphone
  audio, which tmux cannot supply. That path is manual-only; note it
  as not covered rather than faking it.

### Session scope and lifecycle

- Voice mode and voice settings are session-scoped: enabling voice or
  changing a backend in one session must not bleed into the next
  session unless explicitly saved as defaults through the setup flow.
- `/clear` is a known fault line: after a /clear, voice state must end
  up in a coherent, intentional state (restored or reset per product
  intent — discover which by observing, then hold it consistent),
  never a half-state where the composer says one thing and the audio
  engine believes another. The historical bug was voice mode silently
  lost or wedged after /clear.
- Resume the session and check the same: no stale voice composer, no
  phantom narration state, settings match what the resumed session
  actually had.

### TTS playback controls

- Start, pause, resume, stop, and speed change each take effect
  immediately — not after the current sentence, not on the next
  section.
- **Attachment interaction**: every control must work, and never
  crash, while a file is attached to the composer or was part of the
  turn being narrated. Pause/resume with an attachment present has
  crashed before; treat this combination as a first-class probe, not
  an edge case.
- Pausing exposes a working stop: stopping from the paused state ends
  playback cleanly (historically the stop control broke specifically
  during pause).
- With no speech backend configured or reachable, starting narration
  fails with a visible error and the surrounding UI (chat or reader)
  stays fully usable.

### Backend management

- The setup flow lists available TTS/STT backends and applies a choice
  immediately.
- **Swapping backends mid-playback must interrupt the running worker.**
  The old backend's audio stops; no orphaned process keeps speaking,
  and the next narration uses the new backend. The historical bug left
  a stale worker running after a swap — verify the old audio actually
  stops, and where possible check for leftover speech processes, not
  just silence in the pane.
- Backend choice is cached within the session (no re-prompt per
  utterance) but scoped per the session rules above.

### Karaoke correctness

The highlight must track the audio. The desync family is the richest
regression class in this component (a dozen distinct fixes), so test
on hostile text, not lorem ipsum:

- **Punctuation runs**: text dense with punctuation (ellipses, quotes,
  parenthetical asides, hyphenated compounds). Spoken-word count and
  rendered-word count have disagreed before, accumulating drift.
- **Equations / inline math**: the highlight must not leak past an
  equation onto the following word, and must not run ahead at equation
  boundaries.
- **First word**: narration must not freeze with the first word
  permanently highlighted while audio continues (a broken
  position-zero check did exactly this).
- **Figures and captions**: highlight must not drift when narration
  crosses a figure or its caption.
- **Markdown structure**: headings, list markers, and other rendering
  artifacts that exist on screen but may or may not be spoken — the
  tracker must stay aligned across them.

Judge alignment by watching highlight progression against elapsed
audio over a long passage: small jitter is tolerable, monotonic drift
or a stuck/leaping highlight is an issue.

### Containment between speech and chat

- TTS used from the reading view (read-aloud) must not leak state into
  the chat surface: after narration ends or is stopped, the chat
  composer, footer, and turn handling behave as if narration never
  happened. The historical bug left read-aloud's TTS-only state active
  in chat.
- Narration content must not enter the model prompt or chat history as
  a side effect of being spoken. Check the session JSONL, not the pane.

### Interruption and recovery

- Interrupt narration aggressively: keystrokes, a new prompt submitted
  mid-speech, toggling voice mode off during playback, starting a new
  narration while one is running. Every path must recover to a clean
  state — no stuck audio, no wedged composer, no double playback.
- Quit the TUI during active narration: the process exits and audio
  stops with it.

## How to test it

First establish the platform: if `/voice` is absent from the slash
menu on Linux, record the skip per the platform section and move on to
whatever TTS/narration surface does exist. Do not burn time hunting
for a command the build does not contain.

On a capable platform: enable the feature flag, drive the TUI through
tmux (recipe in the README), and work through the sections above. For
karaoke, have the agent produce or present a document seeded with the
hostile-text classes (equations, punctuation runs, headings, lists, a
figure with caption) and narrate it end to end. For audio-pipeline
verification, the strongest available check is a loopback round-trip
(speak known text, capture, STT it back); where that infrastructure
is absent, fall back to control-latency and state observations and say
so in the coverage notes.

Prioritize the seams the history points at: /clear and resume while
voice is on, attachments present during pause/resume, backend swap
mid-playback, read-aloud followed by normal chat. Each of these has
broken before; each is a regression probe, not optional extra credit.

Hold-to-speak with a real microphone is out of scope for an automated
run; list it under not-covered.

Report per the README: issues with exact reproductions, divergences
citing the section above, and honest coverage notes — especially
which probes were platform-skipped versus actually exercised.
