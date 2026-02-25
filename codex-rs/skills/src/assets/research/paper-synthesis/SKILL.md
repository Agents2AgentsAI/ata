---
name: paper-synthesis
description: Synthesize academic papers into structured summaries and pedagogical deep dives. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference to analyze.
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis (Main Agent)

This skill orchestrates paper synthesis. The actual synthesis work is done by subagents running the `$paper-synthesizer` skill.

## CRITICAL: No Exploration

**Do NOT do any of these before spawning the subagent:**
- Do NOT read any SKILL.md files (you already have the instructions)
- Do NOT run `rg --version`, `ls`, or any diagnostic commands
- Do NOT read `research-context.md`
- Do NOT read KB cards to "check" them — a single `rg` search is enough
- Do NOT call `paper_search` when you already have a URL or arXiv ID

**The optimal single-paper flow (with KB enabled) is exactly 6 tool calls:**
1. `exec_command: rg "PAPER_ID" ~/.ata/knowledge-base/cards/` (KB check — 1 call)
2. `spawn_agent` (1 call)
3. `wait` (1 call)
4. `exec_command: cat staging_file` (1 call)
5. `present_reading_view` (1 call)
6. `update_document_section` × N (fill sections)

Then 2 more for KB persistence. That's it. Any additional tool calls are waste.

**If KB is disabled** (no `$kb` skill available), skip step 1 (KB check) and skip KB persistence after presenting. The flow is: spawn subagent → wait → read staging → present → fill sections. That's it.

## Rules

1. **Always use subagents** — one per paper, parallel for multi-paper. Never synthesize in the main agent context.
2. **Use `agent_type: "synthesizer"`** when spawning subagents for fast output.
3. **Subagent prompts must include `$paper-synthesizer`** to trigger the subagent skill. Do not write custom synthesis instructions.
4. **No KB references in prose.** Never say "as summarized in your KB." Present explanations as your own understanding.
5. **No re-researching.** After the subagent returns, do NOT call `web.run`, `web_search`, `attach_url_files`, or open any URLs. The subagent already fetched and read the paper. Use the subagent's output as your source material.
6. **NEVER re-resolve known papers.** If you already have a URL, arXiv ID, or DOI for a paper (from paper discovery, user-provided links, or any prior step), pass it directly to the subagent. Do NOT call `paper_search` to "verify", "look up", or "confirm" papers that already have identifiers. This wastes time and API quota. `paper_search` is ONLY for papers where you have nothing but a title or author name.

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
2. **Quick KB check (1 tool call max, skip if KB is disabled)** — run `exec_command: rg "PAPER_ID" ~/.ata/knowledge-base/cards/` where PAPER_ID is the arXiv ID, DOI, or identifier. If it finds a match, read that one card and check for a Deep Dive section. If a Deep Dive exists → `present_reading_view` → done. If no match or no Deep Dive → continue to step 3. Do NOT read the KB skill docs. Do NOT list the KB directory. Do NOT read multiple cards. **If KB is disabled**, skip this step entirely and go straight to step 3.
3. Spawn one subagent via `spawn_agent`. Then call `wait` for the subagent to complete — it returns a staging file path.
4. **Read the staging file** via `exec_command` (e.g., `cat ~/.ata/staging/paper-1706.03762.md`).
5. **Present the result immediately** — your VERY NEXT tool call after reading the staging file MUST be `present_reading_view`. Do NOT write to KB before presenting. Do NOT output text before presenting. Do NOT plan all sections in your reasoning first — call the tool NOW with just the section headings, then fill each section one at a time.
6. **Persist to KB directly (skip if KB is disabled)** — after presenting, write the KB card yourself using `exec_command` with a heredoc. Do NOT spawn a KB subagent for single papers — fire-and-forget subagents are unreliable due to rate limits. Instead, do it in one call:

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

**If KB is disabled:** Skip step 6 entirely. Do not write cards or journal entries. After presenting, delete the staging file with `exec_command: rm ~/.ata/staging/paper-*.md` so it doesn't accumulate.

