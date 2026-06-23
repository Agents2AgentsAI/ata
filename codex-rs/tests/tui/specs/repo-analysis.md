# Repo analysis — behavioral spec

This spec covers ata's repository analysis surface: the `repo_*` agent
tools that analyze a remote Git repository from a URL, and the
repository-understanding layer (`repo_context` and its companions)
that gives the agent semantic context about the repo it is working
in. Like the workspace spec, this document stays at capability level:
no exact error strings, no schema field lists, no on-disk layout. The
testing agent discovers concrete tool names and arguments at run time
from the session JSONL and the features menu; code-level churn must
not invalidate this document.

## What repo analysis is for

Two distinct jobs, two tool families:

1. **Understanding a repo you don't have** — the user pastes a GitHub
   or GitLab URL and asks what's in it, whether it's maintained,
   what models it defines, how to run it, what it depends on, whether
   its dependencies clash with the local environment, what config
   knobs it exposes, what export/deployment paths exist. The remote
   analysis family answers these without the user ever cloning
   anything by hand.
2. **Understanding the repo you're in** — for coding tasks in the
   current working repo, the understanding layer retrieves
   definitions, references, dependency neighborhoods, and task-aware
   evidence for a file or symbol, so the agent navigates by structure
   instead of blind grep.

The two families must not be confused with each other or with the
workspace feature: workspace repos are local checkouts under aliases
managed by `ata workspace`; the remote analysis tools take URLs and
maintain their own bounded clone cache; the understanding layer
operates on the session's working directory.

## The remote analysis family

Nine tools exist, all taking an https repo URL (note: there is no
`repo_search` tool — discovering repos is the job of web/paper search
surfaces, not this family):

- **Summarize** (`repo_clone_and_summarize`): the entry point. Given
  a URL (optionally a branch/tag), returns a high-level map —
  directory tree, README excerpt, key files. Accepts GitHub and
  GitLab; tolerates `/tree/<branch>` suffixes.
- **Structure probes**: find ML model definitions
  (`repo_find_models`, optionally narrowed to a framework), find
  runnable entrypoints (`repo_find_entrypoints`, optionally narrowed
  to train/eval/infer/export, with a CLI-args summary when
  detectable), extract input/output tensor shapes
  (`repo_extract_io_shapes`, optionally narrowed to a model class),
  find model-export pipelines (`repo_find_export_paths` — ONNX,
  TensorRT, TFLite, CoreML, OpenVINO, TorchScript), and extract
  config-key schemas from config files and CLI parsers
  (`repo_extract_config_schema`).
- **Dependency tools**: extract Python requirements from the standard
  manifest formats (`repo_extract_requirements`), and diff a repo's
  requirements against a local requirements file
  (`repo_diff_requirements`), separating hard conflicts from
  merely-missing packages. Extraction is manifest-scoped: only the
  declared dependency lists count (`install_requires`/`extras_require`
  in `setup.py`, `[project].dependencies` in `pyproject.toml`, the
  lines of `requirements.txt`). Package metadata — a project name, a
  version string, a README path — is never scraped in as a dependency.
  A `setup.py` that declares no dependencies yields an empty set, not
  false entries that then surface as bogus `missing_locally` in the
  diff.
- **Health** (`repo_get_health`): maintenance signals (license,
  recency, stars, issues, releases, CI) fetched from the GitHub API
  with no clone at all. GitHub only; for GitLab the summarize tool is
  the documented fallback. An authenticated GitHub token raises the
  rate allowance; without one the tool works at anonymous limits.

### Required behavior

- **Clone discipline**: clones are shallow, size-capped, and land in
  a process-local cache with bounded total size and eviction. A
  second analysis of the same repo must reuse the cached clone, not
  re-fetch. Results are cached for a bounded window, so repeated
  identical calls are fast and cheap.
- **Bounded output**: every probe caps its result count and truncates
  long content (trees are depth-capped, READMEs trimmed). A huge repo
  degrades by truncation, never by hanging, flooding the context
  window, or filling the disk past the cache bound.
- **Honest empties**: a repo with no models / no entrypoints / no
  requirements / no export paths returns an empty result presented as
  such — not an error, and not hallucinated entries. The agent must
  then tell the user "none found", not invent.
- **Static analysis honesty**: these are heuristic, grep-class probes
  over a shallow clone. They must surface what the source declares
  (file paths, line context) so claims are verifiable, and must not
  execute repo code under any circumstance.
