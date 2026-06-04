//! Tool spec builders for the agent-direct research tools registered via
//! [`crate::tools::spec_plan::add_research_tools`].
//!
//! Each builder returns a minimal `ToolSpec::Function`. Param shape is
//! described loosely with `additionalProperties: true` so the agent can pass
//! the full typed shape from `codex_research_tools::types::*` without having
//! to mirror every optional field here; the
//! [`ResearchBridgeHandler`](super::research::ResearchBridgeHandler) calls
//! `parse_arguments` against the real typed struct on dispatch, so wrong
//! shapes surface as an error the model can correct on retry.

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
fn boolean_prop(description: &str) -> JsonSchema {
    JsonSchema::boolean(Some(description.to_string()))
}
fn string_array_prop(description: &str) -> JsonSchema {
    JsonSchema::array(JsonSchema::string(None), Some(description.to_string()))
}

// ---------- Names ----------

pub const PAPER_SEARCH_TOOL_NAME: &str = "paper_search";
pub const PAPER_GET_TOOL_NAME: &str = "paper_get";
pub const PAPER_CITATIONS_TOOL_NAME: &str = "paper_citations";
pub const PAPER_REFERENCES_TOOL_NAME: &str = "paper_references";
pub const PAPER_RECOMMENDATIONS_TOOL_NAME: &str = "paper_recommendations";

pub const HN_SEARCH_TOOL_NAME: &str = "hn_search";
pub const HN_GET_THREAD_TOOL_NAME: &str = "hn_get_thread";

pub const PATENT_SEARCH_TOOL_NAME: &str = "patent_search";
pub const PATENT_GET_TOOL_NAME: &str = "patent_get";

pub const ZOTERO_SEARCH_TOOL_NAME: &str = "zotero_search";
pub const ZOTERO_GET_TAGS_TOOL_NAME: &str = "zotero_get_tags";
pub const ZOTERO_GET_RECENT_TOOL_NAME: &str = "zotero_get_recent";
pub const ZOTERO_ADVANCED_SEARCH_TOOL_NAME: &str = "zotero_advanced_search";
pub const ZOTERO_GREP_TEXT_TOOL_NAME: &str = "zotero_grep_text";
pub const ZOTERO_SEARCH_NOTES_TOOL_NAME: &str = "zotero_search_notes";
pub const ZOTERO_GET_ITEM_TOOL_NAME: &str = "zotero_get_item";
pub const ZOTERO_GET_ITEM_CITATION_TOOL_NAME: &str = "zotero_get_item_citation";
pub const ZOTERO_GET_FULLTEXT_TOOL_NAME: &str = "zotero_get_fulltext";
pub const ZOTERO_GET_NOTES_TOOL_NAME: &str = "zotero_get_notes";
pub const ZOTERO_GET_ANNOTATIONS_TOOL_NAME: &str = "zotero_get_annotations";
pub const ZOTERO_GET_ATTACHMENTS_TOOL_NAME: &str = "zotero_get_attachments";
pub const ZOTERO_GET_COLLECTIONS_TOOL_NAME: &str = "zotero_get_collections";
pub const ZOTERO_LIST_GROUPS_TOOL_NAME: &str = "zotero_list_groups";
pub const ZOTERO_GET_COLLECTION_ITEMS_TOOL_NAME: &str = "zotero_get_collection_items";
pub const ZOTERO_CREATE_COLLECTION_TOOL_NAME: &str = "zotero_create_collection";
pub const ZOTERO_FIND_OR_CREATE_COLLECTION_TOOL_NAME: &str = "zotero_find_or_create_collection";
pub const ZOTERO_CREATE_ITEMS_TOOL_NAME: &str = "zotero_create_items";
pub const ZOTERO_UPDATE_ITEMS_TOOL_NAME: &str = "zotero_update_items";
pub const ZOTERO_ADD_ITEMS_TO_COLLECTION_TOOL_NAME: &str = "zotero_add_items_to_collection";
pub const ZOTERO_CREATE_ATTACHMENT_LINK_TOOL_NAME: &str = "zotero_create_attachment_link";

