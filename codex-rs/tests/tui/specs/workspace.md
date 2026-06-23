# Workspace — behavioral spec

This spec describes what the workspace feature does for users and
agents, and the outcomes that must hold. It deliberately avoids
implementation detail (field names, exact error strings, on-disk
layout): the testing agent discovers concrete syntax at run time via
`ata workspace --help` and `search-commands`, so code-level churn does
not invalidate this document. If a behavior here changes, that is a
product decision, not a refactor.

## What a workspace is for

A workspace is a self-contained environment that ata manages for
multi-repo and research work. The use cases it must serve:

1. **Multi-repo work**: gather several repos in one place under short
   aliases, pin any of them to exact commits, and reproduce the whole
   set elsewhere from a small exported spec file.
2. **Execution runs**: spin up isolated working copies off a repo for
   experiments, track their status, and tear them down cleanly.
3. **Resource tracking**: register papers, datasets, artifacts, links,
   and similar resources so they can be found later without
   re-discovery.
4. **Containment and accountability**: when ata's own agent does the
   above, everything it touches stays inside the workspace, and the
   human can reconstruct what was done from an audit trail.
5. **Multiple contexts**: several workspaces coexist; switching between
   them is explicit, and nothing leaks from one into another.

There are three doors to the same state, and they must agree:

- the `ata workspace` CLI — the full surface,
- `/workspace` in the TUI — a small read/switch dispatcher,
- the workspace agent skill — ata's agent driving the CLI under
  containment rules.

## Capabilities and required behavior

### Lifecycle and switching

- A user can create, list, inspect, select, and delete workspaces.
  Creation reports the new workspace's identifier in a form that
  scripts can capture cleanly.
- A default workspace always exists and cannot be deleted.
- Deletion is destructive, so it requires an explicit force step;
  without it, nothing is deleted and the user is told why.
- Selecting a workspace persists across processes: a freshly started
  TUI (and the workspace CLI) resolves to the selected workspace, and
  the selected workspace's root becomes a writable sandbox root on
  boot. The working directory does NOT change — ata runs where it was
  launched. An already-running TUI keeps its existing sandbox roots
  and must tell the user a restart is needed for the new root.
- Selectors are forgiving but never guessy: an exact id or name
  resolves; an ambiguous name (duplicates) is an error that lists the
  candidates; a near-miss is an error that suggests close matches. A
  partial match must never silently pick one.
- Stale state self-heals: a selection pointing at a deleted workspace
  falls back gracefully rather than erroring forever.

### Repos

- Adding a repo is one command that does the whole job: validates the
  URL and the workspace's host policy, clones efficiently (a shared
  cache across workspaces; the configured clone policy governs depth
  and scope), registers the repo under its alias, and records the
  operation in the audit trail. The recorded state must match what is
  actually on disk.
- A repo can be pinned to an exact commit and unpinned. Pinning
  records intent; applying a spec (materialize) is what makes the
  working copy match the pin. After materializing, the checkout is at
  the pinned commit — verifiably, on disk.
- Removing a repo removes both the manifest entry and the files, and
  is audited. No orphans in either direction.
- Host policy: by default any https host is allowed; a workspace can
  restrict hosts to a list, and a blocked host is refused with an
  error that names the host and the policy.

### Runs

- A run is created off a registered repo, gets its own isolated
  working area plus standard places for outputs and logs, and starts
  in a created status. Status can be updated through its lifecycle.
- Removing a run cleans up everything it created, including any git
  worktree linkage to the source repo.

### Resources and manifest integrity

- Generic resource entries (links, datasets, artifacts, snapshots,
  indexes, papers) can be added, read back, and removed. Bad input
  (malformed JSON, unknown collection, unknown id) is a clear error
  and changes nothing.
- Every mutation is atomic and serialized: concurrent writers cannot
  corrupt the manifest or lose each other's writes, and a version
  counter advances with every change.
- A consistency check (`validate`) detects drift between the manifest
  and the disk in both directions: registered things that are missing,
  and things on disk that are not registered.

### Path resolution and containment

- Logical locations (a repo file, a run directory, notes, caches) are
  resolved to real paths through a resolver, which is the only
  sanctioned way to obtain workspace paths.
- Containment is absolute: traversal (`..`), absolute-path injection,
  and symlinks that point outside the workspace must all be refused.
  No resolved path may land outside the workspace.
