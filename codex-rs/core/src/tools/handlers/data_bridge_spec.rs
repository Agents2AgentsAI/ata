//! Minimal `ToolSpec::Function` builders for the agent-direct data tools
//! registered via [`crate::tools::spec_plan::add_data_tools`].
//!
//! Each spec is a thin wrapper around the toolkit's typed input params; the
//! `DataBridgeHandler` parses and dispatches against the real typed struct
//! from `codex_data_tools::types::*`, so the param shape here uses
//! `additionalProperties: true` to let the agent pass the full typed shape
//! without mirroring every optional field.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

// ---------- Helper ----------

fn function_tool(
    name: &str,
    description: &str,
    required: &[&str],
    properties: &[(&str, JsonSchema)],
) -> ToolSpec {
    let params_map: BTreeMap<String, JsonSchema> = properties
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    let required_owned: Vec<String> = required.iter().map(|s| (*s).to_string()).collect();
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(params_map, Some(required_owned), Some(true.into())),
        output_schema: None,
    })
}

fn string_prop(description: &str) -> JsonSchema {
    JsonSchema::string(Some(description.to_string()))
}
fn integer_prop(description: &str) -> JsonSchema {
    JsonSchema::integer(Some(description.to_string()))
}
fn string_array_prop(description: &str) -> JsonSchema {
    JsonSchema::array(JsonSchema::string(None), Some(description.to_string()))
}

// ---------- Names ----------

pub const DATASET_SEARCH_TOOL_NAME: &str = "dataset_search";
pub const DATASET_GET_TOOL_NAME: &str = "dataset_get";
pub const DATASET_LIST_FILES_TOOL_NAME: &str = "dataset_list_files";
pub const DATASET_DOWNLOAD_TOOL_NAME: &str = "dataset_download";
pub const HF_DATASET_INFO_TOOL_NAME: &str = "hf_dataset_info";
pub const KAGGLE_DATASET_INFO_TOOL_NAME: &str = "kaggle_dataset_info";
pub const KAGGLE_COMPETITIONS_TOOL_NAME: &str = "kaggle_competitions";
pub const KAGGLE_COMPETITION_LIST_FILES_TOOL_NAME: &str = "kaggle_competition_list_files";
pub const KAGGLE_COMPETITION_DOWNLOAD_TOOL_NAME: &str = "kaggle_competition_download";

// ---------- Dataset search / get ----------

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

// ---------- HuggingFace file ops ----------

// @agent-facing
pub fn create_dataset_list_files_tool() -> ToolSpec {
    function_tool(
        DATASET_LIST_FILES_TOOL_NAME,
        r#"List files in a HuggingFace dataset repository without downloading bytes.

Use when:
- You need to see what files a dataset contains before deciding to download.
- The user asks "what's in dataset X?" or wants to know the partition layout.

Returns a list of `{path, size_bytes}` entries. Optionally filter to a subdirectory with `path`, or pin to a `revision` (branch/tag/commit SHA)."#,
        &["dataset_id"],
        &[
            (
                "dataset_id",
                string_prop(
                    "Required. HuggingFace dataset id (e.g. `squad`, `huggingface/dataset-name`).",
                ),
            ),
            (
                "path",
                string_prop(
                    "Optional. Subdirectory within the repo to list (e.g. `data/`). Defaults to repo root.",
                ),
            ),
            (
                "revision",
                string_prop(
                    "Optional. Branch, tag, or commit SHA to pin the listing to. Defaults to main.",
                ),
            ),
        ],
    )
}

