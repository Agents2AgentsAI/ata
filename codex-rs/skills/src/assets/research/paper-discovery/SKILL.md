---
name: paper-discovery
description: "REQUIRED for any paper search or discovery. Do NOT call paper_search, paper_citations, or paper_recommendations directly — always open this SKILL.md first and follow its workflow. Use when a user asks a research question, wants to learn about a topic, asks how something is done in the literature, or wants to find/discover papers. Examples: 'how do people train RL for robotic grasping', 'find me papers on VLAs', 'what are the best methods for X'. Provides structured landscape briefings with faceted search, reading view presentation, and automatic synthesis."
metadata:
  short-description: Discover and rank papers for a topic
---

# Paper Discovery

## RULES — read these first, follow them exactly

1. **ALWAYS auto-continue.** After presenting the discovery reading view, IMMEDIATELY proceed to `$paper-synthesis` (multi-paper path) then `$cross-paper-report`. Do NOT stop at a paper list. Do NOT wait for user input. The ONLY exception: user explicitly said "just list", "just find", "don't synthesize", or "discovery only".
2. **ALWAYS use exactly these 4 sections** in the reading view — no other structure:
   - `The Landscape` — 2-3 paragraphs: what is this field, main challenges, paradigms
   - `Approaches` — 3-6 approach clusters, each with Key idea / Papers / Tradeoff
   - `Key Insights` — 3-5 findings (NOT paper titles), end with "Proceeding to synthesize [N] papers..."
   - `Open Questions` — 2-3 bullets
3. **NEVER** create sections like "What you asked for", "Summary", or "Papers Found". Every section must contain **analysis**, not paper lists.
4. **NEVER** output a flat list of paper titles. Cluster papers into approaches and explain what each approach does.
5. **ALWAYS** present via `present_reading_view`. Never as chat text.
6. **ALWAYS** call `present_reading_view` with headings-only content FIRST, then fill sections via `update_document_section`.

### Reading View Template (EXACT FORMAT — use this)

```
present_reading_view(title="[Topic] — Research Landscape", content=
"## The Landscape\n\n## Approaches\n\n## Key Insights\n\n## Open Questions")
```

Then fill each section:

**Section 0 — The Landscape**: 2-3 short paragraphs. What is this field about? Main challenges and paradigms. Cite as Author (Year) — no IDs inline.

**Section 1 — Approaches**:
```
#### 1. [Approach Name]
**Key idea**: [1-2 sentences]
**Papers**: Author1 (Year), Author2 (Year)
**Tradeoff**: [1 sentence]

#### 2. [Approach Name]
...
```

**Section 2 — Key Insights**: 3-5 insights extracted from the papers. Each is a finding, not a paper title.
```
- **[Insight]**: [2-3 sentences grounded in specific papers. Cite as Author (Year).]
- **[Insight]**: ...

Proceeding to synthesize [N] papers for full analysis...
```

**Section 3 — Open Questions**: 2-3 bullet points, one sentence each.

**Hard rule: no section may exceed 40 lines.** The reading view is a terminal — each section should fit on one screen.

---

## Prohibitions

- Do NOT read other SKILL.md files (you already have the instructions)
- Do NOT run `rg --version`, `ls`, or any diagnostic commands before searching
- Do NOT read `research-context.md` — use conversation context instead
- Do NOT read individual KB cards one by one — a single `rg` search is enough
- Do NOT open arXiv URLs for ANY reason — `paper_search` and `paper_citations` already return titles, authors, years, and abstracts
- Do NOT use `web.run`, `web_search`, or any web browsing to look up papers
- Do NOT call `paper_get` for papers returned by `paper_search` or `paper_citations` — those APIs already return all metadata. `paper_get` is ONLY useful for bare S2 IDs with no metadata.
- Do NOT include `citeturn*view*` or similar citation markers — write citations as Author (Year) only

**Speed matters.** The user should see results within 30 seconds. Every tool call before `paper_search` is delay the user feels.

---

## Modes

Three modes:

1. **Citation-focused** — user references a specific paper and asks for related/recent/citing work. Use `paper_citations` + 1-2 keyword searches.
2. **Explore** — user has a research question or topic. Use faceted keyword search + citation expansion.
3. **Discovery** — user has a research base, wants adjacent papers. Use KB seeds + citation/recommendation APIs.

**Mode detection:**
- User just read a paper and asks "find more like this", "what cites this" → **citation-focused**
- User asks a question or describes a topic → **explore**
- User invokes `$paper-discovery` with no arguments → **discovery**

---

## EXPLORE MODE

Explore mode answers research questions by mapping the landscape of approaches and surfacing the most important papers.

### Phase 0: Check Existing Knowledge (1 tool call max)

**Skip if KB is disabled.** Run `rg "tag1\|tag2\|tag3" ~/.ata/knowledge-base/cards/` with 2-3 tags. Do NOT list KB, read cards, or call `$kb`.

### Phase 1: Decompose the Question into Facets

Parse the user's question into 3-5 search facets. Each targets a different angle.

**Facet design — CRITICAL for search quality:**

1. **Encode the relationship, not just keywords.** "how is X used for Y" → facets like `"X as supervision for Y"`, `"X-guided Y training"`, NOT just `"X Y"`.
2. **Disambiguate overloaded terms.** "attention supervision" matches both eye-tracking AND transformer attention. Use specific phrases.
3. **Vary abstraction level.** One narrow (exact technique), one broad (research area), one alternative terminology.
4. **Use field-specific vocabulary.** What do researchers actually write in paper titles?
5. **Test mentally.** "If I search this exact string, could most results be unrelated?" If yes, narrow it.

