---
name: conversation-report
description: Generate a focused report from the current conversation and persist insights. Use when a user has been exploring papers, asking questions, and wants to capture what was discussed. Triggers -- "make a report from this", "save what we discussed", "write up what we covered", "turn this into a document".
metadata:
  short-description: Report from conversation exploration
---

# Conversation Report

Generate a focused report reflecting the user's specific exploration during this conversation. This is NOT a generic survey — it is a cleaned-up, organized version of what was actually discussed.

For exhaustive deep reports, use `$cross-paper-report`. For quick orientations, use `$research-briefing`. This skill captures the personalized exploration of a single conversation session.

## Phase 1: Conversation Analysis

Scan the current conversation to identify:

1. **KB cards referenced** — which cards were read, discussed, or cited during the conversation
2. **Papers discussed** — which papers were the subject of questions (may overlap with KB cards)
3. **Questions asked** — what the user wanted to understand (these become the report's structure)
4. **Comparisons made** — which papers or methods were compared and along what dimensions
5. **Insights emerged** — explanations, connections, or understanding that developed through Q&A
6. **User interest signals** — what the user seemed most interested in (asked follow-ups about, expressed surprise at, asked to explore further)

From this analysis, derive:
- **The implicit question**: "What was this conversation trying to understand?" — this becomes the report's framing
- **The exploration path**: The sequence of topics that tells the story of the user's investigation

## Phase 2: Generate the Report

### Core Principle: Clean Up, Don't Regenerate

The report is a **cleaned-up version of the conversation**, not a regeneration from scratch. Pull explanations you already gave during the conversation and organize them — do not re-explain everything from scratch. If the conversation had a great explanation of flow matching, that explanation goes into the report almost verbatim, just cleaned up for readability and organized into the right section.

This means:
- Reuse phrasing and explanations from the conversation
- Fix conversational artifacts (hedging, self-correction, tangents) into clean prose
- Organize by theme/question, not by chronological order of when things came up
- Add light connective tissue between sections, but do not pad with new analysis

### Report Structure

The structure is personalized, not formulaic. Organize by what the user explored:

```markdown
## [Topic derived from conversation]
### Session: [Date]

### The Question
[1-2 paragraphs: what the user was trying to understand, derived from
the conversation flow. Frame it as the implicit research question that
drove the exploration.]

### What We Found
[Organized by the questions asked, not by paper. Each subsection
is an answer to something the user explored:]

#### [Question/topic the user explored]
[The explanation that emerged from the conversation, cleaned up and
organized. References specific papers and KB cards. 1-3 paragraphs.]

#### [Another question/topic]
[...]

#### [Comparison that was discussed]
[If the user compared approaches, present the comparison here with
the specific dimensions they cared about.]

### Key Insights
[3-5 bullet points: the most important things that emerged from
this conversation that weren't obvious at the start. These are the
"aha moments" — connections, surprises, or clarifications.]

### Papers Referenced
| Paper | Card ID | Role in This Conversation |
|-------|---------|--------------------------|
| [Title] | [card-id] | [How it was discussed — "main focus", "comparison point", "mentioned in passing"] |
| ... | | |

### Open Questions
[Things that came up but weren't resolved — natural next steps for
the user's research. Frame as actionable items:]
- [Question] — try `$paper-discovery [topic]` or `$paper-synthesis [paper]`
- [Question] — needs empirical testing / reading [specific paper]
```

## Phase 3: Deliver and Persist

Four outputs, in order. The chat response is mandatory; the remaining three depend on KB tools being available.

### 1. Chat Response (Mandatory)
Present the full organized report in the conversation so the user sees it immediately. This is the primary deliverable — the organized narrative of what was explored and learned.

No markdown file. No PDF. The chat response is the deliverable. Insights get persisted to KB cards (step 2) and the journal (step 3) — those are the durable artifacts, not a duplicate document.

### 2. KB Card Updates
Check if insights from the conversation should be persisted to KB cards:

1. For each KB card referenced in the conversation, check if the discussion produced insights not in the card.
2. If yes, offer to update: "This conversation produced insights about [papers]. Want me to update the KB cards with these findings? This would add [brief description] to [card-ids]."
3. If the user agrees, apply the `$kb-update` protocol (read card, append Discussion Notes, write card).

This connects conversation-report to kb-update — the report presents the conversation in chat, while kb-update persists paper-specific insights back to individual cards for future reference.

### 3. Research Journal Entry
Append a structured entry to `<kb_path>/research-journal.md`. If the file doesn't exist yet, create it. New entries are **prepended** (newest first) so the most recent session is at the top.

The journal entry is much shorter than the full chat report — it's a structured summary for future reference, not a copy of the full narrative.

**Entry format:**

```markdown
## [Date] — [Topic derived from conversation]

### Explored
[2-4 bullet points: what topics/papers were investigated this session]
- Compared VQ-VAE vs DCT+BPE action tokenization across LAPA, FAST, and GR00T
- Investigated inference latency requirements for real-time bimanual control

### Conclusions
[2-4 bullet points: what the user concluded or decided]
- VQ-VAE codebook provides graceful OOD degradation that DCT+BPE also achieves
- Both approaches constrain output space to finite vocabulary — this is the key shared insight

### Open Questions
[1-3 bullet points: unresolved threads]
- How do these tokenization approaches handle deformable objects?
- Does codebook size need to scale with action space dimensionality for bimanual (14-DOF)?

### Cards Touched
[List of KB cards that were read, updated, or created]
- paper-lapa (updated: added Discussion Notes on OOD handling)
- paper-fast (updated: added connection to LAPA)
- paper-groot-n1 (read only)

---
```

**How to prepend:** Read the existing `research-journal.md` content (if any), compose the new entry, then write the new entry followed by the existing content back to the file.

### 4. Research Context Update (If Applicable)
After generating the report and journal entry, check if the conversation revealed new user preferences or project context:

- Did the user express a priority? (e.g., "I care most about inference latency")
- Did the user dismiss an approach? (e.g., "I'm not interested in pure RL methods")
- Did the user make a key decision? (e.g., "I'm going with VQ-VAE tokenization")
- Did the user respond well to a particular explanation style?

If yes, offer to update `<kb_path>/research-context.md`:
- "This conversation revealed some preferences. Want me to update your research context? I'd add: [specific items]."
- If the user agrees, read `<kb_path>/research-context.md` (create if it doesn't exist), merge the new information into the appropriate section, and write it back.

**Research context format** (create with these sections if new):

```markdown
## Research Context

### Project
[What the user is working on]

### Priorities
[What dimensions matter most]

### Not Interested In
[Approaches the user has dismissed]

### Framings That Work
[Explanation styles that clicked]

### Key Decisions Made
[Conclusions reached, with dates]
```

## Presentation (Main Agent Only)

**This section applies to the main agent only.** If this skill is loaded in a subagent, return results as text to the main agent — do NOT call `present_reading_view`.

**Phase 1 (Outline):** IMMEDIATELY call `present_reading_view` with `document_id` set to a unique slug, `title` to the report title, and `content` containing ONLY the `## ` section headings with empty bodies. Example content: `"## The Question\n\n## What We Found\n\n## Key Insights\n\n## Open Questions"`. This opens the reading view instantly with "Generating..." placeholders.

**Phase 2 (Fill):** The tool result will tell you to fill section 0. Immediately call `update_document_section(document_id, section_index=0, content="...")` with the FULL content for that section — do not output any text, just make the tool call. Each tool result tells you the next section to fill. Continue calling `update_document_section` for each subsequent section until all are filled.

When the user asks follow-up questions about a specific section, use the most efficient update tool:
- `append_to_section` — to add new information at the end of a section (most common for follow-up questions)
- `patch_document_section` — to change specific text within a section (for corrections or targeted edits)
- `update_document_section` — to fully rewrite a section (only when the entire section needs to change)

## Anti-Patterns

- **NEVER regenerate explanations from scratch.** Reuse what was said in the conversation. The value is that the report reflects the user's actual exploration, not a generic synthesis.
- **NEVER organize by paper.** Organize by question/topic. The user explored questions, not papers — papers are evidence within the answers.
- **NEVER produce a generic survey.** If the report could have been written without the conversation, it's wrong. The report should clearly reflect what the user asked about and what they learned.
- **NEVER skip the Open Questions section.** Every conversation leaves threads to pull. Surfacing them is part of the value.
- **NEVER copy the full chat report into the journal entry.** The journal entry is a short structured summary (Explored / Conclusions / Open Questions / Cards Touched) — not a duplicate of the report. Keep it under 20 lines.

## Graceful Degradation

- **Short conversation (< 3 substantive exchanges)**: Still produce the chat report, but note that it's brief and suggest continuing the exploration to deepen it. Journal entry may be just 2-3 bullets.
- **No KB cards referenced**: The conversation may have been about papers not yet in the KB. Still produce the report, but note which papers lack KB cards and suggest `$paper-synthesis` to create them. Skip KB card updates (step 2) but still write journal entry.
- **No KB configured**: Present the report in chat only. Note that a KB path is needed for journal and context persistence.
- **Conversation was unfocused**: If the conversation covered many unrelated topics, organize into clearly separated sections rather than forcing a unified narrative. Journal entry should note the multiple topics explored.
