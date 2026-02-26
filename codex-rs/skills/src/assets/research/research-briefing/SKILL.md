---
name: research-briefing
description: Generate a concise research briefing from KB cards or a topic. Use when a user wants a quick overview, orientation, summary of approaches, or wants to understand what their options are before diving deep. Use instead of cross-paper-report when the user wants brevity over depth. Examples -- "give me a quick overview of VLA approaches", "what are my options for sim-to-real", "summarize what I have on diffusion policies", "brief me on action tokenization methods".
metadata:
  short-description: Quick orientation briefing from KB cards
---

# Research Briefing

A 2-4 page orientation document that cuts through paper complexity to extract the one core idea per paper. Answers "what are my options and which should I look at first?" — not "how does every mechanism work in detail."

For full technical walkthroughs, use `$cross-paper-report`. For conversation-derived reports, use `$conversation-report`. This skill is for quick orientation before diving deep.

## Core Principle: Cut Through the Noise

Most papers wrap a small, concrete idea in 15 pages of related work, ablations, and notation. The briefing extracts that one core idea and states it plainly — not in the paper's language, but in yours.

Rules:

- State the core contribution in 1-2 sentences using plain language.
- If the idea is small, say it's small: "This paper's contribution is a single architectural change: replacing the action head with a diffusion decoder."
- If it's incremental, say what it's incremental over: "This is RT-1 but with a VLM backbone instead of a ViT — same tokenized action approach, bigger pretrained model."
- Never mirror the paper's framing of its own importance — papers are designed to sound maximally novel.
- Use the pattern: "[Paper] does [one concrete thing]. The trick is [specific mechanism]. Tradeoff: [what you gain vs. what you lose]."
- Per-paper depth: 3-5 sentences max. Must include the one core idea (plainly stated), one specific number or result, how it relates to the other approaches, and what it trades off.
- **Citation formatting**: cite as **Author (Year)** in prose. Never put DOIs or arXiv IDs inline in paragraphs — they break reading flow. Collect full references (with IDs) in a References section at the end of the briefing if the user needs them for follow-up.
- **Never reference the KB in explanations.** Do not say "as summarized in your KB" or "your KB card says." The KB is infrastructure — present content as if you understand the papers directly.

## Phase 0: Source Material

This skill works best with KB cards, but it can also work without them.

### Path A: KB Available (preferred)

1. Determine `<kb_path>` per the `$kb` skill and verify KB exists.
2. List all cards per `$kb`.
3. Filter cards whose tags, titles, or topics relate to the user's question or requested topic.
4. If related unrequested cards exist, present them: "I also found cards for X, Y — want me to include them in the briefing?"
5. If no cards match the topic but KB is enabled, tell the user: "No KB cards found for this topic. Run `$paper-synthesis` on relevant papers first, then re-run this briefing." The briefing synthesizes from existing KB content — it does not read papers itself.

### Path B: No KB, but conversation context available

If KB is disabled but the conversation already contains synthesis output (from `$paper-synthesis` staging files, prior explanations, or user-provided paper details), use that content as source material. Extract the same fields as Phase 1 (core contribution, key result, approach family, tradeoffs, status) from whatever is available.

### Path C: No KB, no prior context (cold start)

If KB is disabled and no prior synthesis exists in the conversation, run a lightweight discovery:

1. Use `paper_search` with 2-3 facets from the user's topic (same facet design as `$paper-discovery` Phase 1).
2. Fetch the top 8-12 results sorted by citation count.
3. Use the paper abstracts and metadata as source material for the briefing — this produces a shallower briefing than Path A, but still gives the user actionable orientation.
4. Note in the briefing: "This briefing is based on paper abstracts only. For deeper analysis, run `$paper-synthesis` on papers of interest."

**Path C is a fallback.** The briefing will lack the depth of a KB-sourced briefing — abstracts don't contain method details, specific numbers, or implementation insights. But it still answers "what are my options?" and gives the user a starting point.