// @agent-facing
pub fn create_dataset_download_tool() -> ToolSpec {
    function_tool(
        DATASET_DOWNLOAD_TOOL_NAME,
        r#"Download files from a HuggingFace dataset to a local directory.

Use when:
- The user explicitly asked to download a dataset (or a subset of its files) to disk.
- A prior `dataset_list_files` / `dataset_get` confirmed the files and size fit the user's constraints.

If `files` is omitted, all files in the dataset are downloaded — confirm size with `dataset_list_files` first for large datasets. Returns count and total bytes downloaded; the bytes land at `output_path`."#,
        &["dataset_id", "output_path"],
        &[
            (
                "dataset_id",
                string_prop("Required. HuggingFace dataset id."),
            ),
            (
                "output_path",
                string_prop("Required. Local directory to write files into. Created if missing."),
            ),
            (
                "files",
                string_array_prop(
                    "Optional. Subset of file paths (as returned by `dataset_list_files`) to download. Defaults to all files.",
                ),
            ),
            (
                "revision",
                string_prop("Optional. Branch, tag, or commit SHA. Defaults to main."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_hf_dataset_info_tool() -> ToolSpec {
    function_tool(
        HF_DATASET_INFO_TOOL_NAME,
        r#"Fetch HuggingFace-specific dataset metadata (alias for `dataset_get` with `source=huggingface`).

Use when:
- You specifically need HuggingFace metadata and want to avoid an ambiguous source lookup.
- The user mentions a HuggingFace dataset by name and you want extended detail (README, splits, licenses).

Returns the same shape as `dataset_get`. Prefer `dataset_get` for cross-source code paths; use this when the source is unambiguously HuggingFace."#,
        &["dataset_id"],
        &[
            (
                "dataset_id",
                string_prop("Required. HuggingFace dataset id (e.g. `squad`)."),
            ),
            (
                "include_files",
                JsonSchema::boolean(Some(
                    "Optional. If true, include the file list in the response.".to_string(),
                )),
            ),
            (
                "include_readme",
                JsonSchema::boolean(Some(
                    "Optional. If true, include the dataset README.".to_string(),
                )),
            ),
        ],
    )
}

// ---------- Kaggle ops (credentials required) ----------

// @agent-facing
pub fn create_kaggle_dataset_info_tool() -> ToolSpec {
    function_tool(
        KAGGLE_DATASET_INFO_TOOL_NAME,
        r#"Fetch metadata for a Kaggle dataset by id.

Use when:
- The user references a Kaggle dataset (e.g. `owner/dataset-slug`) and you need description, size, files, or license.
- Before downloading, to confirm fit.

Requires `KAGGLE_USERNAME` + `KAGGLE_KEY` env vars. Returns Kaggle dataset detail. Returns an error if credentials are missing — surface it to the user; do not retry blindly."#,
        &["dataset_id"],
        &[
            (
                "dataset_id",
                string_prop(
                    "Required. Kaggle dataset id in `owner/slug` form (e.g. `zillow/zecon`).",
                ),
            ),
            (
                "include_files",
                JsonSchema::boolean(Some(
                    "Optional. If true, include the file list.".to_string(),
                )),
            ),
            (
                "include_readme",
                JsonSchema::boolean(Some(
                    "Optional. If true, include the dataset README/description.".to_string(),
                )),
            ),
        ],
    )
}

// @agent-facing
pub fn create_kaggle_competitions_tool() -> ToolSpec {
    function_tool(
        KAGGLE_COMPETITIONS_TOOL_NAME,
        r#"List or search Kaggle competitions.

Use when:
- The user asks "what Kaggle competitions are about X?" or "find me an active NLP competition".
- You need a competition id to feed into `kaggle_competition_list_files` / `kaggle_competition_download`.

Requires `KAGGLE_USERNAME` + `KAGGLE_KEY`. Returns competition entries with id, title, deadline, reward, organization, and evaluation metric. Filter with `search` (free text), `category`, or `sort_by`."#,
        &[],
        &[
            (
                "search",
                string_prop(
                    "Optional. Free-text query to filter competitions by title/description.",
                ),
            ),
            (
                "category",
                string_prop("Optional. Kaggle category filter (e.g. `featured`, `research`)."),
            ),
            (
                "sort_by",
                string_prop(
                    "Optional. Sort key (e.g. `latestDeadline`, `prize`, `numberOfTeams`).",
                ),
            ),
            (
                "limit",
                integer_prop("Optional. Max competitions to return."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_kaggle_competition_list_files_tool() -> ToolSpec {
    function_tool(
        KAGGLE_COMPETITION_LIST_FILES_TOOL_NAME,
        r#"List files attached to a Kaggle competition.

Use when:
- You have a competition id (from `kaggle_competitions`) and need to see its data files before downloading.
- The user asks "what data does competition X provide?".

Requires `KAGGLE_USERNAME` + `KAGGLE_KEY` AND that the user has accepted the competition's rules on kaggle.com — otherwise the API returns 403. Returns `{path, size_bytes}` entries. No bytes downloaded."#,
        &["competition"],
        &[(
            "competition",
            string_prop("Required. Kaggle competition id/slug (e.g. `titanic`)."),
        )],
    )
}

// @agent-facing
pub fn create_kaggle_competition_download_tool() -> ToolSpec {
    function_tool(
        KAGGLE_COMPETITION_DOWNLOAD_TOOL_NAME,
        r#"Download files from a Kaggle competition to a local directory.

Use when:
- The user explicitly asked to download competition data.
- `kaggle_competition_list_files` confirmed size/contents fit constraints.

Requires `KAGGLE_USERNAME` + `KAGGLE_KEY` AND prior rules acceptance on kaggle.com. If `files` is omitted, all files are downloaded. Returns count and total bytes; bytes land at `output_path`."#,
        &["competition", "output_path"],
        &[
            (
                "competition",
                string_prop("Required. Kaggle competition id/slug."),
            ),
            (
                "output_path",
                string_prop("Required. Local directory to write files into. Created if missing."),
            ),
            (
                "files",
                string_array_prop(
                    "Optional. Subset of file paths to download. Defaults to all files.",
                ),
            ),
        ],
    )
}
