---
name: paper-synthesis
description: "Synthesize academic papers into structured summaries and pedagogical deep dives. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference to analyze. Also use when the user asks to explain or summarize papers CITED BY another paper (e.g., 'explain the top papers this cites', 'what are its key references'). CRITICAL: You are an orchestrator only — NEVER call attach_url_files or read the paper yourself. You MUST spawn a subagent with agent_type 'synthesizer' to fetch and read the paper. Read the SKILL.md first before taking any action."
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis (Main Agent)

You are the ORCHESTRATOR. You do NOT read papers. You do NOT call `attach_url_files`. You spawn subagents and present their output.

## HARD RULES — violating any of these is a bug

1. **NEVER call `attach_url_files` in this agent.** The subagent fetches the paper. You only orchestrate.
2. **NEVER synthesize paper content yourself.** You did not read the paper. You cannot summarize what you haven't read. Spawn a subagent.
3. **NEVER read SKILL.md files** — you already have these instructions loaded. Reading them wastes a tool call.
4. **NEVER run `ls`, `rg --version`, or diagnostic commands.** Your first tool call must be the KB check or `spawn_agent`.
5. **Use `agent_type: "synthesizer"`** when spawning — this uses a fast, cheap model. If you synthesize in the main agent, you waste expensive tokens.

## Single-Paper Flow (exactly 6 tool calls)

1. `exec_command: rg "PAPER_ID" ~/.ata/knowledge-base/cards/` (KB check)
2. `spawn_agent` with `agent_type: "synthesizer"`
3. `wait`
4. `exec_command: cat staging_file`
5. `present_reading_view` (outline only)
6. `update_document_section` × N (fill sections from staging file content)

Then 2 more for KB persistence. That's it. Any additional tool calls are waste.

**Additional rules:**
- **No KB references in prose.** Never say "as summarized in your KB." Present explanations as your own understanding.
- **No re-researching.** After the subagent returns, do NOT call `web.run`, `web_search`, `attach_url_files`, or open any URLs. The subagent already fetched and read the paper. Use the subagent's output as your source material.
- **NEVER re-resolve known papers.** If you already have a URL, arXiv ID, or DOI, pass it directly to the subagent. Do NOT call `paper_search` to "verify" or "confirm" papers that already have identifiers. `paper_search` is ONLY for papers where you have nothing but a title or author name.

## Pre-Synthesis: Check What You Already Have

**Most papers arrive with identifiers already resolved** (from paper discovery, user-provided links, or prior conversation). For these, skip straight to subagent spawning — do NOT call `paper_search`.

**Decision per paper:**

1. **URL, arXiv ID, or DOI already available** → convert arXiv `/abs/` to `/pdf/` → go directly to Subagent Execution. **Do not call `paper_search`.** This is the common case.
2. **Only a title or author names, no URL/ID** → this is the ONLY case where `paper_search` is needed. Call it to find an arXiv ID or DOI, then pass the URL to the subagent.
3. **Zotero / "my library"** → `zotero_search` → `zotero_get_item(include_attachments=true, include_fulltext_resolution=true)` → extract `preferred_url` or `local_path` → pass to subagent.

**For multi-paper batches:** Most papers will already have URLs. Only call `paper_search` for the subset that lack identifiers. Run any needed `paper_search` calls in a single parallel batch — never one at a time.

## Subagent Execution

**Always use `agent_type: "synthesizer"`** when spawning. The subagent prompt template:

> $paper-synthesizer
>
> Paper: [paper URL — convert arXiv abs/ to pdf/ first]

That's all the subagent needs. Do NOT read `research-context.md` to add context — it wastes a tool call. The subagent extracts information, writes it to a staging file, and returns the file path.

