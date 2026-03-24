---
name: paper-synthesis
description: "REQUIRED for any paper explanation or synthesis. Do NOT call attach_url_files and summarize inline — always use this skill. Use when given an arXiv URL, DOI, paper title, or Zotero reference."
metadata:
  short-description: Summarize and explain research papers
---

# Paper Synthesis (Main Agent)

You are an **orchestrator**. Spawn subagents to read papers, present results, persist to KB.

**Your tools:** `exec_command`, `spawn_agent`, `wait`, `present_reading_view`, `update_document_section`.
**Subagent-only (never call these):** `attach_url_files`, `crop_and_store_figure`, `paper_get`.

## The Flow

1. **Resolve the paper** — URL/ID/DOI directly. Only `paper_search` for bare titles. arXiv `/abs/` → `/pdf/`. Zotero → `$zotero`.
2. **KB check** (skip if disabled) — `rg "PAPER_ID" ${CODEX_KB_PATH}/cards/`. If card exists, present it directly.
3. **Spawn subagent** — The subagent skill is `paper-synthesizer` (NOT `paper-synthesis`). Use exactly:
   ```
   spawn_agent(agent_type="synthesizer", message="$paper-synthesizer\n\nPaper: [URL]")
   ```
4. **Wait** — returns staging file path.
5. **Read staging file** — `exec_command: cat [path]`.
6. **Present** — `present_reading_view` with headings, then `update_document_section` for each. Let the paper determine section names. Each section 15-30 lines, max 40, min 8. Include figures from staging file (`![caption](path)`) near relevant sections.
7. **KB persist** (skip if disabled) — write card via heredoc, append to journal, cleanup staging.
8. **Cleanup** — `exec_command: for f in ${CODEX_KB_PATH}/staging/paper-*.md; do unlink "$f"; done`

**Multi-paper:** Spawn in batches of 8 (system limit ~20 threads), one `wait` per batch. For KB, spawn fire-and-forget `$kb` subagent with all card contents.

## Math

`$...$` inline, `$$...$$` display. Never backtick code spans for math — reading view renders LaTeX via KaTeX. Always blank line before list items.

## KB Persistence

```
exec_command: cat <<'CARD_EOF' > ${CODEX_KB_PATH}/cards/paper-[slug].md
---
id: paper-[slug]
title: "[title]"
tags: [tags]
capsule: "[one-line summary]"
source: { type: paper, refs: ["[arXiv ID or DOI]"] }
status: current
date_added: [YYYY-MM-DD]
contributed_by: research-agent
---
[staging file body content]
CARD_EOF
```

Then append to `research-journal.md` and delete staging file. Follow-up persistence: spawn fire-and-forget `$kb` subagent when Q&A produces new insights. Card ID convention: `paper-[kebab-case-slug]`.

## Graceful Degradation

No KB: skip checks and persistence. PDF fails: synthesize from abstract. Title only: `paper_search`, ask for URL if not found. Subagent failures: present successful results, note failures.
