//! Minimal `ToolSpec::Function` builders for the agent-direct data tools
//! registered via [`crate::tools::spec_plan::add_data_tools`].
//!
//! Each spec is a thin wrapper around the toolkit's typed input params; the
//! `DataBridgeHandler` handles the actual parsing and dispatch.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const DATASET_SEARCH_TOOL_NAME: &str = "dataset_search";
pub const DATASET_GET_TOOL_NAME: &str = "dataset_get";

// @agent-facing
pub fn create_dataset_search_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::string(Some(
                "Required. Free-text search query for datasets.".to_string(),
            )),
        ),
        (
            "source".to_string(),
            JsonSchema::string(Some(
                "Optional. Filter to a specific source. Supported: `huggingface`, `kaggle`. Kaggle paths require `KAGGLE_USERNAME` + `KAGGLE_KEY` credentials; HuggingFace works without auth for public datasets."
                    .to_string(),
            )),
        ),
        (
            "limit".to_string(),
            JsonSchema::number(Some(
                "Optional. Max number of results to return. Defaults to a small list.".to_string(),
            )),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: DATASET_SEARCH_TOOL_NAME.to_string(),
        description: r#"Search public dataset catalogs (HuggingFace; optionally Kaggle) for datasets matching a query.

Use when:
- The user asks "find a dataset for X", "are there public datasets about Y?", or wants to discover a corpus before downloading it.
- You need a dataset id to feed into `dataset_get`.

Returns a list of matches with id, author, downloads, description snippet, and tags. No bytes are downloaded — this is metadata only. Use `dataset_get` to inspect a specific dataset by id."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["query".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_dataset_get_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "dataset_id".to_string(),
        JsonSchema::string(Some(
            "Required. Dataset identifier as returned by `dataset_search` (e.g. `squad`, `imdb`, `huggingface/dataset-name`).".to_string(),
        )),
    ), (
        "source".to_string(),
        JsonSchema::string(Some(
            "Optional. `huggingface` (default) or `kaggle`. Kaggle requires credentials.".to_string(),
        )),
    )]);
    ToolSpec::Function(ResponsesApiTool {
        name: DATASET_GET_TOOL_NAME.to_string(),
        description: r#"Fetch detailed metadata for a single dataset by id.

Use when:
- You already have a dataset id (from `dataset_search`) and need fields like full description, size on disk, file list, license, or modality.
- Before downloading, to confirm size / license fit user constraints.

Returns dataset detail including description, size_bytes, downloads, tags, files (if available), and license. No bytes are downloaded — call `dataset_list_files` / `dataset_download` for that."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["dataset_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