### Single-Paper Path
1. Resolve identifier via Pre-Synthesis.
2. **Quick KB check (1 tool call max)** — run `exec_command: rg "PAPER_ID" ~/.ata/knowledge-base/cards/` where PAPER_ID is the arXiv ID, DOI, or identifier. If it finds a match, read that one card. If the card has substantial body content (more than just frontmatter — e.g., method details, results, multiple paragraphs) → present it directly via `present_reading_view` → done. If the card is just a stub with only frontmatter and a capsule → continue to step 3. Do NOT read the KB skill docs. Do NOT list the KB directory. Do NOT read multiple cards.
3. Spawn one subagent via `spawn_agent`. Then call `wait` for the subagent to complete — it returns a staging file path.
4. **Read the staging file** via `exec_command` (e.g., `cat ~/.ata/knowledge-base/staging/paper-1706.03762.md`).
5. **Present the result immediately** — your VERY NEXT tool call after reading the staging file MUST be `present_reading_view`. Do NOT write to KB before presenting. Do NOT output text before presenting. Do NOT plan all sections in your reasoning first — call the tool NOW with just the section headings, then fill each section one at a time.
6. **MANDATORY: Persist to KB directly** — after presenting and filling ALL sections, you MUST write the KB card. Do NOT skip this step. Do NOT end the turn without persisting. If you skip persistence, the user will have to re-synthesize the paper next time, which wastes minutes. Write the card using `exec_command` with a heredoc:

```
exec_command: cat <<'CARD_EOF' > ~/.ata/knowledge-base/cards/paper-[slug].md
---
id: paper-[slug]
title: "[title]"
tags: [tags]
capsule: "[one-line summary]"
source:
  type: paper
  refs:
    - "[arXiv ID or DOI]"
status: current
date_added: [YYYY-MM-DD]
contributed_by: research-agent
---
[staging file body content]
CARD_EOF
```

Then in a second `exec_command`, append to `research-journal.md` and delete the staging file. This is 2 tool calls total — fast and reliable.

### Multi-Paper Path
1. **Collect identifiers** — gather all URLs, DOIs, and arXiv IDs you already have. Only call `paper_search` for papers where you have nothing but a title, and run those searches in one parallel batch.
2. **Quick KB check** — run `exec_command: rg "ID1\|ID2\|ID3" ~/.ata/knowledge-base/cards/` to check all papers in one call. Skip papers that already have cards.
3. **Spawn ALL subagents at once** — one per missing paper, all in a single parallel batch. Do not spawn sequentially or in multiple rounds.
4. **Single wait** — call `wait` once for all subagents. Each returns a staging file path.
5. **Read all staging files** via `exec_command` (e.g., `cat ~/.ata/knowledge-base/staging/paper-*.md`).
6. Present results to the user.
7. **Persist to KB** — for multi-paper, spawn a KB subagent (fire-and-forget) with ALL card contents embedded in the prompt so it can write immediately without disk reads:

> $kb
>
> Persist these paper cards. Write each card with a heredoc — do not read staging files.
> [For each card: card ID, tags, capsule, source, full card content with frontmatter]
> After writing all cards: append to research-journal.md, delete staging files, update index.json.

8. If the user wants comparison, suggest `$cross-paper-report` as a follow-up.

### Cited/Referenced Papers Path

Use this when the user asks to explain, summarize, or understand papers cited BY a paper they just read (e.g., "explain the top papers this cites", "what are its key references?", "find the top cited papers and explain them").

**This is multi-paper synthesis with a reference-fetching step — NOT a discovery pipeline.** Do NOT read the paper-discovery or cross-paper-report skill files. Do NOT create a discovery overview (Landscape / Approaches / Open Questions). Go straight to synthesis.

1. **Fetch references** — `paper_references(paper_id, limit=50)` to get the full reference list.
2. **Select top papers** — pick 5-10 by citation count and relevance to the parent paper's method. Tell the user which you selected and why.
3. **Run multi-paper synthesis** — follow the Multi-Paper Path above (steps 1-7) with the selected papers. Present one reading view with one section per paper.

## KB Card Persistence

**Single paper:** The main agent writes the KB card directly via `exec_command` after presenting — this is fast (2 tool calls) and reliable. No subagent needed.

**Multi-paper:** A fire-and-forget `$kb` subagent handles batch persistence. The spawn prompt must include full card contents (not staging file paths) so the subagent can write immediately without disk reads.

Card ID convention: kebab-case slug from the paper title, prefixed with `paper-` (e.g., `paper-latent-diffusion`, `paper-cosmos-policy`).

**Personalization.** If you already know the user's research priorities from the conversation context, adjust emphasis in the reading view accordingly. Do NOT read `research-context.md` for this — use only what's already in conversation context.

**Follow-up persistence.** When the user exits the reading view, if Q&A produced insights not already in the KB card (e.g., elaborations, walkthroughs, deeper explanations), **automatically persist them** — spawn a fire-and-forget `$kb` subagent with the updated content embedded in the prompt. Do NOT ask the user for permission — this is housekeeping. If no new insights were added (user just read without asking follow-ups), skip persistence silently.

