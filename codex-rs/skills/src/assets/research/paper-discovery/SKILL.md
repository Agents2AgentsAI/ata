---
name: paper-discovery
description: "Discover and rank papers for a research topic. Use when a user asks a research question, wants to find papers, or asks how something is done in the literature."
metadata:
  short-description: Discover and rank papers for a topic
---

# Paper Discovery

## Core Behavior

1. **Auto-continue.** After presenting the reading view, IMMEDIATELY proceed to `$paper-synthesis` then `$cross-paper-report`. Exception: user says "just list", "don't synthesize", or "discovery only".
2. **Present a landscape analysis, not a paper list.** Cluster papers into approaches with tradeoffs. Never output flat lists or sections named "Summary."
3. **Speed matters.** Go straight to searching. Every tool call before `paper_search` is delay the user feels.

## Hard Rules

- Do NOT open arXiv URLs — `paper_search`/`paper_citations` already return all metadata
- Do NOT call `paper_get` for papers already returned by search/citations — only for bare S2 IDs
- No academic citations — reference papers by title or method name

## Modes

1. **Citation-focused** — user references a specific paper → `paper_citations` + 1-2 keyword searches
2. **Explore** — research question → faceted keyword search + citation expansion
3. **Discovery** — user has a research base → KB/Zotero seeds + citation/recommendation APIs

---

## EXPLORE MODE

### Phase 0: Check Existing Knowledge (1 call max)

**Skip if KB disabled.** `rg "tag1\|tag2\|tag3" ${CODEX_KB_PATH}/cards/` with 2-3 tags.

### Phase 1: Decompose into Facets

Parse the question into 3-5 search facets targeting different angles.

**Facet design is critical** — vague queries return junk. Encode the relationship, disambiguate terms, vary abstraction level, use field vocabulary.

**BAD**: `"large language model interpretability explanations for AI models"`
**GOOD**: `"LLM-generated explanations for neural network predictions"`, `"self-rationalization large language models faithfulness"`

### Phase 2: Search

**Citation-focused:** 3 calls — `paper_citations(id, limit=30)` + two `paper_search` variants.

**General explore:** Keyword search (2-3 facets, `paper_search(query, limit=10, sort_by=citation_count)` + recent variant per facet), then citation expansion on top 3-5 seeds. **Budget: ≤ 20 API calls.**

### Phase 3: Analyze and Organize

Deduplicate (DOI → arXiv → title fuzzy). Mark known KB papers. Cluster into 3-6 approaches. Rank into reading order: Start Here, Core Methods, Cutting Edge.

### Phase 4: Present

Use `present_reading_view` with topic-appropriate headings. Each section ≤ 40 lines. Typical sections: Landscape, Approaches, Key Insights, Reading Plan, Open Questions — adapt freely. Include key figures from papers via `crop_and_store_figure` when they aid understanding.

Follow-ups: use `append_to_section` with `foldable=true`.

---

## DISCOVERY MODE

Uses KB/Zotero as seeds to find adjacent papers.

1. **Gather seeds** — list KB cards (skip if disabled), extract paper IDs. No KB: use `$zotero` or ask for 3-5 seed IDs.
2. **Search** — from top 5 seeds: `paper_citations` + `paper_references` (limit=15 each), topic searches, optional `paper_recommendations`. **Budget: ≤ 25 calls.**
3. **Merge and present** — dedup, filter out known papers, rank by citations + recency, present in same reading view format.

---

## Post-Discovery

**Skip if KB disabled.** Prepend to `<kb_path>/research-journal.md`: date, mode, paper count, top 2-3 finds, open questions. If request reveals priorities, offer to note in `research-context.md`.

## Pipeline

After discovery: `$paper-synthesis` → `$cross-paper-report` (default). ≤ 12 papers = standard, > 12 = large-set mode.
