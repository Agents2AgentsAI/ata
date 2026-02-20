---
name: paper-synthesis
description: Synthesize academic papers into structured summaries and pedagogical deep dives. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference to analyze.
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis (Main Agent)

This skill orchestrates paper synthesis. The actual synthesis work is done by subagents running the `$paper-synthesizer` skill.

## Rules

1. **Always use subagents** — one per paper, parallel for multi-paper. Never synthesize in the main agent context.
2. **Use `agent_type: "synthesizer"`** when spawning subagents for fast output.
3. **Subagent prompts must include `$paper-synthesizer`** to trigger the subagent skill. Do not write custom synthesis instructions.
4. **No KB references in prose.** Never say "as summarized in your KB." Present explanations as your own understanding.
5. **No re-researching.** After the subagent returns, do NOT call `web.run`, `web_search`, `attach_url_files`, or open any URLs. The subagent already fetched and read the paper. Use the subagent's output as your source material.
6. **No unnecessary exploration.** Do not call `ls`, `exec_command`, or read skill files from disk. The skill instructions are already loaded.

## Pre-Synthesis: Resolve the Paper

Choose **one** path based on user input:

- **URL, arXiv ID, or DOI** → convert arXiv `/abs/` to `/pdf/` → pass URL to subagent
- **Paper title or author names** → `paper_search` to find arXiv ID or DOI → pass URL to subagent
- **Zotero / "my library"** → `zotero_search` → `zotero_get_item(include_attachments=true, include_fulltext_resolution=true)` → extract `preferred_url` or `local_path` → pass to subagent

## Subagent Execution

**Always use `agent_type: "synthesizer"`** when spawning. The subagent prompt template:

> $paper-synthesizer
>
> Paper: [paper URL — convert arXiv abs/ to pdf/ first]
> [If main agent has KB path: KB path: [value]]
> [For Zotero papers: item key, PDF path, and any notes already retrieved]
> [If research-context.md exists: User priorities: … Emphasize: … User project: …]

### Single-Paper Path
1. Check KB status and search for existing cards per `$kb` (if KB path is available). Optionally read `<kb_path>/research-context.md`.
2. If a card with a Deep Dive exists → read it per `$kb` → `present_reading_view` → done.
3. Resolve identifier via Pre-Synthesis.
4. Spawn one subagent via `spawn_agent`. Then call `wait` for the subagent to complete.
5. Present the result (see Presentation below).

### Multi-Paper Path
1. Check KB status and search for existing cards per `$kb` for each paper in parallel. Optionally read `research-context.md`.
2. Skip papers that already have cards. Resolve identifiers for missing papers.
3. Spawn one subagent per missing paper, in parallel.
4. Collect card IDs. Tell the user which cards were written.
5. If the user wants comparison, suggest `$cross-paper-report` as a follow-up.

## Post-Synthesis

**Journal.** After card is written and reading view presented, append to `<kb_path>/research-journal.md`:

```markdown
## [Date] — Synthesized: [Paper Title]
- Card: `[card-id]` (created) | Source: [URL or "Zotero item KEY"]
```

**Personalization.** If `research-context.md` exists, use it to adjust emphasis. Watch for preference signals; offer to update when detected.

**Follow-up persistence.** When the user exits the reading view, check if Q&A produced insights not in the KB card. If so, offer to persist via `$kb-update`.

## Presentation

The subagent returns raw extracted information from the paper. You decide how to present it based on what the user asked and the nature of the content.

**Choose the format:**
- **Full synthesis / deep dive / explain** → use `present_reading_view` (two-phase, below)
- **Quick question** ("what's the main idea?", "how does X work?") → answer directly in chat
- **Brief summary** → chat for short responses, reading view for longer ones

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## Introduction\n\n## Core Method\n\n## Experiments\n\n## Discussion"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

For follow-up questions, use `append_to_section`, `patch_document_section`, or `update_document_section`.

## Graceful Degradation

- **No KB configured**: Skip KB checks. Spawn the subagent directly and present the result.
- **No `paper_get`**: Rely on `attach_url_files`; extract metadata from paper text.
- **PDF download fails**: Synthesize from abstract and user context. Note the limitation.
- **User provides only a title**: Search with available tools. If not found, ask for a URL or arXiv ID.
- **No Zotero tools**: Tell the user Zotero requires API key config; fall back to URL path.
