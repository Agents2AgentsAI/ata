use serde_json::Value;
use serde_json::json;

/// Tool definition with schema
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: &'static str,
    pub native_name: &'static str,
    pub mcp_name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// Get all data tool definitions
#[must_use]
pub fn all_tool_defs() -> Vec<ToolDef> {
    let mut defs = Vec::new();

    #[cfg(feature = "huggingface")]
    {
        defs.extend(huggingface_tools());
    }

    #[cfg(feature = "kaggle")]
    {
        defs.extend(kaggle_tools());
    }

    // Common tools that work across sources
    defs.extend(common_tools());

    defs
}

fn common_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            id: "dataset_search",
            native_name: "dataset_search",
            mcp_name: "search_datasets",
            description: "Search for datasets across HuggingFace and Kaggle. Returns dataset metadata including name, description, size, download count, and tags.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query string"
                    },
                    "source": {
                        "type": "string",
                        "enum": ["huggingface", "kaggle"],
                        "description": "Filter to a specific source. If not specified, searches all configured sources."
                    },
                    "author": {
                        "type": "string",
                        "description": "Filter by dataset author/owner"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter by tags (e.g., 'image', 'nlp', 'tabular')"
                    },
                    "modality": {
                        "type": "string",
                        "enum": ["image", "video", "audio", "text", "tabular", "point_cloud"],
                        "description": "Filter by data modality"
                    },
                    "sort_by": {
                        "type": "string",
                        "enum": ["downloads", "likes", "updated", "created"],
                        "description": "Sort order for results"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Maximum number of results to return (default: 20)"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "dataset_get",
            native_name: "dataset_get",
            mcp_name: "get_dataset",
            description: "Get detailed information about a specific dataset including description, files, splits, and metadata. Dataset ID format: 'owner/name' for HuggingFace, 'kaggle:owner/name' for Kaggle.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dataset_id": {
                        "type": "string",
                        "description": "Dataset identifier. Format: 'owner/name' (HuggingFace) or 'kaggle:owner/name' (Kaggle)"
                    },
                    "include_files": {
                        "type": "boolean",
                        "description": "Whether to include file listing (default: true)"
                    },
                    "include_readme": {
                        "type": "boolean",
                        "description": "Whether to include README content (default: true)"
                    }
                },
                "required": ["dataset_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "dataset_list_files",
            native_name: "dataset_list_files",
            mcp_name: "list_dataset_files",
            description: "List all files in a dataset with their sizes. Useful for understanding dataset structure before downloading.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dataset_id": {
                        "type": "string",
                        "description": "Dataset identifier"
                    },
                    "path": {
                        "type": "string",
                        "description": "Filter to files under this path"
                    },
                    "revision": {
                        "type": "string",
                        "description": "Git revision/branch (default: 'main')"
                    }
                },
                "required": ["dataset_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "dataset_download",
            native_name: "dataset_download",
            mcp_name: "download_dataset",
            description: "Download a dataset to a local directory. Can download entire dataset or specific files.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dataset_id": {
                        "type": "string",
                        "description": "Dataset identifier"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Local directory to download to"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific files to download. If not specified, downloads all files."
                    },
                    "revision": {
                        "type": "string",
                        "description": "Git revision/branch (default: 'main')"
                    }
                },
                "required": ["dataset_id", "output_path"],
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(feature = "huggingface")]
fn huggingface_tools() -> Vec<ToolDef> {
    vec![ToolDef {
        id: "hf_dataset_info",
        native_name: "hf_dataset_info",
        mcp_name: "get_huggingface_dataset",
        description: "Get HuggingFace-specific dataset information including dataset card, configs, and splits.",
        input_schema: json!({
            "type": "object",
            "properties": {
                "repo_id": {
                    "type": "string",
                    "description": "HuggingFace dataset repository ID (e.g., 'squad', 'openai/webgpt_comparisons')"
                },
                "revision": {
                    "type": "string",
                    "description": "Git revision (default: 'main')"
                }
            },
            "required": ["repo_id"],
            "additionalProperties": false
        }),
    }]
}

#[cfg(feature = "kaggle")]
fn kaggle_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            id: "kaggle_dataset_info",
            native_name: "kaggle_dataset_info",
            mcp_name: "get_kaggle_dataset",
            description: "Get Kaggle-specific dataset information including usability rating, file types, and column info.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "owner": {
                        "type": "string",
                        "description": "Dataset owner username"
                    },
                    "slug": {
                        "type": "string",
                        "description": "Dataset slug/name"
                    }
                },
                "required": ["owner", "slug"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kaggle_competitions",
            native_name: "kaggle_competitions",
            mcp_name: "list_kaggle_competitions",
            description: "List Kaggle competitions with their datasets.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "search": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["featured", "research", "getting-started", "masters", "playground"],
                        "description": "Competition category"
                    },
                    "sort_by": {
                        "type": "string",
                        "enum": ["deadline", "prize", "numberOfTeams"],
                        "description": "Sort order"
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kaggle_competition_list_files",
            native_name: "kaggle_competition_list_files",
            mcp_name: "list_kaggle_competition_files",
            description: "List files available for a Kaggle competition, including file sizes.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "competition": {
                        "type": "string",
                        "description": "Kaggle competition slug (for example, 'titanic')"
                    }
                },
                "required": ["competition"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            id: "kaggle_competition_download",
            native_name: "kaggle_competition_download",
            mcp_name: "download_kaggle_competition_data",
            description: "Download one or more files from a Kaggle competition to a local directory.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "competition": {
                        "type": "string",
                        "description": "Kaggle competition slug (for example, 'titanic')"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Local directory where competition files will be downloaded"
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Specific files to download. If omitted, all competition files are downloaded."
                    }
                },
                "required": ["competition", "output_path"],
                "additionalProperties": false
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_defs_returns_tools() {
        let defs = all_tool_defs();
        assert!(!defs.is_empty());

        // Check that dataset_search is always present
        assert!(defs.iter().any(|d| d.id == "dataset_search"));
    }

    #[test]
    fn tool_ids_are_unique() {
        let defs = all_tool_defs();
        let mut ids: Vec<&str> = defs.iter().map(|d| d.id).collect();
        ids.sort();
        let original_len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), original_len, "Duplicate tool IDs found");
    }

    #[test]
    fn tool_schemas_are_valid_json() {
        let defs = all_tool_defs();
        for def in defs {
            assert!(
                def.input_schema.is_object(),
                "Tool {} schema is not an object",
                def.id
            );
            assert!(
                def.input_schema.get("type").is_some(),
                "Tool {} schema missing 'type'",
                def.id
            );
        }
    }
}
