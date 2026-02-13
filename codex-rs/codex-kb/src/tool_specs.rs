use serde_json::Value;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: &'static str,
    pub native_name: &'static str,
    pub mcp_name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[must_use]
pub fn all_tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            id: "kb_write_card",
            native_name: "kb_write_card",
            mcp_name: "kb_write_card",
            description: "Write a knowledge card to the KB. Creates or updates the card file, updates the index, and upserts the card row in each topic overview.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Kebab-case card identifier" },
                    "title": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Lowercase topic tags" },
                    "capsule": { "type": "string", "description": "One-line summary (~100 chars)" },
                    "source_type": { "type": "string", "description": "Source type: paper, repo, blog, etc." },
                    "source_refs": { "type": "array", "items": { "type": "string" }, "description": "Source references (DOIs, URLs, etc.)" },
                    "status": { "type": "string", "enum": ["current", "superseded", "speculative", "stub"] },
                    "tensions": { "type": "array", "items": { "type": "string" }, "description": "IDs of cards this one is in tension with" },
                    "supersedes": { "type": "array", "items": { "type": "string" }, "description": "IDs of cards this one supersedes" },
                    "figures": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "Relative path to figure from KB root (e.g. 'assets/paper-lapa/fig-003-002.png')" },
                                "caption": { "type": "string", "description": "Figure caption" },
                                "page": { "type": "integer", "description": "PDF page the figure was extracted from" }
                            },
                            "required": ["path"],
                            "additionalProperties": false
                        },
                        "description": "Figures attached to this card"
                    },
                    "contributed_by": { "type": "string", "description": "Agent or user who contributed this card" },
                    "body": { "type": "string", "description": "Markdown body content" }
                },
                "required": ["id", "title", "tags", "body"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_write_overview",
            native_name: "kb_write_overview",
            mcp_name: "kb_write_overview",
            description: "Overwrite the OVERVIEW.md for a topic. Used for Tier 2 (comparative analysis) regeneration.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "topic": { "type": "string", "description": "Topic name (directory name under topics/)" },
                    "content": { "type": "string", "description": "Full markdown content for OVERVIEW.md" }
                },
                "required": ["topic", "content"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_suggest_tags",
            native_name: "kb_suggest_tags",
            mcp_name: "kb_suggest_tags",
            description: "Suggest tags for a card based on title and capsule, matched against the existing tag taxonomy.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "capsule": { "type": "string" }
                },
                "required": ["title"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_status",
            native_name: "kb_status",
            mcp_name: "kb_status",
            description: "Get KB status: card count, per-topic staleness, and tag taxonomy.",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_search",
            native_name: "kb_search",
            mcp_name: "kb_search",
            description: "Search knowledge base cards by text query. Searches titles, capsules, tags, body content, and source refs. Returns matching card summaries with context.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (searches across all card fields)" },
                    "tag": { "type": "string", "description": "Optional: filter results to cards with this tag" },
                    "status": { "type": "string", "description": "Optional: filter by status (current, superseded, speculative, stub)" },
                    "limit": { "type": "integer", "description": "Max results to return (default 20)" }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_read_card",
            native_name: "kb_read_card",
            mcp_name: "kb_read_card",
            description: "Read a knowledge card by ID. Returns the full card content (frontmatter + body).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card ID (kebab-case)" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_delete_card",
            native_name: "kb_delete_card",
            mcp_name: "kb_delete_card",
            description: "Delete a knowledge card by ID. Removes the card file, cleans up topic overviews, and updates the index.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Card ID (kebab-case) to delete" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_write_file",
            native_name: "kb_write_file",
            mcp_name: "kb_write_file",
            description: "Write an arbitrary file under the KB directory. Use for explanations, generated PDFs, assets, or any artifact that lives alongside cards. Parent directories are created automatically. Path must be relative (no leading slash, no '..' traversal).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path under the KB root (e.g. 'explanations/my-report.md' or 'explanations/my-report.tex')" },
                    "content": { "type": "string", "description": "File content (text)" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_read_file",
            native_name: "kb_read_file",
            mcp_name: "kb_read_file",
            description: "Read an arbitrary file under the KB directory. Use for reading explanations, assets, or other artifacts. Path must be relative (no leading slash, no '..' traversal).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path under the KB root (e.g. 'explanations/my-report.md')" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kb_list_cards",
            native_name: "kb_list_cards",
            mcp_name: "kb_list_cards",
            description: "List all cards in the knowledge base. Optionally filter by tag or status. Returns card summaries (id, title, tags, capsule, status).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tag": { "type": "string", "description": "Optional: filter to cards with this tag" },
                    "status": { "type": "string", "description": "Optional: filter by status" },
                    "limit": { "type": "integer", "description": "Max results (default 50)" }
                },
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::all_tool_defs;

    #[test]
    fn tool_ids_are_unique() {
        let defs = all_tool_defs();
        let mut ids: Vec<_> = defs.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), defs.len());
    }

    #[test]
    fn all_tools_have_schemas() {
        for def in all_tool_defs() {
            assert!(
                def.input_schema.get("type").is_some(),
                "tool {} missing schema type",
                def.id
            );
        }
    }
}
