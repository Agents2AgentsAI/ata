# Model providers and authentication — behavioral spec

This spec describes how ata selects, switches, authenticates, and
fails over between model providers (OpenAI, Anthropic, Gemini, the ATA
account, and whatever else the binary exposes), and the outcomes that
must hold. It avoids implementation detail (exact model names, picker
wording, config keys): the testing agent discovers the concrete
surface at run time via `/model`, `--help`, the login/account
commands, and the session JSONL. If a behavior here changes, that is a
product decision, not a refactor.

Token budget warning: this component is the one place where careless
probing spends real money on multiple providers at once. Every probe
that reaches a live model must use a one-line prompt with a one-word
expected answer ("respond with just hi"). Prefer probes that never
reach a model at all: picker navigation, credential-absent paths,
auth-state inspection, and JSONL reads are free. Never loop a prompt
to "see if it's flaky".

## What this surface is for

1. **Model choice**: the user picks a model and a reasoning effort,
   in-session, and the choice takes effect on the next turn.
2. **Multi-provider**: models from different providers coexist in one
   install; the user can move between them mid-session and the
   conversation continues coherently.
3. **Auth**: each provider has its own credential path (OAuth login,
   API key, keyring entry, ATA account); the user can sign in, sign
   out, and tell at a glance which identity each provider is using.
4. **Honesty under failure**: when a provider can't serve a request
   (no credential, invalid key, quota, outage), the user is told what
   happened and what to do. ata must never quietly answer with a
   different model than the one the user selected.

## Capabilities and required behavior

### Model selection (`/model` two-step flow)

- `/model` opens a picker; choosing a model leads to a second step for
  reasoning effort; confirming applies the pair and reports the new
  model and effort, and the session's status surface agrees with what
  was reported.
- Backing out of step two returns to step one, not to chat; backing
  out of step one cancels with nothing changed.
- Stepping through both steps without changing anything is a clean
  no-op.
- The effort step marks the currently-active setting only when the
  chosen model is the active model; for a different model it shows
  that model's default, with a sane default effort per model (default
  effort has been wrong before — verify, don't assume).
- The picker reflects what the install actually knows: a curated
  visible set plus an escape hatch for models outside it, and the
  models cache must not hide newer models the provider has released.
- The picker is unavailable while a turn is in flight, with a clear
  message — it must not open into a corrupted state.

### Switching and stickiness

- A model selected via the picker is used for the very next turn.
  Verify in the session JSONL which model actually served the turn;
  the pane confirmation alone has lied before (sticky model names).
- The selection persists where it should: across `/clear`, across
  process restart, and across resume of the same session, per
  whatever scope the product defines — but whatever the scope is, the
  displayed model and the serving model must never disagree.
- Switching to a model on a *different provider* mid-session keeps
  the conversation: prior turns remain visible and the new provider
  receives coherent history (check the JSONL for what was sent, since
  each provider re-encodes history differently).
- Resuming a session that ran on a non-OpenAI provider must restore
  the full chat history and accept a next turn. Gemini specifically
  lost resume history once; treat any non-default provider + resume
  combination as a regression hotspot.

### Thinking and streaming

- On a reasoning-capable model, reasoning content and answer content
  are separated correctly in the JSONL — not interleaved, not
  dropped, not duplicated — and the stream renders progressively
  rather than arriving in one late lump. Gemini's thinking and
  streaming each took multiple fixes; one short reasoning prompt per
  provider is enough to check the seams.
- Changing reasoning effort actually changes what is requested
  (verify in the JSONL request metadata, not the pane).

### Authentication and precedence

- Each provider's auth status is inspectable without spending tokens,
  and shows which credential source is in use.
- When multiple credentials exist for one provider (e.g. a ChatGPT
  login and an API key in the environment), a documented precedence
  decides — historically the ChatGPT token wins over the env key —
  and the active source is visible to the user, not silent.