pub const REPO_CLONE_AND_SUMMARIZE_TOOL_NAME: &str = "repo_clone_and_summarize";
pub const REPO_FIND_MODELS_TOOL_NAME: &str = "repo_find_models";
pub const REPO_EXTRACT_REQUIREMENTS_TOOL_NAME: &str = "repo_extract_requirements";
pub const REPO_FIND_ENTRYPOINTS_TOOL_NAME: &str = "repo_find_entrypoints";
pub const REPO_EXTRACT_IO_SHAPES_TOOL_NAME: &str = "repo_extract_io_shapes";
pub const REPO_GET_HEALTH_TOOL_NAME: &str = "repo_get_health";
pub const REPO_FIND_EXPORT_PATHS_TOOL_NAME: &str = "repo_find_export_paths";
pub const REPO_EXTRACT_CONFIG_SCHEMA_TOOL_NAME: &str = "repo_extract_config_schema";
pub const REPO_DIFF_REQUIREMENTS_TOOL_NAME: &str = "repo_diff_requirements";

// ---------- Paper search family (Feature::ResearchPaperSearch) ----------

// @agent-facing
pub fn create_paper_search_tool() -> ToolSpec {
    function_tool(
        PAPER_SEARCH_TOOL_NAME,
        "Search academic literature (Semantic Scholar, arXiv, OpenAlex) for papers matching a query. Returns matches with title, authors, year, venue, citation count, ids (DOI, arXiv, S2), and optionally abstract. No PDFs are downloaded.",
        &["query"],
        &[
            (
                "query",
                string_prop("Required. Free-text search query for academic papers."),
            ),
            (
                "year_from",
                integer_prop("Optional. Earliest publication year (inclusive)."),
            ),
            (
                "year_to",
                integer_prop("Optional. Latest publication year (inclusive)."),
            ),
            (
                "limit",
                integer_prop("Optional. Max results. Defaults to a small list."),
            ),
            (
                "include_abstract",
                boolean_prop("Optional. Include abstracts when true."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_paper_get_tool() -> ToolSpec {
    function_tool(
        PAPER_GET_TOOL_NAME,
        "Get detailed paper information by DOI, arXiv ID, or Semantic Scholar ID. Use after paper_search to fetch full metadata + abstract.",
        &["paper_id"],
        &[(
            "paper_id",
            string_prop("DOI, arXiv id, or Semantic Scholar id."),
        )],
    )
}

// @agent-facing
pub fn create_paper_citations_tool() -> ToolSpec {
    function_tool(
        PAPER_CITATIONS_TOOL_NAME,
        "List papers that cite a given paper. Paginated; pass offset+limit to walk the list.",
        &["paper_id"],
        &[
            (
                "paper_id",
                string_prop("DOI, arXiv id, or Semantic Scholar id."),
            ),
            ("offset", integer_prop("Optional. Pagination offset.")),
            ("limit", integer_prop("Optional. Page size.")),
            (
                "fields",
                string_array_prop("Optional. Subset of fields to return."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_paper_references_tool() -> ToolSpec {
    function_tool(
        PAPER_REFERENCES_TOOL_NAME,
        "List papers cited BY a given paper (its bibliography). Paginated.",
        &["paper_id"],
        &[
            (
                "paper_id",
                string_prop("DOI, arXiv id, or Semantic Scholar id."),
            ),
            ("offset", integer_prop("Optional. Pagination offset.")),
            ("limit", integer_prop("Optional. Page size.")),
            (
                "fields",
                string_array_prop("Optional. Subset of fields to return."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_paper_recommendations_tool() -> ToolSpec {
    function_tool(
        PAPER_RECOMMENDATIONS_TOOL_NAME,
        "Recommend papers similar to a positive set (and optionally dissimilar to a negative set). Backed by Semantic Scholar's recommendations API.",
        &["positive_paper_ids"],
        &[
            (
                "positive_paper_ids",
                string_array_prop("Required. Paper ids to seed the recommendation."),
            ),
            (
                "negative_paper_ids",
                string_array_prop("Optional. Paper ids to push the recs away from."),
            ),
            ("limit", integer_prop("Optional. Max recommendations.")),
        ],
    )
}

// ---------- Hacker News (Feature::ResearchHackerNews) ----------

// @agent-facing
pub fn create_hn_search_tool() -> ToolSpec {
    function_tool(
        HN_SEARCH_TOOL_NAME,
        "Search Hacker News stories and comments via the Algolia HN API. Returns hits with title, url, author, points, num_comments, created_at, and story_id.",
        &["query"],
        &[
            ("query", string_prop("Free-text search query.")),
            (
                "tags",
                string_prop(
                    "Optional. Algolia HN tag filter, e.g. `story`, `comment`, `(story,front_page)`.",
                ),
            ),
            ("limit", integer_prop("Optional. Max hits.")),
        ],
    )
}

// @agent-facing
pub fn create_hn_get_thread_tool() -> ToolSpec {
    function_tool(
        HN_GET_THREAD_TOOL_NAME,
        "Fetch a full Hacker News discussion thread (story + its descendant comments) by story id, returned as nested JSON.",
        &["story_id"],
        &[
            (
                "story_id",
                string_prop("HN story id (the numeric id from `hn_search` results)."),
            ),
            (
                "max_depth",
                integer_prop("Optional. Limit comment-tree depth."),
            ),
        ],
    )
}

// ---------- Patents (Feature::ResearchPatents) ----------

// @agent-facing
pub fn create_patent_search_tool() -> ToolSpec {
    function_tool(
        PATENT_SEARCH_TOOL_NAME,
        "Search worldwide patents via the EPO Open Patent Services API. Requires EPO_CONSUMER_KEY + EPO_CONSUMER_SECRET (or the [research] toml equivalents). Returns publication ids, titles, dates, applicants.",
        &["query"],
        &[
            ("query", string_prop("Free-text search query.")),
            ("limit", integer_prop("Optional. Max results.")),
            ("offset", integer_prop("Optional. Pagination offset.")),
        ],
    )
}

// @agent-facing
pub fn create_patent_get_tool() -> ToolSpec {
    function_tool(
        PATENT_GET_TOOL_NAME,
        "Fetch a single patent's full bibliographic + abstract record by publication id. EPO credentials required.",
        &["publication_id"],
        &[(
            "publication_id",
            string_prop("EPO publication id (e.g. `EP1234567`)."),
        )],
    )
}

// ---------- Zotero (Feature::ResearchZotero) — 21 tools ----------

// @agent-facing
pub fn create_zotero_search_tool() -> ToolSpec {
    function_tool(
        ZOTERO_SEARCH_TOOL_NAME,
        "Search the user's Zotero library by free-text query. Requires ZOTERO_API_KEY + ZOTERO_USER_ID (or [research] toml).",
        &["query"],
        &[
            ("query", string_prop("Free-text search query.")),
            ("limit", integer_prop("Optional. Max hits.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_get_tags_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_TAGS_TOOL_NAME,
        "List tags across the user's Zotero library.",
        &[],
        &[("limit", integer_prop("Optional. Max tags."))],
    )
}

// @agent-facing
pub fn create_zotero_get_recent_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_RECENT_TOOL_NAME,
        "List the user's recently added Zotero items, newest first.",
        &[],
        &[("limit", integer_prop("Optional. Max items."))],
    )
}

// @agent-facing
pub fn create_zotero_advanced_search_tool() -> ToolSpec {
    function_tool(
        ZOTERO_ADVANCED_SEARCH_TOOL_NAME,
        "Advanced Zotero search with structured filters (year range, item type, tags, authors).",
        &[],
        &[
            ("query", string_prop("Optional. Free-text query.")),
            ("tags", string_array_prop("Optional. Filter by tags.")),
            (
                "item_type",
                string_prop("Optional. e.g. `journalArticle`, `book`."),
            ),
            ("year_from", integer_prop("Optional.")),
            ("year_to", integer_prop("Optional.")),
            ("limit", integer_prop("Optional.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_grep_text_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GREP_TEXT_TOOL_NAME,
        "Grep across the full-text content of the user's Zotero attachments. Use when you need to find a phrase across multiple PDFs.",
        &["pattern"],
        &[
            ("pattern", string_prop("Required. Regex or literal phrase.")),
            ("limit", integer_prop("Optional. Max matches.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_search_notes_tool() -> ToolSpec {
    function_tool(
        ZOTERO_SEARCH_NOTES_TOOL_NAME,
        "Search Zotero notes (free-form text notes attached to items) by query.",
        &["query"],
        &[
            ("query", string_prop("Required. Free-text query.")),
            ("limit", integer_prop("Optional.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_get_item_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_ITEM_TOOL_NAME,
        "Fetch a single Zotero item's full metadata by item key.",
        &["item_key"],
        &[(
            "item_key",
            string_prop("Required. Zotero item key (8-char identifier)."),
        )],
    )
}

// @agent-facing
pub fn create_zotero_get_item_citation_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_ITEM_CITATION_TOOL_NAME,
        "Render a Zotero item as a formatted citation in a given style (e.g. APA, MLA).",
        &["item_key"],
        &[
            ("item_key", string_prop("Required. Zotero item key.")),
            (
                "style",
                string_prop("Optional. Citation style (default: `chicago-author-date`)."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_zotero_get_fulltext_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_FULLTEXT_TOOL_NAME,
        "Fetch the extracted full text of a Zotero item's attachment (PDF text body).",
        &["item_key"],
        &[("item_key", string_prop("Required. Zotero item key."))],
    )
}

// @agent-facing
pub fn create_zotero_get_notes_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_NOTES_TOOL_NAME,
        "Fetch the notes attached to a single Zotero item.",
        &["item_key"],
        &[("item_key", string_prop("Required. Zotero item key."))],
    )
}

// @agent-facing
pub fn create_zotero_get_annotations_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_ANNOTATIONS_TOOL_NAME,
        "Fetch reader annotations (highlights, sticky notes) attached to a Zotero item.",
        &["item_key"],
        &[
            ("item_key", string_prop("Required. Zotero item key.")),
            ("limit", integer_prop("Optional.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_get_attachments_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_ATTACHMENTS_TOOL_NAME,
        "List the attachments (PDFs, snapshots, etc.) belonging to a Zotero item.",
        &["item_key"],
        &[("item_key", string_prop("Required. Zotero item key."))],
    )
}

// @agent-facing
pub fn create_zotero_get_collections_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_COLLECTIONS_TOOL_NAME,
        "List collections (folders) in the user's Zotero library.",
        &[],
        &[("limit", integer_prop("Optional."))],
    )
}

// @agent-facing
pub fn create_zotero_list_groups_tool() -> ToolSpec {
    function_tool(
        ZOTERO_LIST_GROUPS_TOOL_NAME,
        "List the Zotero groups the user has access to (shared libraries).",
        &[],
        &[],
    )
}

// @agent-facing
pub fn create_zotero_get_collection_items_tool() -> ToolSpec {
    function_tool(
        ZOTERO_GET_COLLECTION_ITEMS_TOOL_NAME,
        "List items inside a specific Zotero collection.",
        &["collection_key"],
        &[
            (
                "collection_key",
                string_prop("Required. Zotero collection key."),
            ),
            ("limit", integer_prop("Optional.")),
        ],
    )
}

// @agent-facing
pub fn create_zotero_create_collection_tool() -> ToolSpec {
    function_tool(
        ZOTERO_CREATE_COLLECTION_TOOL_NAME,
        "Create a new Zotero collection (folder).",
        &["name"],
        &[
            ("name", string_prop("Required. New collection name.")),
            (
                "parent_collection_key",
                string_prop("Optional. Parent collection to nest under."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_zotero_find_or_create_collection_tool() -> ToolSpec {
    function_tool(
        ZOTERO_FIND_OR_CREATE_COLLECTION_TOOL_NAME,
        "Look up a Zotero collection by name (case-insensitive); create it if it does not exist. Returns the collection key.",
        &["name"],
        &[
            (
                "name",
                string_prop("Required. Collection name to find or create."),
            ),
            (
                "parent_collection_key",
                string_prop("Optional. Parent collection."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_zotero_create_items_tool() -> ToolSpec {
    function_tool(
        ZOTERO_CREATE_ITEMS_TOOL_NAME,
        "Create one or more Zotero items from supplied JSON (the same shape Zotero's import API expects).",
        &["items"],
        &[(
            "items",
            JsonSchema::array(
                JsonSchema::object(BTreeMap::new(), None, Some(true.into())),
                Some("Required. Array of item JSON objects.".to_string()),
            ),
        )],
    )
}

// @agent-facing
pub fn create_zotero_update_items_tool() -> ToolSpec {
    function_tool(
        ZOTERO_UPDATE_ITEMS_TOOL_NAME,
        "Patch existing Zotero items (each entry must include `key` and `version` plus the fields to update).",
        &["items"],
        &[(
            "items",
            JsonSchema::array(
                JsonSchema::object(BTreeMap::new(), None, Some(true.into())),
                Some("Required. Array of patch JSON objects.".to_string()),
            ),
        )],
    )
}

// @agent-facing
pub fn create_zotero_add_items_to_collection_tool() -> ToolSpec {
    function_tool(
        ZOTERO_ADD_ITEMS_TO_COLLECTION_TOOL_NAME,
        "Add one or more existing Zotero items to a collection.",
        &["collection_key", "item_keys"],
        &[
            (
                "collection_key",
                string_prop("Required. Target collection key."),
            ),
            (
                "item_keys",
                string_array_prop("Required. Item keys to add."),
            ),
        ],
    )
}

// @agent-facing
pub fn create_zotero_create_attachment_link_tool() -> ToolSpec {
    function_tool(
        ZOTERO_CREATE_ATTACHMENT_LINK_TOOL_NAME,
        "Attach a URL link to a Zotero parent item (e.g. for archival pages or papers without a downloadable PDF).",
        &["parent_item_key", "url"],
        &[
            (
                "parent_item_key",
                string_prop("Required. Item to attach to."),
            ),
            ("url", string_prop("Required. URL to link.")),
            (
                "title",
                string_prop("Optional. Display title for the link."),
            ),
        ],
    )
}

// ---------- Repo analysis family (Feature::ResearchRepoAnalysis) ----------
//
// These tools shallow-clone a public Git repository (GitHub or GitLab) into a
// process-local cache and run static analysis. The cache is bounded, LRU-evicted,
// and lives under `$TMPDIR/codex-research-repos`. Clone fetches are filtered
// (`--depth=1 --filter=blob:limit=1m`) and total checkout size is capped.

// @agent-facing
pub fn create_repo_clone_and_summarize_tool() -> ToolSpec {
    function_tool(
        REPO_CLONE_AND_SUMMARIZE_TOOL_NAME,
        r#"Shallow-clone a public Git repository and return a summary.

Use when:
- The user gives a `https://github.com/owner/repo` or `https://gitlab.com/owner/repo` URL and asks "what's in this repo?".
- You need a high-level map (directory tree, README snippet, key files) before any deeper analysis.

Returns: `{url, commit_sha?, directory_tree, readme_snippet?, key_files, total_files}`. The directory tree is depth-capped, the README is truncated to ~2000 chars, and the result is cached for 30 minutes."#,
        &["repo_url"],
        &[
            (
                "repo_url",
                string_prop(
                    "Required. https URL to a GitHub or GitLab repository (e.g. `https://github.com/owner/repo`). `/tree/<branch>` suffixes are accepted.",
                ),
            ),
            (
                "branch",
                string_prop(
                    "Optional. Branch, tag, or single ref to check out. Defaults to the repo's default branch.",
                ),
            ),
        ],
    )
}

// @agent-facing
pub fn create_repo_find_models_tool() -> ToolSpec {
    function_tool(
        REPO_FIND_MODELS_TOOL_NAME,
        r#"Find ML model class definitions (PyTorch / TensorFlow / JAX) in a public Git repo.

Use when:
- The user asks "what models are defined in this repo?" or "find the model class".
- You're orienting before diving into training/inference code.

Returns a list of `{file_path, class_name, line_number, context}` entries. Matches classes inheriting from `nn.Module`, `tf.keras.Model`, `flax.linen.Module`, or similar framework base classes. Results capped at 300 entries."#,
        &["repo_url"],
        &[
            (
                "repo_url",
                string_prop("Required. https URL to a GitHub or GitLab repository."),
            ),
            (
                "framework",
                string_prop(
                    "Optional. Constrain to one of: `pytorch`, `tensorflow`, `jax`. If omitted, all three are searched.",
                ),
            ),
        ],
    )
}

// @agent-facing
pub fn create_repo_extract_requirements_tool() -> ToolSpec {
    function_tool(
        REPO_EXTRACT_REQUIREMENTS_TOOL_NAME,
        r#"Extract Python dependency requirements from a public Git repo.

Use when:
- The user asks "what does this repo depend on?" or "what Python packages does it need?".
- Before running or reproducing the repo locally, to see if your environment is compatible.

Parses `requirements*.txt`, `pyproject.toml`, `setup.py`, `setup.cfg`, `Pipfile`, and `environment.yml`/`yaml`. Returns `{repo_url, dependencies, source_files}` where dependencies are deduplicated and rendered as `name<constraint>` (e.g. `torch>=2.2`)."#,
        &["repo_url"],
        &[(
            "repo_url",
            string_prop("Required. https URL to a GitHub or GitLab repository."),
        )],
    )
}

// @agent-facing
pub fn create_repo_find_entrypoints_tool() -> ToolSpec {
    function_tool(
        REPO_FIND_ENTRYPOINTS_TOOL_NAME,
        r#"Find executable entrypoint scripts (train / eval / infer / export) in a public Git repo.

Use when:
- The user asks "how do I run training?" or "where's the eval script?".
- You need to know which file to run before suggesting an invocation.

Returns `{file_path, kind, cli_args_summary?, config_file?}` per match. Detection combines filename heuristics (e.g. `train.py`, `infer.py`) with content checks (`__main__` guard + topic keywords). Each entry includes a CLI-args summary when the script uses argparse/click."#,
        &["repo_url"],
        &[
            (
                "repo_url",
                string_prop("Required. https URL to a GitHub or GitLab repository."),
            ),
            (
                "task_hint",
                string_prop(
                    "Optional. Constrain to one of: `train`, `eval`, `infer`, `export`. If omitted, all four are searched.",
                ),
            ),
        ],
    )
}

// @agent-facing
pub fn create_repo_extract_io_shapes_tool() -> ToolSpec {
    function_tool(
        REPO_EXTRACT_IO_SHAPES_TOOL_NAME,
        r#"Extract model input/output tensor shapes from a public Git repo's source.

Use when:
- The user asks "what input does this model take?" or "what's the output shape?".
- You need shape info to plan a benchmark, port, or conversion.

Returns `{input_shapes?, output_shapes?, dtype?, source_file, source_line}` per match. Greps for assignments like `input_size=...`, `image_size=...`, `output_shape=...`, `num_classes=...` across `.py/.yaml/.yml/.json/.toml`, plus `torch.randn(...)`/`np.zeros(...)` constructors."#,
        &["repo_url"],
        &[
            (
                "repo_url",
                string_prop("Required. https URL to a GitHub or GitLab repository."),
            ),
            (
                "model_class",
                string_prop(
                    "Optional. Only return shapes from files mentioning this class name, useful when a repo has multiple models.",
                ),
            ),
        ],
    )
}

// @agent-facing
pub fn create_repo_get_health_tool() -> ToolSpec {
    function_tool(
        REPO_GET_HEALTH_TOOL_NAME,
        r#"Fetch repository health signals from GitHub (no clone).

Use when:
- The user asks "is this repo maintained?" or "how active is it?".
- You want to assess whether a candidate dependency is safe to adopt.

Returns `{license?, last_commit_date?, stars, open_issues, releases_count, ci_passing?}`. Calls the GitHub REST API only — no clone, no rate-limited git operations. GitLab URLs are not supported by this tool; use `repo_clone_and_summarize` instead for those. With `GITHUB_TOKEN` set, allowed rate scales up; without it, falls back to anonymous limits."#,
        &["repo_url"],
        &[("repo_url", string_prop("Required. https GitHub URL."))],
    )
}

// @agent-facing
pub fn create_repo_find_export_paths_tool() -> ToolSpec {
    function_tool(
        REPO_FIND_EXPORT_PATHS_TOOL_NAME,
        r#"Find model-export pipelines (ONNX, TensorRT, TFLite, CoreML, OpenVINO, TorchScript) in a public Git repo.

Use when:
- The user asks "can this model be exported to ONNX?" or "is there a TensorRT path?".
- Before recommending a deployment route, to confirm one exists.

Returns `{file_path, target_format, framework_version?}` per match. `framework_version` is inferred from the repo's declared requirements (e.g. `torch>=2.2`). Greps for known export calls like `torch.onnx.export`, `tf.lite.TFLiteConverter`, `ct.convert`, `torch.jit.script`/`trace`."#,
        &["repo_url"],
        &[(
            "repo_url",
            string_prop("Required. https URL to a GitHub or GitLab repository."),
        )],
    )
}

// @agent-facing
pub fn create_repo_extract_config_schema_tool() -> ToolSpec {
    function_tool(
        REPO_EXTRACT_CONFIG_SCHEMA_TOOL_NAME,
        r#"Extract config-key schemas from a public Git repo's YAML/JSON/TOML/CFG files and argparse usage.

Use when:
- The user asks "what config options does this repo accept?" or "what CLI flags can I pass?".
- Before running a script, to surface available knobs.

Returns a list of `{config_file, format, key_params: [{name, default?, description?}]}`. `format` is one of `yaml`/`json`/`toml`/`cfg`/`argparse`. Each schema is capped at 200 keys; total schemas capped at 200."#,
        &["repo_url"],
        &[(
            "repo_url",
            string_prop("Required. https URL to a GitHub or GitLab repository."),
        )],
    )
}

// @agent-facing
pub fn create_repo_diff_requirements_tool() -> ToolSpec {
    function_tool(
        REPO_DIFF_REQUIREMENTS_TOOL_NAME,
        r#"Diff a public Git repo's Python requirements against a local requirements file.

Use when:
- The user asks "will my environment work with this repo?" or "what's missing locally?".
- Before installing, to surface conflicts vs. just-missing.

Returns `{repo_url, conflicts, missing_locally, compatible, repo_total_deps, local_total_deps}` where `conflicts` lists `name: repo requires X, local has Y` entries. `local_requirements_path` must be a path the agent can read locally (a `requirements*.txt`/`pyproject.toml`/etc.)."#,
        &["repo_url", "local_requirements_path"],
        &[
            (
                "repo_url",
                string_prop("Required. https URL to a GitHub or GitLab repository."),
            ),
            (
                "local_requirements_path",
                string_prop(
                    "Required. Local filesystem path to a requirements file to compare against.",
                ),
            ),
        ],
    )
}
