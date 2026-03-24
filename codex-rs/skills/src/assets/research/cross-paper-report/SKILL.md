---
name: cross-paper-report
description: Generate integrated cross-paper explanations from KB cards. Use when a user asks to explain, compare, synthesize, understand, or deep-dive into one or more knowledge base cards, or asks how cards relate or work together.
---

# Cross-Paper Report

> For conversation-derived reports, use `$conversation-report`. For LaTeX PDF output, use `$latex-report` after generating the narrative.

You MUST produce a **deep narrative explanation** — no exceptions, no shortcuts. A short summary is NEVER acceptable.

### Word-Count Floors

Hard minimums. If any section falls below, STOP and expand before proceeding.

| Tier | Word floor | When to use |
|------|-----------|-------------|
| **Focal** | 800–2500 words | 1–4 cards (all focal), or the 2–4 most novel/complex/central in a 5–12 set |
| **Supporting** | 400–600 words | Remaining papers in a 5–12 set |

## Core Principles

Three principles govern all output from this skill:

1. **TTS-readable** — Output is read aloud. Keep paragraphs short, pacing natural. No walls of text.
2. **Teach, don't summarize** — The reader has not read the paper. They should *understand* the method after reading your explanation. Ground everything in concrete numbers from the paper. Explain every equation intuitively. Establish *why* before *how*.
3. **Self-contained sections** — Each section in the reading view loads independently. Never forward-reference ("as we'll see in Section 3"). Explain concepts completely where they appear. Define every term on first use.

### Hard Rules

- **No academic citations.** Never "Name et al. (YYYY)". Use paper titles or natural phrasing ("the LAPA paper showed…"). TTS reads these aloud.
- **Never reference the KB.** Present content as if you understand the papers directly.
- **Details block** at the end of each subsection: 1–3 sentences collecting model names, dimensions, hyperparameters. The narrative must be understandable without it.
- **Voice:** Second-person for procedural walkthroughs ("You take two frames…"). Neutral third-person for framing and results. Do NOT open problem statements with "You want to…".

## Prerequisites

Determine `<kb_path>` per `$kb` (default `${CODEX_KB_PATH}`). **If KB is disabled**, work from card content in the conversation. Skip all KB reads/writes.

## Scale Detection

After reading all requested cards:
- **≤ 12 cards:** Follow Phases 0–2 below.
- **> 12 cards:** Jump to **Large-Set Synthesis** at the end.

## Phase 0: Related Card Discovery

**Skip if KB is disabled or running as a cluster subagent.**

1. List all cards per `$kb`. Scan tags/connections for related unrequested cards.
2. If found, present them: "Your KB also has cards for [list] — want to include any?"
3. **Auto-include without asking** on "full survey" / "explain them all", `$paper-discovery` pipeline, or auto-continue flow. **Skip entirely** if user says "only these cards."

## Phase 1: Deep Technical Explanation

Read all cards per `$kb`. **If KB is disabled**, use conversation context.

### Coverage-First

**Never drop cards.** If 10 are relevant, all 10 appear. Use tiering or Large-Set Synthesis — never cut.

### Tiered Depth

- **1–4 cards:** All focal.
- **5–12 cards:** 2–4 focal (most novel/complex/central), rest supporting.
- **13+:** Large-Set Synthesis.

Cluster subagents: all cards focal, skip user confirmation. When user confirmed scope or triggered from `$paper-discovery`, proceed without asking.

### Focal Card Elements

Organize in whatever sequence tells the clearest story. Integrate equations, analogies, and Details blocks where they naturally belong:

- Problem (plain-language gap), architecture, training pipeline, equations with intuition, results with specific numbers and baselines, limitations

### Supporting Card Structure

5 substantive paragraphs: problem & core idea, method, key technical detail, results with numbers, connection to focal cards. No stage walkthroughs or Details blocks needed.

## Phase 2: Cross-Card Comparative Synthesis

Produce when 2+ cards. Skip for single-card requests. Read `research-context.md` (if KB enabled) to frame around user priorities.

Compare along concrete technical dimensions with specific numbers and implementation details. Close with a **How the Papers Relate** prose section.

Before completing: verify word-count floors are met. If any section is below, expand it.

## Presentation (Main Agent Only)

**Cluster subagents never call `present_reading_view`** — return content as text.

**Section length:** Each section ≤ 40 lines. Focal papers split into 2–3 `## ` sub-sections (Problem & Core Idea, Method, Results & Analysis). Supporting papers: single `## ` section each. Comparison: `## How the Papers Relate`, split if longer.

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id`, `title`, and `content` containing ONLY `## ` headings with empty bodies.

**Phase 2 (Fill):** The tool result tells you the next section. Call `update_document_section` for each sequentially.

**Markdown:** Always put a blank line before list items — without it, the parser treats them as plain text.

**Follow-ups:** `append_to_section` with `foldable=true` for expansions, `patch_document_section` for corrections, `update_document_section` only for full rewrites. Keep sections ≤40 visible lines. No editorial labels in headings.

## Post-Report Housekeeping

**Skip if KB is disabled.**

**1. Journal** — Prepend to `<kb_path>/research-journal.md`: date, title, papers compared, key findings (1-2 bullets), card IDs touched.

**2. Research context** — If user shows preference signals during Q&A, briefly offer to note in `research-context.md`. Never block the interaction.

**3. Follow-up persistence** — When Q&A produces new insights, spawn fire-and-forget `$kb` subagent to persist them (card IDs + new insights, 2-4 sentences each).

---

## Large-Set Synthesis (> 12 cards)

### Step 1: Cluster Assignment

Group into 3–6 thematic clusters (4–8 cards each). Present to user: cluster names with rationale, card IDs, bridge cards, and a shared terminology glossary (5–10 cross-cluster terms).

### Step 2: Per-Cluster Sub-Reports

Launch one subagent per cluster in parallel. Overrides: skip Phase 0, skip Presentation (return text), all cards focal, no user confirmation, include shared glossary.

Each returns: per-card walkthroughs + within-cluster synthesis, cluster summary, cross-cluster interface (key terms, equations, specific numbers, bridge points).

If a subagent fails, proceed with remaining. Note the gap, offer retry.

### Step 3: Meta-Synthesis

New analytical layer connecting clusters (not a summary). Required sections: Scope & Framing, Cross-Cluster Evolution Tracing, Design Space Mapping, Failure Mode Lineage, Key Equations Across the Stack, How the Papers Relate, Practical Takeaways. Use cross-cluster interfaces as primary data source. Ground in specific numbers.