**Example — BAD facets** for "interpretability of AI using LLMs":
- `"large language model interpretability explanations for AI models"` — too vague, returns noise like decision trees and ChatGPT benchmarks

**Example — GOOD facets** for the same query:
- `"LLM-generated explanations for neural network predictions"` — encodes the relationship
- `"self-rationalization large language models faithfulness"` — specific technique name
- `"language model as post-hoc explainer black-box model"` — alternative framing
- `"natural language explanations model interpretability"` — broader but still targeted

### Phase 2: Search

#### Citation-Focused Path (user just read a paper)

3 tool calls:
1. `paper_citations(known_paper_id, limit=30)`
2. `paper_search(topic_keywords, year_from=current_year-1, limit=10, sort_by=citation_count)`
3. `paper_search(alternative_keywords, limit=10, sort_by=citation_count)`

Then present immediately.

#### General Explore Path (topic/question)

**Step 1 — Keyword Search (first, no subagents):**
Run 2-3 facets in parallel:
- Per facet: `paper_search(query, limit=10, sort_by=citation_count)`
- Per facet (recent): `paper_search(query, year_from=current_year-1, limit=5)`

**Present these immediately** in the reading view — don't wait for citation expansion.

**Step 2 — Citation Expansion (1-hop, after presenting):**
Top 3-5 most-cited papers as seeds:
- `paper_citations(seed_id, limit=15)`
- `paper_references(seed_id, limit=15)`

Merge, deduplicate, update reading view.

**Total API budget: ≤ 20 calls.** No 2-hop expansion, no author tracking.

### Phase 3: Analyze and Organize

1. **Deduplicate** by DOI → arXiv ID → S2 ID → title fuzzy match.
2. **Mark known papers** if KB had matches — annotate, don't remove.
3. **Annotate provenance** meaningfully: "Cited by 4 seed papers" (good), "Found via keyword search" (bad — user doesn't care about your search mechanics).
4. **Cluster into 3-6 approaches** — the core intellectual work. Each cluster: approach name, 2-3 papers, key idea, strengths/limitations.
5. **Build reading order**: Start Here (2-3), Core Methods (4-8), Cutting Edge (3-5), Deep Dives (optional 2-4).

### Phase 4: Present the Briefing

Follow the EXACT template from the Rules section above. 4 sections: The Landscape, Approaches, Key Insights, Open Questions.

### Citation Formatting

- **In prose**: Author (Year) only. Never inline DOIs or arXiv IDs.
- **References section at end**: full citations with IDs (the one place IDs appear).

### Follow-Up Expansion

- Do NOT rewrite sections with longer paragraphs
- Use `append_to_section` with `foldable=true` for expandable detail blocks
- For full explanations, direct to `$paper-synthesis` or `$cross-paper-report`

### Graceful Degradation

- No S2 API key: skip recommendations, keyword + citations suffice
- Sparse results: broaden facets or add a 4th
- Very broad topic: focus on highest-citation papers

---

## DISCOVERY MODE

Discovery mode finds papers adjacent to what you already know. Uses KB/Zotero as seeds.

### Phase 1: Gather Seeds

1. List KB cards via `$kb` (skip if disabled). Extract paper IDs and topics.
2. If `topic:<topic>`, filter to matching cards.
3. If no KB: try `zotero_get_recent(limit=50)`. If neither: ask for 3-5 seed paper IDs.

### Phase 2: Search

From top 5 seeds:
1. `paper_citations(seed_id, limit=15)` each
2. `paper_references(seed_id, limit=15)` each
3. `paper_search(topic_tag, year_from=current_year-1, limit=10)` per unique topic
4. (Optional) `paper_recommendations(seed_ids, limit=10)`

**Budget: ≤ 25 API calls.**

### Phase 3: Merge, Deduplicate, Rank

Dedup by DOI → arXiv → title fuzzy. Filter out papers already in KB. Rank by citations + recency.

### Phase 4: Present

Same 4-section reading view as explore mode. Same rules.

---

## Post-Discovery Housekeeping

**Skip entirely if KB is disabled.**

**1. Journal entry** — Prepend to `<kb_path>/research-journal.md`:

```markdown
## [Date] — Discovery: [Topic/Question]

### Explored
- Mode: [explore / discovery / citation-focused]
- Found [N] new papers across [M] strategies

### Top Finds
- [2-3 most promising papers with one-line descriptions]

### Open Questions
- [1-2 threads worth pursuing next]

---
```

**2. Research context** — If the request reveals priorities, offer to note in `research-context.md`.

## Presentation

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with headings-only content.
**Phase 2 (Fill):** Fill sections sequentially via `update_document_section`.

**Markdown:** Always put a blank line before list items.

**Follow-ups:** Use `append_to_section` with `foldable=true`. Do NOT rewrite sections into walls of text.

## Pipeline Paths (after discovery)

1. **Quick orientation** — `$research-briefing` for 2-4 page overview
2. **Full deep analysis** — `$paper-synthesis` (multi-paper) → `$cross-paper-report` (DEFAULT for auto-continue)
3. **Interactive exploration** — user chats about specific papers

### Scale-Aware Branching
- **≤ 12 papers**: Standard pipeline
- **> 12 papers**: `cross-paper-report` activates large-set mode (clustered sub-reports)
