---
name: zotero
description: >-
  Zotero library management through the `ata zotero ...` CLI namespace. Use
  when the user mentions Zotero, their library, a collection, group libraries,
  or "my papers", and you need to search, inspect, organize, import, or update
  Zotero records without exposing the large native Zotero tool family.
metadata:
  short-description: Zotero via ata zotero
policy:
  allow_implicit_invocation: true
---

# Zotero Management

Use the `ata zotero ...` CLI namespace through `exec_command`. Do not rely on the old `zotero_*` native tool family.

## Core Rules

- Prefer `ata zotero ...` over direct API calls when the task is about Zotero
- Start with `ata zotero status` when Zotero behavior is unclear; it reports the effective Zotero mode, scope, and fallback path
- Prefer the task-oriented commands over low-level composition:
  - `ata zotero collections --compact`
  - `ata zotero find-repos --query "..."` or `--collection "..."`
  - `ata zotero resolve-paper --query "..."` or `--item-key ...`
- Expect JSON output from most low-level `ata zotero ...` commands, but use `--compact` for discovery-oriented reads when available
- If the request sounds like a curated bucket, folder, or repo set (for example "source repos", "my papers", "collection", "folder", or a topic collection), always list collections first instead of guessing by query
- When the request is about implementations, GitHub links, or source repos, start with `ata zotero collections --compact`, identify the relevant collection, inspect it with `ata zotero collection items --collection-key ... --compact`, then use `ata zotero find-repos --collection "..."`
- When one collection clearly matches, stay with that collection. Do not fan out into multiple synonymous `find-repos --query ...` calls in parallel before inspecting the first result.
- If `ata zotero collection items --collection-key ... --compact` prints `No items.`, treat that collection as empty and stop retrying it unless the library scope changes.
- For repo discovery, run one bounded discovery pass, inspect the returned candidates, and only broaden once if you still have fewer than a small starter set of useful repos.
- Only use `ata zotero search-commands "<intent>"` when the right Zotero subcommand is still unclear after considering `status`, `collections`, `collection items`, `find-repos`, `resolve-paper`, `search`, groups, and direct item inspection
- Omit `--library-type` and `--library-id` unless the user names a specific library or group
- When the user names a collection, go straight to collection commands; do not scan unrelated collections
- When you need a paper PDF source, inspect the item first and prefer resolved document URLs over title guessing

## Common Workflows

### Collection-first repo discovery

Use this when the user wants source repos, implementations, GitHub links, or topic-curated papers from Zotero.

1. Check mode if needed: `ata zotero status`.
2. List collections first: `ata zotero collections --compact`.
3. Identify the right collection key, then inspect it directly: `ata zotero collection items --collection-key ... --compact`.
4. For repo discovery, start with exactly one bounded command: `ata zotero find-repos --collection "..."` if the collection is the source of truth, otherwise one targeted `ata zotero find-repos --query "..."`.
5. If the first repo-discovery pass returns no repos, broaden once using the matched collection name or the strongest paper title signal; do not spray many parallel synonym queries.
6. Only inspect the most promising items with: `ata zotero item get --item-key ... --include-attachments --include-fulltext-resolution --compact`.
7. Prefer explicit repo URLs, webpage items, linked related items, and local fallback results before broad whole-library search.

### Search and inspect

Use this for open-ended discovery when collections are not the obvious starting point.

1. Start with `ata zotero resolve-paper --query "..."`.
2. If you need broader candidate search, use `ata zotero search --query "..." --compact`.
3. Read the item: `ata zotero item get --item-key ... --include-attachments --include-fulltext-resolution --compact`.
4. If the item exposes a preferred PDF URL, hand that URL to `attach_url_files`.
5. Use `ata zotero search-commands "..."` only if the correct Zotero subcommand is still unclear.

### Collections

1. List collections: `ata zotero collections`.
2. Read items in one collection: `ata zotero collection items --collection-key ...`.
3. Create or reuse a collection:
   - `ata zotero collection create --name "..." ...`
   - `ata zotero collection find-or-create --name "..." ...`
4. Add items without disturbing existing memberships:
   - `ata zotero collection add-items --collection-key ... --item-keys KEY1 KEY2`

### Bulk item mutations

Use JSON payload commands for nested Zotero structures:

- `ata zotero items create --json-file payload.json`
- `ata zotero items update --json-file payload.json`
- `ata zotero advanced-search --json-file payload.json`
- `ata zotero grep-text --json-file payload.json`

Inline JSON is also allowed with `--json`, but prefer `--json-file` for large payloads.

### Linked attachments

Create direct PDF or repo links under an item:

`ata zotero attachment create-link --parent-item-key ... --title "PDF" --url "https://..."`.

## Command Reference

| Command | Purpose |
|---------|---------|
| `ata zotero status` | Show the effective Zotero mode, scope, and fallback path |
| `ata zotero collections --compact` | List collections so you can choose the right one |
| `ata zotero resolve-paper --query "..."` | Resolve one paper and enrich it with document metadata |
| `ata zotero find-repos --query "..."` | Find repository URLs in Zotero items, collections, or linked records |
| `ata zotero search-commands "<intent>"` | Rank the most relevant Zotero CLI subcommands for an intent |
| `ata zotero search --query ...` | Keyword search across titles, creators, and tags |
| `ata zotero tags` | List tags |
| `ata zotero recent` | Recent items |
| `ata zotero search-notes --query ...` | Search notes and annotations |
| `ata zotero item get --item-key ...` | Item metadata with optional attachment/fulltext resolution |
| `ata zotero item citation --item-key ...` | BibTeX / CSL JSON / APA citation |
| `ata zotero item fulltext --item-key ...` | Indexed fulltext fallback |
| `ata zotero item notes --item-key ...` | Notes for an item |
| `ata zotero item annotations ...` | Annotations for an item or library scope |
| `ata zotero item attachments --item-key ...` | Attachment metadata |
| `ata zotero collections` | List collections |
| `ata zotero collection items --collection-key ...` | Items in one collection |
| `ata zotero collection create --name ...` | Create a collection |
| `ata zotero collection find-or-create --name ...` | Find or create collection |
| `ata zotero collection add-items ...` | Add existing items to a collection |
| `ata zotero groups list` | List accessible groups |
| `ata zotero items create --json-file ...` | Bulk item creation |
| `ata zotero items update --json-file ...` | Bulk item updates |
| `ata zotero attachment create-link ...` | Linked attachment creation |

## Deep Reading

When the user wants you to read a paper from Zotero:

1. `ata zotero resolve-paper --query "..."` or `ata zotero resolve-paper --item-key ...`
2. `ata zotero item get --item-key ... --include-attachments --include-fulltext-resolution`
3. Extract `document_resolution.preferred_url` if present
4. Call `attach_url_files` with that PDF URL
5. Only fall back to local-path handling if no URL is available

Never use `curl` + `pdftotext` when `attach_url_files` can read the PDF directly.
