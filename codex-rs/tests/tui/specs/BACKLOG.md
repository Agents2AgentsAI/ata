# Behavioral spec backlog

Ranked from the founders' commit history (2026-02-04 through 2026-06-07,
~1,180 non-merge commits by Nima and Tho). Ranking signal: how much the
founders built and rebuilt a component (churn), how many distinct bug
fixes it accumulated (each past fix is a regression class worth
probing), and recency. Components with existing specs (workspace,
knowledge base, code intel, research, repo analysis, scheduling, goals)
are excluded except where history shows the spec has a gap.

Like the existing specs, every entry here stays at capability level:
the testing agent discovers concrete commands, tool names, and wording
at run time via `--help`, `search-commands`, the features menu, and the
session JSONL.

## Ranked table

Items 1-10 were converted to specs on 2026-06-10/11. Their "what to
test" sections below are kept as historical motivation; the specs are
the contract now. Items 11-14 remain open.

| # | Component | Status | Churn | Regression history |
|---|-----------|--------|-------|--------------------|
| 1 | Reading view / document reader (TUI + browser) | done: `reading-view.md` | ~120 commits | duplication, fold loss, resume leaks, replay bugs, karaoke desync |
| 2 | Voice mode / TTS / STT | done: `voice-tts.md` | ~60 commits | state leaks, pause/resume crashes, backend swap bugs |
| 3 | PDF ingestion + compaction with PDFs | done: `pdf-ingestion.md` + continuity matrix | ~40 commits | compaction broke with PDFs at least three separate times |
| 4 | Model providers + auth | done: `model-providers.md` | ~55 commits | OAuth, thinking, streaming, resume history, sticky model names, keyring |
| 5 | Trajectory forks (cards) + whiteboard | done: `cards-whiteboard.md` | ~10 commits, June 2026 | CI broke twice in first week |
| 6 | Feature flag system / features menu | done: `feature-flags.md` | ~15 commits | missing flag entries crashed /research; switching bugs x4 |
| 7 | Session continuity (resume, fork, /clear, compaction) | done: `session-continuity.md` | cross-cutting, ~30 commits | per-feature state lost or leaked on resume, repeatedly |
| 8 | Skills system (loader, categories, picker, bundled extraction) | done: `skills-system.md` | ~54 commits | frontmatter parsing, category registration, binary path bugs |
| 9 | Subagents / multi-agent | done: `subagents.md` | ~20 commits | PDF errors inside subagents, LSP broke under subagents, /side stack overflow |
| 10 | ATA account / Supabase / mobile pairing (private) | done: `account-supabase.md` | ~25 commits | auth recovery, device registration, session expiry, quota |
| 11 | Coordination relay / fleet (private) | open | ~15 commits | worker auth, directed messaging, rate limits |
| 12 | MCGS / spine search / kernels (ata-plus) | open; test via its own harness first | ~110 commits | race conditions, fake solutions, memory growth |
| 13 | TUI input surface (reverse search, file popup, mouse, tooltips) | open | ~15 commits | ctrl-r cancel bug, popup tab instability, scroll bugs |
| 14 | Zotero local instance + paper search additions | open; extend `research.md` | ~10 commits past research spec | cache deserialization, local fallback opt-out, date-bounded search |

## Top components: what to test and how

### 1. Reading view / document reader

History: section duplication and fold loss on re-present, token gating
in prompt, replay guard bugs, "chat leak" of reading content, bounds
check panic, pack bug while browsing, empty-section merging,
auto-navigation on reopen, browser-mode WebSocket buffering for late
connections, resume auto-opening the browser when it should not,
figure extraction accuracy and asset-route availability on first use.

- Present the same document twice in one session and across a
  resume. Sections must not duplicate, fold state must survive
  re-presentation, and a resumed session must not replay or re-open
  the view uninvited. Motivated by: "prevent reading-view section
  duplication and fold loss on re-present", "resume replay check
  moved before browser mode block", "don't auto-open browser reading
  view on resume".
- Verify containment between the view and the chat: content shown in
  the reading view must not leak into chat history or the model
  prompt beyond what the gating intends. Check the session JSONL for
  what actually entered the prompt, not the pane. Motivated by:
  "reading view tokens gate", "TUI reading view space-resume, chat
  leak, and PAUSE markers".
- Drive navigation adversarially: scroll past both ends, jump between
  sections while content is still streaming, browse while a question
  is being answered, resize the terminal mid-render. Motivated by:
  "reading_view: bounds check", "fix pack in browsing reading view",
  "allow browsing reading view while questions are being answered".
