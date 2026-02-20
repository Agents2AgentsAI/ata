---
name: kb
description: Knowledge Base operations for card-based research knowledge management. Use when reading, writing, searching, or managing KB cards and files. Other research skills reference this skill via $kb for KB operations.
metadata:
  short-description: Knowledge base card operations
policy:
  allow_implicit_invocation: true
---

# Knowledge Base (KB)

The knowledge base is a directory of markdown files with YAML frontmatter, organized for research knowledge management. All operations use standard file tools (read, write, ls, grep).

## KB Path

The KB directory is `~/.ata/knowledge-base`. Use this path directly — do not read config.toml to determine it.

Resolve `~` to the user's home directory. The KB path is referred to as `<kb_path>` throughout this document.

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

## Graceful Degradation

- If `<kb_path>` doesn't exist, tell the user and offer to initialize it.
- If `index.json` doesn't exist, proceed without it — it's optional metadata.
- If `research-context.md` doesn't exist, skip personalization steps.
