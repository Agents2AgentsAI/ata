---
name: paper-discovery
description: Use when a user asks a research question, wants to learn about a research topic, asks how something is done in the literature, or wants to discover papers. Examples -- "how do people train RL for robotic grasping", "what are the best methods for sim-to-real transfer", "I want to learn about diffusion policies", "find me papers on VLAs". Provides structured landscape briefings with approaches, key insights, and automatic synthesis. Also handles KB/Zotero-based proactive discovery.
metadata:
  short-description: Discover and rank papers for a topic
---

# Paper Discovery

**Output format**: Always present final reports using `present_reading_view` (not as regular text). See the Presentation section below.

## CRITICAL: No Exploration

**Do NOT do any of these before starting the search:**
- Do NOT read any SKILL.md files (you already have the instructions)
- Do NOT run `rg --version`, `ls`, or any diagnostic commands
- Do NOT read `research-context.md` — use conversation context instead
- Do NOT read individual KB cards one by one — a single `rg` search is enough
- Do NOT open arXiv URLs for ANY reason — not to verify existence, not to read abstracts, not to confirm metadata. `paper_search` and `paper_citations` already return titles, authors, years, and abstracts.
- Do NOT use `web.run`, `web_search`, or any web browsing to look up papers — the paper API tools are your only source.
- Do NOT call `paper_get` for papers returned by `paper_search` or `paper_citations` — those APIs already return titles, authors, years, abstracts, and IDs. Calling `paper_get` per-result wastes API quota (14+ unnecessary calls per session) and adds 20+ seconds of latency. `paper_get` is ONLY useful if you have a bare S2 ID with no metadata.
- Do NOT include `citeturn*view*` or similar citation markers in reading view content — these are internal artifacts that appear as garbage to the user. Write citations as Author (Year) only.

**Speed matters.** The user should see results within 30 seconds of their request. Every tool call you make before `paper_search` or `paper_citations` is delay the user feels. The paper API returns all metadata you need — never supplement with web browsing.

## Modes

Three modes:

1. **Citation-focused** — user references a specific paper and asks for related/recent/citing work. Use `paper_citations` + 1-2 keyword searches. **This is the most common mode when the user just read a paper.**
2. **Explore** — user has a research question or topic to learn about. Use faceted keyword search + citation expansion.
3. **Discovery** — user already has a research base, wants adjacent papers. Use KB seeds + citation/recommendation APIs.

**Mode detection:**
- User just read/synthesized a paper and asks "find more like this", "recent work in this area", "what cites this", "what's new in this field" → **citation-focused** (use the paper they just read as seed)
- User asks a question or describes a topic to learn about → **explore**
- User invokes `$paper-discovery` with no arguments → **discovery**


## Auto-Continue Detection (CRITICAL)

**Before doing ANY search work**, classify the user's request into one of two categories:

### The Default: Always Auto-Continue

**Auto-continue is the default behavior.** After presenting the discovery reading view, you MUST automatically continue: invoke `$paper-synthesis` for all top papers (multi-paper path, parallel subagents), wait for results, then invoke `$cross-paper-report` to produce the integrated explained report. Do NOT stop at the discovery map. Do NOT present `$paper-synthesis` commands for the user to click. The user asked a research question — they want answers, not a bibliography.

**The Key Insights section** (formerly "Reading Plan") should say "Proceeding to synthesize [N] papers..." After the discovery reading view is shown, immediately proceed to synthesis without waiting for user input.

### Exception: Discovery Only

Only stop at the discovery map (without auto-continuing to synthesis) when the user **explicitly** asks for just a list:
- "just find papers", "just list papers", "don't synthesize", "discovery only"
- "what cites X" (citation lookup — the user already has the paper)
- "show me recent work on X" (browsing, not studying)

**When in doubt, auto-continue.** A user asking "find papers about how X can be used for Y" is asking a research question — they want to understand X, not just get titles. The word "find" does not mean "stop at a list."

## Pipeline Paths

After discovery, three paths exist:

1. **Quick orientation** — `$research-briefing` gives a 2-4 page overview with core ideas per paper
2. **Full deep analysis** — `$paper-synthesis` (multi-paper) → `$cross-paper-report` for the integrated narrative
3. **Interactive exploration** — Chat about specific papers, ask follow-ups

For auto-continue requests (see above), enforce path 2 automatically. Do not mark the request complete until cross-paper-report is done.

### Scale-Aware Pipeline Branching

- **≤ 12 papers**: Standard pipeline. `cross-paper-report` runs Phases 1–3 in a single pass.
- **> 12 papers**: `cross-paper-report` automatically activates **large-set mode** (clustered sub-reports with meta-synthesis).

---

## EXPLORE MODE

