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
    }

    #[cfg(feature = "zotero")]
    {
        defs.extend([
            ToolDef {
                id: "zotero_search",
                native_name: "zotero_search",
                mcp_name: "",
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
                id: "zotero_get_tags",
                native_name: "zotero_get_tags",
                mcp_name: "",
                description: "List Zotero tags with pagination metadata for autocomplete and filtering flows.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 0
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 200,
                            "default": 100
                        }
                    },
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_recent",
                native_name: "zotero_get_recent",
                mcp_name: "",
                description: "List recently added or modified Zotero items in descending time order.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 0
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 100,
                            "default": 25
                        },
                        "item_type": { "type": "string" },
                        "sort_by": {
                            "type": "string",
                            "enum": ["date_added", "date_modified"],
                            "default": "date_added"
                        },
                        "max_chars_per_item": { "type": "integer" }
                    },
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_advanced_search",
                native_name: "zotero_advanced_search",
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
                description: "Get full Zotero metadata for an item, with optional attachment and document-source enrichment.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "max_chars_per_item": { "type": "integer" },
                        "include_attachments": { "type": "boolean", "description": "Include attachment metadata in response" },
                        "include_fulltext_resolution": { "type": "boolean", "description": "Include document source resolution in response" }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_item_citation",
                native_name: "zotero_get_item_citation",
                mcp_name: "",
                description: "Generate a citation for a Zotero item (BibTeX, CSL JSON, or APA).",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "item_key": { "type": "string" },
                        "library_type": { "type": "string" },
                        "library_id": { "type": "string" },
                        "format": { "type": "string", "enum": ["bibtex", "csl_json", "apa"] }
                    },
                    "required": ["item_key"],
                    "additionalProperties": false
                }),
            },
            ToolDef {
                id: "zotero_get_fulltext",
                native_name: "zotero_get_fulltext",
                mcp_name: "",
                description: "Get indexed fulltext fallback for a Zotero item (text only, no images) and resolve canonical document sources (arXiv/PDF URL/local path).",
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
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
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
                mcp_name: "",
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

    #[cfg(feature = "hackernews")]
    {
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
    }

    #[cfg(feature = "patents")]
    {
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
    fn zotero_get_tags_schema_exposes_pagination_fields() {
        assert_schema_has_field("zotero_get_tags", "offset");
        assert_schema_has_field("zotero_get_tags", "limit");
    }

    #[test]
    fn zotero_get_recent_schema_exposes_sorting_and_filters() {
        assert_schema_has_field("zotero_get_recent", "item_type");
        assert_schema_has_field("zotero_get_recent", "sort_by");
        assert_schema_has_field("zotero_get_recent", "max_chars_per_item");
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
    fn zotero_get_item_schema_exposes_enrichment_flags() {
        assert_schema_has_field("zotero_get_item", "include_attachments");
        assert_schema_has_field("zotero_get_item", "include_fulltext_resolution");
    }

    #[test]
    fn zotero_get_item_citation_schema_exposes_format_controls() {
        assert_schema_has_field("zotero_get_item_citation", "format");
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
