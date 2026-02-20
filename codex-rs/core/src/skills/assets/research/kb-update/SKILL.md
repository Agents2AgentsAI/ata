---
name: kb-update
description: Update KB cards with new insights from conversation. Use when the user asks follow-up questions about a paper or topic and the discussion reveals understanding not captured in existing cards. Also use when the user explicitly asks to save or remember an insight. Examples -- "save this insight", "remember this", "add this to the card", "update the card with what we discussed".
metadata:
  short-description: Update KB cards from conversation
---

# KB Update

Persist insights from conversation back to KB cards so the knowledge base grows with use. This skill is lightweight and fast — read card, append insight, write card. No PDF generation, no LaTeX, no subagents.

## When to Trigger

Invoke this skill when any of the following occur during conversation:

1. **Follow-up Q&A produces a substantive explanation not in the card** — the user asks "how does X handle Y?" and the answer reveals mechanism details, edge cases, or intuitions not captured in the original card.
2. **Explicit save request** — the user says "save this", "remember this", "add this to the card", "update the KB with this".
3. **Connection discovery** — a comparison between papers reveals a relationship not recorded in either card's `## Connections` section (e.g., "both papers discretize actions to constrain the output space, but via different mechanisms").
4. **Correction or refinement** — the user corrects or refines understanding of a method (e.g., "actually, the codebook isn't fixed during fine-tuning — they allow it to adapt").

## Update Protocol

### Step 1: Identify the Target Card

- Determine which KB card(s) the conversation insight applies to.
- Read each relevant card per `$kb` to get the current content.

### Step 2: Identify What the Conversation Added

Classify the new insight into one of these categories:

- **Mechanism insight**: A deeper understanding of how something works that goes beyond the original card's explanation.
- **Edge case or failure mode**: How the method behaves in specific scenarios not covered by the card.
- **Comparison insight**: How this paper relates to another paper in a way not captured in the Connections section.
- **Correction**: A factual correction to the card's content (rare — only when the original synthesis was inaccurate).
- **Practical implication**: A takeaway about when or how to use the method that emerged from discussion.

### Step 3: Append to Discussion Notes

Add the insight under a `## Discussion Notes` section at the end of the card body, with a date header. If the section already exists, append under the existing section with a new date header (or under the existing date header if it's the same day).

Format:

```markdown
## Discussion Notes

### YYYY-MM-DD
**Q: [The question or topic that prompted this insight]**
[The explanation or insight, written as clear prose. 2-6 sentences typically.
Include specific details — numbers, mechanisms, comparisons — not vague summaries.]

**Connection discovered:** [If the insight reveals a connection to another card,
note it here with the other card's ID or paper name.]
```

Multiple insights on the same day go under the same date header:

```markdown
### 2026-02-13
**Q: How does the VQ-VAE codebook handle out-of-distribution actions?**
The codebook snaps to nearest entries, so OOD actions get mapped to the closest known action pattern. This means the policy degrades gracefully (to known behaviors) rather than catastrophically. This is unlike continuous action spaces where OOD inputs can produce arbitrary outputs.

**Q: Why use 4 codebook positions instead of more?**
With 8 values per position and 4 positions, 8^4 = 4,096 possible latent actions. The paper found this is enough to cover the action space for tabletop manipulation. More positions would increase expressiveness but slow down the VLM's next-token prediction — each position is one autoregressive step.
```

### Step 4: Update Connections (If Applicable)

If the insight reveals a connection to another card:

1. Add or update the connection in the source card's `## Connections` section with a one-line description.
2. Also update the other card's `## Connections` section to note the reverse relationship.

Example addition to Connections:

```markdown
## Connections
- **FAST (paper-fast)**: Both discretize actions to constrain the output space — LAPA via VQ-VAE codebook, FAST via DCT+BPE tokenization. Both achieve graceful degradation on OOD inputs as a side effect.
```

### Step 5: Write Updated Card

Write the updated card per `$kb`. The card ID and frontmatter remain unchanged — only the body is modified.

## What NOT to Update

- **Do not modify the original Summary section.** That represents the paper's content as synthesized.
- **Do not modify the Architecture section.** Unless correcting a factual error.
- **Do not modify the Training Pipeline section.** Unless correcting a factual error.
- **Do not modify the Deep Dive section.** That represents the original synthesis walkthrough.
- **Discussion Notes are the user's evolving understanding layered on top of the original synthesis.** They supplement, not replace.

Exception: If the user explicitly says "correct the architecture section" or "fix the training description", then modify the relevant section and note the correction in Discussion Notes.

## Bulk Update After Conversation

When invoked at the end of a longer conversation (e.g., the user says "save what we discussed to the cards"), scan the conversation for all insights that apply to KB cards and batch-update them:

1. List all KB cards that were referenced or discussed.
2. For each card, collect all insights from the conversation that apply to it.
3. Group insights by card and append them all under today's date header.
4. Write all updated cards.

Report what was updated:
```
Updated 3 KB cards with conversation insights:
- paper-lapa: Added 2 insights (OOD handling, codebook size rationale)
- paper-groot-n1: Added 1 insight (cross-embodiment action normalization)
- paper-fast: Added 1 connection (shared discretization property with LAPA)
```

## Research Context Awareness

During any kb-update interaction, watch for signals that the user is expressing a research preference or priority — not just a paper-specific insight. Examples:

- "I don't care about training cost, only inference latency" → This is a priority, not a card insight.
- "I'm not interested in pure RL approaches" → This is a "Not Interested In" item.
- "The tradeoff framing really helps me understand this" → This is a framing preference.
- "I've decided to go with VQ-VAE tokenization" → This is a key decision.

When you detect such signals, offer to update `<kb_path>/research-context.md` in addition to the card update:
- "I also noticed you expressed a preference about [X]. Want me to record that in your research context so future briefings and reports account for it?"
- If the user agrees, read `<kb_path>/research-context.md` (create if it doesn't exist), merge the new item into the appropriate section (Priorities, Not Interested In, Framings That Work, or Key Decisions Made), and write it back.

This is lightweight and optional — never block a card update on research context. If the user says no or ignores the offer, proceed with the card update alone.

## Graceful Degradation

- **No KB configured**: Tell the user that a KB path is needed to persist insights. Offer to present the insights in chat instead.
- **Card not found**: If the referenced card doesn't exist, suggest running `$paper-synthesis` first to create it.
- **Ambiguous card reference**: If it's unclear which card the insight applies to, ask the user to specify.
