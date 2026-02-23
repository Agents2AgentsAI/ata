---
name: hn-synthesis
description: Search Hacker News for practitioner discussions on a topic, tool, library, or paper, and synthesize findings into structured KB cards. Use when a user asks about community sentiment, real-world experience, practitioner opinions, or HN discussions about a technology, project, or research paper.
metadata:
  short-description: Synthesize Hacker News discussions into KB knowledge cards
---

# HN Synthesis

Search Hacker News for practitioner discussions and synthesize them into structured knowledge base cards. This captures real-world signal that academic papers miss: deployment war stories, performance gotchas, community sentiment, alternative approaches practitioners actually use, and links to tools/repos that emerge in discussion threads.

## Rules

1. **Always use subagents** — discovery and thread analysis are both done by subagents. The main agent does not call `hn_search` or `hn_get_thread` directly.
2. **Start immediately.** When the user asks to search HN, spawn the discovery subagent right away.
3. **No KB references in prose.** Never say "as found in your KB." Present findings as direct analysis.

## When to Use This Skill

- User asks "what do people think about X?"
- User asks about real-world experience with a tool, library, or technique
- User wants community sentiment or practitioner signal on a topic
- User asks to search Hacker News for something
- As a complement to `paper-synthesis` when you want to understand how practitioners view a research contribution
- When investigating whether a technology has real-world traction beyond papers

## Execution: All Work via Subagents

The main agent orchestrates but does not call `hn_search` or `hn_get_thread` directly. Two subagent types handle the work:

- **`$hn-discoverer`** — searches HN, deduplicates, ranks, returns top thread IDs
- **`$hn-synthesizer`** — retrieves and analyzes one HN thread, returns extracted signal

### Main Agent Flow

1. **Spawn discovery subagent** immediately with the user's topic and any date constraints:

> $hn-discoverer
>
> Topic: [what the user is investigating]
> [If date range specified: Date range: YYYY-MM-DD to YYYY-MM-DD]
> [If specific keywords requested: Keywords: [list]]

2. **Wait** for the discovery subagent to return thread IDs.
3. **Spawn one `$hn-synthesizer` subagent per thread** (in parallel), passing the URLs from discovery:

> $hn-synthesizer
>
> Thread ID: [HN item ID]
> HN URL: [discussion URL]
> Article URL: [linked URL, if any]
> Topic context: [brief context from discovery results]

4. **Wait** for all analysis subagents to complete — each returns a staging file path.
5. **Read all staging files** via `exec_command` (e.g., `cat ~/.ata/knowledge-base/staging/hn-*.md`).
6. **Present** a unified summary to the user immediately (see Presentation). Do NOT write to KB before presenting.
7. If multiple threads were analyzed, include the cross-thread synthesis sections (see Phase 4) in the reading view.
8. **Spawn a KB subagent** (fire-and-forget, do NOT call `wait`) to persist the card in the background:

> $kb
>
> Process staged HN synthesis. You MUST complete ALL 4 steps below — do not stop after writing the card.
> Card ID: [e.g. hn-ml-ai-agents-2026-02-20]
> Tags: [relevant tags]
> Staging files: [list all ~/.ata/knowledge-base/staging/hn-<thread_id>.md files]
> User signals: [1-2 sentences about what the user asked for and any interests/preferences revealed, e.g. "User asked about community sentiment on AI agents. Interested in practical deployment experiences and tool recommendations."]
>
> Step 1. Read all staging files. Combine the thread analyses into a single KB card using the HN card body structure (Overview, Threads Analyzed table, Community Sentiment, Key Arguments, Practitioner Reports, Resources Surfaced, Alternative Approaches, Open Questions, Connections). Add frontmatter with source_type: hackernews, refs, tags, capsule. Write to ~/.ata/knowledge-base/cards/. Update index.json.
> Step 2. Append to research-journal.md (prepend newest first): "## [date] — HN Synthesis: [topic]\n- Card: `[card-id]` | Threads: [count]"
> Step 3. Update research-context.md with any new interests or preferences from the user signals. Read the file first (create if missing with sections: Project, Priorities, Not Interested In, Key Decisions Made). Merge — don't overwrite existing content.
> Step 4. Delete staging files.
>
> Confirm completion of each step before moving to the next.

### Pre-Synthesis Check (Optional)

**Skip when the user explicitly asks to search** ("search hackernews for...", "find HN posts about..."). Go straight to spawning the discovery subagent.

Only check KB when the request is ambiguous ("what do people think about X?"):
1. Search cards per `$kb` with the topic query. Look for `source_type: hackernews`.
2. If a matching recent card exists, return it and ask if the user wants a fresh search.

### User provides a specific HN URL

Extract the item ID and skip discovery — spawn a single `$hn-synthesizer` subagent directly.

## Phase 3: KB Card Persistence

KB writes happen in the **background** via a fire-and-forget `$kb` subagent, AFTER the reading view is presented. The main agent never writes KB cards directly.

**How it works:** Each hn-synthesizer subagent writes its thread analysis to `~/.ata/knowledge-base/staging/hn-<thread_id>.md` (with YAML frontmatter containing thread metadata) and returns only the file path. The main agent reads all staging files for presentation. After the reading view is presented, it spawns a `$kb` subagent with the card ID, tags, and list of staging file paths. The KB subagent reads from disk, combines the analyses, and handles all KB operations.

### Card ID Convention

Use kebab-case IDs prefixed with `hn-`:
- Single topic: `hn-{topic-slug}` (e.g., `hn-openclaw`, `hn-rust-async-runtime`)
- Specific discussion: `hn-discuss-{slug}` (e.g., `hn-discuss-llamacpp-launch`)
- Comparison/survey: `hn-survey-{slug}` (e.g., `hn-survey-vector-databases-2025`)

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

## Presentation

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the synthesis title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## Overview\n\n## Community Sentiment\n\n## Key Arguments\n\n## Practitioner Reports"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

Write follow-up answers as straight content — no editorial labels like "(clearer explanation)" or "(expanded)" in headings or topic lines.

## Graceful Degradation

- **No KB configured**: Output the full synthesis in chat; skip card storage.
- **Discovery subagent returns no threads**: Report "No Hacker News discussions found for this topic" and suggest alternative search terms or broader queries.
- **Threads have few comments**: Adjust expectations in the synthesis — note that community signal is thin and findings should be treated as preliminary.

## Integration with Paper Synthesis

When used alongside `paper-synthesis`, cross-reference the findings:

- If a paper card exists in the KB for the topic, mention it in "Connections" and note how community reception aligns with or diverges from the paper's claims.
- If community discussions surface papers not yet in the KB, note them in "Resources Surfaced" as candidates for `paper-synthesis`.
- The `paper-discovery` skill can use HN synthesis cards to identify which research directions have real practitioner traction.
