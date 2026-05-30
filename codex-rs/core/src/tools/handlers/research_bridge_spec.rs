//! Minimal `ToolSpec::Function` builders for the agent-direct research tools
//! registered via [`crate::tools::spec_plan::add_research_tools`].

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const PAPER_SEARCH_TOOL_NAME: &str = "paper_search";
pub const HN_SEARCH_TOOL_NAME: &str = "hn_search";

// @agent-facing
pub fn create_paper_search_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::string(Some(
                "Required. Free-text search query for academic papers.".to_string(),
            )),
        ),
        (
            "year_from".to_string(),
            JsonSchema::number(Some(
                "Optional. Earliest publication year (inclusive).".to_string(),
            )),
        ),
        (
            "year_to".to_string(),
            JsonSchema::number(Some(
                "Optional. Latest publication year (inclusive).".to_string(),
            )),
        ),
        (
            "limit".to_string(),
            JsonSchema::number(Some(
                "Optional. Max results. Defaults to a small list.".to_string(),
            )),
        ),
        (
            "include_abstract".to_string(),
            JsonSchema::boolean(Some(
                "Optional. Include abstracts in results when true.".to_string(),
            )),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: PAPER_SEARCH_TOOL_NAME.to_string(),
        description: r#"Search academic literature (Semantic Scholar, arXiv, OpenAlex) for papers matching a query.

Use when:
- The user asks "find papers about X", "what's the literature on Y?", or wants to ground a claim in cited work.
- You need a paper id to feed into a follow-up paper_get / paper_citations call.

Returns matches with title, authors, year, venue, citation count, ids (DOI, arXiv, S2), and optionally abstract. No PDFs are downloaded — this is metadata only."#
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
pub fn create_hn_search_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::string(Some(
                "Required. Free-text search query for Hacker News stories and comments.".to_string(),
            )),
        ),
        (
            "tags".to_string(),
            JsonSchema::string(Some(
                "Optional. Algolia HN tag filter, e.g. `story`, `comment`, `(story,front_page)`."
                    .to_string(),
            )),
        ),
        (
            "limit".to_string(),
            JsonSchema::number(Some(
                "Optional. Max hits to return. Defaults to a small list.".to_string(),
            )),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: HN_SEARCH_TOOL_NAME.to_string(),
        description: r#"Search Hacker News stories and comments via the Algolia HN API.

Use when:
- The user asks "what's the HN take on X?", "are people on HN discussing Y?", or wants developer-community context for a topic.
- You need current sentiment / discussion threads to supplement formal research.

Returns hits with title, url, author, points, num_comments, created_at, and story_id. Combine with `hn_get_thread` (if available) to read a specific discussion."#
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
