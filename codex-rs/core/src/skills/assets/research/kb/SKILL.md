---
name: kb
description: Knowledge Base operations for card-based research knowledge management. Use when reading, writing, searching, updating, or managing KB cards and files. Also use when persisting conversation insights back to cards (the update protocol). Other research skills reference this skill via $kb for KB operations.
metadata:
  short-description: Knowledge base card operations
policy:
  allow_implicit_invocation: true
---

# Knowledge Base (KB)

The knowledge base is a directory of markdown files with YAML frontmatter, organized for research knowledge management. All operations use standard file tools (read, write, ls, grep).

## KB Path

Use `${CODEX_KB_PATH}` as the KB directory. The runtime injects this variable per turn.
If `${CODEX_KB_PATH}` is missing, fall back to `~/.ata/knowledge-base`.
The resolved KB path is referred to as `<kb_path>` throughout this document.

## Directory Layout

```
<kb_path>/
  cards/              # One .md file per knowledge card
    <card-id>.md
    ...
  topics/             # Per-tag overview documents
    <tag>/
      OVERVIEW.md
  index.json          # Tag taxonomy and topic staleness tracking
  research-context.md # User priorities and preferences
  research-journal.md # Chronological session log
```

## Card Format

Each card is a markdown file with YAML frontmatter:

```markdown
---
id: latent-diffusion
title: Latent Diffusion Models
tags:
  - diffusion
  - generative
capsule: Diffusion in latent space for efficient image generation.
source:
  type: paper
  refs:
    - "arxiv:2112.10752"
status: current
tensions: []
supersedes: []
figures:
  - path: figures/ldm-architecture.png
    caption: Architecture overview
    page: 3
date_added: 2025-01-15
date_updated: ~
contributed_by: research-agent
---

## Summary

[Card body content here...]
```

### Frontmatter Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Kebab-case, lowercase identifier (e.g., `latent-diffusion`) |
| `title` | string | yes | Human-readable title |
| `tags` | string[] | no | Lowercase topic tags |
| `capsule` | string | no | One-line summary |
| `source` | object | no | `type` (e.g., "paper", "hackernews") and `refs` (e.g., arXiv IDs, DOIs) |
| `status` | string | no | One of: `current`, `superseded`, `speculative`, `stub` |
| `tensions` | string[] | no | Unresolved questions or contradictions |
| `supersedes` | string[] | no | Card IDs this card replaces |
| `figures` | object[] | no | Attached figures with `path`, optional `caption` and `page` |
| `date_added` | date | no | YYYY-MM-DD when card was created |
| `date_updated` | date | no | YYYY-MM-DD when card was last updated |
| `contributed_by` | string | no | Who/what created this card |

### Card ID Rules

- Lowercase alphanumeric and hyphens only: `[a-z0-9-]+`
- Must not start or end with a hyphen
- Examples: `latent-diffusion`, `paper-lapa`, `hn-rust-async-runtime`

## Operations

All operations use standard file tools. Here is how to perform each KB operation:

### Search Cards

Search card contents by keyword or tag:
```
grep "<query>" <kb_path>/cards/*.md
```

Search by tag in frontmatter:
```
grep "^  - <tag>" <kb_path>/cards/*.md
```

### Read a Card

```
read <kb_path>/cards/<card-id>.md
```

### Write a Card

Write a markdown file with YAML frontmatter to `<kb_path>/cards/<card-id>.md`. Ensure the `cards/` directory exists first. The file format is `---\n<yaml>\n---\n\n<body>`.

Set `date_added` on new cards. Set `date_updated` when modifying existing cards.

### List Cards

```
ls <kb_path>/cards/
```

To get card summaries, read each card and extract the frontmatter.

### Delete a Card

```
rm <kb_path>/cards/<card-id>.md
```

### KB Status

To check KB status:
1. Check if `<kb_path>` exists: `ls <kb_path>/`
2. Count cards: `ls <kb_path>/cards/*.md`
3. Read index: `read <kb_path>/index.json` (if it exists)

### Read/Write Arbitrary KB Files

For files like `research-context.md`, `research-journal.md`, or topic overviews:
```
read <kb_path>/<relative-path>
write <kb_path>/<relative-path>
```

### Initialize KB

If the KB directory doesn't exist, create the structure:
```
mkdir -p <kb_path>/cards
mkdir -p <kb_path>/topics
```

Optionally create an empty `index.json`:
```json
{
  "topics": {},
  "tag_taxonomy": []
}
```

## index.json

