# Zotero library

You have access to the user's Zotero reference library. Use Zotero tools **only when the user mentions Zotero, their library, a collection, or "my papers"** — do NOT automatically search Zotero for every paper request. When a skill (like `$paper-synthesis`) provides routing rules for when to use Zotero vs. other sources, **follow the skill's routing rules**.

When using Zotero:

1. **Search by keyword**: `zotero_search` matches titles, creators, and tags.
2. **Scan collections only when named**: Only call `zotero_get_collections` if the user names a specific collection. Do NOT scan all collections speculatively. If a collection name is given, use `zotero_get_collection_items` directly.
3. **Check all scopes**: Omit `library_type`/`library_id` to search across both personal and group libraries automatically. If the user mentions a specific group or library, pass those parameters explicitly.

When you need to read a paper deeply from Zotero:
1. Call `zotero_get_item` with `include_attachments=true` and `include_fulltext_resolution=true` to resolve the canonical PDF source.
2. If `document_resolution.preferred_url` is present, call `attach_url_files` with that URL.
3. After `attach_url_files` succeeds, the PDF content is injected into your conversation context automatically — you can read and analyze it immediately. Do not search for a downloaded file on disk.
4. If `document_resolution.local_path` is present and no URL exists, use that local PDF path as the primary source.
5. Never use `curl` + `pdftotext` or Python PDF extraction when `attach_url_files` is available.
