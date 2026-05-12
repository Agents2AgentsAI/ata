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
    let mut defs = Vec::new();

    defs.extend([
            ToolDef {
                id: "paper_search",
                native_name: "paper_search",
                mcp_name: "search_papers",
                description: "Search for academic papers across Semantic Scholar, arXiv, and OpenAlex.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "year_from": { "type": "integer" },
                        "year_to": { "type": "integer" },
                        "fields_of_study": { "type": "array", "items": { "type": "string" } },
                        "source": { "type": "string" },
                        "sort_by": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "include_abstract": { "type": "boolean" },
                        "fields": { "type": "array", "items": { "type": "string" } },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "paper_get",
                native_name: "paper_get",
                mcp_name: "get_paper",
                description: "Get detailed paper information by DOI, arXiv ID, or Semantic Scholar ID.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "paper_id": { "type": "string" } },
                    "required": ["paper_id"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "paper_citations",
                native_name: "paper_citations",
                mcp_name: "get_citations",
                description: "Get papers that cite a paper.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "paper_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "fields": { "type": "array", "items": { "type": "string" } },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["paper_id"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "paper_references",
                native_name: "paper_references",
                mcp_name: "get_references",
                description: "Get papers referenced by a paper.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "paper_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "fields": { "type": "array", "items": { "type": "string" } },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["paper_id"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "paper_recommendations",
                native_name: "paper_recommendations",
                mcp_name: "get_recommendations",
                description: "Get paper recommendations based on example papers. Provide paper IDs (DOI, arXiv ID, or S2 ID) as positive examples of papers you like. Optionally provide negative examples of papers to avoid.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "positive_paper_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paper IDs (DOI, arXiv ID, or S2 ID) of papers to use as positive examples"
                        },
                        "negative_paper_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Paper IDs of papers to use as negative examples (papers to avoid)"
                        },
                        "limit": { "type": "integer" },
                        "fields": { "type": "array", "items": { "type": "string" } },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["positive_paper_ids"],
                    "additionalProperties": false
                }),
            },
    ]);

    defs.extend([
            ToolDef {
                id: "hn_search",
                native_name: "hn_search",
                mcp_name: "search_hackernews",
                description: "Search Hacker News stories and comments via the Algolia API. Filter by content type (story, comment, show_hn, ask_hn), minimum points/comments, date range, author, or story ID. Sort by relevance or date.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "content_type": {
                            "type": "string",
                            "enum": ["story", "comment", "show_hn", "ask_hn"],
                            "description": "Filter by content type"
                        },
                        "sort_by": {
                            "type": "string",
                            "enum": ["relevance", "date"],
                            "description": "Sort order (default: relevance)"
                        },
                        "min_points": { "type": "integer", "description": "Minimum points threshold" },
                        "min_comments": { "type": "integer", "description": "Minimum comment count" },
                        "date_from": { "type": "string", "description": "Start date (YYYY-MM-DD)" },
                        "date_to": { "type": "string", "description": "End date (YYYY-MM-DD)" },
                        "author": { "type": "string", "description": "Filter by HN username" },
                        "story_id": { "type": "integer", "description": "Filter comments by parent story ID" },
                        "offset": { "type": "integer", "description": "Page number (0-based)" },
                        "limit": { "type": "integer", "description": "Results per page (1-100, default 20)" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "hn_get_thread",
                native_name: "hn_get_thread",
                mcp_name: "get_hackernews_thread",
                description: "Get a Hacker News story or comment by ID with its nested comment tree. Use to read discussions about a paper, tool, or library.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_id": { "type": "integer", "description": "HN item ID" },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum comment nesting depth (1-20, default 5)"
                        },
                        "max_comments": {
                            "type": "integer",
                            "description": "Maximum total comments to return (1-500, default 200)"
                        }
                    },
                    "required": ["item_id"],
                    "additionalProperties": false
                }),
            },
    ]);

    defs.extend([
            ToolDef {
                id: "patent_search",
                native_name: "patent_search",
                mcp_name: "search_patents",
                description: "Search patents worldwide via the European Patent Office (EPO) Open Patent Services. Use this tool instead of web search whenever the user asks about patents. Covers 90+ patent offices with daily updates. Filter by keyword, inventor, assignee, CPC code, and date range. Returns structured patent metadata including titles, abstracts, inventors, assignees, and classification codes.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Text search across patent title and abstract. Use \"*\" to match all patents when filtering only by assignee, inventor, or date." },
                        "inventor": { "type": "string", "description": "Filter by inventor name" },
                        "assignee": { "type": "string", "description": "Filter by patent assignee/applicant (e.g. \"Apple Inc\", \"Google LLC\")" },
                        "cpc_code": { "type": "string", "description": "Filter by CPC classification code prefix" },
                        "date_from": { "type": "string", "description": "Publication date start (YYYY-MM-DD)" },
                        "date_to": { "type": "string", "description": "Publication date end (YYYY-MM-DD)" },
                        "sort_by": {
                            "type": "string",
                            "enum": ["relevance", "date"],
                            "description": "Sort order (default: relevance)"
                        },
                        "size": { "type": "integer", "minimum": 10, "maximum": 100, "description": "Results per page (10-100, default 25)" },
                        "after": { "type": "string", "description": "Cursor for next page (from previous response)" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "patent_get",
                native_name: "patent_get",
                mcp_name: "get_patent",
                description: "Get detailed patent information from EPO. Use this tool to retrieve full metadata for a specific patent, including title, abstract, inventors, assignees, CPC codes, and claims text.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "patent_id": { "type": "string", "description": "Patent publication number (e.g. \"EP1000000A1\", \"US11234567B2\")" }
                    },
                    "required": ["patent_id"],
                    "additionalProperties": false
                }),
            },
    ]);

    defs
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::all_tool_defs;

    fn assert_schema_has_field(tool_id: &str, field: &str) {
        let defs = all_tool_defs();
        let def = defs
            .iter()
            .find(|tool| tool.id == tool_id)
            .unwrap_or_else(|| panic!("missing tool definition for {tool_id}"));
        let properties = def
            .input_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .unwrap_or_else(|| panic!("tool schema missing properties object for {tool_id}"));
        assert!(
            properties.contains_key(field),
            "tool schema for {tool_id} is missing field `{field}`",
        );
    }

    #[test]
    fn paper_search_schema_exposes_all_supported_params() {
        assert_schema_has_field("paper_search", "sort_by");
        assert_schema_has_field("paper_search", "max_chars_per_item");
    }

    #[test]
    fn paper_relation_schemas_expose_output_budget_and_field_filters() {
        for tool_id in ["paper_citations", "paper_references"] {
            assert_schema_has_field(tool_id, "fields");
            assert_schema_has_field(tool_id, "max_chars_per_item");
        }
    }

    #[test]
    fn tool_ids_are_unique() {
        let defs = all_tool_defs();
        let mut ids = defs.iter().map(|def| def.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), defs.len());
    }
}
