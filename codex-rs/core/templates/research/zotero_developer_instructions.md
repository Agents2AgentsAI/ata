# Zotero library

You have access to the user's Zotero library through the `ata zotero ...` CLI namespace. Use Zotero only when the user mentions Zotero, their library, a collection, group libraries, or "my papers". Do not automatically route every paper task through Zotero.

When using Zotero:

1. Start with `ata zotero status` when the current behavior is unclear. It shows the effective Zotero mode, scope, and whether a local fallback path is available.
2. For named folders, curated buckets, or source-repo requests, always list collections first:
   - `ata zotero collections --compact`
   - `ata zotero collection items --collection-key ... --compact`
3. When one collection clearly matches, stay with that collection instead of launching multiple synonymous repo searches in parallel.
4. For repository discovery, prefer one bounded pass with `ata zotero find-repos --collection "..."` when the collection is the source of truth, otherwise `ata zotero find-repos --query "..."` before broad keyword search.
5. If `ata zotero collection items --collection-key ... --compact` prints `No items.`, treat that collection as empty and stop retrying it unless the library scope changes.
6. Only broaden repo discovery once if the first bounded pass returns too few usable repos.
7. For paper resolution, prefer `ata zotero resolve-paper --query "..."` or `ata zotero resolve-paper --item-key ...`.
8. Use `ata zotero search-commands "..."` only as fallback when the correct Zotero subcommand is still unclear after considering `status`, `collections`, `collection items`, `find-repos`, `resolve-paper`, and `search`.
9. When using low-level read commands, prefer compact discovery output first (`--compact`) and use full JSON only when you need exact structured fields.
10. Prefer linked Zotero relations and explicit URLs over fuzzy title matching.

When you need to read a paper deeply from Zotero:

1. Call `ata zotero item get --item-key ... --include-attachments --include-fulltext-resolution`.
2. If `document_resolution.preferred_url` is present, call `attach_url_files` with that URL.
3. If only a local path exists, use that path as the primary source.
4. Never use `curl` + `pdftotext` when `attach_url_files` can read the PDF directly.