### Multi-Paper Path
1. **Collect identifiers** — gather all URLs, DOIs, and arXiv IDs you already have. Only call `paper_search` for papers where you have nothing but a title, and run those searches in one parallel batch.
2. **Quick KB check (skip if KB is disabled)** — run `exec_command: rg "ID1\|ID2\|ID3" ~/.ata/knowledge-base/cards/` to check all papers in one call. Skip papers that already have cards. **If KB is disabled**, skip this step and spawn subagents for all papers.
3. **Spawn ALL subagents at once** — one per missing paper, all in a single parallel batch. Do not spawn sequentially or in multiple rounds.
4. **Single wait** — call `wait` once for all subagents. Each returns a staging file path.
5. **Read all staging files** via `exec_command` (e.g., `cat ~/.ata/staging/paper-*.md`).
6. Present results to the user.
7. **Persist to KB (skip if KB is disabled)** — for multi-paper, spawn a KB subagent (fire-and-forget) with ALL card contents embedded in the prompt so it can write immediately without disk reads:

> $kb
>
> Persist these paper cards. Write each card with a heredoc — do not read staging files.
> [For each card: card ID, tags, capsule, source, full card content with frontmatter]
> After writing all cards: append to research-journal.md, delete staging files, update index.json.

8. If the user wants comparison, suggest `$cross-paper-report` as a follow-up.

## KB Card Persistence

**Skip this entire section if KB is disabled.** When KB is off, do not write cards, journal entries, or staging files. Present the synthesis and move on.

**Single paper:** The main agent writes the KB card directly via `exec_command` after presenting — this is fast (2 tool calls) and reliable. No subagent needed.

**Multi-paper:** A fire-and-forget `$kb` subagent handles batch persistence. The spawn prompt must include full card contents (not staging file paths) so the subagent can write immediately without disk reads.

Card ID convention: kebab-case slug from the paper title, prefixed with `paper-` (e.g., `paper-latent-diffusion`, `paper-cosmos-policy`).

**Personalization.** If you already know the user's research priorities from the conversation context, adjust emphasis in the reading view accordingly. Do NOT read `research-context.md` for this — use only what's already in conversation context.

**Follow-up persistence (skip if KB is disabled).** When the user exits the reading view and Q&A produced new insights not already in the KB card, automatically spawn a fire-and-forget `$kb` subagent (do NOT call `wait`) to persist them. Do not ask the user. Include the card ID and a summary of new insights from the Q&A in the subagent prompt:

> $kb
>
> Update KB card with follow-up insights. Do NOT ask the user — this is automatic.
> Card ID: [card-id]
> Card file: ~/.ata/knowledge-base/cards/[card-id].md
> New insights from Q&A:
> [Summarize each new insight: the question asked and the substantive answer, 2-4 sentences each]
>
> Read the card, append insights under `## Discussion Notes` per the update protocol, set `date_updated`, write the card back.

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

**Section structure:** Use **4-5 sections** for a standard paper synthesis. Each section should be **15-30 lines** (one screen in a terminal). Do not create more than 6 sections — thin sections (< 8 lines) should be merged with adjacent ones.

Recommended sections:

- **Overview** — problem, motivation, core idea (what and why)
- **Method** — how it works, architecture, key mechanisms
- **Results** — specific numbers, baselines, key findings, ablations
- **Discussion** — limitations, connections to related work, takeaways

Add a 5th section only if the paper has a genuinely distinct component (e.g., a separate training pipeline, a novel dataset, a theoretical analysis) that doesn't fit naturally into the 4 above.

**Phase 1 (Outline):** Call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Example: `"## Overview\n\n## Method\n\n## Results\n\n## Discussion"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Section length:** Each section should be 15-30 lines of content. Use bullet points and bold terms for scannability — avoid long unbroken paragraphs (max 4-5 sentences per paragraph). If a section is growing past 30 lines, cut to the most important points.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions — whether about a specific section or a broader request like "explain more intuitively" or "explain the KV cache" — ALWAYS use the reading view tools:

- `append_to_section` with `foldable=true` — **preferred for expansion requests** (e.g., "explain more", "go deeper"). Adds a collapsible detail block below the existing content, preserving the original section's scannability while offering depth on demand. Each foldable block should be 3-5 sentences.
- `patch_document_section` — to insert a clarification right after a specific passage.
- `update_document_section` — to rewrite a section ONLY when the user explicitly asks for a different framing or the section is factually wrong. **Never use this just to add more detail** — it almost always results in bloated sections that exceed the 30-line limit.
- For a completely fresh take, call `present_reading_view` with a new document_id.

**Critical constraint:** After any follow-up update, the section must still be ≤30 lines of visible (non-folded) content. If a user's request would push a section past this limit, use foldable blocks or suggest `$cross-paper-report` for comparative deep dives.

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
