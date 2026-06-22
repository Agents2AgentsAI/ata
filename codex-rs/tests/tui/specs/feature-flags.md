# Feature flags — behavioral spec

This spec describes what the feature flag system does for users and the
outcomes that must hold. It deliberately avoids implementation detail
(flag names, registry structure, exact menu wording): the testing agent
discovers the live flag set at run time via the features menus
(`/experimental`, `/research`), `--help`, and the config file, so churn
in the flag roster does not invalidate this document. The historical
failure mode here is a hard crash at runtime when a flag exists in
config or code but is missing from the feature registry — registry
drift after merges is a recurring class, so the core probe is
enumerate-and-toggle-everything, not spot checks.

## What the flag system is for

ATA ships features at different maturity levels behind flags. The
system must let a user:

1. **Discover** what optional features exist, with their current
   on/off state, through in-app menus: a general experimental-features
   panel (`/experimental`) and a research-tools panel (`/research`).
2. **Persist** a choice: toggles saved from a panel are written to the
   user's config and hold across restarts.
3. **Override per session**: CLI switches (`--enable <flag>` /
   `--disable <flag>`) flip a flag for that process only, without
   touching the saved config.
4. **Trust the gate**: an enabled feature's surface (slash commands,
   tools, menus) is present; a disabled feature's surface is absent —
   for the user and for the in-app agent alike.

## Capabilities and required behavior

### Enumeration and toggling never crash

- Every flag the binary knows about — everything visible in both
  panels, everything accepted by `--enable`/`--disable`, everything
  representable in the config file — can be toggled on and off, in any
  combination, across restarts, without a crash, a hang, or a process
  exit. A flag that can be set but crashes the binary when its surface
  is touched is the bug this spec exists to catch: a missing registry
  entry once made `/research` crash at runtime, and the panels
  themselves have exited the process on save. Saving a panel must
  return to chat with the TUI process still alive.
- Config written by an older or newer version may contain flags this
  binary does not know. Unknown or stale flag entries must be handled
  gracefully (ignored or warned about), never crash at startup or when
  a panel opens. The flag roster shifts with upstream merges; this is
  the drift case.
- Unknown flag names given to `--enable`/`--disable` are a clear error
  naming the flag, not a silent no-op and not a crash.

### Cancel writes nothing; save writes exactly the toggles

This is a just-fixed bug class and is required behavior:

- Dismissing a panel without saving (Esc) changes nothing: not the
  running session, not the config file on disk. Byte-compare the
  config before and after an open–toggle–Esc sequence.
- Saving (Enter) persists exactly the toggles the user made — no
  more, no fewer. Flags the user did not touch keep their prior
  values; flags toggled then toggled back are unchanged.
- Each panel owns its flags: saving one panel must not rewrite or
  reset flags owned by the other.
- **The result of a save must round-trip.** Whatever the panel showed
  enabled when the user pressed Enter must still be enabled after a
  restart. A panel whose save mechanism clears a master/parent flag (the
  `/research` panel does this so per-family flags take sole authority)
  must persist *every* enabled family explicitly, not just the changed
  rows — otherwise the dependency closure that turns parent-off into
  children-off force-disables an untouched-but-enabled family that
  carries no config key, and toggling one row off silently takes down the
  whole surface on reload. Probe it: enable several families, toggle one
  off, save, restart, and confirm the others survived.

### Persistence and session-only overrides

- A panel-saved toggle survives restart: the panel shows the saved
  state on next launch and the feature behaves accordingly.
- `--enable`/`--disable` apply for that process only. After the
  process exits, the config file is unchanged and a plain restart
  reverts to the saved state. A session-only override must never be
  silently promoted to a persisted setting (e.g. by opening and
  saving an unrelated panel while the override is active).
- When a session override and the saved config disagree, the running
  session follows the override, and what the panel displays must not
  lie about what is in effect.

### Gating is real, on a defined boundary

- An enabled feature actually adds its surface; a disabled one
  actually removes it. "Removed" means the slash command is not
  dispatchable and the tool is absent from what the model is offered
  — judge by the session JSONL (the toolset and tool calls recorded
  there), not the pane. A disabled feature whose tool the agent can
  still call is a gating failure even if no menu shows it.