Explore mode answers research questions by mapping the landscape of approaches and surfacing the most important papers to read. No existing KB or Zotero library is required -- but if available, they enrich the results.

### Explore Phase 0: Check Existing Knowledge (1 tool call max)

**Skip this phase entirely if KB is disabled.** Proceed directly to Phase 1.

**This is optional and must be fast — 1 tool call maximum.**

Run `rg "tag1\|tag2\|tag3" ~/.ata/knowledge-base/cards/` with 2-3 relevant tags. This tells you which papers the user already has.

Do NOT:
- Do NOT list the entire KB directory
- Do NOT read individual KB cards
- Do NOT read `research-context.md` — use conversation context for user priorities
- Do NOT read `research-journal.md`
- Do NOT call `$kb` — a single `rg` command is faster

If KB or Zotero aren't configured, skip this entirely and proceed with search.

### Explore Phase 1: Decompose the Question

Parse the user's question into 3-5 search facets. Each facet targets a different angle of the problem using different terminology and framing.

#### Facet Design Principles

1. **Encode the relationship, not just keywords.** If the user asks "how is X used to do Y," your facets must capture the directionality. A query like `"X Y"` is ambiguous — it matches papers *about* X and papers *using* X for Y equally. Instead, phrase facets that make the relationship explicit (e.g., `"X as supervision for Y"`, `"X-guided Y training"`, `"using X to improve Y"`).

2. **Disambiguate overloaded terms.** Many research terms are ambiguous. Before generating facets, identify terms that could mean multiple things and pick the reading that matches the user's intent. For example, if the user asks about "using human attention to train models," `"attention supervision"` is ambiguous (it matches both human eye-tracking work AND transformer attention mechanism papers). Prefer specific phrases like `"eye tracking weak supervision"` or `"human gaze training signal"`.

3. **Vary the abstraction level.** Include at least one narrow/specific facet (exact method name or technique), one broader facet (the general research area), and one using alternative terminology (how a different community might describe the same idea).

4. **Use field-specific vocabulary.** Think about what terms researchers in this area actually use in paper titles and abstracts. Avoid overly casual or generic phrasing.

5. **Test each facet mentally.** Before using a facet, ask: "If I search this exact string, could most of the top results be about something unrelated to what the user wants?" If yes, add qualifying words to narrow it.

### Explore Phase 2: Search (Fast, Then Deep)

**Speed matters.** The user should see initial results quickly. Do NOT launch large-scale citation expansion before showing anything.

#### Citation-Focused Path (MOST COMMON — user just read a paper)

Use this path when the user references a specific paper (from a previous synthesis, URL, or conversation context) and asks for related/recent/citing work. This includes: "find more like this", "recent work in this area", "what cites this", "what's new in this field", "find the most recent work".

**This is 3 tool calls before presenting results:**
1. **`paper_citations(known_paper_id, limit=30)`** — what builds on this paper? This is the primary data source.
2. **`paper_search(topic_keywords, year_from=current_year-1, limit=10, sort_by=citation_count)`** — recent papers in the same area.
3. **`paper_search(alternative_keywords, limit=10, sort_by=citation_count)`** — broader coverage.

Then immediately present results. Do NOT do facet decomposition, KB exploration, or research-context reading.

**KB check (optional, 1 call max; skip if KB is disabled):** If KB is available, run `rg "relevant_tag" ~/.ata/knowledge-base/cards/` to mark which found papers are already in KB. Do NOT read individual cards.

#### General Explore Path (topic/question-based)

**Step 1 — Keyword Search (do this first, directly, no subagents):**
Run 2-3 facets from Phase 1 through `paper_search` in parallel:

- Per facet: `paper_search(query, limit=10, sort_by=citation_count)`
- Per facet (recent): `paper_search(query, year_from=current_year-1, limit=5)`

This gives you 15-30 papers in a few seconds. **Present these immediately** in the reading view (Phase 4) — don't wait for citation expansion.

**Step 2 — Citation Expansion (1-hop only, after initial results are shown):**
Pick the top 3-5 most-cited papers from Step 1 as seeds. For each:

- `paper_citations(seed_id, limit=15)`
- `paper_references(seed_id, limit=15)`

This is **1-hop only** — no 2nd hop. Merge with Step 1 results, deduplicate, and update the reading view sections with any important new finds.

**Step 3 — Recommendations (optional, skip if slow):**
If `paper_recommendations` is available, call it once with the top 3 seed IDs. `limit=10`. Merge any truly new papers into results.

**Total API budget:** Aim for ≤ 20 paper API calls total. Do NOT do 2-hop expansion, author tracking, or multi-batch recommendations — these are slow and produce diminishing returns for an initial exploration.

