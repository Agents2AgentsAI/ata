---
name: hn-synthesis
description: Search Hacker News for practitioner discussions on a topic, tool, library, or paper, and synthesize findings into structured KB cards. Use when a user asks about community sentiment, real-world experience, practitioner opinions, or HN discussions about a technology, project, or research paper.
metadata:
  short-description: Synthesize Hacker News discussions into KB knowledge cards
---

# HN Synthesis

Search Hacker News for practitioner discussions and synthesize them into structured knowledge base cards. This captures real-world signal that academic papers miss: deployment war stories, performance gotchas, community sentiment, alternative approaches practitioners actually use, and links to tools/repos that emerge in discussion threads.

## When to Use This Skill

- User asks "what do people think about X?"
- User asks about real-world experience with a tool, library, or technique
- User wants community sentiment or practitioner signal on a topic
- User asks to search Hacker News for something
- As a complement to `paper-synthesis` when you want to understand how practitioners view a research contribution
- When investigating whether a technology has real-world traction beyond papers

## Execution: Use Subagents

For multi-topic or multi-thread analysis, launch one subagent per major thread to keep the main context clean. Each subagent should invoke this skill via the Skill tool.

### Subagent Prompt Template

> Invoke the `hn-synthesis` skill and follow its complete workflow for this thread. Execute every phase: Thread Retrieval, Discussion Analysis, and KB Card Storage. Skip the "Execution: Use Subagents" section — you ARE the subagent.
>
> Thread ID: [HN item ID]
> Topic context: [what the user is investigating]
> KB path: [value from kb_status]

### What Subagents Return

Each subagent writes the KB card directly via `kb_write_card`. After completing, the subagent returns a concise report:
- Card ID that was written
- Thread title, URL, points, comment count
- 3-5 sentence summary of the key community signal
- Notable links or resources surfaced in the discussion
- Consensus vs. dissent ratio (rough characterization)

### Main Agent Role

1. Call `kb_status` to get `kb_path`
2. Run the Discovery phase to find relevant threads
3. Launch subagents for the top threads (in parallel when multiple)
4. Collect subagent reports and present a unified summary to the user
5. If multiple threads were analyzed, produce a cross-thread synthesis (see Phase 4)

## Phase 0: Pre-Synthesis Check

Before searching, check the KB for existing coverage:

1. Call `kb_status` to get `kb_path`.
2. Call `kb_search` with the topic query. Look for cards with `source_type: hackernews`.
3. If a matching card exists and is recent (check `date_updated`), return it and ask the user if they want a fresh search.
4. Otherwise, proceed to Phase 1.

## Phase 1: Discovery

Use `hn_search` to find relevant discussions. Run multiple searches to maximize coverage:

### Strategy A: Direct Topic Search
```
hn_search(query: "<topic>", content_type: "story", sort_by: "relevance", min_points: 5, limit: 20)
```

### Strategy B: Recent Discussions
```
hn_search(query: "<topic>", content_type: "story", sort_by: "date", limit: 15)
```

### Strategy C: High-Signal Discussions
```
hn_search(query: "<topic>", content_type: "story", sort_by: "relevance", min_points: 50, limit: 10)
```

### Strategy D: Comment-Level Signal
```
hn_search(query: "<topic>", content_type: "comment", sort_by: "relevance", min_points: 3, limit: 15)
```
Comments often contain the most actionable practitioner insights. When a comment is highly relevant, note its `story_id` to retrieve the full parent thread.

### Deduplication and Ranking

After gathering results from all strategies:
1. Deduplicate by story ID.
2. Rank by a combination of: relevance to query, points (community endorsement), comment count (discussion depth), recency.
3. Select the **top 3-7 threads** for deep analysis. Prefer threads with 10+ comments over high-point threads with few comments, since discussion depth matters more than upvotes for extracting practitioner signal.

## Phase 2: Thread Retrieval and Analysis

For each selected thread, call `hn_get_thread` to retrieve the full discussion:

```
hn_get_thread(item_id: "<story_id>", max_depth: 8, max_comments: 200)
```

### What to Extract from Each Thread

Analyze the thread and extract these dimensions:

1. **Core Topic**: What is being discussed? (product launch, blog post, paper, Show HN, Ask HN)

2. **Community Sentiment**: Overall reception.
   - Positive signals: upvotes, enthusiastic comments, "this is great because..."
   - Negative signals: criticism, skepticism, "the problem with this is..."
   - Characterize as: overwhelmingly positive, mostly positive, mixed, mostly negative, controversial (strong opinions both ways)

3. **Key Arguments**: The substantive technical points made.
   - **For**: What practitioners like, what problems it solves, reported successes
   - **Against**: Criticisms, limitations identified, failure reports, concerns
   - **Nuance**: Conditional opinions ("great for X, terrible for Y"), caveats, edge cases

4. **Practitioner Experience**: Real-world deployment reports.
   - "We used X at our company and..." — these are gold. Capture specifics: scale, context, outcome.
   - "I tried X and found..." — individual experience reports.
   - Comparisons to alternatives: "We switched from X to Y because..."

5. **Resources Surfaced**: Links and references shared in comments.
   - Blog posts, tutorials, documentation
   - Alternative tools/libraries mentioned
   - Related papers or research
   - GitHub repos

6. **Notable Voices**: Comments from recognized experts, library authors, or people with deep domain experience. Note their perspective and why it carries weight.