## Phase 1: Read and Analyze Source Material

For each relevant card (or paper abstract in Path C):

1. Read the card per `$kb` to get the full content. For Path B, read from conversation context. For Path C, use the paper_search results directly.
2. Extract:
   - The core contribution (from Summary or Deep Dive — the single most important idea)
   - The key result (one specific number with context)
   - The approach family (what paradigm does this belong to?)
   - Tradeoffs (what does this approach gain and lose?)
   - Status (current state of the art? Superseded? Foundational?)

## Phase 2: Cluster into Approaches

Group the papers into 2-5 approach families based on their core paradigm. Each approach should have a clear name and a 1-sentence description of the shared strategy.

## Phase 2.5: Read Research Context (Optional, skip if KB is disabled)

If `<kb_path>/research-context.md` exists, use it to tailor the briefing:

- **Priorities**: Weight the Recommendation section toward the user's documented priorities (e.g., if they care about inference latency, lead with approaches that address it).
- **Not Interested In**: De-emphasize or briefly note approaches the user has dismissed, rather than giving them equal airtime.
- **Framings That Work**: Use explanation styles that match the user's preferences (e.g., tradeoff framing, concrete numbers).

If the file doesn't exist, skip this step — the briefing still works without it.

## Phase 3: Generate the Briefing

The briefing is delivered in the **chat response only** — presented directly in conversation. This is the sole deliverable. No markdown file, no PDF. A briefing is a quick orientation — persistence adds latency without value. If the user wants to re-read a previous briefing, they can re-run it (it reads from KB cards, which are stable). If the user wants a polished typeset document, they should use `$cross-paper-report`.

**Research journal tip:** If the user has a `research-journal.md` in their KB, previous journal entries may show related topics they've already explored — mention this connection if relevant (e.g., "You explored action tokenization on [date] — this briefing covers the broader landscape around that topic").

### Briefing Structure

```markdown
## Research Briefing: [Topic]
### [Date]

### The Landscape
[1 paragraph: what problem space this covers, what the main tension is,
where the field is heading. 4-6 sentences max.]

### Approaches at a Glance

#### [Approach 1 Name] (N papers)
**Core idea:** [1-2 sentences — the actual contribution, plainly stated]
**Key paper:** [Title] ([Year]) — [why this is the best representative]
**Tradeoff:** [what you gain vs. what you lose in 1 sentence]
**Best for:** [when to use this approach]

#### [Approach 2 Name] (N papers)
**Core idea:** [1-2 sentences]
**Key paper:** [Title] ([Year]) — [why this is the best representative]
**Tradeoff:** [what you gain vs. what you lose]
**Best for:** [when to use this approach]

...

### Per-Paper Quick Reference

| Paper | Year | Core Idea (1 sentence) | Key Number | Status |
|-------|------|----------------------|------------|--------|
| RT-2 | 2023 | VLM predicts tokenized actions directly | 2x generalization on novel objects | Superseded by OpenVLA |
| OpenVLA | 2024 | Open 7B VLA with LoRA adaptation | 16.5% avg improvement with fine-tuning | Current best bootstrap |
| ... | | | | |

### Recommendation
[1-2 paragraphs: Given what you're trying to do, here's what I'd suggest
looking at first and why. Reference specific papers. Be opinionated —
the whole point of a briefing is to help the user decide where to focus.]

### Dive Deeper
For full technical walkthroughs of any paper above:
- `$cross-paper-report [card-ids]` for deep multi-paper analysis
- `$paper-synthesis [arxiv-url]` for single-paper deep dive
- Chat about any paper and ask follow-up questions — insights will be saved to KB cards per `$kb`
```

## Presentation (Main Agent Only)

**This section applies to the main agent only.** If this skill is loaded in a subagent, return results as text to the main agent — do NOT call `present_reading_view`.

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the briefing title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## The Landscape\n\n## Approaches at a Glance\n\n## Recommendation"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

