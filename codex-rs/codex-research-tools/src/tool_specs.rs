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
        ]);
    }

    #[cfg(feature = "zotero")]
    {
        defs.extend([
            ToolDef {
                id: "zotero_search",
                native_name: "zotero_search",
                mcp_name: "search_library",
                description: "Search Zotero items by keyword query across titles, creators, and tags. For topic-based lookups, also check zotero_get_collections — users often organize papers into named collections that keyword search alone may miss.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "item_type": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_advanced_search",
                native_name: "zotero_advanced_search",
                mcp_name: "advanced_search",
                description: "Evaluate multi-condition client-side search over bounded Zotero candidates.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "conditions": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "field": {
                                        "type": "string",
                                        "enum": [
                                            "title",
                                            "creator",
                                            "year",
                                            "tag",
                                            "item_type",
                                            "publication",
                                            "doi",
                                            "note",
                                            "annotation",
                                            "fulltext"
                                        ]
                                    },
                                    "operation": {
                                        "type": "string",
                                        "enum": [
                                            "contains",
                                            "not_contains",
                                            "equals",
                                            "not_equals",
                                            "regex",
                                            "before_year",
                                            "after_year",
                                            "is_empty",
                                            "is_not_empty"
                                        ]
                                    },
                                    "value": { "type": "string" },
                                    "case_sensitive": { "type": "boolean" }
                                },
                                "required": ["field", "operation"],
                                "additionalProperties": false
                            }
                        },
                        "join_mode": { "type": "string", "enum": ["all", "any"] },
                        "sort_by": {
                            "type": "string",
                            "enum": ["title", "year", "date_added", "date_modified", "relevance"]
                        },
                        "sort_direction": { "type": "string", "enum": ["asc", "desc"] },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "item_type": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["conditions"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_grep_text",
                native_name: "zotero_grep_text",
                mcp_name: "grep_library_text",
                description: "Run bounded literal or regex matching across Zotero metadata, notes, annotations, and fulltext.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "match_mode": { "type": "string", "enum": ["literal", "regex"] },
                        "case_sensitive": { "type": "boolean" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "parent_item_key": { "type": "string" },
                        "query_hint": { "type": "string" },
                        "item_type": { "type": "string" },
                        "fields": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["title", "abstract", "extra", "note", "annotation", "fulltext", "tag"]
                            }
                        },
                        "limit_items": { "type": "integer" },
                        "limit_matches": { "type": "integer" },
                        "max_matches_per_item": { "type": "integer" },
                        "context_chars": { "type": "integer" },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "required": ["pattern"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_search_notes",
                native_name: "zotero_search_notes",
                mcp_name: "search_notes",
                description: "Search Zotero note and annotation text using grep-style matching.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "match_mode": { "type": "string", "enum": ["literal", "regex"] },
                        "case_sensitive": { "type": "boolean" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "parent_item_key": { "type": "string" },
                        "include_annotations": { "type": "boolean" },
                        "limit": { "type": "integer" },
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
                id: "zotero_get_annotations",
                native_name: "zotero_get_annotations",
                mcp_name: "get_annotations",
                description: "Get annotations in an item or across a Zotero library.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer" },
                        "include_parent_context": { "type": "boolean" },
                        "max_chars_per_item": { "type": "integer" }
                    },
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
                id: "zotero_get_collections",
                native_name: "zotero_get_collections",
                mcp_name: "get_collections",
                description: "List Zotero collections (user-organized folders). When a user asks about papers on a topic, scan collection names for matches — a collection named after the topic likely contains all relevant papers. Use zotero_get_collection_items to retrieve its contents.",
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
                id: "zotero_list_groups",
                native_name: "zotero_list_groups",
                mcp_name: "list_groups",
                description: "List Zotero groups accessible to the user.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "user_id": { "type": "string" },
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
                description: "List items in a Zotero collection by collection_key. Use after finding a relevant collection via zotero_get_collections.",
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

    #[cfg(feature = "pdf_images")]
    {
        defs.push(ToolDef {
            id: "pdf_extract_figures",
            native_name: "pdf_extract_figures",
            mcp_name: "extract_pdf_figures",
            description: "Download a PDF and extract embedded figures as PNG images using pdfimages. Filters out small icons and decorations by size and dimensions. Returns metadata for each extracted figure.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pdf_url": {
                        "type": "string",
                        "description": "URL to download the PDF from"
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Directory to save extracted PNG images"
                    },
                    "min_size_kb": {
                        "type": "integer",
                        "description": "Minimum file size in KB to keep (default 5)"
                    },
                    "min_width": {
                        "type": "integer",
                        "description": "Minimum pixel width to keep (default 100)"
                    },
                    "min_height": {
                        "type": "integer",
                        "description": "Minimum pixel height to keep (default 100)"
                    }
                },
                "required": ["pdf_url", "output_dir"],
                "additionalProperties": false
            }),
        });
    }

    #[cfg(feature = "latex")]
    {
        defs.push(ToolDef {
            id: "latex_compile",
            native_name: "latex_compile",
            mcp_name: "compile_latex",
            description: "Write LaTeX source to a file and compile it to PDF using pdflatex. Runs multiple passes to resolve references. Returns structured result with errors/warnings parsed from the log.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "LaTeX source content"
                    },
                    "output_dir": {
                        "type": "string",
                        "description": "Directory to write .tex and .pdf files"
                    },
                    "filename": {
                        "type": "string",
                        "description": "Base name without extension (defaults to 'document')"
                    }
                },
                "required": ["content", "output_dir"],
                "additionalProperties": false
            }),
        });
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
    fn zotero_search_schema_exposes_output_budget() {
        assert_schema_has_field("zotero_search", "max_chars_per_item");
    }

    #[test]
    fn zotero_advanced_search_schema_exposes_conditions_and_sorting() {
        assert_schema_has_field("zotero_advanced_search", "conditions");
        assert_schema_has_field("zotero_advanced_search", "join_mode");
        assert_schema_has_field("zotero_advanced_search", "sort_by");
    }

    #[test]
    fn zotero_grep_text_schema_exposes_matching_and_bounds() {
        assert_schema_has_field("zotero_grep_text", "pattern");
        assert_schema_has_field("zotero_grep_text", "fields");
        assert_schema_has_field("zotero_grep_text", "limit_matches");
    }

    #[test]
    fn zotero_search_notes_schema_exposes_note_controls() {
        assert_schema_has_field("zotero_search_notes", "query");
        assert_schema_has_field("zotero_search_notes", "include_annotations");
        assert_schema_has_field("zotero_search_notes", "limit");
    }

    #[test]
    fn zotero_get_annotations_schema_exposes_scope_and_parent_context_controls() {
        assert_schema_has_field("zotero_get_annotations", "item_key");
        assert_schema_has_field("zotero_get_annotations", "include_parent_context");
        assert_schema_has_field("zotero_get_annotations", "max_chars_per_item");
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
