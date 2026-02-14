---
name: paper-discovery
description: Use when a user asks a research question, wants to learn about a research topic, asks how something is done in the literature, or wants to discover papers. Examples -- "how do people train RL for robotic grasping", "what are the best methods for sim-to-real transfer", "I want to learn about diffusion policies", "find me papers on VLAs". Provides structured landscape briefings with approaches, best papers, and reading plans. Also handles KB/Zotero-based proactive discovery.
metadata:
  short-description: Discover and rank papers for a topic
---

# Paper Discovery

**Output format**: Always present final reports using `present_document` (not as regular text). See the Presentation section below.

Two modes of operation:

1. **Explore mode** (`explore:<question or topic>`) -- You have a research question or want to learn about a topic. Get a structured landscape briefing: main approaches, best papers for each, state of the art, and a reading plan.
2. **Discovery mode** (default / `topic:` / `authors` / `recent`) -- You already have a research base. Find papers adjacent to what you already know using citation graphs, recommendations, trends, and author tracking.

## Invocation Modes

- `$paper-discovery explore: how do people train RL for robotic grasping` -- explore a question or topic from scratch
- `$paper-discovery explore: best methods for sim-to-real transfer in manipulation` -- practical "how to" questions work too
- `$paper-discovery` -- discovery mode: full KB scan, all 4 strategies
- `$paper-discovery topic:<topic>` -- discovery mode: focused on a specific topic tag
- `$paper-discovery authors` -- discovery mode: author-tracking only
- `$paper-discovery recent` -- discovery mode: trend scanning only

**Mode detection**: If the user's input is a question or describes something they want to learn about (even without the `explore:` prefix), use explore mode. If the user just invokes `$paper-discovery` with no arguments, use discovery mode.


## Mandatory Pipeline Contract

This skill is discovery-first. After discovery, the user has three paths:

1. **Quick orientation** — `$research-briefing` gives a 2-4 page overview with core ideas per paper (recommended first step)
2. **Full deep analysis** — `$paper-synthesis` to create deep per-paper cards, then `$cross-paper-report` for the integrated narrative + PDF
3. **Interactive exploration** — Chat about specific papers, ask follow-ups, then `$conversation-report` when ready to capture what was discussed

For multi-paper requests that ask for a final explained report, enforce the full pipeline: `paper-discovery` → `paper-synthesis` → `cross-paper-report`. Do not mark the request complete until step 3 is done.

### Scale-Aware Pipeline Branching

The pipeline adapts based on the number of shortlisted papers:

- **≤ 12 papers**: Standard pipeline. `cross-paper-report` runs Phases 1–3 in a single pass with full 800+ words per card.
- **> 12 papers**: `cross-paper-report` automatically activates **large-set mode** — it clusters papers into groups of 4–8, launches parallel subagents for per-cluster deep reports, then produces a meta-synthesis connecting the clusters. No manual intervention is needed; scale detection is built into the `cross-paper-report` skill.

When presenting the reading plan in explore mode, note the expected pipeline mode:
- "This set of [N] papers will use **standard synthesis** (deep per-card walkthroughs in one report)."
- "This set of [N] papers will use **large-set synthesis** (clustered sub-reports with cross-cluster meta-synthesis) for depth at scale."

---

## EXPLORE MODE

Explore mode answers research questions by mapping the landscape of approaches and surfacing the most important papers to read. No existing KB or Zotero library is required -- but if available, they enrich the results.

### Explore Phase 0: Check Existing Knowledge (Optional)

Before searching externally, check if the user already has relevant material:

1. If KB is available, call `kb_status` then `kb_list_cards`. Filter cards whose tags or titles relate to the user's question.
2. **Read research context**: If `research-context.md` exists at the KB root (via `kb_read_file`), use it to bias discovery:
   - **Priorities**: Weight ranking toward papers relevant to the user's priorities (e.g., if they care about inference latency, rank latency-focused papers higher)
   - **Not Interested In**: De-prioritize papers in areas the user has dismissed
   - **Recent journal entries**: Optionally read `research-journal.md` to check what the user has explored recently — avoid re-discovering papers from recent sessions