**Markdown formatting:** Always put a blank line before numbered list items (`1.`, `2.`, etc.) and before bullet list items (`-`, `*`). Without a blank line, the markdown parser treats `2.`, `3.`, etc. as plain text instead of list items, so they lose their formatting. This also applies to content after paragraphs, blockquotes, and code blocks.

When the user asks follow-up questions about a specific section, use the most efficient update tool:

- `append_to_section` with `foldable=true` — **preferred for expansion requests** (e.g., "explain more", "go deeper on X"). Adds a collapsible detail block below existing content, preserving scannability. Each foldable block: 3-5 sentences.
- `append_to_section` (without foldable) — to add genuinely new information at the end of a section.
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits).
- `update_document_section` — to fully rewrite a section ONLY when the user asks for a different framing or the section is factually wrong. **Never use this just to add more detail.**

**Critical constraint:** After any follow-up update, the section must still be ≤30 lines of visible (non-folded) content. If a request would push past this limit, use foldable blocks.

Write follow-up answers as straight content — no editorial labels like "(clearer explanation)" or "(expanded)" in headings or topic lines.

## Post-Briefing Housekeeping

**Skip this entire section if KB is disabled.** When KB is off, do not write journal entries, research-context updates, or spawn `$kb` subagents. The briefing in the reading view is the sole output.

After presenting the briefing, do these:

**1. Journal entry** — Append to `<kb_path>/research-journal.md`. Prepend (newest first):

```markdown
## [Date] — Briefing: [Topic]

### Explored
- Briefed on [N] papers covering [topic]
- Approaches identified: [list approach families briefly]

### Recommendation
- [1-2 sentences: what was recommended and why]

### Cards Touched
- [card-ids] (read)

---
```

**2. Research context detection** — If the user's questions or reactions reveal priorities (e.g., "which of these is fastest at inference?", "I don't need the simulation-only ones"), offer to note it in `research-context.md`. This is especially valuable during briefings because the user is actively deciding what to focus on.

**3. Follow-up Q&A persistence** — When the user exits the reading view and Q&A produced new insights not already in the relevant KB cards, automatically spawn a fire-and-forget `$kb` subagent (do NOT call `wait`) to persist them. Do not ask the user. Include the card IDs and a summary of new insights in the subagent prompt:

> $kb
>
> Update KB cards with follow-up insights. Do NOT ask the user — this is automatic.
> Cards to update: [card-id-1], [card-id-2], ...
> New insights from Q&A:
> [For each card: card ID, the question asked, and the substantive answer, 2-4 sentences each]
>
> For each card: read it, append insights under `## Discussion Notes` per the update protocol, set `date_updated`, write the card back.

## Anti-Patterns

- **NEVER write more than 5 sentences per paper.** This is a briefing, not a deep dive. If you're writing multi-paragraph walkthroughs, you're doing cross-paper-report's job.
- **NEVER mirror the paper's self-assessment of novelty.** Every paper claims to be revolutionary. Your job is to say plainly what it actually does.
- **NEVER skip the Recommendation section.** The whole point is to help the user decide where to focus. Be opinionated.
- **NEVER generate a briefing with zero source material.** If no KB cards, conversation context, or paper search results are available, ask the user for a topic or paper URLs.

## Graceful Degradation

- **No KB configured**: Use Path B (conversation context) or Path C (paper search). Note in the output that a KB-sourced briefing would be deeper.
- **Few cards (1-2)**: Still produce the briefing, but note that coverage is thin and suggest adding more papers via `$paper-discovery` or `$paper-synthesis`.
- **Cards lack depth**: If cards are shallow (abstract-only synthesis), note this in the briefing and suggest re-running `$paper-synthesis` with full PDF access.
- **Path C paper search returns few results**: Broaden the search facets or note that the topic is niche. A thin briefing is better than no briefing.
