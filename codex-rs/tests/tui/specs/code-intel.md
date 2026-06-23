# Code intelligence (behavioral spec)

This spec describes what ata's code intelligence features do for users
and agents, and the outcomes that must hold. It deliberately avoids
implementation detail (parameter names, exact error strings, internal
types): the testing agent discovers concrete syntax at run time from
the tool descriptions the model sees and from live probing. If a
behavior here changes, that is a product decision, not a refactor.

## What code intelligence is for

When a user asks a code-understanding question ("where is X defined",
"who calls Y", "what type does Z return", "rename A to B"), ata's
agent should answer from indexed knowledge of the codebase instead of
spraying shell greps and whole-file reads. Two complementary tool
classes serve this:

1. **A structural tool**: always available, syntax-level. Fast symbol
   search, caller and test tracing, project structure, exact
   implementation source, scoped regex search, line-window reads, and
   chunking support for files too big to read whole. It also carries a
   knowledge layer: humans or the agent can attach definitions and
   category marks to files and symbols, and persist them across
   sessions.
2. **A semantic tool**: delegates to per-language servers when they
   are available. Type-aware definitions, references, hover info,
   interface/trait implementations, call hierarchies, diagnostics,
   and refactor previews (rename, code actions) emitted as reviewable
   patches rather than applied edits.

The use cases these must serve:

- **Navigation**: find a symbol by approximate name anywhere in the
  project; jump from a use to its definition; list everything a file
  defines.
- **Comprehension**: read the exact source of one symbol without the
  rest of the file; see the project's shape and language mix at a
  glance; get type and doc info at a point.
- **Impact analysis**: who calls this function, which tests exercise
  it, what does it call in turn.
