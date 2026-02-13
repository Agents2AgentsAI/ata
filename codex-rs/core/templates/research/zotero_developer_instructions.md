# Zotero library

You have access to the user's Zotero reference library. When the user asks about papers on a topic — even without mentioning Zotero — always check Zotero alongside any academic search:

1. **Search by keyword**: `zotero_search` matches titles, creators, and tags.
2. **Scan collections**: `zotero_get_collections` lists user-organized folders. If a collection name matches the topic, retrieve its items with `zotero_get_collection_items` — this catches papers that keyword search misses.
3. **Check all scopes**: Omit `library_type`/`library_id` to search across both personal and group libraries automatically. If the user mentions a specific group or library, pass those parameters explicitly.
