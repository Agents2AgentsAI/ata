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

    #[cfg(feature = "paper_search")]
    {
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
                id: "paper_search_sota",
                native_name: "paper_search_sota",
                mcp_name: "search_sota",
                description: "Search state-of-the-art benchmark results via Papers With Code.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "task": { "type": "string" },
                        "dataset": { "type": "string" },
                        "limit": { "type": "integer" }
                    },
                    "required": ["task"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "paper_find_repos",
                native_name: "paper_find_repos",
                mcp_name: "find_code_repos",
                description: "Find code repositories associated with a paper.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "paper_id": { "type": "string" } },
                    "required": ["paper_id"],
                    "additionalProperties": false
                }),
            },
        ]);
    }

    #[cfg(feature = "zotero")]
    {
        defs.extend([
            ToolDef {
                id: "zotero_search",
                native_name: "zotero_search",
                mcp_name: "search_library",
                description: "Search Zotero items by query.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "item_type": { "type": "string" },
                        "fields": { "type": "array", "items": { "type": "string" } },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_item",
                native_name: "zotero_get_item",
                mcp_name: "get_item_details",
                description: "Get full Zotero metadata for an item.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_fulltext",
                native_name: "zotero_get_fulltext",
                mcp_name: "get_item_fulltext",
                description: "Get indexed fulltext for a Zotero item.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_notes",
                native_name: "zotero_get_notes",
                mcp_name: "get_item_notes",
                description: "Get notes attached to a Zotero item.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_attachments",
                native_name: "zotero_get_attachments",
                mcp_name: "get_item_attachments",
                description: "Get attachment metadata for a Zotero item.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_search_by_tag",
                native_name: "zotero_search_by_tag",
                mcp_name: "search_by_tag",
                description: "Search Zotero items by tag.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "item_type": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["tags"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_collections",
                native_name: "zotero_get_collections",
                mcp_name: "get_collections",
                description: "List Zotero collections.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" }
                    },
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_collection_items",
                native_name: "zotero_get_collection_items",
                mcp_name: "get_collection_items",
                description: "List items in a Zotero collection.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "collection_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "item_type": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["collection_key"],
                    "additionalProperties": false
                }),
            },
        ]);
    }

    #[cfg(feature = "repo_analysis")]
    {
        defs.extend([
            ToolDef {
                id: "repo_clone_and_summarize",
                native_name: "repo_clone_and_summarize",
                mcp_name: "clone_and_summarize",
                description: "Shallow-clone a repo and summarize its structure.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string" },
                        "branch": { "type": "string" }
                    },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_find_models",
                native_name: "repo_find_models",
                mcp_name: "find_model_definitions",
                description: "Find model class definitions in a repo.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string" },
                        "framework": { "type": "string" }
                    },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_extract_requirements",
                native_name: "repo_extract_requirements",
                mcp_name: "extract_requirements",
                description: "Extract dependency requirements from a repo.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "repo_url": { "type": "string" } },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_find_entrypoints",
                native_name: "repo_find_entrypoints",
                mcp_name: "find_entrypoints",
                description: "Find training/eval/inference/export entrypoints.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string" },
                        "task_hint": { "type": "string" }
                    },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_extract_io_shapes",
                native_name: "repo_extract_io_shapes",
                mcp_name: "extract_io_shapes",
                description: "Extract model input/output shape hints.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string" },
                        "model_class": { "type": "string" }
                    },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_get_health",
                native_name: "repo_get_health",
                mcp_name: "get_repo_health",
                description: "Get repo health and maintenance signals.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "repo_url": { "type": "string" } },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_find_export_paths",
                native_name: "repo_find_export_paths",
                mcp_name: "find_export_paths",
                description: "Find model export and conversion code paths.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "repo_url": { "type": "string" } },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_extract_config_schema",
                native_name: "repo_extract_config_schema",
                mcp_name: "extract_config_schema",
                description: "Extract training config schema and defaults.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "repo_url": { "type": "string" } },
                    "required": ["repo_url"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "repo_diff_requirements",
                native_name: "repo_diff_requirements",
                mcp_name: "diff_requirements",
                description: "Compare repo dependencies to a local requirements file.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string" },
                        "local_requirements_path": { "type": "string" }
                    },
                    "required": ["repo_url", "local_requirements_path"],
                    "additionalProperties": false
                }),
            },
        ]);
    }

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
    fn zotero_search_schema_exposes_optional_field_projection_and_budget() {
        assert_schema_has_field("zotero_search", "fields");
        assert_schema_has_field("zotero_search", "max_chars_per_item");
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
