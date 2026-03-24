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

- User asks "what do people think about X?" or about real-world experience with a tool/library/technique
- User wants community sentiment or practitioner signal on a topic
- User asks to search Hacker News for something
- As a complement to `paper-synthesis` for practitioner perspective on research
- When investigating whether a technology has real-world traction beyond papers

## Execution: All Work via Subagents

Two subagent types: **`$hn-discoverer`** (searches HN, deduplicates, ranks, returns top thread IDs) and **`$hn-synthesizer`** (retrieves and analyzes one HN thread, returns extracted signal).

### Main Agent Flow

1. **Spawn `$hn-discoverer`** immediately (the subagent skill is `hn-discoverer`, NOT `hn-synthesis`):
   ```
   spawn_agent(agent_type="discoverer", message="$hn-discoverer\n\nTopic: [what the user is investigating]\n[If date range specified: Date range: YYYY-MM-DD to YYYY-MM-DD]\n[If specific keywords requested: Keywords: [list]]")
   ```

2. **Wait** for thread IDs.
3. **Spawn `$hn-synthesizer` subagents in batches of 8** (the subagent skill is `hn-synthesizer`, NOT `hn-synthesis`). 20-thread system limit — spawning >10 at once causes silent failures. Wait once per batch, then spawn next. Cap at top 25 threads ranked by points x relevance.
   ```
   spawn_agent(agent_type="synthesizer", message="$hn-synthesizer\n\nThread ID: [HN item ID]\nHN URL: [discussion URL]\nArticle URL: [linked URL, if any]\nTopic context: [brief context from discovery results]")
   ```

4. **Wait ONCE per batch** — pass all subagent IDs in a single `wait` call. Do NOT call `wait` per subagent or poll in a loop.
5. **Read all staging files** via `exec_command` (e.g., `cat ${CODEX_KB_PATH}/staging/hn-*.md`).
6. **Present** a unified summary immediately (see Presentation). Do NOT write to KB before presenting.
7. If 3+ threads were analyzed, include cross-thread synthesis sections (see Cross-Thread Synthesis).
8. **Spawn a `$kb` subagent (skip if KB is disabled)** — fire-and-forget, do NOT call `wait`:

> $kb
>
> Process staged HN synthesis. Complete ALL steps: read staging files, write combined KB card, update journal+context, delete staging files.
> Card ID: [e.g. hn-ml-ai-agents-2026-02-20]
> Tags: [relevant tags]
> Staging files: [list all ${CODEX_KB_PATH}/staging/hn-<thread_id>.md files]
> User signals: [1-2 sentences about what the user asked for and interests revealed]

**If KB is disabled:** Skip step 8. Delete staging files with `exec_command: for f in ${CODEX_KB_PATH}/staging/hn-*.md; do unlink "$f"; done`. **Use `unlink`, not `rm -f`** — the sandbox blocks `rm` but allows `unlink`.

### Pre-Synthesis Check (Optional)

**Skip when the user explicitly asks to search.** Only check KB when the request is ambiguous and KB is enabled: search cards for `source_type: hackernews`. If a matching recent card exists, return it and ask if the user wants a fresh search. If KB is disabled, skip entirely.

### User provides a specific HN URL

Extract the item ID and skip discovery — spawn a single `$hn-synthesizer` subagent directly.

## KB Card Persistence

**Skip if KB is disabled.** KB writes happen in the background via fire-and-forget `$kb` subagent, AFTER presenting. Each hn-synthesizer writes its analysis to `${CODEX_KB_PATH}/staging/hn-<thread_id>.md` (with YAML frontmatter). The main agent reads staging files for presentation, then spawns `$kb` with card ID, tags, and staging file paths.

### Card ID Convention

Kebab-case, prefixed with `hn-`: `hn-{topic-slug}`, `hn-discuss-{slug}`, or `hn-survey-{slug}`.

## Cross-Thread Synthesis (3+ Threads)

When 3+ threads were analyzed, add before "Connections":

```markdown
## Signal Evolution
<How has community opinion evolved across threads from different dates?>

## Consensus Map
<Broadly agreed (high confidence):>
- ...
<Contested (actively debated):>
- ...
<Acknowledged gaps:>
- ...
```

## Presentation

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id`, `title`, and `content` containing ONLY `## ` section headings with empty bodies. This opens the reading view with "Generating..." placeholders.

**Phase 2 (Fill):** Call `update_document_section(document_id, section_index=0, content="...")` with full section content. Each tool result tells you the next section. Continue until all filled.

**Markdown:** Always put a blank line before list items (`1.`, `-`, `*`). Without it, items after the first lose formatting.

**Follow-ups:** Use `append_to_section` with `foldable=true` for expansion requests (3-5 sentences per foldable block). Use `patch_document_section` for corrections. Use `update_document_section` only for full rewrites. Keep sections ≤30 visible (non-folded) lines. No editorial labels like "(expanded)" in headings.

## Graceful Degradation

- **No KB**: Output full synthesis in chat; skip card storage.
- **No threads found**: Report and suggest alternative search terms.
- **Few comments**: Note that signal is thin and findings are preliminary.

## Integration with Paper Synthesis

When used alongside `paper-synthesis`, cross-reference: mention existing paper cards in "Connections," note community-surfaced papers in "Resources Surfaced" as `paper-synthesis` candidates, and use HN cards to identify research directions with practitioner traction.