- Multiple sign-ins must coexist in the keyring without clobbering
  each other: signing into a second provider (or a second account)
  must not log the first out or corrupt its entry.
- Signing out removes the credential from the store and the provider
  immediately reflects the signed-out state.
- Secrets hygiene is absolute: API keys and tokens must never appear
  in the pane, the logs, or the session JSONL — including in error
  messages about those same credentials.

### Failure and degradation

- A model whose provider has no credential: selecting it (or starting
  a turn on it) produces a clear error naming the provider and the
  way to sign in. No crash, and the TUI stays usable — a missing
  provider has crashed the binary before.
- An invalid or revoked key: the provider's rejection surfaces as an
  honest auth error, not a generic failure and not a retry loop.
- **No silent model substitution, ever.** If the selected model is
  unavailable (quota exhausted, key invalid, model retired), ata must
  say so and stop or ask — it must not transparently answer with a
  cheaper or different model while still displaying the selected one.
  Recent live runs saw a silent fallback to a mini-tier model when
  quota ran out; the JSONL showed the substitute model serving turns
  the UI attributed to the selected one. Probe this class hard: any
  mismatch between the displayed model and the serving model in the
  JSONL is an issue, whatever the cause.

### Fast mode (`/fast` service tier)

- `/fast` toggles a faster service tier on the current provider; the
  toggle state is visible, and the JSONL request metadata reflects
  the tier actually requested.
- The tier interacts with model selection coherently: switching
  models or providers while fast mode is on must either carry the
  tier (if the target supports it) or say it doesn't apply — not
  silently drop it, and not send an unsupported tier the provider
  rejects.
- Toggling off restores the normal tier on the next turn.

## How to test it

Free probes first, and exhaust them before spending a token:

- Drive `/model` through tmux (recipe in the README): two-step
  navigation, back-navigation, no-op apply, the in-flight block,
  picker inventory vs the escape hatch. None of this calls a model.
- Inspect auth state per provider via the relevant commands. Note
  which credential sources exist on this machine before touching
  anything, and restore them at the end.
- Credential-absent paths: in a scratch HOME (or with the relevant
  env vars unset and keyring entries absent), select each provider's
  model and start a turn. The expected outcome is an error, so a
  correct run costs nothing. Same for an obviously-invalid key
  (e.g. `OPENAI_API_KEY=sk-invalid...`): the turn must fail honestly.
  Never modify or delete the user's real keyring entries — use
  isolation, not destruction.

Then the paid probes, each a single one-line prompt:

- One turn per available provider to confirm the selected model
  serves it (JSONL is the arbiter).
- One provider switch mid-session, then one turn.
- One resume of a non-OpenAI session, then one turn.
- One short reasoning prompt per reasoning-capable provider for the
  thinking/streaming checks.
- One turn with `/fast` on, checking the JSONL tier metadata.

For precedence, set up both credential sources for one provider and
run a single cheap turn, then read the JSONL/auth state to see which
source served it — one turn answers the question.

For the no-silent-substitution class, you usually cannot exhaust a
real quota on purpose (and must not try). Instead: compare displayed
model vs JSONL serving model on every paid probe above as a standing
check, and use the invalid-key path to verify the failure shape is an
error rather than a swap. If any session lying around from earlier
runs hit a quota, mine its JSONL for substitution evidence for free.

Adversarial extras, invent more: kill the TUI mid-picker and restart
(selection must be the old or the new value, never a corrupted
between-state); switch models while a turn is in flight via any path
the UI leaves open; point the picker at the escape-hatch model name
with a typo and confirm the error is honest; grep every artifact you
produced (pane captures, logs, JSONLs) for fragments of the keys you
used.

Clean up: restore the original model/effort selection, fast-mode
state, and every credential and env var you touched, pass or fail.
Report per the README: issues with exact reproductions, divergences
citing the section above, and for every substitution or stickiness
finding, the JSONL lines that prove it.