- **Safe refactoring**: a rename or quickfix is previewed as a patch
  the user (or the agent's normal edit path) can apply deliberately.
  Preview never touches the disk.
- **Multi-repo awareness**: several project roots can be registered at
  once; queries scope to one or span all of them coherently.

## Capabilities and required behavior

### Structural queries

- Symbol search by query string finds functions, types, methods, and
  similar across the project. Listing supports filtering by symbol
  kind and by file; an unsupported kind is a clear error naming the
  valid ones.
- Caller search returns call sites with file and line. Test search
  returns test symbols referencing a target, and when nothing is
  found it distinguishes "no tests reference this" from "no test
  symbols are indexed at all". Test search is a filtered view of the
  same reference facts caller search uses, not an independent grep: a
  reference caller search reports must not vanish from test search.
  Any test caller search sees — a `#[test]` unit test in source, an
  integration test under `tests/` — appears in test search when it
  references the target.
- Implementation retrieval returns the source of exactly the requested
  symbol. Local-variable listing is scoped to one function.
- The project structure view renders a tree of indexed files with a
  per-language breakdown and an indexed file count, at a controllable
  depth. The breakdown and count appear in both text and
  machine-readable output, so a single structure call is the complete
  answer for layout and language mix without any shell file listing.
- Scoped search supports a code-only mode that skips comments and
  strings; results carry file, line, and context.
- A line-window read (peek) works on any indexed file, including one
  created moments earlier in the session. Chunk indexing reports
  stable byte ranges so a huge file can be processed piecewise.
- Containment and index-eligibility are separate decisions. A file
  that sits inside a registered root but is skipped by the indexer
  (e.g. it exceeds the index size threshold) must still be peekable
  and chunkable — those read content directly. Reporting such an
  in-root file as "outside project root" is a false containment
  diagnosis and a bug; a genuinely absent file fails as not-found.

### Semantic queries

- Definition, references, hover, per-file symbol listing, and
  workspace-wide symbol search behave as their names promise, with
  results that cite real locations.
- Positions are addressed either by 1-based line/column or by symbol
  name. Name resolution is best-effort but never guessy: a name
  matching several symbols in the file is an error that lists the
  candidates; a name matching nothing says so. Zero or missing
  positions are rejected up front. When a name resolves to exactly one
  symbol, it lands on that symbol's identifier token — not the return
  type, the signature, or a same-named token elsewhere. By-name and
  by-position queries for the same symbol must agree.
- A slightly-off cursor self-corrects: empty results trigger retries
  at nearby positions, the correction is disclosed in the output, and
  the behavior can be switched off per call.
- Interface/trait implementation lookup and call hierarchy (both
  directions, via an explicit two-step prepare-then-query flow) work
  where the language server supports them. The second step without
  the first is a clear error.
- Diagnostics for a file return current errors/warnings with
  severity, position, and originating server, or an explicit
  "nothing wrong" rather than empty silence.

### Refactor previews

- Rename produces a patch in the same format the agent's normal
  editing tool consumes. Code-action preview does the same for
  quickfix-style actions, with a way to pick among several by title.
  A quickfix the language server actually offers (e.g. remove an
  unnecessary `mut`) must be previewable — including actions the
  server returns deferred (with `data`) that need a resolve round-trip
  before they carry an edit. "No previewable code actions" is honest
  only when the server genuinely offered none.
- Previews never modify any file. Verifiable: file contents are
  byte-identical before and after a preview call.
- Edits that would touch an unreasonable number of files or exceed a
  size budget are refused rather than emitted truncated.

### Multi-root

- Roots can be added, removed, and listed at runtime. The listing
  shows each root's path and which intelligence capabilities it has.
  A capability flag reflects what the root can actually do: it claims
  `lsp` only when a server is startable for a language present in that
  root. A root holding only TypeScript must not advertise `lsp` when
  the TypeScript server binary is absent, even if another root's
  server (e.g. rust-analyzer) is available in the shared registry.
  A server binary that resolves on PATH but fails to start (e.g. a
  rust-analyzer rustup proxy whose toolchain component is missing) is
  not "startable": once startup fails this session the root stops
  advertising `lsp` for that language rather than keeping the
  optimistic claim.
- Queries accept an optional root scope. Unscoped search-style queries
  fan out across all roots; results from overlapping roots are
  deduplicated by canonical file identity, so the same file reached
  through two overlapping roots yields one result, not one per root.
  An unknown root name is a clear error, as is a file that no
  registered root contains.
- Removing and re-adding a root yields a working index, not a stale
  cached one.

### Annotations

- A definition can be attached to a symbol or a file. First-time
  define and overwriting redefine are distinct operations; define
  refuses to clobber an existing entry.
- Files can be marked with built-in categories (test, docs, config,
  generated, entry point) or any custom tag; an empty mark is
  rejected.
- Annotations survive a save/load round-trip, and the save/load
  reports say how many of each kind were persisted or applied.

### Output discipline

- Result counts honor a per-call limit with a hard ceiling. When more
  exists than was returned, the output says so; counts never lie.
- Oversized output is truncated with an explicit notice. When the
  caller asked for machine-readable output, truncation still yields
  valid parseable output with the truncation flagged inside it, never
  a half-cut blob.

### Degradation and freshness

- A file whose language has no server configured, or whose server
  cannot start, produces an error that explains the situation and
  points at the structural fallback. It must not hang or fail
  silently. Server startup failures come with diagnosis, and a server
  that becomes startable mid-session (e.g. the agent installed a
  missing dependency) is retried rather than blacklisted forever.
- First contact with a file or workspace may wait for server
  readiness; repeat queries on the same file are fast.
- The index tracks reality: a file edited (by the agent or by an
  outside process), created, or deleted after indexing is reflected in
  subsequent query results. Answers grounded in a pre-edit snapshot,
  presented without caveat, are stale-index bugs.

### Routing contract

This is judged from the session JSONL, never from rendered text.

- A code-understanding question (definition, callers, references,
  type info, project structure) must route to the code-intelligence
  tool classes, not to shell grep/find/cat, when the tools are
  registered and the project is indexable. A correct-looking answer
  produced by shelling out is a routing failure worth reporting.
- A rename request must go through the rename preview path, not
  manual find-and-replace across files.
- Falling back from the semantic tool to the structural tool (or to
  plain text search) is correct exactly when the semantic tool is
  unavailable or has demonstrably failed for that file; the JSONL
  should show the attempt or the unavailability, not a reflexive
  shell-out.
- A successful native tool result is the answer. Re-running shell
  grep/sed/nl/cat to re-verify or re-read what a native call already
  returned is a routing failure, even when the final text is correct.
  The JSONL should not show a native success followed by a redundant
  shell search for the same thing.
- Simple file reads and ordinary edits are allowed to use the normal
  read/patch path; the contract covers understanding questions, not
  every file touch.

## How to test it

Build the binary per the README. These tools live inside the agent,
so there are two probing modes: directed ("use the code intelligence
tool to ...") to exercise a specific capability, and undirected
(natural questions that never name a tool) to test routing. After
every model-driven probe, read the session JSONL for the actual tool
calls and arguments; the pane text alone proves nothing.

Build a small fixture repo of your own with known ground truth: a
couple of languages (e.g. Rust plus Python or TypeScript), a function
with several distinct callers, a test that references it, a trait or
interface with one implementation, two same-named symbols in one
file, and one file large enough to overflow a single read. Knowing
the true answers is what lets you catch confident wrong ones.

Work through the capability sections above, then go adversarial.
Minimum probe classes, invent more:

- **Stale index**: edit a file from outside the session (shell, not
  the agent) between two queries about it; the second answer must
  reflect the edit. Also: create a brand-new file and query it
  immediately; delete an indexed file and ask about its symbols.
- **Edited mid-flight**: while a long structural query or server
  warmup is plausible, rewrite the target file and verify the result
  is either fresh or clearly attributable, never silently corrupt.
- **Nonexistent symbols**: ask about a symbol that does not exist,
  by direct tool probe and by natural question. Required: a clean
  not-found, no hallucinated location, no fabricated implementation.
- **Ambiguity**: query the duplicated symbol name by name; the error
  must list candidates, not pick one.
- **Mixed languages**: confirm each language in the fixture answers
  structurally; pick one language with no server installed and verify
  the semantic tool degrades with guidance while the structural tool
  still works; check the structure view's language breakdown matches
  reality.
- **Very large files**: oversized results carry the truncation
  notice; machine-readable mode stays parseable under truncation;
  chunk ranges tile the file completely and overlap as stated.
- **Preview integrity**: hash every file in the fixture, run rename
  and code-action previews, re-hash. Any difference is a bug. Then
  apply the emitted patch through the normal edit path and confirm it
  applies cleanly.
- **Bad positions and paths**: zero/negative/past-end-of-file
  positions, relative vs absolute path handling as documented, a file
  outside every root, an unknown root name.
- **Roots churn**: add a second root, query across both, remove and
  re-add one, confirm the index is live and capability flags are
  truthful.
- **Silent failure hunting**: wherever a call succeeds, verify the
  implied effect or claim (annotation actually persisted to disk,
  reported counts match a manual recount, "no diagnostics" on a file
  you deliberately broke is a finding).

For routing probes, ask natural questions: where something is
defined, who calls it, what would break if it changed, rename a
symbol, summarize the structure of the project. Vary the wording
between runs; verbatim reuse turns this back into a script. Judge
each from the JSONL: which tool class was called, with what
operation, and whether shell search was used where the tools should
have been.

Report per the README: issues with exact reproduction steps,
divergences citing the section above, routing failures quoting the
session log, and coverage notes (including which languages had live
servers during the run, since semantic coverage depends on the host
machine).