7. **Unresolved Questions**: Open questions the community raised but didn't answer — these signal gaps in the field.

## Phase 3: KB Card Storage

If `kb_write_card` is available, store the synthesis as a KB card.

### Card ID Convention

Use kebab-case IDs prefixed with `hn-`:
- Single topic: `hn-{topic-slug}` (e.g., `hn-openclaw`, `hn-rust-async-runtime`)
- Specific discussion: `hn-discuss-{slug}` (e.g., `hn-discuss-llamacpp-launch`)
- Comparison/survey: `hn-survey-{slug}` (e.g., `hn-survey-vector-databases-2025`)

### Card Frontmatter

```yaml
title: "HN: <descriptive title>"
source_type: hackernews
refs:
  - "hn:<primary_story_id>"
  - "hn:<additional_story_id>"
tags:
  - <primary domain, e.g. "mlops", "robotics", "llm">
  - <specific topic, e.g. "vector-db", "fine-tuning", "deployment">
  - "community-signal"
contributed_by: hn-synthesis
capsule: "<one-line summary of the community signal>"
```

### Card Body Structure

```markdown
## Overview

<2-3 paragraph executive summary: what was discussed, overall community reception,
and the key takeaway for someone evaluating this technology/approach.>

## Threads Analyzed

| Thread | Points | Comments | Date | Link |
|--------|--------|----------|------|------|
| <title> | <points> | <num_comments> | <date> | <hn_url> |

## Community Sentiment

**Overall**: <overwhelmingly positive / mostly positive / mixed / mostly negative / controversial>

<1-2 paragraphs characterizing the sentiment distribution. What do supporters emphasize?
What do critics emphasize? Is there a clear majority view or genuine split?>

## Key Arguments

### In Favor
<Numbered list of substantive pro arguments with attribution when notable.
Each item should be 2-3 sentences explaining the argument, not a bare bullet.>

### Against / Concerns
<Same format for criticisms, skepticism, and limitations identified by the community.>

### Nuanced Takes
<Conditional opinions, trade-off analyses, "it depends on..." perspectives.
These are often the most valuable signals.>

## Practitioner Reports

<Paragraph-form summaries of real-world deployment or usage reports.
For each, note: who (role/context if stated), what they tried, scale/context,
outcome, and any specific numbers or metrics shared.
If no practitioner reports exist in the threads, note "No first-hand deployment
reports found in these threads." and skip this section.>

## Resources Surfaced

<Bulleted list of links, repos, blog posts, papers, and alternative tools
mentioned in the discussions. For each, note what it is and why it was mentioned.>

## Alternative Approaches Mentioned

<What alternatives did commenters suggest? For each alternative, note:
what it is, who recommended it, and the claimed advantage over the subject.
This section captures the "have you tried X instead?" signal.>

## Open Questions

<Questions raised but not resolved in the discussions. These indicate
gaps in community knowledge or genuine unsettled debates.>

## Connections

<List 3-5 related KB cards or topics with one-line descriptions of the relationship.
If this topic relates to papers already in the KB, cross-reference them.>
```

## Phase 4: Cross-Thread Synthesis (Multiple Threads)

When 3+ threads were analyzed on the same topic, add a synthesis section to the card body before "Connections":

```markdown
## Signal Evolution

<How has community opinion evolved over time? Compare sentiment across threads
from different dates. Note shifts in: adoption rate, common complaints,
emerging consensus, or new alternatives entering the conversation.>

## Consensus Map

<What the community broadly agrees on (high confidence signal):>
- ...

<What remains contested (low confidence / actively debated):>
- ...

<What the community doesn't know yet (acknowledged gaps):>
- ...
```

## Phase 5: Longer Analysis Report (Optional)

For particularly rich topics (5+ threads, complex sentiment landscape), additionally save a longer report:

```
kb_write_file(
  path: "community-analysis/<topic-slug>.md",
  content: <full analysis with per-thread breakdowns>
)
```

This complements the card (which is a structured summary) with the full analytical narrative.

## Presentation

When the synthesis is complete, call `present_document` to present it in sectioned reading mode. Set `document_id` to a unique slug, `title` to the synthesis title, and `content` to the full markdown analysis. End your response after calling this tool and wait for user interaction.

If the user asks follow-up questions about a specific section, enhance that section and call `update_document_section` with the section index and refined content.

## Graceful Degradation

- **No KB tools configured**: Output the full synthesis in chat; skip card storage.
- **No `hn_search` available**: Tell the user that Hacker News search requires the `research-hackernews` feature flag and suggest they add it to their build.
- **No threads found**: Report "No Hacker News discussions found for this topic" and suggest alternative search terms or broader queries.
- **Threads have few comments**: Adjust expectations in the synthesis — note that community signal is thin and findings should be treated as preliminary.
- **User provides a specific HN URL**: Extract the item ID and go directly to Phase 2 (skip discovery).

## Integration with Paper Synthesis

When used alongside `paper-synthesis`, cross-reference the findings:

- If a paper card exists in the KB for the topic, mention it in "Connections" and note how community reception aligns with or diverges from the paper's claims.
- If community discussions surface papers not yet in the KB, note them in "Resources Surfaced" as candidates for `paper-synthesis`.
- The `paper-discovery` skill can use HN synthesis cards to identify which research directions have real practitioner traction.