- The inverse must also hold: a feature whose description advertises a
  named tool must actually deliver that tool when enabled. If a flag's
  menu wording says it "adds the X tool", then with the flag on, X must
  appear in the offered toolset (and the agent must be able to call it);
  if the agent, asked "do you have access to X?", says no, the feature
  is advertised-but-unwired — a gating failure as real as the disabled
  case. Verify every flag that names a tool, not just one.
- Flag changes apply on a stated boundary. The panels say saved
  changes take effect for the next conversation: the running session
  must not half-apply them, and a fresh conversation must fully
  reflect them. Whatever the boundary is (immediate, next turn, next
  conversation), observed behavior must match what the UI claims.
- Flags that gate other features (research gates its sub-tools)
  enforce the dependency closure: disabling the parent leaves no
  orphaned child surface callable.
- Panels respect turn state: while a model turn is in flight, the
  panel either refuses to open with a clear message or behaves safely
  — it must not corrupt the in-flight turn.

### Defaults match documentation

- On a fresh HOME with no config, each flag starts at its documented
  default (`--help`, docs, panel hints). Experimental features default
  off unless documented otherwise. The panels on first launch must
  show exactly that default state, and the defaults in code, panel,
  and docs must agree.

## How to test it

Run the real binary against a throwaway HOME so config writes are
yours to inspect and discard. Discover the flag roster at run time:
open `/experimental` and `/research`, list every row and its state,
and cross-check against the config file and `--help`.

The core probe is exhaustive: for every flag in both panels, toggle it
on, save, restart, verify state and surface; toggle it off, save,
restart, verify again. Slow, but this is the probe that catches
registry drift — a missing entry tends to crash only when the specific
flag's surface is exercised. After each enable, poke the feature's
surface (its slash command, its tool via a real model prompt) and
check the session JSONL for which tools were actually offered and
called.

Then the targeted probes:

- **Esc/Enter discipline**: snapshot the config file; open a panel,
  toggle several rows, Esc; the file must be byte-identical. Repeat
  with Enter; diff the file and confirm exactly the toggled flags
  changed.
- **Session overrides**: launch with `--enable` of a default-off flag
  and `--disable` of a default-on one; confirm both take effect in the
  session and the config file never changes; restart plain and confirm
  reversion. Try an unknown flag name and expect a named error.
- **Drift simulation**: hand-edit the config to contain a plausible
  but unknown flag key, and remove a known one; the binary and both
  panels must start and open cleanly.
- **Gating via JSONL**: with a feature off, ask the in-app agent to do
  the thing the feature provides; the gated tool must not appear in
  the offered toolset nor be callable. Enable it, start a fresh
  conversation, and confirm the tool now appears and works.
- **Advertised-tool delivery**: for every flag whose menu wording
  *unconditionally* names a tool it "adds", enable the flag, start a
  fresh conversation, and confirm the named tool is actually offered —
  check the session JSONL toolset and ask the agent directly "do you have
  access to `<tool>`?". A flag that unconditionally advertises a tool it
  never registers is a real defect (the agent answers no, the JSONL shows
  the tool absent). The inverse is also a defect: wording must not name an
  agent tool the build cannot register.
  - **Provider-gated tools are conditional, not unconditional.** Some
    tools only exist when a backing build feature/provider is compiled
    in. `repo_context` (semantic repository understanding) is the
    example: it is served by a private provider, so on a build without it
    the tool is correctly absent. The wording must reflect that ("where
    the backing provider is built in"), and on a non-serving build the
    agent answering "no `repo_context`" is *expected*, not a defect — the
    defect is wording that promises it unconditionally. MCGS is the
    stronger case: there is no core `mcgs_search` agent tool at all (the
    engine is reached through the `codex mcgs run` CLI and the MCP search
    path), so the flag must not promise one. Sweep all such flags, and
    distinguish "unconditional promise, tool absent" (defect) from
    "conditional wording, tool absent because the provider isn't in this
    build" (correct).
- **Boundary honesty**: save a toggle mid-conversation and verify the
  running session is unaffected while the next conversation reflects
  it — the panel's claim about when changes apply must be true.
- **In-flight**: open each panel during a running turn; expect a clean
  refusal or safe behavior, never a wedged turn or crash.

Restore or delete the throwaway HOME at the end, pass or fail. Report
per the README: issues with exact reproductions, divergences citing
the section above, and which flags were and were not exercised.
