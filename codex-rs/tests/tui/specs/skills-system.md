# Skills system — behavioral spec

This spec describes what the skills system does for users and what
outcomes must hold. It avoids implementation detail (struct names,
exact frontmatter fields beyond the documented ones, cache file
formats): the testing agent discovers concrete syntax at run time. If
a behavior here changes, that is a product decision, not a refactor.

## What skills are for

A skill is a markdown instruction package (a `SKILL.md` with YAML
frontmatter plus optional supporting files) that teaches ata's agent
how to do a specific job. Skills are how most ATA features reach the
agent: research, workspace management, environment adaptation, spec
writing, and more all ship as bundled skills. The system must serve:

1. **Bundled capability**: skills shipped inside the binary are
   available out of the box, on a fresh machine, with zero setup.
2. **User extension**: a user can drop their own skills under their
   ata home's skills directory and have them picked up.
3. **Repo-local skills**: a project can carry skills in its repo
   (`.codex/skills/`) that apply when ata runs in that project.
4. **Advertising**: the agent knows what skills exist — names and
   descriptions are placed in its context every session, so it can
   choose to use one without the user naming it.
5. **Activation**: a skill's full instructions enter the conversation
   when invoked — explicitly (user or agent names it) or implicitly
   (the agent runs a skill's script or reads a skill's doc, and the
   skill body is pulled in behind it).
6. **Control**: the user can see every discovered skill and turn
   individual skills on or off from inside the TUI.

## Capabilities and required behavior

### Discovery across roots — THE regression probe

Skills are discovered from several roots, and all of them must
contribute. This wiring was lost once in an upstream merge and only
recently restored, so root coverage is the single most important
check in this spec: a pass means every root below is represented in
the live skill list, not merely that "some skills" showed up.

- **Bundled upstream skills**: extracted from the binary into the ata
  home's skills directory under a `.system` cache.