- **Bad input is a clear refusal**: non-URL strings, unsupported
  hosts, local filesystem paths, private/nonexistent repos, and a
  GitLab URL given to the GitHub-only health tool each produce an
  explanatory failure the model can relay — not a hang, panic, or
  silent empty success.
- **Gating**: the family is on by default but feature-gated; when the
  user disables repo analysis in the research-tools surface, the
  tools are absent from the session entirely (not present-but-erroring).

### Routing contract (judged from the session JSONL)

- Given a repo **URL** and an analysis question, the model should
  reach for the matching `repo_*` tool — not shell out to `git clone`
  by hand, and not answer from memory of the repo's name.
- Given a question about a repo that is **already on disk** (the cwd,
  or a checkout registered under a workspace alias), the model should
  use local means — shell, grep, file reads, or the understanding
  layer — and must not re-clone the repo through the URL tools.
- "Is this repo maintained?" with a GitHub URL should hit the
  no-clone health tool, not trigger a full clone.
- An environment-compatibility question ("will this run in my env?")
  should route to the requirements diff against a real local file,
  not to a guess.
- The right tool with a wrong synthesis (e.g. the tool returned
  nothing and the answer asserts specifics) is a failure even though
  routing was correct.

## The repository-understanding layer

Behind an experimental feature toggle ("Repository Understanding",
off by default), ata builds a semantic index of the current working
repo and exposes it to the agent. `repo_context` is the semantic
surface **where the backing provider is built in**; it is served by a
private provider, so a public build (which compiles a no-op provider)
intentionally does not register it. On such a build, enabling
Repository Understanding does not summon `repo_context`; repository
understanding instead routes through the repo-analysis tools that the
build does offer — structure probes, manifest parsing,
clone-and-summarize for remote targets, and the indexed `code_intel`
calls for the working repo. That fallback is the correct behavior, not
a defect (cross-reference "provider-gated tools are conditional" in
`feature-flags.md`). The contract below describes the `repo_context`
surface for builds that serve it:

- A **`repo_context` tool** that retrieves context for a target: the
  whole repo (broad tasks like onboarding or architecture review), a
  directory, a file, or a symbol within a file. Output includes the
  target's code, a dependency map (imports, importers, callers,
  callees), and prioritized evidence. A documentation-trace mode
  finds docs referencing a code symbol. A task goal and constraints
  can steer retrieval; follow-up calls with the same goal reuse
  session state and return deltas instead of repeating exploration.
- An **ambient bootstrap summary** of the repo injected into the
  session context once indexing completes, plus guidance steering the
  model toward the tool for navigation-shaped work.
- **Auto-invocation**: for user turns that look navigation-shaped,
  relevant context can be injected without an explicit tool call.

### Required behavior

- With the feature off (the default), none of this exists: no tool in
  the session, no injected summary, no behavior change. Toggling it
  on is what introduces the surface.
- Registration tracks real capability: `repo_context` is exposed only
  when the feature is on **and** this build's provider can actually
  serve repository understanding. On a build that cannot serve it the
  tool is not registered at all — enabling the feature must never put a
  callable `repo_context` in the session whose first call only reports
  "not available in this build". An advertised-but-unwired tool is a
  gating failure, not a working surface.
- Indexing must not block the session. Before the index is ready, a
  query degrades gracefully (a clear not-ready answer) rather than
  hanging the turn; the model is told to fall back to ordinary
  exploration, not to fabricate.
- Targets resolve forgivingly: relative paths from the session
  directory, absolute paths, directories as first-class targets. A
  target that doesn't exist is a clear error naming the problem. An
  unsupported mode is rejected with the valid options.
- Claims are grounded: dependency edges, callers, and evidence cite
  real files and symbols that exist in the checkout. A fabricated
  edge is a critical failure.
