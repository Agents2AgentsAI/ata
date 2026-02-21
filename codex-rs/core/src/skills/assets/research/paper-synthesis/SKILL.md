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
> [For Zotero papers: item key, PDF path, and any notes already retrieved]
> [If research-context.md exists: User priorities: … Emphasize: … User project: …]

The subagent extracts information, writes it to a staging file, and returns the file path. All KB operations (card creation, journal writes) are done by a background KB subagent.

### Single-Paper Path
1. Resolve identifier via Pre-Synthesis.
2. **Quick KB check** — search for an existing card with this paper's ID per `$kb`. If a card with a Deep Dive exists → read it → `present_reading_view` → done. Do not explore the KB beyond this one search. If no match, continue.
3. Spawn one subagent via `spawn_agent`. Then call `wait` for the subagent to complete — it returns a staging file path.
4. **Read the staging file** via `exec_command` (e.g., `cat ~/.ata/staging/paper-1706.03762.md`).
5. **Present the result immediately** (see Presentation below). Do NOT write to KB before presenting.
6. **Spawn a KB subagent** (fire-and-forget, do NOT call `wait`) to persist the card in the background:

> $kb
>
> Process staged paper card.
> Card ID: [kebab-case slug, e.g. paper-attention-is-all-you-need]
> Tags: [relevant tags from the paper content]
> Staging file: ~/.ata/staging/paper-[identifier].md
> User signals: [1-2 sentences about what the user asked for and any interests/preferences revealed, e.g. "User asked for a detailed explanation of the Transformer with tables. Interested in attention mechanisms and NLP architectures."]
>
> 1. Read the staging file. Create a KB card in ~/.ata/knowledge-base/cards/ with proper frontmatter (id, title, tags, capsule, source, status: current, date_added, contributed_by: research-agent). Update index.json with any new tags.
> 2. Append to research-journal.md (prepend newest first): "## [date] — Synthesized: [title]\n- Card: `[card-id]` | Source: [URL]"
> 3. Update research-context.md with any new interests or preferences from the user signals. Read the file first (create if missing with sections: Project, Priorities, Not Interested In, Key Decisions Made). Merge — don't overwrite existing content.
> 4. Delete the staging file.

### Multi-Paper Path
1. Resolve identifiers for all papers.
2. **Quick KB check** — search for existing cards per `$kb` for each paper. Skip papers that already have cards.
3. Spawn one subagent per missing paper, in parallel.
4. Wait for all subagents — each returns a staging file path.
5. **Read all staging files** via `exec_command` (e.g., `cat ~/.ata/staging/paper-*.md`).
6. Present results to the user.
7. **Spawn one KB subagent** (fire-and-forget) listing all staging files:

> $kb
>
> Process staged paper cards.
> Cards: [list of card IDs and their staging file paths]
> Tags: [tags per card]
> User signals: [interests/preferences revealed by the multi-paper request]
>
> For each staging file: read it, create a KB card with proper frontmatter, update index.json. Append each to research-journal.md (prepend newest first). Update research-context.md with new interests from user signals (read first, merge, don't overwrite). Delete staging files when done.

6. If the user wants comparison, suggest `$cross-paper-report` as a follow-up.

## KB Card Persistence

KB writes happen in the **background** via a fire-and-forget `$kb` subagent, AFTER the reading view is presented. The main agent never writes KB cards directly.

**How it works:** The paper-synthesizer subagent writes its analysis to `~/.ata/staging/paper-<identifier>.md` (with YAML frontmatter containing metadata) and returns only the file path. The main agent reads the file for presentation. After the reading view is presented, it spawns a `$kb` subagent with just the card ID, tags, and staging file path. The KB subagent reads from disk, formats the card, and handles all KB operations.

Card ID convention: kebab-case slug from the paper title, prefixed with `paper-` (e.g., `paper-latent-diffusion`, `paper-cosmos-policy`).

**Personalization.** If `research-context.md` exists, use it to adjust emphasis in the reading view. The KB subagent automatically updates research-context.md and research-journal.md in the background — include a "User signals" line in the spawn prompt describing what the user asked for and any preferences revealed.

**Follow-up persistence.** When the user exits the reading view, check if Q&A produced insights not in the KB card. If so, offer to persist using the update protocol in `$kb`.

## Presentation

The subagent returns raw extracted information from the paper. You decide how to present it based on what the user asked and the nature of the content.

**Choose the format:**
- **Full synthesis / deep dive / explain** → use `present_reading_view` (two-phase, below)
- **Quick question** ("what's the main idea?", "how does X work?") → answer directly in chat
- **Brief summary** → chat for short responses, reading view for longer ones

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## Introduction\n\n## Core Method\n\n## Experiments\n\n## Discussion"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions — whether about a specific section or a broader request like "explain more intuitively" or "explain the KV cache" — ALWAYS use the reading view tools. Prefer `update_document_section` to rewrite the section with the answer woven in at the relevant location (keeps explanations inline where the concept appears). Use `patch_document_section` to insert content right after a specific passage. Use `append_to_section` only when adding genuinely new content that belongs at the end. For a completely fresh take, call `present_reading_view` with a new document_id. Never fall back to plain text for follow-ups on a topic with an active reading view. Write the answer as straight prose that continues the section's voice — no editorial labels like "(clearer explanation)" or "(expanded)", and no bold/italic topic-line prefixes like "**On the efficiency gains:**" or "*Regarding caching:*". Just write the content directly.

## Graceful Degradation

- **No KB configured**: Skip KB checks. Spawn the subagent directly and present the result.
- **No `paper_get`**: Rely on `attach_url_files`; extract metadata from paper text.
- **PDF download fails**: Synthesize from abstract and user context. Note the limitation.
- **User provides only a title**: Search with available tools. If not found, ask for a URL or arXiv ID.
- **No Zotero tools**: Tell the user Zotero requires API key config; fall back to URL path.