The index tracks tag taxonomy and topic staleness:

```json
{
  "topics": {
    "diffusion": {
      "last_regen": "2025-06-01",
      "cards_since_regen": 3
    }
  },
  "tag_taxonomy": ["diffusion", "generative", "robotics"]
}
```

- `tag_taxonomy`: Set of all known tags. Register new tags when writing cards.
- `topics[tag].last_regen`: Date when the topic overview was last regenerated.
- `topics[tag].cards_since_regen`: Number of cards added since last overview regeneration. When this is high, the topic overview is stale and should be regenerated.

After writing a card, update the index:
1. Read `<kb_path>/index.json`
2. Add the card's tags to `tag_taxonomy`
3. Increment `cards_since_regen` for each of the card's tags
4. Write the updated index back

After regenerating a topic overview, reset staleness:
1. Set `last_regen` to today's date
2. Set `cards_since_regen` to 0

## Topic Overviews

Topic overviews live at `<kb_path>/topics/<tag>/OVERVIEW.md`. They are prose summaries of all cards with a given tag, useful for orientation. Regenerate when `cards_since_regen` is high.

## Updating Cards with Conversation Insights

Persist insights from conversation back to KB cards so the knowledge base grows with use. This is lightweight: read card, append insight, write card.

### When to Update

1. **Follow-up Q&A produces a substantive explanation not in the card** — the user asks "how does X handle Y?" and the answer reveals mechanism details, edge cases, or intuitions not captured in the original card.
2. **Explicit save request** — the user says "save this", "remember this", "add this to the card".
3. **Connection discovery** — a comparison between papers reveals a relationship not recorded in either card's `## Connections` section.
4. **Correction or refinement** — the user corrects or refines understanding of a method.

### Update Protocol

1. **Identify target card(s)** — determine which KB card(s) the insight applies to. Read each per the operations above.
2. **Classify the insight** — mechanism insight, edge case, comparison, correction, or practical implication.
3. **Append to Discussion Notes** — add under a `## Discussion Notes` section at the end of the card body with a date header. If the section exists, append under the existing date header (same day) or add a new one.

Format:

```markdown
## Discussion Notes

### YYYY-MM-DD
**Q: [The question or topic that prompted this insight]**
[The explanation or insight, written as clear prose. 2-6 sentences.
Include specific details — numbers, mechanisms, comparisons.]

**Connection discovered:** [If applicable, note the other card's ID or paper name.]
```

Multiple insights on the same day go under the same date header.

4. **Update Connections** — if the insight reveals a cross-paper connection, add it to both cards' `## Connections` sections with a one-line description.
5. **Write updated card** — card ID and frontmatter remain unchanged — only the body is modified. Set `date_updated`.

### What NOT to Update

- **Do not modify Summary, Architecture, Training Pipeline, or Deep Dive sections.** Those represent the original synthesis. Discussion Notes supplement, not replace.
- Exception: if the user explicitly asks to correct a section, modify it and note the correction in Discussion Notes.

### Bulk Update

When the user says "save what we discussed": scan the conversation for all insights, group by card, append under today's date header, and write all updated cards. Report what was updated.

### Research Context Awareness

During updates, watch for signals that express research preferences (not paper-specific insights):
- "I don't care about training cost, only inference latency" → priority
- "I'm not interested in pure RL approaches" → exclusion
- "I've decided to go with VQ-VAE tokenization" → key decision

Offer to update `<kb_path>/research-context.md` in addition to the card update. If the file doesn't exist, create it with sections: Project, Priorities, Not Interested In, Framings That Work, Key Decisions Made.

### Clear KB

When the user asks to "clear the KB", "reset the KB", or "wipe the KB", do all of the following without asking clarifying questions:

1. Delete all content: `rm -rf <kb_path>/cards/* <kb_path>/topics/* <kb_path>/briefings/* <kb_path>/explanations/* <kb_path>/assets/* <kb_path>/staging/*`
2. Reset `<kb_path>/index.json` to: `{"tag_taxonomy": [], "topics": {}}`
3. Clear `<kb_path>/research-journal.md` to: `# Research Journal\n`
4. **Keep** `<kb_path>/research-context.md` — it contains user preferences, not card data.

Confirm completion with a count of deleted cards and a note that research-context.md was preserved.

## Graceful Degradation

- If `<kb_path>` doesn't exist, tell the user and offer to initialize it.
- If `index.json` doesn't exist, proceed without it — it's optional metadata.
- If `research-context.md` doesn't exist, skip personalization steps.
