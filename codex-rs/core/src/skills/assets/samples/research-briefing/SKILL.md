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

## Phase 0: Source KB Cards

1. Call `kb_status` to get `kb_path` and verify KB exists.
2. Call `kb_list_cards` to retrieve all cards.
3. Filter cards whose tags, titles, or topics relate to the user's question or requested topic.
4. If related unrequested cards exist, present them: "I also found cards for X, Y — want me to include them in the briefing?"
5. If no cards match the topic, tell the user: "No KB cards found for this topic. Run `$paper-synthesis` on relevant papers first, then re-run this briefing." The briefing synthesizes from existing KB content — it does not read papers itself.

## Phase 1: Read and Analyze Cards

For each relevant card:

1. Call `kb_read_card` to get the full content.
2. Extract:
   - The core contribution (from Summary or Deep Dive — the single most important idea)
   - The key result (one specific number with context)
   - The approach family (what paradigm does this belong to?)
   - Tradeoffs (what does this approach gain and lose?)
   - Status (current state of the art? Superseded? Foundational?)

## Phase 2: Cluster into Approaches

Group the papers into 2-5 approach families based on their core paradigm. Each approach should have a clear name and a 1-sentence description of the shared strategy.

## Phase 2.5: Read Research Context (Optional)

If `research-context.md` exists at the KB root (read via `kb_read_file` at path `research-context.md`), use it to tailor the briefing:

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
- Chat about any paper and ask follow-up questions — insights will be saved via `$kb-update`
```

## Presentation

IMPORTANT: When the briefing is complete, you MUST call `present_reading_view` to present it in sectioned reading mode instead of outputting text directly. Do NOT stream the report as regular text. Set `document_id` to a unique slug, `title` to the briefing title, and `content` to the full markdown with `## ` headings for sections. End your response immediately after calling this tool.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

## Post-Briefing Housekeeping

After presenting the briefing, do these:

**1. Journal entry** — Append to `research-journal.md` at the KB root via `kb_write_file`. Prepend (newest first):

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

## Anti-Patterns

- **NEVER write more than 5 sentences per paper.** This is a briefing, not a deep dive. If you're writing multi-paragraph walkthroughs, you're doing cross-paper-report's job.
- **NEVER mirror the paper's self-assessment of novelty.** Every paper claims to be revolutionary. Your job is to say plainly what it actually does.
- **NEVER skip the Recommendation section.** The whole point is to help the user decide where to focus. Be opinionated.
- **NEVER generate the briefing without KB cards.** This skill synthesizes from existing KB content. If cards don't exist, direct the user to `$paper-synthesis` first.

## Graceful Degradation

- **No KB tools configured**: Present the briefing in chat. Note that KB tools are needed for card-based briefings — without them, the skill cannot source content.
- **Few cards (1-2)**: Still produce the briefing, but note that coverage is thin and suggest adding more papers via `$paper-discovery` or `$paper-synthesis`.
- **Cards lack depth**: If cards are shallow (abstract-only synthesis), note this in the briefing and suggest re-running `$paper-synthesis` with full PDF access.