3. If Zotero is available, call `zotero_search` with a short keyword from the question (limit 10).
4. Collect any matching paper IDs -- these become **boost seeds**:
   - Use them as additional positive examples in Strategy C (recommendations)
   - Use them as additional roots in Strategy B (citation expansion)
   - Mark them as "already in KB/Zotero" in the final output so the user knows what they already have vs. what's new
4. If the user already has extensive coverage (10+ matching cards), note this in the briefing: "You already have strong coverage of X. Here's what's new or what you might be missing."

This step is best-effort. If KB or Zotero aren't configured, skip it and proceed with external search only.

### Explore Phase 1: Decompose the Question

Parse the user's question into 3-5 search facets. Each facet targets a different angle of the problem.

**Example**: "how do people train RL for robotic grasping"
- Facet 1: `"reinforcement learning robotic grasping"` (direct match)
- Facet 2: `"sim-to-real transfer grasping"` (common training paradigm)
- Facet 3: `"dexterous manipulation policy learning"` (broader framing)
- Facet 4: `"grasp planning deep learning"` (alternative terminology)

**Example**: "best methods for sim-to-real transfer in manipulation"
- Facet 1: `"sim-to-real transfer robot manipulation"`
- Facet 2: `"domain randomization manipulation policy"`
- Facet 3: `"real-world robot learning from simulation"`
- Facet 4: `"domain adaptation robotic control"`

### Explore Phase 2: Multi-Angle Search (Hierarchical, Parallel Subagents)

The search strategies are **hierarchical, not equal-weight**. Citation graph expansion is the primary engine — keyword search supplements it.

Launch 2-3 subagents in parallel:

#### Seed Selection (Before Launching Subagents)

Determine seed papers for citation expansion:

1. **Best case — KB cards exist**: Use cards from Phase 0 as seeds (they have paper IDs ready)
2. **User mentions specific papers**: Use those as seeds
3. **Neither available**: Do a quick keyword bootstrap — run 2-3 facets from Phase 1 through `paper_search` with `limit=5, sort_by=citation_count` to find 3-5 highly-cited papers. These become the seeds. Then proceed to citation expansion.

Never do pure keyword discovery without citation expansion. If you have no seeds at all, ask the user: "Can you name 2-3 papers you already know in this area? That helps me map the field much faster via citation graphs."

#### Primary Strategy: Seed-Based Citation Expansion

This is the core discovery engine. "You have a paper, you see what cites it, you have the field."

**2-hop expansion:**

- **Hop 1**: For each seed paper (up to 10 seeds):
  - Call `paper_references` (limit 30) — what do these papers build on?
  - Call `paper_citations` (limit 30) — what builds on them?
  - This produces the immediate citation neighborhood

- **Hop 2**: From hop 1 results, select the top 10 most-cited papers that are NOT already seeds:
  - Call `paper_citations` for each (limit 15) — what builds on the next layer?
  - This extends reach to the broader field

**Bridge detection**: Papers appearing in 3+ citation neighborhoods are "bridge papers" — these connect subfields and are almost always important. Flag them prominently in results.

**Recency filter**: Within citation results, separately track papers from the last 2 years. These are the cutting edge — they cite the established work but push beyond it.

Tag each result with `discovery_source: "citation_graph"` and include provenance: "Found via citation graph (cited by N of your seed papers)" or "Bridge paper (appears in N citation neighborhoods)".

#### Secondary Strategy: Targeted Keyword Search

Supplements citation expansion to catch papers too new to have citation connections yet.

For 2-3 facets (not 4-5 — keep it narrow):
1. Call `paper_search` with the facet query, `limit=10`, `sort_by=citation_count`
2. Call `paper_search` with the facet query, `year_from` = current year - 1, `limit=10`, `sort_by=citation_count` (very recent only)
3. Merge and deduplicate across facets

This is explicitly framed as "supplement to citation graph, not replacement." Its purpose is to catch papers that are too new to have citation connections yet.

Tag each result with `discovery_source: "keyword_search"`.

#### Supplementary Strategy: Recommendations + Author Tracking

Lighter weight — fills gaps that citation and keyword search might miss:

**Embedding recommendations** (if `paper_recommendations` is available):
- From the top 5 seed papers, call `paper_recommendations` with `limit=15`
- This catches conceptually related papers that use different terminology