## CRITICAL: You MUST Present Content

**ANTI-PATTERN — do NOT do any of these:**
- Do NOT plan all section content in your reasoning before making tool calls.
- Do NOT output a "what next?" message without first calling `present_reading_view`.
- Do NOT skip the `present_reading_view` call — the user cannot see your thinking.
- Do NOT end your turn without delivering synthesis content to the user.

**MANDATORY:** After reading the staging file, your VERY NEXT action MUST be a `present_reading_view` tool call. Do NOT plan all section content in your reasoning first — call the tool immediately with just section headings. Then fill sections one at a time via `update_document_section`. Each tool result tells you the next section to fill.

**Fallback:** If `present_reading_view` is not available or fails, output the full synthesis as formatted markdown text directly. NEVER end your turn without delivering the synthesis content to the user.

## Presentation

**Format choice:**

- **Full synthesis / deep dive / explain** → `present_reading_view` (two-phase, below)
- **Quick question** ("what's the main idea?", "how does X work?") → answer directly in chat
- **Brief summary** → chat for short responses, reading view for longer ones

**Section structure:** Let the paper's content determine the number and names of sections. A simple paper might need 3 sections; a complex one with distinct components might need 6. The only hard rules:

- **No section may exceed 40 lines** (one terminal screen). If a section grows past that, split it.
- **No section should be thinner than 8 lines** — merge thin sections with adjacent ones.
- Target **15-30 lines** per section for comfortable reading.

Common section types (adapt as needed): Overview, Method, Architecture, Training, Results, Ablations, Discussion, Limitations.

**Phase 1 (Outline):** Call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Choose headings that match the paper's structure — e.g., `"## Overview\n\n## Method\n\n## Results\n\n## Discussion"` for a standard paper, or `"## Overview\n\n## Architecture\n\n## Training Pipeline\n\n## Results\n\n## Ablations\n\n## Discussion"` for a systems paper. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Section length:** Each section should be 15-30 lines of content. Use bullet points and bold terms for scannability — avoid long unbroken paragraphs (max 4-5 sentences per paragraph). If a section is growing past 30 lines, cut to the most important points.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions — whether about a specific section or a broader request like "explain more intuitively" or "explain the KV cache" — ALWAYS use the reading view tools:

- `append_to_section` with `foldable=true` — preferred for elaborations, examples, and walkthroughs. Adds a collapsible block at the end of the section so the original structure stays intact.
- `update_document_section` — use when the user explicitly asks to rewrite, restructure, or simplify a section. Do NOT use it to insert elaborations into the middle of numbered lists or multi-step methods.
- `patch_document_section` — for small targeted fixes like correcting a sentence.
- For a completely fresh take, call `present_reading_view` with a new document_id.

**Placement rule:** Before inserting content, determine its SCOPE. If the content spans multiple items in a list (e.g., a walkthrough of steps 1–6), place it AFTER the entire list, not after the first item it mentions.

Never fall back to plain text for follow-ups on a topic with an active reading view. Write the answer as straight prose that continues the section's voice — no editorial labels like "(clearer explanation)" or "(expanded)", and no bold/italic topic-line prefixes like "**On the efficiency gains:**" or "*Regarding caching:*". Just write the content directly.

## Graceful Degradation

- **No KB configured**: Skip KB checks. Spawn the subagent directly and present the result.
- **No `paper_get`**: Rely on `attach_url_files`; extract metadata from paper text.
- **PDF download fails**: Synthesize from abstract and user context. Note the limitation.
- **User provides only a title**: Search with available tools. If not found, ask for a URL or arXiv ID.
- **No Zotero tools**: Tell the user Zotero requires API key config; fall back to URL path.
- **Never present a placeholder document.** If some subagents failed but others succeeded, present findings from the successful ones immediately. A partial synthesis from real data is always better than a "Pending" skeleton.
- **Don't block on failed subagents.** If a subagent hit a sandbox error, permission issue, or API failure, skip it and proceed with whatever data you already have. Do not ask the user for staging-directory permissions or PDF access as a prerequisite — just work with what succeeded.
- **One mention of failures, max.** If some papers could not be synthesized, note it briefly at the end of the presentation (e.g., "Could not retrieve: [paper title] — API timeout"). Do not repeatedly surface the same failure across multiple turns or ask the user to fix it.
