# Research tools — behavioral spec

This spec describes ata's research tool surface: academic papers,
Hacker News, patents, datasets, and Zotero. It deliberately avoids
implementation detail (parameter names, exact error strings, internal
modules): the testing agent discovers concrete tool names and argument
shapes at run time from the session JSONL and from `--help` on the CLI
surfaces. If a behavior here changes, that is a product decision, not a
refactor.

## What the research surface is for

ata answers research questions with dedicated, structured tools instead
of generic web search or shell scraping. The use cases it must serve:

1. **Literature work**: find papers on a topic across multiple academic
   indexes, fetch one paper by a public identifier, walk its citation
   graph in both directions, and get recommendations seeded by example
   papers.
2. **Practitioner signal**: search Hacker News stories and comments,
   and read a full discussion thread, to capture deployment experience
   and community sentiment that papers miss.
3. **Prior art**: search worldwide patents by keyword, inventor,
   assignee, classification, and date, and fetch one patent's full
   record.
4. **Datasets**: search Hugging Face and Kaggle, inspect a dataset's
   files and metadata before committing, and download it (or specific
   files) to a chosen local directory. Kaggle competitions get the same
   list/inspect/download treatment.
5. **Reference management**: search, read, organize, and write to the
   user's Zotero library, against either the local desktop app or the
   cloud account.

The overarching contract is **routing**: when a question is shaped for
one of these tools, the agent must reach the answer through that tool.
A correct-looking answer produced by generic web search or a shell
fetch of the upstream website is a failure even when the prose is
right. The session JSONL is the evidence, not the rendered text.

## Capabilities and required behavior

### Papers

- Topic search spans three academic indexes (Semantic Scholar, arXiv,
  OpenAlex). A search can be scoped to one source, filtered by year
  and month range and field of study, sorted (including recent-first),
  and paginated. Callers control verbosity: which fields come back,
  whether abstracts are included, and a per-item character cap so
  results stay digestible.
- If one index is slow or down, the search returns partial results
  from the others within a bounded time rather than hanging or failing
  the whole call.
- A single paper can be fetched by DOI, arXiv id, or Semantic Scholar
  id. Source selection is identifier-aware: when the id is an arXiv id
  and the primary upstream (Semantic Scholar) is rate-limited or down,
  the single-paper fetch resolves natively from arXiv rather than
  failing the whole call. The native single-paper contract must not
  hinge on one upstream being healthy.
  Citations (who cites it) and references (what it cites) are
  separate, paginated lookups with the same verbosity controls.
  Recommendations take a list of example paper ids as positive seeds,
  optionally negative seeds to steer away from, and return similar
  work.
- A nonexistent or garbage paper id is a clear error, and the agent
  must relay it honestly. The agent must never fabricate a paper,
  invent a DOI, or quietly substitute a different paper without
  saying so. On a no-results topic the agent says so plainly; if it
  offers an adjacent real paper instead, it flags the substitution.
- **Routing**: a paper-finding prompt routes to the paper search tool,
  typically several calls with paraphrased queries fanned across
  sources, not one literal query. No generic web search, no shell
  fetch of arxiv.org. **Known open issue**: a natural-language "look
  up arxiv NNNN.NNNNN" currently routes to a shell fetch of the arXiv
  page instead of the dedicated single-paper tool, which only fires
  when named explicitly. Treat as confirmed, not a new finding; what
  would be new is topic *search* regressing the same way.

### Hacker News

- Story and comment search with filters: content type (story, comment,
  Show HN, Ask HN), minimum points, minimum comment count, date range,
  author, parent story; sortable by relevance or date; paginated with
  a bounded per-page size. No credentials needed, ever.
- A full discussion thread can be fetched by item id with caps on
  comment depth and total comments, so a megathread cannot blow up the
  context.
- **Routing**: a Hacker News question MUST reach Hacker News through
  the dedicated search tool. Not generic web search, not a shell fetch
  of news.ycombinator.com. Cited URLs in the answer must be real HN
  URLs.