**Author tracking**:
- From the top 10 papers found across all strategies, identify authors appearing on 2+ papers
- For each, call `paper_search` with `"author:<name>"`, `year_from` = current year - 1, `limit=5`
- This catches very recent work from key researchers

Tag results with `discovery_source: "recommendation"` or `discovery_source: "author_track"` respectively.

Skip embedding recommendations if `paper_recommendations` is not available.

### Explore Phase 3: Analyze and Organize

After all subagents return, the main agent:

#### Step 1: Deduplicate
Same dedup logic as discovery mode (DOI → arXiv ID → S2 ID → title fuzzy match).

#### Step 1b: Mark Already-Known Papers
If Phase 0 found KB or Zotero matches, cross-reference against the merged paper pool. Mark matching papers as "already in KB" or "already in Zotero" -- do NOT remove them (they're still valuable for the landscape map), but annotate them in the output so the user knows which ones are new vs. already in their library.

#### Step 1c: Annotate Discovery Provenance
For each paper, record how it was found:
- "Found via citation graph (cited by N of your seed papers)" — papers discovered through citation expansion
- "Bridge paper (appears in N citation neighborhoods)" — papers connecting multiple subfields
- "Found via keyword search (no citation connection yet)" — papers found through keyword search without citation links to seeds
- "Found via recommendation (embedding similarity to [seed paper])" — papers found through S2 recommendations
- "Found via author tracking ([author name]'s recent work)" — papers found through author expansion

This provenance helps the user understand which papers are well-connected to their existing knowledge vs. isolated finds.

#### Step 2: Identify Approaches
Cluster the papers into 3-6 **distinct approaches or method families**. For each cluster, identify:
- The approach name (e.g., "Sim-to-Real with Domain Randomization", "Direct Real-World RL", "Offline RL from Demonstrations")
- 2-4 representative papers
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

- **In narrative sections** (The Landscape, Approaches, recommendations): cite as **Author (Year)** only. Example: "Domain randomization (Tobin et al., 2017) is the canonical sim-to-real lever." Never: "Domain randomization (Tobin et al., 2017; arXiv 1703.06907; DOI 10.xxx)."
- **In the Reading Plan**: include IDs on their own line as a synthesis command: `→ $paper-synthesis 1703.06907`. This is where IDs belong — actionable, not decorative.
- **In a References section at the end**: list full citations with IDs. This is the one place DOIs and arXiv IDs appear in full.
- **For web sources**: cite as `(source name)` in prose; collect full URLs in References.

The same rule applies to both explore mode and discovery mode output.

### Explore Phase 4: Present the Briefing

```
## Research Briefing: [question/topic]
### [Date]

### The Landscape

[2-3 paragraph overview: What is this field about? What are the main challenges?
What are the dominant paradigms? Where is the field heading?
Cite papers as Author (Year) only — no IDs inline.]

### Approaches

#### 1. [Approach Name]
**Key idea**: [1-2 sentences]
**Representative papers**:
- [Paper] ([Year]) -- [why it matters, 1 sentence] [discovery provenance]
- [Paper] ([Year]) -- [why it matters] [discovery provenance]
**Strengths**: [1 sentence]
**Limitations**: [1 sentence]

#### 2. [Approach Name]
...

### Reading Plan

#### Start Here
1. **[Title]** -- [Authors] ([Year])
   [Why to read this first -- 1 sentence]
   Discovery: [provenance, e.g., "Bridge paper — cited by 4 seed papers"]
   → `$paper-synthesis [arXiv ID or DOI]`

#### Core Methods
2. **[Title]** -- [Authors] ([Year])
   Approach: [which approach cluster]
   [What you'll learn -- 1 sentence]
   Discovery: [provenance]
   → `$paper-synthesis [arXiv ID or DOI]`
...

#### Cutting Edge
...

### Open Questions
[2-3 bullet points: What's unsolved? Where is the field going?
What should someone entering this area pay attention to?]

### Next Steps
- `$research-briefing` — Quick 2-3 page orientation of the discovered papers (recommended first step)
- `$paper-synthesis [ID]` → `$cross-paper-report` — Full deep analysis pipeline
- Chat interactively about specific papers, then `$conversation-report` when ready

### References
[Full citations with IDs — the one place DOIs and arXiv IDs appear:]
- Tobin et al., "Domain Randomization for Transferring Deep Neural Networks from Simulation to the Real World," 2017. arXiv:1703.06907
- Mahler et al., "Dex-Net 2.0," RSS 2017. DOI:10.15607/rss.2017.xiii.058
- ...
```

### Explore Mode Graceful Degradation

- **No S2 API key**: Skip Strategy C (recommendations); the other 3 strategies still cover the landscape well
- **No `paper_recommendations` tool**: Skip Strategy C
- **Sparse results for a facet**: The multi-facet design ensures other angles compensate
- **Very broad topic**: Focus on the highest-citation papers and note that the user should narrow their question for deeper coverage
- **Very narrow topic**: Broaden facets and rely more on citation expansion (Strategy B) to find related work
- **No KB tools**: Present the briefing in chat only

---

## DISCOVERY MODE

Discovery mode is for users who already have a research base (KB cards or Zotero library) and want to find what they're missing.

### Execution: Use Subagents

**Always launch discovery sub-agents in parallel.** This is mandatory for performance:

1. The main agent gathers seed data (Phase 1 below)
2. The main agent launches 2-4 sub-agents in parallel (one per active discovery strategy)
3. The main agent merges, deduplicates, and ranks results (Phase 3 below)
4. The main agent presents the final discovery report

#### Subagent Prompt Construction

Each subagent prompt should include:
- The specific strategy to execute (citation explorer, recommendation engine, trend scanner, or author tracker)
- The seed data relevant to that strategy (paper IDs, topic tags, or author names)
- Instructions to return structured JSON-like results with paper metadata

#### What Subagents Return

Each subagent returns a list of discovered papers with:
- Title, authors, year, venue, citation count
- Paper IDs (DOI, arXiv ID, S2 ID) for deduplication
- Discovery source tag (e.g., "citation_graph", "s2_recommendation", "trend_scan", "author_track")
- Brief rationale (e.g., "cited by 3 of your KB papers", "recent work by frequent author X")

### Discovery Phase 1: Gather Seed Data

#### Step 1: Read KB State

1. Call `kb_status` to get `kb_path` and verify KB exists.
2. Call `kb_list_cards` to retrieve all cards.
3. Extract from cards:
   - **Paper IDs**: DOI, arXiv ID, or S2 ID from card `refs` fields
   - **Topics**: from card `tags` fields
   - **Authors**: from card metadata (author fields in card body)

#### Step 2: Topic Filtering (if topic-focused mode)

If the user specified `topic:<topic>`:
- Filter seed cards to those with matching tags
- Narrow paper IDs, topics, and authors to the filtered set
- If no cards match the topic, fall back to using the topic as a keyword search query

#### Step 3: Fallback to Zotero (if no KB)

If `kb_status` shows no cards or KB is not configured:
1. Call `zotero_get_recent` with `limit=50` to get recent library items
2. Call `zotero_get_tags` to get topic tags
3. Extract paper IDs from Zotero items (DOI field)
4. Use Zotero tags as topic seeds

If neither KB nor Zotero is available, ask the user to provide seed paper IDs or topics manually.

### Discovery Phase 2: Discovery Strategies (Parallel Subagents)

Launch the applicable strategies as parallel subagents. Each strategy operates independently.

#### Strategy 1: Citation Graph Explorer

For the top 5-10 seed papers (by citation count or recency):

1. Call `paper_citations` for each seed paper (limit 20 per paper)
2. Call `paper_references` for each seed paper (limit 20 per paper)
3. Track frequency: papers appearing in 2+ citation neighborhoods are "bridge papers"
4. Return deduplicated list with overlap counts and discovery rationale

Tag each result with `discovery_source: "citation_graph"`.

#### Strategy 2: S2 Recommendation Engine

Uses the `paper_recommendations` tool for embedding-based similarity:

1. Group seed paper IDs into batches of up to 5 (S2 API accepts multiple positive examples)
2. Call `paper_recommendations` for each batch with `limit=20`
3. Merge results across batches
4. Return ranked list

Tag each result with `discovery_source: "s2_recommendation"`.

**Graceful degradation**: If `paper_recommendations` is not available or the S2 API key is missing, skip this strategy entirely. The other 3 strategies still provide valuable discovery.

#### Strategy 3: Trend Scanner

For each unique topic tag from seed data:

1. Call `paper_search` with the topic as query, `year_from` = current year - 1, `sort_by=citation_count`
2. Take top 10 results per topic (high-impact recent papers)
3. Return papers with topic association

Tag each result with `discovery_source: "trend_scan"`.

#### Strategy 4: Author Tracker

1. Extract unique first and last authors from seed papers
2. Identify the top 5-10 most-represented authors (appear on multiple seed papers)
3. For each author, call `paper_search` with `"author:<name>"` query, `year_from` = current year - 1
4. Return latest publications per tracked author

Tag each result with `discovery_source: "author_track"`.

### Discovery Phase 3: Merge, Deduplicate, and Rank

#### Step 1: Merge

Collect all papers from all completed strategies into a single pool.

#### Step 2: Deduplicate

Match papers by (in priority order):
1. DOI (exact match)
2. arXiv ID (exact match)
3. S2 paper ID (exact match)
4. Title (case-insensitive fuzzy match -- papers with near-identical titles are likely duplicates)

When merging duplicates, combine discovery sources and rationales.

#### Step 3: Filter Already-Known Papers

Remove papers that are already in the user's knowledge base:
- Match against `kb_list_cards` source refs (DOI, arXiv ID)
- Match against card titles (fuzzy)

Optionally, if Zotero is configured:
- Call `zotero_search` by DOI for candidate papers to check if already in library
- Mark Zotero-present papers as "already in Zotero" rather than removing them (the user may want to synthesize them)

#### Step 4: Rank

Score each paper by composite ranking:

| Signal | Points |
|--------|--------|
| Citation overlap with KB (paper cites or is cited by a KB paper) | +3 per connection |
| Convergence (found by N different strategies) | +2 per strategy |
| Recency bonus (published within last 12 months) | +1 |
| Citation count (log-scaled) | +log10(citations + 1) |

Sort by composite score descending.

### Discovery Phase 4: Present Results

#### Discovery Report

Present the top 15-20 papers as a numbered list. Follow the Citation Formatting Rules above — IDs appear only in the per-paper metadata line and the References section, never in prose.

```
## Paper Discovery Report
### [Date] | Mode: [full-kb / topic:X / authors / recent]

Found N new papers across M discovery strategies.

#### Top Discoveries

1. **[Title]** -- [Authors] ([Year], [Venue])
   Citations: [N] | Score: [composite]
   Discovery: [rationale, e.g., "Cited by 3 KB papers + recommended by S2 based on your VLA papers"]
   Abstract: [first 200 chars]...
   → `$paper-synthesis [arXiv ID or DOI]`

2. ...

### References
[Full citations with IDs collected here]
```

#### Group by Topic

After the ranked list, group papers by topic area with brief rationale for each group.

### Discovery Mode Graceful Degradation

- **No KB configured**: Fall back to Zotero library scan for seeds (Step 3 in Phase 1)
- **No Zotero configured**: Skip Zotero dedup filter; rely on KB-only seeds
- **No KB and no Zotero**: Ask the user to provide 3-5 seed paper IDs or topics manually, then run strategies 2-4
- **No S2 API key**: Skip Strategy 2 (recommendation engine); still run citation graph + keyword search + author tracking
- **No `paper_recommendations` tool**: Skip Strategy 2; proceed with remaining strategies
- **Strategy failure**: If any single strategy fails (API error, timeout), log a warning and continue with results from other strategies
- **No KB tools**: Present the report in chat only
- **Few seed papers (< 3)**: Reduce citation graph exploration but increase trend scanner and author tracker scope

## Presentation

IMPORTANT: When a discovery or explore mode report is complete, you MUST call `present_document` to present it in sectioned reading mode instead of outputting text directly. Do NOT stream the report as regular text. Set `document_id` to a unique slug, `title` to the report title, and `content` to the full markdown with `## ` headings for sections. End your response immediately after calling this tool.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)
