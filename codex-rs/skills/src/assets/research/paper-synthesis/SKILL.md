---
name: paper-synthesis
description: "REQUIRED for any paper explanation or synthesis. Do NOT call attach_url_files and summarize inline — always open this SKILL.md first and follow its workflow. Use when a user asks to explain, summarize, synthesize, or deep-dive into a research paper, or when given an arXiv URL, DOI, paper title, or Zotero reference. The workflow spawns synthesizer subagents, presents results in reading view, and persists to KB."
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis (Main Agent)

You orchestrate paper synthesis. A subagent reads the paper; you present the result in a reading view.

## Your Role

You are an **orchestrator**. Your only job is to spawn subagents, wait for their output, and present the result via `present_reading_view`. The subagent handles all paper reading.

**Your tools (the only tools you call):**
- `exec_command` — for KB checks (`rg`) and reading staging files (`cat`)
- `spawn_agent` — to launch synthesizer subagents
- `wait` — to wait for subagents
- `present_reading_view` — to open the reading view
- `update_document_section` — to fill reading view sections

**Tools the subagent uses (you never call these):**
- `attach_url_files` — the subagent calls this to fetch the PDF
- `paper_get` — the subagent handles metadata

## The Flow

**Single paper (KB enabled) — exactly 6 tool calls:**

1. `exec_command: rg "PAPER_ID" ${CODEX_KB_PATH}/cards/` — quick KB check
2. `spawn_agent` with `agent_type: "synthesizer"` and prompt: `$paper-synthesizer\n\nPaper: [URL]`
3. `wait` for the subagent — it returns a staging file path
4. `exec_command: cat [staging file path]` — read the subagent's output
5. `present_reading_view` — open with section headings only
6. `update_document_section` × N — fill each section

Then 2 more calls for KB persistence. That's it.

**Single paper (KB disabled) — exactly 4 tool calls:**

1. `spawn_agent` — same as above
2. `wait` — returns staging file path
3. `exec_command: cat [staging file path]` — read output
4. `present_reading_view` + `update_document_section` × N — present result

After presenting, clean up: `exec_command: for f in ${CODEX_KB_PATH}/staging/paper-*.md; do unlink "$f"; done`

## Pre-Synthesis

Most papers arrive with a URL, arXiv ID, or DOI already known. Go straight to spawning the subagent.

**Only use `paper_search` when you have nothing but a title or author name.** If you already have a URL or ID, pass it directly to the subagent.

- **arXiv URL** → convert `/abs/` to `/pdf/` → spawn subagent
- **DOI or S2 ID** → pass as-is → spawn subagent
- **Title only** → `paper_search` to find ID → spawn subagent
- **Zotero** → `zotero_search` → `zotero_get_item` → extract URL → spawn subagent

## Subagent Prompt

```
$paper-synthesizer

Paper: [paper URL — use /pdf/ for arXiv]
```

Always use `agent_type: "synthesizer"`. That's the complete prompt — the subagent skill handles everything else.

## KB Check (skip when KB is disabled)

**Single paper:** `exec_command: rg "PAPER_ID" ${CODEX_KB_PATH}/cards/` — one call. If a card already exists, present it directly via `present_reading_view`. Otherwise, spawn the subagent.

**Multi-paper:** `exec_command: rg "ID1\|ID2\|ID3" ${CODEX_KB_PATH}/cards/` — one call for all. Skip papers that already have cards.

## Multi-Paper Path

1. Gather all URLs/IDs. Use `paper_search` only for papers with just a title.
2. KB check in one call (skip if KB disabled).
3. Spawn subagents in batches of 8 (system limit ~20 threads). One `wait` call per batch.
4. Read all staging files: `exec_command: cat ${CODEX_KB_PATH}/staging/paper-*.md`
5. Present via `present_reading_view`.
6. KB persistence (skip if KB disabled): spawn a fire-and-forget `$kb` subagent with all card contents embedded in the prompt.

## Presenting the Result

**After reading the staging file, immediately call `present_reading_view`.** This is always your next action after reading the staging file — present first, persist to KB second.

**Phase 1 (Outline):** Call `present_reading_view` with section headings only:

```
document_id: "paper-[slug]"
title: "[Paper Title]"
content: "## Overview\n\n## Method\n\n## Results\n\n## Discussion"
```

**Phase 2 (Fill):** The tool result tells you which section to fill. Call `update_document_section` for each section sequentially. Each section: 15-30 lines, bullet points and bold terms for scannability.

**Sections:** Let the paper's content determine the number and names of sections. A simple paper might need 3 sections; a paper with a novel dataset, a separate training pipeline, and a theoretical analysis might need 6. The only hard rules:

- **No section may exceed 40 lines** (one terminal screen). If a section grows past that, split it.
- **No section should be thinner than 8 lines** — merge thin sections with adjacent ones.
- Target **15-30 lines** per section for comfortable reading.

Common section types (adapt as needed): Overview, Method, Architecture, Training, Results, Ablations, Discussion, Limitations.

**Markdown:** Always put a blank line before list items (`1.`, `-`).

**Fallback:** If `present_reading_view` is unavailable, output the full synthesis as formatted markdown directly in chat.

## Follow-Up Questions

When the user asks follow-ups, always use reading view tools:

- `append_to_section` with `foldable=true` — preferred for "explain more" / "go deeper". Adds a collapsible block below existing content. 3-5 sentences per block.
- `patch_document_section` — insert a clarification after a specific passage.
- `update_document_section` — rewrite a section only when the user asks for a different framing or there's a factual error.
- New `present_reading_view` with a different document_id — for a completely fresh take.

**Constraint:** Sections stay ≤30 lines of visible (non-folded) content after updates. Use foldable blocks for anything beyond that.

Write follow-up content as straight prose — no labels like "(expanded)" or bold topic prefixes.

## KB Persistence (skip when KB disabled)

**Single paper:** Write the card directly via `exec_command` with a heredoc after presenting:

```
exec_command: cat <<'CARD_EOF' > ${CODEX_KB_PATH}/cards/paper-[slug].md
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

Then append to `research-journal.md` and delete the staging file in a second `exec_command`.

**Multi-paper:** Spawn a fire-and-forget `$kb` subagent with all card contents in the prompt.

**Follow-up persistence (KB only):** When the user exits the reading view after Q&A that produced new insights, spawn a fire-and-forget `$kb` subagent to persist them.

Card ID convention: `paper-[kebab-case-slug]` (e.g., `paper-latent-diffusion`).

## Graceful Degradation

- **No KB**: Skip KB checks and persistence. Spawn subagent → present result.
- **PDF download fails**: Synthesize from abstract and conversation context. Note the limitation.
- **User provides only a title**: Use `paper_search`. If not found, ask for a URL.
- **Subagent failures**: Present results from successful subagents. Note failures briefly at the end ("Could not retrieve: [title] — [reason]").