- **Routing contract**: with the feature on **and the provider built
  in**, navigation-shaped requests ("where is X defined", "what calls
  Y", "give me an overview of this codebase") should go through
  `repo_context`; pinpoint lookups the model can do in one grep may
  stay in shell. The model must not narrate using repo context without
  having called the tool, and must not re-read with shell a file the
  tool already returned in full. On a build that does not serve
  `repo_context` (the public/no-op-provider case), there is no
  `repo_context` to route through: the same navigation-shaped requests
  are answered correctly through the indexed `code_intel` calls and
  shell, and the absence of a `repo_context` call in the session JSONL
  is expected, not a routing-contract violation.

### Companion tools (status: in flux)

The understanding layer has designed companions: a feedback channel
(report which retrieved items helped or were missing), a
change-impact query (blast radius of edits against a base revision),
and a live overlay (push unsaved buffer state so subsequent context
queries see in-progress edits, with a status probe). As of this
writing these are not registered in shipping sessions. The testing
agent must first discover whether they exist in the probed build (the
session JSONL shows the registered tool set); if absent, record that
and move on — their absence is expected, not an issue. If present,
hold them to the same standards: bounded output, honest empties,
grounded claims, clear refusal of bad targets, and overlay state that
actually changes subsequent context results.

## How to test it

Build the binary, then drive everything through the TUI with real
model turns; the remote family and the understanding layer have no
CLI of their own. The deterministic anchor is the session JSONL:
which tools were registered, which were called, with what arguments,
and what they returned. Rendered prose can look right while the wrong
tool ran — judge the JSONL.

Useful fixtures: a tiny well-known public repo for summarize/health
(e.g. `octocat/Hello-World`); a small real ML repo with a model
class, a train script, and a requirements file for the structure and
dependency probes; a local scratch requirements file with one
deliberate version conflict for the diff tool; the ata repo itself
(large, mixed-language) for bounds; and a freshly created empty
GitHub repo for the honest-empties class.

Work through each tool with a natural prompt, verify the answer
against the actual repo content (clone it yourself out-of-band to
check claims), then go adversarial — minimum classes, invent more:

- **Empty/degenerate repos**: empty repo, README-only repo, repo with
  no Python at all given to the ML-shaped probes. Expect honest
  "none found" end to end — JSONL result empty AND the model's answer
  saying so.
- **Huge repos**: a very large repo through summarize and the probes.
  Expect truncated-but-useful output, a bounded cache footprint on
  disk, and no hung turn.
- **No recognizable structure**: a real but non-ML repo asked "what
  models does this define?" — the failure mode to catch is the model
  padding an empty tool result with invented classes.
- **Malformed manifests**: a repo whose requirements/config files are
  syntactically broken. Parsing should skip or surface the bad file,
  not crash or silently return half an answer presented as whole.
- **URL abuse**: not-a-URL, http instead of https, ssh remotes, a
  local path, a private repo, a nonexistent repo, a GitLab URL to the
  health tool, URL with branch suffix to every tool that takes one.
  Each should end in a clear model-relayed refusal; none should hang
  or leak a partial clone outside the cache.
- **Already-cloned vs URL**: register a repo under a workspace alias,
  then ask questions about "the repo I added" — the model should work
  the local checkout, not re-clone via URL tools. Then ask about the
  same repo by URL in a fresh session and expect the opposite
  routing.
- **Cache behavior**: same analysis twice — second call visibly
  cheaper (JSONL timing / no re-clone on disk); cache directory stays
  within bounds after analyzing several repos.
- **Feature gating**: disable repo analysis, confirm the tools vanish
  from the registered set and a URL question falls back to honest
  shell-or-refusal behavior. Enable Repository Understanding: on a
  build that serves it, confirm `repo_context` appears and the
  bootstrap summary lands, then disable and confirm both are gone. On a
  build that does not serve it (public/no-op provider), confirm
  `repo_context` does **not** appear — that absence is correct gating,
  and navigation-shaped work routing through `code_intel`/shell instead
  is the expected behavior, not a defect.

For the understanding layer, first confirm whether the probed build
serves `repo_context` at all (enable the feature, check the session
JSONL toolset). If it does not, record that and judge navigation-shaped
work against the `code_intel`/shell fallback instead — the
`repo_context`-specific checks below do not apply. On a build that
serves it, run inside a real mid-sized repo with the feature on: ask
for a codebase overview, a symbol's callers, docs about a symbol, and a
goal-scoped exploration ("I want to fix X — what should I read?").
Verify cited files and edges exist. Probe not-ready (query immediately
after enabling on a cold repo), bad targets, bad modes, and the
narration rule (claimed-but-never-called is a contract violation
visible only in the JSONL).

Vary the wording of every live-model task between runs — "what's in
this repo", "give me the lay of the land of <url>", "can I run this
on my machine", "is this project dead?", "where do I start reading
training code" — verbatim reuse turns this back into a script.

Report per the README: issues with exact reproductions, divergences
citing the section above, routing-contract violations quoting the
session log, and coverage notes including which companion tools were
present in the probed build.
