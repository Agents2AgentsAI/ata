# Knowledge base — behavioral spec

This spec describes what the research knowledge base (KB) does for
users and agents, and the outcomes that must hold. Like
`workspace.md`, it avoids implementation detail; the testing agent
discovers concrete syntax at run time (`ata workspace --help`,
`search-commands`, the in-app skill text). Containment, locking, and
resolver mechanics are specified in `specs/workspace.md` — this spec
only states what is KB-specific and cross-references the rest.

## What the KB is, honestly

The KB is deliberately thin. It is not a database or a service; it is:

1. a **directory convention** — each workspace owns a knowledge-base
   area with standard subareas (cards, topic overviews, briefings,
   explanations, assets, staging) plus an index file, a research
   journal, and a research-context file;
2. a **resolver namespace** — `@kb/...` resolves into that area, with
   the same containment guarantees as every other workspace path, and
   the `kb` alias is reserved so a repo can never shadow it;
3. **per-turn plumbing** — when the feature is enabled, every model
   turn resolves the active workspace's KB location, exposes it to all
   shells the turn spawns (tool calls, subagents, the user shell), and
   makes that directory writable without approval prompts;
4. a **skill contract** — a KB skill teaches the agent the card
   format, the file operations, reset semantics, and degradation
   rules; other research skills (paper synthesis, briefings, HN
   synthesis, conversation reports) read and write through it.

There is no KB-specific CLI command surface beyond the generic
workspace ones (resolve, locked execution). Test the convention and
the plumbing; do not expect commands that don't exist.

There is also no native KB search/list *agent tool*, and that is by
design. Unlike the API-backed research families (paper search, HN,
patents, Zotero, repo analysis) which expose their own tools, the KB is
plain files under the workspace. The agent searches and lists cards with
ordinary shell tools (`rg`/`grep`/`ls`/`cat`) over the exposed KB path —
that is the intended interface. The absence of a `kb_search`-style tool
when the feature is enabled is correct, not an advertised-tool-delivery
defect; do not flag it as one.

## Capabilities and required behavior

### Workspace scoping

- Each workspace has its own KB; nothing written in one workspace's
  KB appears in another's. A global/default workspace serves sessions
  with no explicit selection.
- The KB location a turn sees follows the active workspace, with the
  same precedence as workspace selection generally (a project-pinned
  workspace beats a session selection; both beat the default).
- Stale selection self-heals: if the selected workspace no longer
  exists, KB operations fall back to the default workspace rather
  than erroring or writing into a dead path.
- Creating a workspace provisions the KB skeleton (the standard
  subareas exist on disk).

### Plumbing and sandbox

- With the feature enabled, every shell spawned during a turn —
  direct tool calls, subagent shells, and the interactive user shell —
  agrees on the same KB location, exposed through the shell
  environment. The agent can write under it without an approval
  prompt, in every sandbox mode that otherwise restricts writes.
- The writable carve-out is exactly the KB directory. It must not
  widen: a path that merely starts with a similar prefix, or escapes
  via traversal or symlink, gets no exemption.
- The feature is toggleable from the TUI's research-tools panel
  (listed as "Knowledge Base", on by default). Disabled, the plumbing
  is absent: skills that consume the KB must degrade per their own
  rules (work from conversation context, or run shallow discovery and
  say so) instead of failing or silently writing anyway.
- Disabled means the agent is told *nothing* about the KB: no
  `CODEX_KB_PATH`, no KB skill, no mention of a KB or its location in
  the prompt. That is the whole contract. It is NOT a confidentiality
  boundary: the card files still exist on disk, and a fully-permissioned
  shell agent that goes looking with broad filesystem search (`rg`/`grep`
  over `$HOME`/`/tmp`) may still find and read them. That is the agent's
  own doing, out of scope, and not a defect — disabling the flag removes
  the KB from what we hand the agent, it does not sandbox the user's own
  files away. Confidentiality is the sandbox's job, not a feature flag's.

### Resolver namespace

- `@kb` resolves to the KB root of the addressed workspace; `@kb/sub`
  resolves inside it. Traversal, absolute injection, and reserved-
  alias claims are refused exactly as in the workspace spec.
- The resolver answer and the per-turn environment answer must agree:
  for the same workspace, both name the same directory.

### Locking

- KB writes can be serialized through the workspace locked-execution
  command at the KB lock level (see workspace spec for lock ordering
  and pass-through semantics). Two locked KB writers serialize; the
  loser waits rather than interleaving.

### Content contract (the skill)

When the agent does KB work, the skill defines required behavior:

- **Cards** are one file each: structured metadata header (id, title,
  tags, status, source, dates, optional figures) plus a markdown
  body. Ids are kebab-case. New cards get a creation date; edits get
  an update date and register new tags in the index.
- **The index** tracks the tag taxonomy and per-topic staleness so
  topic overviews know when to regenerate.
- **Conversation insights** are appended to a card's discussion-notes
  section with a date, never by rewriting the original synthesis
  sections. Cross-card connections update both cards.
- **Reset** ("clear/reset/wipe the KB") is an immediate action: no
  clarifying questions, all content areas emptied, the index and
  journal reset, and the research-context file preserved. The user is
  told how many cards were deleted.
- **Degradation**: missing KB → say so and offer to initialize;
  missing index → proceed without; missing research-context → skip
  personalization. None of these may crash a skill or be silently
  papered over.
- **Voice**: KB-fed output (briefings, explanations) never names the
  KB as a source. The KB is infrastructure, not a citation.

An agent that reaches the right end state the wrong way — writing
outside the KB, hand-building the path instead of using the
environment/resolver, skipping the index update, rewriting synthesis
sections — fails the contract even if the files look right.

### Consumers (boundary, not duplication)

Paper synthesis writes cards through a staging area and cleans
staging up afterward; briefings read cards and follow the three-path
degradation above; figures saved into the KB's assets area must be
servable to the reading view for the workspace that owns them. The
internals of those skills belong to their own future specs; here,
test only the KB-touching edges: cards actually land, staging doesn't
leak, asset figures actually render for the active workspace.

## How to test it

Prefix everything (`kbtest-`), restore the default workspace
selection, and delete everything you created at the end, pass or
fail.

Ground truth is the disk. For every claim, check the directory of the
workspace you addressed: did the card file appear there and not in
another workspace's KB, did the index change, did staging empty out.
A success message with no file is an issue; a file in the wrong
workspace is a worse one.

Mechanical probes (CLI + filesystem):

- Create two workspaces; verify each gets the KB skeleton; resolve
  `@kb` in both and confirm distinct, contained paths that match what
  a fresh TUI turn's environment reports for the selected workspace.
- Selection precedence: project pin vs session selection vs default,
  each time checking where a KB write actually lands.
- Delete the selected workspace out from under a session and confirm
  the fallback-to-default behavior, with no writes into the deleted
  path.
- Locked KB execution: two concurrent locked writers appending to the
  same file — nothing lost, no interleaving.

Adversarial classes — minimum set, invent more:

- **Containment**: `@kb/../`, absolute paths, a symlink inside the KB
  pointing outside, a repo alias of `kb` — all refused (workspace
  spec rules apply; verify they hold through the KB door too).
- **Sandbox edges**: in a write-restricted sandbox mode, confirm a
  write inside the KB needs no approval while a write to a sibling
  directory with a similar name still does. Then disable the feature
  and confirm the carve-out is gone.
- **Malformed content**: drop a card with a broken metadata header, a
  garbage index file, a card whose id violates the naming rule — do
  KB-consuming flows report and continue, or crash/silently skip?
- **Tampering**: delete the index or the whole KB directory mid-flow;
  the skill's degradation rules say exactly what should happen.

Live-model TUI tasks (tmux recipe in the README; judge the session
JSONL, not the rendered prose — confirm the skill loaded and check
which paths were actually written). Vary the wording every run:

- "Save what we just figured out about X as a knowledge card" — card
  lands in the active workspace's KB, valid header, index updated.
- "What do I already have on <topic>?" — reads through the KB,
  doesn't re-research what's already carded.
- Ask a follow-up that yields a real insight, then "remember that" —
  discussion notes appended, synthesis sections untouched.
- "Wipe my knowledge base" — immediate, no questions, context file
  survives, count reported. Verify on disk.
- Switch workspaces mid-session (restart as required), repeat a save,
  and confirm the write followed the workspace.
- A task whose easy shortcut is writing to a hardcoded home path
  instead of the workspace KB — the trajectory must use the
  environment/resolver, not a guessed path.
- With the KB feature disabled, ask for a briefing — confirm the agent
  was handed nothing about the KB (no `CODEX_KB_PATH`, no KB skill, no KB
  mention in the prompt), the degradation path was taken and disclosed,
  and there are no KB writes in the JSONL. Do NOT count it against the
  feature if a yolo agent then runs its own broad filesystem search and
  happens to read a card file: that is the agent circumventing on its
  own, not the disabled flag leaking. The defect would be the flag
  telling the agent about the KB; finding files by independent search is
  out of scope.

Report per the README: issues with exact reproduction commands,
divergences citing the section above, skill-contract violations
quoting the session log, and coverage notes.