**Do NOT use web browsing for paper metadata.** Never open arXiv URLs, run web searches to "confirm" paper metadata, read abstracts from arXiv pages, or attempt to access any paper pages. The `paper_search` and `paper_citations` APIs already return titles, authors, years, abstracts, and valid IDs. Web browsing for paper metadata wastes minutes and often gets blocked by the sandbox. Use ONLY the paper API tools.

### Explore Phase 3: Analyze and Organize

After all subagents return, the main agent:

#### Step 1: Deduplicate
Same dedup logic as discovery mode (DOI → arXiv ID → S2 ID → title fuzzy match).

#### Step 1b: Mark Already-Known Papers
If Phase 0 found KB or Zotero matches, cross-reference against the merged paper pool. Mark matching papers as "already in KB" or "already in Zotero" -- do NOT remove them (they're still valuable for the landscape map), but annotate them in the output so the user knows which ones are new vs. already in their library.

#### Step 1c: Annotate Discovery Provenance
For each paper, record how it was found. **Provenance should be meaningful to the reader** — it tells them how well-connected a paper is to the field, not which API call found it.

Good provenance (tells the user something useful):
- "Cited by 4 seed papers" — well-connected, likely foundational
- "Bridge paper — connects [subfield A] and [subfield B]" — interdisciplinary importance
- "Recent (2025), not yet cited by established work" — cutting edge, less vetted
- "[Author]'s latest work" — continuity from a key researcher

Bad provenance (do NOT use — the user doesn't care about your search mechanics):
- "Found via keyword search (query string)" — meaningless to the reader
- "Found via citation graph" — too generic, says nothing about the paper
- "Found via recommendation" — internal plumbing, not insight

#### Step 2: Identify Approaches
Cluster the papers into 3-6 **distinct approaches or method families**. For each cluster, identify:
- The approach name (a clear, descriptive label for the method family)
- 2-3 representative papers
- The key idea in 1-2 sentences
- Strengths and limitations

This is the core intellectual work of explore mode -- turning a bag of papers into a structured understanding.

#### Step 3: Build Reading Order
Organize papers into tiers:

1. **Start Here** (2-3 papers): The most accessible, well-cited papers that give a broad overview. Prefer survey papers if any were found.
2. **Core Methods** (4-8 papers): The best paper from each approach cluster. These are the ones to synthesize with `$paper-synthesis`.
3. **Cutting Edge** (3-5 papers): Recent papers (last 1-2 years) with the newest results or approaches.
4. **Deep Dives** (optional, 2-4 papers): Highly specialized papers for specific subtopics the user might want to explore further.

### Citation Formatting Rules

**Keep DOIs, arXiv IDs, and long URLs out of prose paragraphs.** They break reading flow and add no value inline.

- **In narrative sections** (The Landscape, Approaches, recommendations): cite as **Author (Year)** only. Example: "This approach was introduced by Smith et al. (2020)." Never: "This approach was introduced by Smith et al. (2020; arXiv 2003.XXXXX; DOI 10.xxx)."
- **In Key Insights**: cite as Author (Year) like other sections. Do NOT include `$paper-synthesis` commands — auto-continue handles synthesis automatically.
- **In a References section at the end**: list full citations with IDs. This is the one place DOIs and arXiv IDs appear in full.
- **For web sources**: cite as `(source name)` in prose; collect full URLs in References.

The same rule applies to both explore mode and discovery mode output.

### Explore Phase 4: Present the Briefing

#### Section Length Rules

**Hard rule: no section may exceed 40 lines.** The reading view is a terminal — each section should fit on one screen. This applies to BOTH initial fills AND follow-up rewrites. Never rewrite a section into a wall of text that exceeds these limits.

- **The Landscape**: 2-3 short paragraphs max (~15 lines). Orientation, not literature review.
- **Approaches**: 4-6 lines per approach cluster. Max 5 clusters. Total section ≤ 35 lines.
- **Key Insights**: The 3-5 most important takeaways from the discovered papers — practical findings, surprising results, consensus views, or unresolved debates. Each insight is 2-3 sentences grounded in specific papers (cite as Author (Year)). This is NOT a paper list — it's what someone would learn from reading these papers. Total section ≤ 30 lines. End with "Proceeding to synthesize [N] papers for full analysis..." (auto-continue) or nothing (discovery-only).
- **Open Questions**: 2-3 bullet points, one sentence each.

When in doubt, err on the side of brevity. The reading view is for orientation — deep analysis belongs in `$paper-synthesis`.

#### Follow-Up Expansion Rules

When the user asks for more detail on a section (e.g., "explain the approaches in more detail"):

- **Do NOT rewrite the section** with longer paragraphs. The structured format (Key idea / Papers / Tradeoff) is the correct format for discovery — it's designed for orientation, not deep analysis.
- **Use `append_to_section` with `foldable=true`** to add expandable detail blocks BELOW the existing compact section. Each foldable block should cover one approach with 3-5 sentences of additional context. This preserves the scannable overview while offering depth on demand.
- **If the user wants full explanations**, direct them to `$paper-synthesis` for individual papers or `$cross-paper-report` for comparative deep dives. Discovery is not the right tool for deep explanation — it's a map, not a textbook.
- **Never turn the Approaches section into multi-paragraph prose.** The reading view is a terminal with limited vertical space. Dense paragraphs per approach make the section unscrollable and unnavigable.

Use **4 sections** in the reading view:

```
## The Landscape

[2-3 short paragraphs. What is this field about? Main challenges and paradigms.
Cite as Author (Year) — no IDs inline.]

## Approaches

#### 1. [Approach Name]
**Key idea**: [1-2 sentences]
**Papers**: [Paper] ([Year]), [Paper] ([Year])
**Tradeoff**: [1 sentence]

#### 2. [Approach Name]
...

## Key Insights

[3-5 insights extracted from the discovered papers. Each is a finding, not a paper title.]

- **[Insight]**: [2-3 sentences grounded in specific papers. Cite as Author (Year).]
- **[Insight]**: ...

[If auto-continue: "Proceeding to synthesize [N] papers for full analysis..."]

## Open Questions

- [question 1]
- [question 2]
```

### Explore Mode Graceful Degradation

- **No S2 API key / no `paper_recommendations`**: Skip recommendations; keyword + citations cover the landscape
- **Sparse results**: Broaden facets or add a 4th facet
- **Very broad topic**: Focus on highest-citation papers, suggest narrowing
- **No KB configured**: Skip Phase 0, proceed with search

---

## DISCOVERY MODE

Discovery mode finds papers adjacent to what you already know. Uses KB cards (or Zotero) as seeds.

### Discovery Phase 1: Gather Seeds

1. List KB cards per `$kb` (skip if KB is disabled). Extract paper IDs from `source.refs` and topics from `tags`.
2. If `topic:<topic>` specified, filter to matching cards.
3. If no KB or KB is disabled: try `zotero_get_recent(limit=50)`. If neither: ask user for 3-5 seed paper IDs.

### Discovery Phase 2: Search

From the top 5 seed papers:

1. `paper_citations(seed_id, limit=15)` for each — what builds on them?
2. `paper_references(seed_id, limit=15)` for each — what do they build on?
3. `paper_search(topic_tag, year_from=current_year-1, limit=10)` for each unique topic — recent trends
4. (Optional) `paper_recommendations(seed_ids, limit=10)` if available

**Budget: ≤ 25 API calls total.** Do not use author tracking or 2-hop expansion.

### Discovery Phase 3: Merge, Deduplicate, Rank

Deduplicate by DOI → arXiv → title fuzzy match. Filter out papers already in KB. Rank by citation count + recency + overlap with KB papers.

### Discovery Phase 4: Present

Use the same 4-section reading view format as explore mode: Landscape, Approaches, Key Insights, Open Questions. Same section length rules apply.

## Post-Discovery Housekeeping

**Skip this entire section if KB is disabled.** When KB is off, do not write journal entries or research-context updates. The discovery report in the reading view is the sole output.

After presenting the discovery report, do these:

**1. Journal entry** — Append to `<kb_path>/research-journal.md`. Prepend (newest first):

```markdown
## [Date] — Discovery: [Topic/Question]

### Explored
- Mode: [explore / discovery / topic:X / authors]
- Searched: [brief description of what was searched for]
- Found [N] new papers across [M] strategies

### Top Finds
- [2-3 most promising papers with one-line descriptions]

### Open Questions
- [1-2 threads worth pursuing next]

---
```

**2. Research context detection** — If the user's discovery request or follow-up questions reveal priorities (e.g., "focus on methods that work in real-time", "I'm not interested in simulation-only results"), offer to note it in `research-context.md`.

## Presentation

Use the same outline → fill pattern described in Explore Phase 4. Use 4 sections: `"## The Landscape\n\n## Approaches\n\n## Key Insights\n\n## Open Questions"`.

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with headings-only content.
**Phase 2 (Fill):** Fill sections sequentially via `update_document_section`.

**Markdown:** Always put a blank line before list items.

**Follow-ups:** When the user asks for more detail on a section, use `append_to_section` with `foldable=true` to add expandable blocks below the existing content. Do NOT rewrite sections with longer paragraphs — this violates the section length rules. The compact format is intentional for discovery. For deep explanations, direct the user to `$paper-synthesis` or `$cross-paper-report`.
