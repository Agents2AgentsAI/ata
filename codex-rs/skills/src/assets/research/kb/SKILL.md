---
name: kb
description: Knowledge Base operations for card-based research knowledge management. Use when reading, writing, searching, updating, resetting, or managing KB cards and files. Also handles "reset KB", "clear KB", and "wipe KB" — immediately resets to empty state without asking questions. Other research skills reference this skill via $kb for KB operations.
metadata:
  short-description: Knowledge base card operations
policy:
  allow_implicit_invocation: true
---

# Knowledge Base (KB)

The knowledge base is a directory of markdown files with YAML frontmatter. All operations use standard file tools (read, write, ls, grep).

## KB Path

Use `${CODEX_KB_PATH}` as the KB directory (fallback: `~/.ata/knowledge-base`).
The resolved path is referred to as `<kb>` below.

## Directory Layout

```
<kb>/
  cards/<card-id>.md     # One file per knowledge card
  topics/<tag>/OVERVIEW.md  # Per-tag prose summaries
  index.json             # Tag taxonomy + staleness tracking
  research-context.md    # User priorities and preferences
  research-journal.md    # Chronological session log
```

## Card Format

Each card is `---\n<yaml frontmatter>\n---\n\n<body>`. Example:

```markdown
---
id: latent-diffusion
title: Latent Diffusion Models
tags: [diffusion, generative]
capsule: Diffusion in latent space for efficient image generation.
source:
  type: paper
  refs: ["arxiv:2112.10752"]
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

**Field notes:** `id` (required) — kebab-case `[a-z0-9-]+`, no leading/trailing hyphens. `title` (required). All other fields optional. `status` is one of: `current`, `superseded`, `speculative`, `stub`. `source.type` examples: "paper", "hackernews". `figures` entries have `path` (required), optional `caption` and `page`.

## Operations

| Operation | Command |
|-----------|---------|
| Search by keyword | `grep "<query>" <kb>/cards/*.md` |
| Search by tag | `grep "^  - <tag>" <kb>/cards/*.md` |
| Read card | `read <kb>/cards/<card-id>.md` |
| List cards | `ls <kb>/cards/` |
| Delete card | `rm <kb>/cards/<card-id>.md` |
| Read/write other files | `read/write <kb>/<relative-path>` |
| Initialize KB | `mkdir -p <kb>/cards <kb>/topics` |

**Writing a card:** Write to `<kb>/cards/<card-id>.md`. Ensure `cards/` exists. Set `date_added` on new cards, `date_updated` on modifications. After writing, update `index.json` (add tags to `tag_taxonomy`, increment `cards_since_regen` for each tag).

## Reset / Clear KB

**IMMEDIATE ACTION — no clarifying questions.** On "clear/reset/wipe the KB":

1. Count existing cards: `ls ${CODEX_KB_PATH}/cards/`
2. Delete all content: `exec_command("cd ${CODEX_KB_PATH} && find cards topics briefings explanations assets staging -type f -delete 2>/dev/null; find cards topics briefings explanations assets staging -mindepth 1 -type d -delete 2>/dev/null; true")`
3. Reset `index.json` to `{"tag_taxonomy": [], "topics": {}}`
4. Reset `research-journal.md` to `# Research Journal\n`
5. **Keep** `research-context.md` unchanged.

Confirm: "Reset complete. Deleted N cards. research-context.md preserved."

## index.json

Tracks tag taxonomy and topic staleness:

```json
{
  "topics": {
    "diffusion": { "last_regen": "2025-06-01", "cards_since_regen": 3 }
  },
  "tag_taxonomy": ["diffusion", "generative", "robotics"]
}
```

- `tag_taxonomy`: all known tags — register new tags when writing cards.
- `topics[tag].last_regen` / `cards_since_regen`: track when topic overviews need regeneration. After regenerating an overview, reset `cards_since_regen` to 0 and `last_regen` to today.

## Updating Cards with Conversation Insights

Persist substantive insights from conversation back to KB cards.

**When:** (1) Q&A reveals details not in the card, (2) explicit save request, (3) cross-card connection discovered, (4) correction or refinement.

**How:** Read the card, append under `## Discussion Notes` with a date header, write it back. Do not modify original synthesis sections (Summary, Core Method, etc.) unless explicitly asked.

```markdown
## Discussion Notes

### YYYY-MM-DD
**Q: [Question that prompted this insight]**
[Clear prose explanation, 2-6 sentences with specific details.]

**Connection discovered:** [Other card ID, if applicable.]
```

Multiple insights on the same day share the date header. If a cross-card connection is found, update both cards' `## Connections` sections. Set `date_updated` in frontmatter.

**Research context signals** — if the user expresses preferences ("I don't care about training cost"), offer to update `<kb>/research-context.md` (sections: Project, Priorities, Not Interested In, Framings That Work, Key Decisions Made).

## Graceful Degradation

- `<kb>` missing → tell user, offer to initialize.
- `index.json` missing → proceed without it.
- `research-context.md` missing → skip personalization.