- In browser mode, kill and reopen the browser tab mid-session and
  start the browser late: events must buffer and replay, the port
  must be stable, reconnect must be automatic. Motivated by: "buffer
  reading-view events for late WebSocket connections", "fixed port +
  auto-reconnect".
- Ask the agent to extract a figure from a real PDF on first use of a
  fresh session: the image must actually render (asset route mounted
  before first extraction) and the crop must contain the figure.
  Motivated by: "always mount /assets route so figures work on first
  extraction".
- Check mode interactions: reading view suppressed in plan mode, the
  setup popup's three modes (TUI / browser / disabled) each take
  effect immediately and persist as configured.

### 2. Voice mode / TTS / STT

History: a dozen distinct karaoke desync fixes (punctuation count
mismatch, equation highlight leaking to the next word, first-word
freeze, drift at figures, heading markers), TTS state leaking from
read-aloud into chat, pause/resume crashes when a file was attached,
voice settings leaking across sessions, backend swap leaving a stale
worker running, macOS say backend pause/resume.

- Round-trip test where feasible: have TTS speak a known text and
  verify the audio pipeline end to end (the founders' own TR-030 used
  a loopback device plus STT to get character-perfect verification).
  At minimum, verify start, pause, resume, stop, and speed change
  each take effect immediately and never crash with an attachment
  present. Motivated by: "voice: fix pause, fix crash, fix resume for
  cases file was attached", "pause/resume macOS say TTS backend".
- Probe state leakage: enable voice, /clear, confirm voice state is
  restored or reset per the product intent; switch TTS backend
  mid-playback and confirm the old worker stops. Motivated by:
  "voice_mode: restore after /clear", "cache tts_backend + interrupt
  running worker on backend swap", "tts_only state leak + stop button
  during pause".