- For synthesis-shaped requests ("what do people think about X"), the
  work may be delegated to sub-agents; that is fine. The judgment is
  evidence-based either way: the dedicated HN tools were invoked
  (in the main session or a sub-agent's), and the answer reflects real
  threads. A two-source prompt (papers *and* HN) must show both
  sources actually contributed — a dedicated section or concrete
  results per source — not one source plus hand-waving about the
  other.

### Patents

- Worldwide patent search through the European Patent Office's open
  service. Filters: free text over title/abstract (with a match-all
  wildcard for pure inventor/assignee/date filtering), inventor,
  assignee, classification-code prefix, publication date range.
  Sortable, page size bounded, with cursor-style continuation for the
  next page. Single-patent fetch by publication number returns full
  metadata including claims.
- These tools require a registered EPO credential pair. Without it,
  the call fails with a message that says the tools are not
  configured, names what is needed, and points at where to register.
  The agent must relay that state honestly — offer to proceed without
  patents or explain the setup — and must not silently fall back to
  scraping a patent website and presenting that as patent search.
- **Routing**: a patent question routes to the patent tools, not web
  search, whenever they are configured.

### Datasets

- Dataset search spans Hugging Face and Kaggle, with filters (source,
  author, tags, modality), sorting, and a bounded result count.
  Detail lookup, file listing (with sizes, before any download), and
  download to a caller-chosen directory all key off a dataset
  identifier whose convention distinguishes the two sources. Download
  can take the whole dataset or a named subset of files.
- Kaggle competitions: list/search competitions, list a competition's
  files with sizes, download some or all of them to a chosen
  directory.
- Hugging Face works without credentials for public datasets. Kaggle
  requires credentials for everything: an unscoped search silently
  covers only the sources that are configured, a search explicitly
  scoped to Kaggle without credentials says so in the result rather
  than returning a bare empty list, and any direct Kaggle operation
  without credentials is a clear auth-required error. An explicit
  Kaggle scope — via the `kaggle:` id prefix or a `source` argument set
  to `kaggle` — pins the lookup to Kaggle: a missing-credentials state
  is reported as an auth error, never silently re-routed to Hugging
  Face and surfaced as "dataset not found". The agent must surface the
  credential gap, not paper over it.
- A download that reports success must have put the files on disk at
  the stated location. Check the disk.
- Source-specific info tools exist for Hugging Face and Kaggle dataset
  details. **Known open issue**: their advertised inputs and what they
  actually accept disagree — a call made exactly per the advertised
  shape is rejected, while the generic detail tool's shape works.
  Verify and treat as confirmed if reproduced.
- **Routing**: a dataset-finding prompt goes through dataset search,
  not a shell fetch of the hubs' websites or ad-hoc `pip`/API
  scripting, when the tools are enabled.

### Zotero

- There are two doors: a family of native agent tools covering search
  (keyword, advanced/structured, full-text grep, notes), reads (item,
  citation string, full text, notes, annotations, attachments,
  collections, collection items, groups, tags, recent), and writes
  (create/update items, create or find-or-create collections, add
  items to a collection, link attachments) — and the `ata zotero` CLI
  namespace exposing the same library operations as subcommands, with
  a status command and a command-search helper.
- The agent skill contract prefers the CLI door: Zotero-shaped
  requests ("my papers", "my library", a collection name) activate the
  Zotero skill, which drives `ata zotero` commands, starts from status
  when the mode is unclear, and lists collections before guessing at
  queries when the request sounds like a curated bucket. Reaching the
  right answer by hand-rolled HTTP against the Zotero API is a
  contract violation.
- Mode selection is credential-driven: with no API key configured, ata
  talks to the local Zotero desktop app's API; with a key, it operates
  in remote (cloud) mode with local fallback. The status command
  reports the effective mode, endpoint, auth state, and scope without
  needing the network, and explains *why* the mode is what it is.
- With no key and no running desktop app, operations fail with a clear
  connection error naming the local endpoint, non-zero exit; nothing
  pretends to have a library. Configured keys never appear in
  diagnostics output.
- Structured input (the advanced-search payload) is validated:
  malformed or incomplete payloads produce an error naming what is
  missing, and nothing executes.
- Write operations mutate the user's real library. Test writes only
  against a scratch library or scratch collection, prefix everything
  created, and remove it afterward.

### Cross-cutting

- Each tool family is independently toggleable as a feature. A
  disabled family's tools are absent from the agent's toolset, and a
  user prompt aimed at a disabled family must not crash the session —
  the agent works with what it has and should be honest about the gap.
- Upstream APIs are rate-limited politely and transient failures are
  retried within bounds. From the user's seat: repeated queries get
  faster (cached) rather than hammering the API, a rate-limited or
  down upstream surfaces as a bounded, explained failure (or partial
  results, for multi-source search), and nothing hangs past the tool
  timeout. An internal crash in a tool comes back as an error message,
  never a dead session.
- Credentials resolve from environment, the secrets store, and config,
  with environment winning; a blank value counts as unset. No
  credential value is ever echoed in tool output or logs.

## How to test it

Ground every probe in the session JSONL (`~/.ata/sessions/...`, recipe
in the README): tool names invoked, their arguments, and what was *not*
invoked. The rendered answer alone proves nothing about routing.

Boot the TUI and give the in-app agent real tasks in your own words,
varying the wording between runs — verbatim reuse turns this back into
a script:

- a topic paper search ("find me recent papers on X"); judge that
  paper search ran (usually several paraphrased calls across sources)
  and that no web search or shell fetch substituted for it;
- an HN question ("what's a good HN thread about X", "what do people
  think of Y"); judge HN tool usage, direct or via sub-agents, and
  real HN URLs in the answer;
- a two-source synthesis ("papers on X and what HN says about it");
  judge that both sources contributed concretely;
- a citation-graph walk, a recommendation request seeded by a named
  paper, a patent question, a dataset hunt with an inspect-then-
  download flow, and Zotero reads (and writes only against scratch).

Then go adversarial — minimum classes, invent more:

- **Shell-fallback temptation**: prompts engineered to invite a
  scrape, e.g. "curl the HN front page and tell me the top story",
  "just fetch the arxiv page for ...", "use the Kaggle API directly".
  The dedicated tool should still win (the known single-paper-lookup
  weakness above excepted); a shell fetch of an upstream the tools
  cover is a routing failure.
- **Ambiguous queries**: a query that is plausibly a paper topic, an
  HN topic, and a dataset name at once; one-word queries; queries in
  another language. Judge whether routing stays sane and whether the
  agent asks or states its interpretation rather than guessing
  silently.
- **Nonexistent and malformed ids**: fake DOIs, future arXiv ids,
  garbage patent numbers, dataset ids with the wrong source
  convention, Zotero item keys that do not exist. Every one is a clear
  error relayed honestly; fabrication of a plausible-looking result is
  the bug this class exists to catch.
- **Credential-absent paths**: with no EPO, Kaggle, or Zotero
  credentials, hit each gated surface and judge the failure: does it
  name the gap, does the agent tell the user, does anything silently
  degrade into scraping. Then with a dummy Zotero key, confirm the
  status reporting flips mode without needing a real account.
- **Limits and starvation**: pagination edge cases (zero, one, the
  documented maximum, beyond it, large offsets); tight per-item
  character caps actually truncate; a burst of identical queries
  answers from cache rather than re-hitting the API (response timing
  is a usable signal); and if you can interpose on the network (the
  upstream endpoints are configurable), a dead or slow upstream
  produces a bounded, explained failure or partial multi-source
  results, never a hang or a silent empty success.
- **Toggle matrix**: disable one family, confirm its tools are gone
  from the toolset and a prompt aimed at it degrades gracefully;
  re-enable and confirm recovery. Restore the user's config exactly,
  pass or fail.

Clean up everything: downloaded datasets, scratch Zotero entries,
config edits, tmux sessions.

Report per the README: issues with exact reproduction steps,
divergences citing the section above, routing violations quoting the
session log, and coverage notes (patents and Kaggle in particular may
be credential-blocked; say so rather than skipping silently).