- **Bundled custom categories**: ATA's own skill categories, each
  extracted into a sibling `.system-<category>` cache directory
  (research, workspace, adapt-environment, spec, baseline-build,
  search-orchestrate at the time of writing — derive the live set
  from the binary, don't hardcode this list). Every *advertised*
  category present on disk must surface in the skill list. A binary
  whose `.system` skills load but whose advertised `.system-*`
  categories don't has regressed exactly the way the upstream merge
  broke it.
- **Programmatic-only categories**: a few `.system-*` cache
  directories hold instruction files consumed by ATA code directly by
  path (e.g. `.system-policy-advisors`, the MCGS advisor prompts).
  These deliberately carry no YAML frontmatter and are *not* advertised
  skills. They are installed to disk so the programmatic reader finds
  them, but they must never appear in the advertised list or the
  `/skills` panel. The boundary is centralized: every embedded category
  is either advertised (and therefore loadable, with valid frontmatter)
  or programmatic-only (and therefore excluded from discovery). A
  category must not straddle both.
- **User skills**: directories under the ata home's skills root
  (excluding the `.system*` caches) — each with a `SKILL.md`.
- **Repo skills**: `.codex/skills/` inside the project ata is
  running in.

Required outcomes:

- The advertised list is the union of all roots. Adding a valid skill
  to any root and starting a fresh session makes it appear; removing
  it makes it disappear. No root is silently dead.
- Same-named skills in different roots must not corrupt the list:
  both the resolution (which one wins, or both shown disambiguated)
  and the survival of every other skill must hold.
- On a completely fresh ata home, bundled skills (upstream and custom
  categories alike) self-extract before first use — the picker and
  the agent's advertised list work with no manual setup.

### Advertised list matches disk

What the agent is told exists must match what is actually on disk:

- Every discovered skill appears with a non-empty name and a
  non-empty description taken from its frontmatter. A skill
  advertised with a blank or placeholder description is a bug.
- Nothing is advertised that has no backing `SKILL.md` on disk, and
  no on-disk skill with valid frontmatter is omitted.
- The founders' baseline practice is an exact count: enumerate disk,
  enumerate the advertised list, and reconcile both directions.

### Skill content actually reaches the session

Being listed is not being usable. The strongest historical bug in
this area was a skill that appeared everywhere — picker, advertised
list — but whose body was never injected into the model context, so
the agent improvised instead of following it. Therefore:

- The names-and-descriptions advertisement must be verifiable in the
  session JSONL of a fresh session, not just on screen.
- When a skill is invoked (explicitly or implicitly), its full body
  must appear in the session JSONL as injected context before the
  model's next response. Judge by the JSONL, never by the rendered
  transcript: a model can describe a skill convincingly from the
  one-line description alone.
- A turn where the agent claims to be following a skill whose body
  never entered the context is a failure even if the visible outcome
  looks right.

### Implicit invocation

Skills activate without being named when the agent touches them:

- Running a script that lives inside a skill's directory triggers
  that skill's injection.
- Reading a file inside a skill's directory triggers the same.
- Each skill injects at most once per turn context — repeated touches
  must not duplicate the body.
- Touching files that merely look skill-adjacent (similar paths
  outside any skill root) must not inject anything.

### `/skills` panel

- `/skills` in the TUI opens a panel listing every discovered skill
  with its description.
- Individual skills can be enabled and disabled from the panel. A
  disabled skill leaves the advertised list (verify in the JSONL of
  the next session) and must not inject implicitly. Re-enabling
  restores it.
- The enable/disable choice persists across restarts.
- The panel must agree with the advertised list — same skills, same
  state. A skill visible to the model but absent from the panel (or
  vice versa) breaks the user's control surface.

### Frontmatter tolerance

The loader must be resilient per skill, not per root:

- Multi-line YAML descriptions, unusual or custom category values,
  and missing optional fields parse correctly (multi-line
  descriptions broke the loader once).
- A genuinely malformed skill (broken YAML, missing required fields,
  missing `SKILL.md`) is rejected for that skill only: every other
  skill in the same root and all other roots still load. One bad
  user skill must never blank the bundled set.
- Rejection is visible somewhere the user can find (error surface,
  log, or panel indication) — a skill that vanishes with no trace is
  a silent failure.

### Refresh after binary upgrade

Bundled skills are cached on disk and the cache is fingerprinted so
extraction is skipped when nothing changed. The required capability:

- When the binary's embedded skills differ from the cached copy, the
  cache is refreshed on startup — a user who upgrades ata gets the
  new skill content without deleting anything by hand.
- When the cache matches, startup does not rewrite it.
- Tampering with the cached copy (edit a cached `SKILL.md`, delete a
  marker, delete a whole `.system-*` directory) is healed on the next
  start rather than served stale or crashing.
- User and repo skills are never touched by this refresh — only the
  `.system*` caches are ata's to overwrite.

## How to test it

Use an isolated ata home (point the home env var at a temp dir) so
fresh-extraction, tampering, and enable/disable probes can't damage
the real one. Discover exact paths and panel syntax at run time.

- **Roots first**: on a fresh home, diff the on-disk skill set
  (each `.system*` cache, user root, repo `.codex/skills/`) against
  the advertised list in a fresh session's JSONL. Then plant one
  marker skill per root with a distinctive name and verify each
  arrives. The custom `.system-*` categories are the regression
  probe — check them individually, by name.
- **Injection**: pick a bundled skill and a planted skill; invoke
  each explicitly and implicitly (run its script, read its doc), and
  find the body in the JSONL. Also run a turn that merely sounds like
  a skill's domain without touching it, and confirm what was and
  wasn't injected matches the rules above.
- **Hostile frontmatter**: plant skills with multi-line descriptions,
  odd categories, missing fields, broken YAML, and an empty
  directory. Per-skill rejection, everything else loads, rejection
  visible.
- **Panel**: drive `/skills` through tmux (recipe in the README):
  list completeness, disable → JSONL of next session, restart
  persistence, re-enable.
- **Upgrade refresh**: corrupt and delete pieces of the `.system*`
  caches between runs and confirm healing; touch nothing and confirm
  the cache isn't rewritten (compare mtimes); plant a user skill and
  confirm refresh leaves it alone. If two ata binaries of different
  versions are available, point both at one home and confirm the
  cache follows the running binary.

Then go adversarial on your own judgment: same-named skills across
roots, a skill directory that is a symlink, a skill whose script
deletes its own `SKILL.md` mid-turn, enormous descriptions against
the metadata budget, disabling a skill mid-session.

Clean up the temp home and any tmux sessions regardless of outcome.
Report per the README: issues with exact reproductions, divergences
citing sections above, and JSONL excerpts for every injection claim.