- A small set of alias names is reserved and cannot be claimed by
  repos: both the resolver's own `@`-namespaces (`run`, `kb`, ...) and
  the workspace's on-disk top-level directories (`runs`, `indexes`,
  `knowledge-base`, ...). A repo aliased `runs` would collide with the
  `runs/` namespace, so it is refused.

### Audit

- Significant operations (the compound repo/run commands) leave audit
  entries automatically; an audit entry records what happened, to
  what, when, and in which workspace. The log can be queried and
  filtered (by operation, time, count).
- Callers (including the agent) can append their own entries for
  operations that are not auto-audited.

### Locking

- A command can be run under a workspace-scoped lock so that two
  actors doing conflicting work serialize instead of interleaving: the
  second waits for the first. The locked command's output and exit
  code pass through unchanged. Lock targets are validated against the
  same containment rules as paths.

### Reproducibility (spec round-trip)

- A workspace can be exported as a minimal spec (repos, pins, labels),
  diffed against another workspace, and materialized into a fresh
  workspace, reproducing the repo set and pins exactly. A dry-run
  shows the plan without changing anything.
- Garbage in a spec file must not be silently ignored: a key that is a
  plausible misspelling of a behavior-bearing field (pin-shaped names,
  one-character typos, transpositions) is a hard error naming the
  intended field; other unrecognized keys pass through with a warning
  (custom metadata keys are an intentional extension point).

### TUI (`/workspace`)

- The TUI command covers exactly: show the current workspace, list
  workspaces (with the active one marked), and switch. Bare invocation
  explains usage and points at the CLI for everything else.
- It reads live state: changes made by the CLI while the TUI is open
  show up. Read-only queries are allowed during an in-flight model
  turn and don't disturb it.
- Switching follows the same selector rules as the CLI and gives the
  restart caveat described above; a failed switch leaves the selection
  untouched.

### Agent skill contract

When ata's agent does workspace work, its behavior must follow the
skill's containment rules: resolve paths only through the resolver,
check host policy before cloning, prefer the compound commands over
raw git/file operations, scope every mutation to an explicit
workspace, and leave an audit trail for significant work. An agent run
that reaches the right end state the wrong way (raw clone, hand-built
path, unscoped mutation) is a failure; the session log is the
evidence. The skill must actually activate for workspace-shaped
requests — a perfect end state with the skill never loaded means the
contract was never in force.

## How to test it

Work through the capabilities above with the real binary, discovering
exact syntax via `--help`/`search-commands` as you go. Prefix
everything you create (e.g. `wstest-`), restore the default selection,
and delete all of it at the end, pass or fail.

For anything that claims a disk effect, check the disk, not just the
command output: does the checkout exist and sit at the claimed commit,
is the run's worktree really gone after removal, did the shared clone
cache actually get used. A tiny public repo with more than one branch
(e.g. `octocat/Hello-World`) makes a good fixture: pin to a non-default
branch head so a pin check can't pass by accident.

Then go adversarial — minimum classes, invent more:

- **Containment**: traversal and absolute paths through every resolver
  form; reserved aliases; a symlink inside a checkout pointing outside
  the workspace, resolved through.
- **Malformed input**: broken JSON, unknown collections/ids, non-hex
  pins, non-URL clones, empty selectors.
- **Tampering**: hand-delete managed dirs, drop stray dirs in, corrupt
  the manifest file — does the tool detect, report, and stay usable
  (does a corrupted workspace disappear silently, or is the user
  told)?
- **Concurrency**: parallel mutation storms against one workspace
  (nothing lost, nothing corrupted, version advances); a lock holder
  plus a contender (waits, doesn't interleave).
- **Silent failure hunting**: wherever input is accepted with a
  success exit, verify the implied effect actually happened. This
  class found the spec-file pin bug.

For the TUI layer, drive `/workspace` through tmux (recipe in the
README) and check the cross-process stories: CLI-created workspace
visible live, CLI selection honored on next boot, restart caveat given.

For the skill layer — the part scripts can't do — boot the TUI and
give the in-app agent real workspace tasks in your own words: set up a
workspace with a repo; ask where a file inside a repo lives; switch
workspaces; give it a task whose easy shortcut would break
containment. After each turn, judge the trajectory, not the prose:
read the session JSONL for what was actually executed, confirm the
skill was injected at all, and compare against the contract above.
Vary the wording between runs; verbatim reuse turns this back into a
script.

Report per the README: issues with exact reproduction commands,
divergences citing the section above, skill-contract violations
quoting the session log, and coverage notes.