- Karaoke correctness on hostile text: equations, punctuation runs,
  markdown headings, lists, figures with captions. The highlight must
  track the audio, never freeze on the first word, never run ahead at
  equation boundaries. Motivated by the long karaoke fix series
  ("karaoke sync drift from punctuation word count mismatch",
  "equation highlight leaking to next word", "remove broken pos_ms==0
  check that froze karaoke on first word").
- Verify scope: voice settings configured in one session must not
  bleed into the next unless saved as defaults. Motivated by: "scope
  voice setup to session defaults", "session-scope voice mode".
- Interrupt aggressively during narration (keys, new prompts, voice
  interrupt) and confirm clean recovery rather than stuck audio or a
  wedged composer.

### 3. PDF ingestion and compaction with PDFs

History: compaction broke in the presence of PDFs at least three
separate times across three months; PDF errors inside subagents;
large-PDF lookup; provider-specific handling (Claude, Gemini); cache
eviction for PDF URLs; attachment detection; non-PDF links handed to
the PDF path.

- Attach a local PDF and paste a PDF URL; verify the agent can read
  both, and that a non-PDF URL or a corrupt file produces a clear
  error instead of silent garbage. Motivated by: "pdf urls: handle
  non pdf links better", "pdf: handle errors better" (multiple).
- Force a compaction in a session that contains PDF content, then
  continue the conversation. The session must stay usable and the
  model must not receive mangled context. This regressed repeatedly:
  "compaction: fix handling of pdf files", "compaction: fix
  compaction with pdfs", "pdf url: fix compaction". Judge via the
  session JSONL, since the pane can look fine while the prompt is
  broken.
- Repeat the PDF read across providers (at least two of OpenAI /
  Anthropic / Gemini) since each had its own handling fixes.
- Use a PDF inside a subagent task and confirm errors surface to the
  parent rather than hanging or vanishing. Motivated by: "pdf: handle
  errors in subagents".
- Probe the URL cache: fetch the same PDF URL repeatedly across
  sessions, confirm eviction does not serve stale or truncated
  content. Motivated by: "pdf urls: handle cache eviction".

### 4. Model providers and authentication

History: Gemini OAuth built then fixed five times (thinking,
streaming, resume history); Anthropic max-tokens and API revisions;
sticky model names; default reasoning effort wrong; keyring multiple
sign-in; ChatGPT auth tokens vs OPENAI_API_KEY precedence; model
picker, model cards, and the models cache hiding newer models; missing
provider crashing instead of degrading.

- Switch providers and models mid-session and across restarts: the
  selected model must stick, reasoning effort must match the model's
  default, and the picker must reflect what the cache actually knows
  (including newly released models). Motivated by: "providers: fix
  sticky model names", "fix default reasoning effort", "models cache:
  fix displaying of newer OpenAI models", "fix model reasoning
  switching".
- Resume a session that used a non-OpenAI provider and verify the
  chat history is intact and the next turn works. Motivated by:
  "gemini: fix chat history after resuming".
- Auth precedence and degradation: with both a ChatGPT login and an
  API key present, the right credential wins; with a configured
  provider missing or signed out, the TUI degrades with a clear path
  to login rather than crashing. Motivated by: "ChatGPT auth tokens
  take precedence over OPENAI_API_KEY env", "provider: handle missing
  provider", "auth: fix multiple signin for keyring".
- Exercise a thinking/reasoning model on each provider and check the
  JSONL that reasoning and answer content are separated correctly and
  streaming does not interleave or drop chunks. Motivated by the
  "gemini: fix thinking finally" series and "gemini: fix streaming".
- Verify secrets hygiene: API keys never echo to the pane, logs, or
  the session JSONL. Motivated by: "providers: secured handling of
  api key".

### 5. Trajectory forks (cards) and whiteboard

History: both landed in the first week of June 2026, immediately
followed by CI breakage ("resolve live whiteboard ci failures" twice,
WebRTC close test instability). They hook deep into session, turn, and
app-server event machinery and have never had a behavioral run.

- Discover the surface at run time (slash commands, agent tools,
  app-server events) and map what a card / trajectory fork is from
  the user's point of view: create one, inspect it, resolve or merge
  it back. Verify fork resolution does not corrupt the main thread's
  history (check the JSONL of both trajectories).
- Fork from mid-conversation, continue both branches, then resume the
  session: both trajectories must survive a restart, and the reviewer
  flow (if exposed) must see the right branch.
- Whiteboard lifecycle: start a session, exercise the session
  controls added in "add live whiteboard session controls", end the
  session, and confirm clean teardown (no orphan processes, no
  lingering WebRTC/connection state; the close path was the flaky CI
  area).
- Run whiteboard and a normal agent turn concurrently and confirm
  neither starves or corrupts the other; the tool handler lives
  inside the turn machinery.
- Misuse probes: fork a fork, abandon a whiteboard session without
  closing, kill the TUI mid-fork. State on disk must remain
  consistent.

### 6. Feature flag system and features menu

History: a missing feature registry entry made /research crash at
runtime ("add FeatureSpec entries for 8 codex-private variants (fixes
/research crash)"); feature switching had four distinct fix commits
(zotero switching, disabling research, propagation to the app,
turn-based updates); experimental warnings were added then removed.

- Enumerate every feature in the features menu and toggle each one
  on and off in a live session; after each toggle, the dependent
  slash commands and tools must appear/disappear coherently and
  nothing may crash. The historical failure mode is a hard crash on
  a stale or missing registry entry.
- Toggle a feature mid-session and verify the change applies on the
  intended boundary (immediately vs next turn vs restart), and that
  the agent's available toolset in the session JSONL matches the
  menu state. Motivated by: "research tools: update turn based",
  "fixing features propagation".
- Toggle features that gate other features (research gates zotero,
  kb, paper skills) and verify the dependency closure: disabling the
  parent must not leave orphaned child tools callable. Motivated by:
  "research: fix zotero feature switching", "tools: fix bug in
  disabling research feature".
- Start the binary with unknown/stale feature config from an older
  version and confirm graceful handling, since upstream merges have
  repeatedly shifted the flag set.

### 7. Session continuity (resume, fork, /clear, compaction)

History: this is the axis along which other features keep breaking:
reading view replayed on resume, voice state lost after /clear, Gemini
history lost on resume, document reader events not persisted,
ephemeral rollout miscounting, non-rollout files confusing session
discovery, compact/resume/fork rollback flakiness.

- Build a session that exercises several stateful features (reading
  view open, voice on, a few tool calls, a fork), then resume it and
  diff observed behavior against the pre-resume session. Each
  feature must come back per its own contract, and nothing may
  replay side effects. This is a cross-feature sweep, not a single
  feature test.
- Fork a session and verify the fork is independent: changes in the
  fork must not appear in the original, and both must resume.
  Motivated by the compact_resume_fork rollback flakiness the
  founders repeatedly suppressed rather than fixed.
- /clear and confirm the contract: what is documented to survive
  (settings, voice defaults) survives; what must not (conversation,
  per-session state) is gone, in the JSONL too, not only the pane.
- Pollute the sessions directory with non-rollout JSONL files and
  confirm discovery still finds the right sessions. Motivated by:
  "filter non-rollout jsonl files in session dir discovery".
- Compact, then resume, then continue: the post-resume prompt must be
  built from the compacted state without losing the tail of the
  conversation.

### 8. Skills system

History: skill loader broke on multi-line YAML descriptions; custom
category system added then fixed; bundled skills needed a pre-extract
subcommand; a skill referenced the wrong binary path; the founders'
own TR-017 run verified all bundled skills register in the picker;
skill descriptions were promoted to always-in-context frontmatter.

- Enumerate the skills picker and verify every bundled skill
  registers (the founders' baseline was an exact count) and each has
  a non-empty description visible to the agent; check in the session
  JSONL that skill descriptions actually reach the model context.
- Install a user skill with hostile frontmatter: multi-line YAML
  description, unusual category, missing fields. The loader must
  parse or reject cleanly, never drop the skill silently. Motivated
  by: "handle multi-line YAML description in skill frontmatter",
  "skill loader tests for custom categories".
- Invoke a skill end to end through the in-app agent and confirm the
  skill's instructions were followed and any binary it calls resolves
  (a bundled skill once pointed at a missing ata path: "skill: fix
  ata binary path").
- Run on a fresh HOME: bundled skills must self-extract on first use
  (or via the init subcommand) and the picker must work before any
  manual setup. Motivated by: "ata internal-init-skills subcommand
  pre-extracts bundled skills".
- Verify custom categories render and filter correctly in the picker,
  and that removing a skill's source cleanly removes it.

### 9. Subagents / multi-agent

History: LSP broke specifically under subagents; PDF errors inside
subagents were swallowed; subagent notification timeouts; a dedicated
sub_agents branch; the founders' TR-027..029 manually verified spawn,
parallelism, and lifecycle once.

- Spawn a subagent for a real task and verify lifecycle in the
  parent's JSONL: spawn recorded, progress events flow, completion
  result returned, no orphan process after the parent exits.
- Run two or more subagents in parallel on tasks touching the same
  directory and confirm isolation (no interleaved writes, distinct
  session logs) and that both results reach the parent.
- Give a subagent a task that requires code intel / LSP and one that
  requires reading a PDF; both capabilities have broken specifically
  in the subagent context ("lsp: fix bug with subagents", "pdf:
  handle errors in subagents"). Errors must propagate to the parent,
  not hang it.
- Kill a subagent mid-task (or have its task fail hard) and confirm
  the parent reports the failure and remains usable.
- Confirm containment: a subagent inherits the parent's sandbox and
  workspace roots and cannot write outside them.

### 10. ATA account, Supabase, and mobile pairing (private)

History: email OTP sign-in, device-code flow, invite redemption, JWT
sub-claim extraction and session-expired handling, device
registration, auth recovery fix, session affinity and quota tracking,
subscriptions. Private code, single manual verification, no spec.

- Exercise the full sign-in flow (email OTP and device-code) against
  the real backend in a throwaway account: success, wrong code,
  expired code, and repeated sign-in must all behave; credentials
  must land in the keyring/auth store, not in logs.
- Force an expired/invalid JWT and confirm the client detects the
  session-expired state and prompts re-auth instead of failing
  opaquely. Motivated by: "extract JWT sub claim and handle session
  expired state".
- Register a device for mobile, verify it appears, then revoke and
  confirm access stops. Motivated by: "mobile daemon auth, logging,
  and device registration".
- Verify quota and subscription state surface truthfully in the TUI
  (/account) and that hitting a quota degrades with a clear message.
- All probes must use disposable accounts and clean up server-side
  state where the API allows it.

## Recurring patterns worth standing tests for

These are not components but failure shapes the history repeats:

1. **Upstream merges break ATA wiring.** Every upstream merge
   (v0.115, v0.119 alphas, v0.121, v0.130, v0.134) was followed by
   20-50 fix commits re-wiring reading view rendering, auth/provider
   pickers, feature flags, schemas, and snapshots. A post-merge smoke
   spec that sweeps the top components above would catch this class
   the day of the merge instead of over the following week.
2. **Resume is the universal regression trigger.** Reading view,
   voice, Gemini history, and document reader events each broke under
   resume independently. Any new stateful feature should get a resume
   probe by default (covered by the session continuity spec, #7).
3. **Compaction x rich content.** PDFs broke compaction three times.
   When new content types enter the prompt (figures, whiteboard
   state, cards), compaction with that content present is the first
   thing to probe.
4. **Registry drift crashes at runtime.** Feature specs, tool
   registries, and prompt-inspector entries all rot when code moves;
   the observed failure is a crash on a slash command. Enumerate-and
   -invoke-everything probes (every feature, every skill, every slash
   command) are cheap and catch this.
