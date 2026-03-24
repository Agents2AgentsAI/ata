---
name: conversation-report
description: Generate a focused report from the current conversation and persist insights. Use when a user has been exploring papers, asking questions, and wants to capture what was discussed. Triggers -- "make a report from this", "save what we discussed", "write up what we covered", "turn this into a document".
metadata:
  short-description: Report from conversation exploration
---

# Conversation Report

Generate a cleaned-up, organized version of what was actually discussed in this conversation. This is NOT a generic survey — it reflects the user's specific exploration.

For exhaustive deep reports from KB cards, use `$cross-paper-report`.

## Core Principle: Clean Up, Don't Regenerate

Reuse phrasing and explanations from the conversation — fix conversational artifacts (hedging, self-correction, tangents) into clean prose. Organize by question/topic, not chronologically or by paper. Add light connective tissue but do not pad with new analysis.

## Phase 1: Understand the Conversation

Figure out: what was the implicit research question? What was the exploration path? Which papers, comparisons, and insights emerged? What did the user care most about?

## Phase 2: Generate the Report

Organize by what the user explored. Structure: The Question (what drove the exploration), What We Found (cleaned-up explanations organized by topic, not by paper), Key Insights (3-5 "aha moments"), Papers Referenced (table with role in conversation), Open Questions (actionable items with suggested `$paper-discovery` or `$paper-synthesis` follow-ups).

Present via `present_reading_view`. Each section ≤ 30 lines. Include relevant figures via `crop_and_store_figure` where they aid understanding.

## Phase 3: Persist

Four outputs in order:

1. **Chat response** (mandatory) — the reading view is the deliverable.
2. **KB card updates** (skip if KB disabled) — fire-and-forget `$kb` subagent for each card where discussion produced new insights. Do not ask the user.
3. **Journal entry** (skip if KB disabled) — **prepend** to `<kb_path>/research-journal.md`: date, topics explored, conclusions, open questions, cards touched. Keep under 20 lines. Read existing content first, write new entry + existing back.
4. **Research context** (skip if KB disabled) — if conversation revealed user preferences or project context, offer to update `<kb_path>/research-context.md`.

## Graceful Degradation

Short conversation: still produce the report, note brevity. No KB cards: still report, suggest `$paper-synthesis`. No KB configured: chat-only output. Unfocused conversation: organize into separated sections.
