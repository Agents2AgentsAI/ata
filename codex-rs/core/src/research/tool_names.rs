use std::path::Path;

use crate::config::ResearchToolsToml;
use crate::features::Features;

use rmcp::model::Tool;
use std::collections::BTreeMap;

macro_rules! set_tool_name_for_id {
    ($self:ident, $id:ident, $resolved_name:ident) => {
        match $id {
            "paper_search" => $self.paper_search = $resolved_name,
            "paper_get" => $self.paper_get = $resolved_name,
            "paper_citations" => $self.paper_citations = $resolved_name,
            "paper_references" => $self.paper_references = $resolved_name,
            "paper_recommendations" => $self.paper_recommendations = $resolved_name,
            "zotero_search" => $self.zotero_search = $resolved_name,
            "zotero_get_tags" => $self.zotero_get_tags = $resolved_name,
            "zotero_get_recent" => $self.zotero_get_recent = $resolved_name,
            "zotero_advanced_search" => $self.zotero_advanced_search = $resolved_name,
            "zotero_grep_text" => $self.zotero_grep_text = $resolved_name,
            "zotero_search_notes" => $self.zotero_search_notes = $resolved_name,
            "zotero_get_item" => $self.zotero_get_item = $resolved_name,
            "zotero_get_item_citation" => $self.zotero_get_item_citation = $resolved_name,
            "zotero_get_fulltext" => $self.zotero_get_fulltext = $resolved_name,
            "zotero_get_notes" => $self.zotero_get_notes = $resolved_name,
            "zotero_get_annotations" => $self.zotero_get_annotations = $resolved_name,
            "zotero_get_attachments" => $self.zotero_get_attachments = $resolved_name,
            "zotero_get_collections" => $self.zotero_get_collections = $resolved_name,
            "zotero_list_groups" => $self.zotero_list_groups = $resolved_name,
            "zotero_get_collection_items" => $self.zotero_get_collection_items = $resolved_name,
            "repo_clone_and_summarize" => $self.repo_clone_and_summarize = $resolved_name,
            "repo_find_models" => $self.repo_find_models = $resolved_name,
            "repo_extract_requirements" => $self.repo_extract_requirements = $resolved_name,
            "repo_find_entrypoints" => $self.repo_find_entrypoints = $resolved_name,
            "repo_extract_io_shapes" => $self.repo_extract_io_shapes = $resolved_name,
            "repo_get_health" => $self.repo_get_health = $resolved_name,
            "repo_find_export_paths" => $self.repo_find_export_paths = $resolved_name,
            "repo_extract_config_schema" => $self.repo_extract_config_schema = $resolved_name,
            "repo_diff_requirements" => $self.repo_diff_requirements = $resolved_name,
            "hn_search" => $self.hn_search = $resolved_name,
            "hn_get_thread" => $self.hn_get_thread = $resolved_name,
            _ => {}
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResearchToolAvailability {
    pub has_paper_search: bool,
    pub has_zotero: bool,
    pub has_repo_analysis: bool,
    pub has_hackernews: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchToolContext {
    pub names: ResearchToolNames,
    pub availability: ResearchToolAvailability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchToolNames {
    pub paper_search: String,
    pub paper_get: String,
    pub paper_citations: String,
    pub paper_references: String,
    pub paper_recommendations: String,
    pub zotero_search: String,
    pub zotero_get_tags: String,
    pub zotero_get_recent: String,
    pub zotero_advanced_search: String,
    pub zotero_grep_text: String,
    pub zotero_search_notes: String,
    pub zotero_get_item: String,
    pub zotero_get_item_citation: String,
    pub zotero_get_fulltext: String,
    pub zotero_get_notes: String,
    pub zotero_get_annotations: String,
    pub zotero_get_attachments: String,
    pub zotero_get_collections: String,
    pub zotero_list_groups: String,
    pub zotero_get_collection_items: String,
    pub repo_clone_and_summarize: String,
    pub repo_find_models: String,
    pub repo_extract_requirements: String,
    pub repo_find_entrypoints: String,
    pub repo_extract_io_shapes: String,
    pub repo_get_health: String,
    pub repo_find_export_paths: String,
    pub repo_extract_config_schema: String,
    pub repo_diff_requirements: String,
    pub hn_search: String,
    pub hn_get_thread: String,
}

impl Default for ResearchToolNames {
    fn default() -> Self {
        Self {
            paper_search: "paper_search".to_string(),
            paper_get: "paper_get".to_string(),
            paper_citations: "paper_citations".to_string(),
            paper_references: "paper_references".to_string(),
            paper_recommendations: "paper_recommendations".to_string(),
            zotero_search: "zotero_search".to_string(),
            zotero_get_tags: "zotero_get_tags".to_string(),
            zotero_get_recent: "zotero_get_recent".to_string(),
            zotero_advanced_search: "zotero_advanced_search".to_string(),
            zotero_grep_text: "zotero_grep_text".to_string(),
            zotero_search_notes: "zotero_search_notes".to_string(),
            zotero_get_item: "zotero_get_item".to_string(),
            zotero_get_item_citation: "zotero_get_item_citation".to_string(),
            zotero_get_fulltext: "zotero_get_fulltext".to_string(),
            zotero_get_notes: "zotero_get_notes".to_string(),
            zotero_get_annotations: "zotero_get_annotations".to_string(),
            zotero_get_attachments: "zotero_get_attachments".to_string(),
            zotero_get_collections: "zotero_get_collections".to_string(),
            zotero_list_groups: "zotero_list_groups".to_string(),
            zotero_get_collection_items: "zotero_get_collection_items".to_string(),
            repo_clone_and_summarize: "repo_clone_and_summarize".to_string(),
            repo_find_models: "repo_find_models".to_string(),
            repo_extract_requirements: "repo_extract_requirements".to_string(),
            repo_find_entrypoints: "repo_find_entrypoints".to_string(),
            repo_extract_io_shapes: "repo_extract_io_shapes".to_string(),
            repo_get_health: "repo_get_health".to_string(),
            repo_find_export_paths: "repo_find_export_paths".to_string(),
            repo_extract_config_schema: "repo_extract_config_schema".to_string(),
            repo_diff_requirements: "repo_diff_requirements".to_string(),
            hn_search: "hn_search".to_string(),
            hn_get_thread: "hn_get_thread".to_string(),
        }
    }
}

impl ResearchToolNames {
    #[must_use]
    pub fn from_available_native() -> Self {
        let defs = codex_research_tools::tool_specs::all_tool_defs();
        Self::from_native(&defs)
    }

    pub fn from_native(defs: &[codex_research_tools::tool_specs::ToolDef]) -> Self {
        let mut names = Self::default();
        for def in defs {
            names.set_name_for_id(def.id, def.native_name.to_string());
        }
        names
    }

    pub fn from_mcp_tools(
        defs: &[codex_research_tools::tool_specs::ToolDef],
        mcp_tools: &BTreeMap<String, Tool>,
    ) -> Self {
        let mut names = Self::default();
        for def in defs {
            let matches = find_mcp_tool_matches(def.mcp_name, mcp_tools);
            if let Some(name) = matches.first() {
                names.set_name_for_id(def.id, name.clone());
            }
            if matches.len() > 1 {
                tracing::warn!(
                    mcp_name = def.mcp_name,
                    ?matches,
                    "multiple MCP tools match research tool; using first match",
                );
            }
        }
        names
    }

    fn set_name_for_id(&mut self, id: &str, resolved_name: String) {
        set_tool_name_for_id!(self, id, resolved_name);
    }
}

#[must_use]
pub fn native_tool_availability() -> ResearchToolAvailability {
    let defs = codex_research_tools::tool_specs::all_tool_defs();
    let has_paper_search = defs.iter().any(|def| def.id == "paper_search");
    let has_zotero = defs.iter().any(|def| def.id == "zotero_search");
    let has_repo_analysis = defs.iter().any(|def| def.id == "repo_find_entrypoints");
    let has_hackernews = defs.iter().any(|def| def.id == "hn_search");
    ResearchToolAvailability {
        has_paper_search,
        has_zotero,
        has_repo_analysis,
        has_hackernews,
    }
}

#[must_use]
pub fn configured_native_tool_context(
    research_toml: Option<&ResearchToolsToml>,
    codex_home: &Path,
    cwd: &Path,
    features: &Features,
) -> ResearchToolContext {
    let defs = codex_research_tools::tool_specs::all_tool_defs();
    let research_config =
        crate::tools::handlers::research::build_research_config(research_toml, codex_home, cwd);
    let toolkit =
        codex_research_tools::ResearchToolkit::new(reqwest::Client::new(), research_config);

    let mut names = ResearchToolNames::default();
    let mut availability = ResearchToolAvailability::default();

    for def in defs {
        if !toolkit.is_tool_configured(def.id) {
            continue;
        }
        if !features.is_research_tool_enabled(def.id) {
            continue;
        }

        names.set_name_for_id(def.id, def.native_name.to_string());
        match def.id {
            "paper_search" => availability.has_paper_search = true,
            "zotero_search" => availability.has_zotero = true,
            "repo_find_entrypoints" => availability.has_repo_analysis = true,
            "hn_search" => availability.has_hackernews = true,
            _ => {}
        }
    }

    ResearchToolContext {
        names,
        availability,
    }
}

pub(crate) fn find_mcp_tool_matches(
    mcp_name: &str,
    mcp_tools: &BTreeMap<String, Tool>,
) -> Vec<String> {
    mcp_tools
        .iter()
        .filter(|(qualified, tool)| {
            tool.name.as_ref() == mcp_name || qualified.split("__").last() == Some(mcp_name)
        })
        .map(|(qualified, _)| qualified.clone())
        .collect()
}

#[must_use]
pub(crate) fn should_suppress_research_mcp_tool(qualified_name: &str, features: &Features) -> bool {
    let tool_name = qualified_name.rsplit("__").next().unwrap_or(qualified_name);
    let tool_id = match tool_name {
        _ if tool_name.starts_with("paper_") => Some("paper_search"),
        _ if tool_name.starts_with("zotero_") => Some("zotero_search"),
        _ if tool_name.starts_with("hn_") => Some("hn_search"),
        _ if tool_name.starts_with("patent_") => Some("patent_search"),
        _ if tool_name.starts_with("repo_") => Some("repo_clone_and_summarize"),
        "search_papers"
        | "get_paper"
        | "get_citations"
        | "get_references"
        | "get_recommendations" => Some("paper_search"),
        "search_hackernews" | "get_hackernews_thread" => Some("hn_search"),
        "search_patents" | "get_patent" => Some("patent_search"),
        "clone_and_summarize"
        | "find_model_definitions"
        | "extract_requirements"
        | "find_entrypoints"
        | "extract_io_shapes"
        | "get_repo_health"
        | "find_export_paths"
        | "extract_config_schema"
        | "diff_requirements" => Some("repo_clone_and_summarize"),
        _ => None,
    };

    tool_id.is_some_and(|id| !features.is_research_tool_enabled(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::Feature;
    use codex_research_tools::tool_specs::all_tool_defs;
    use pretty_assertions::assert_eq;

    fn mcp_tool(name: &str) -> Tool {
        Tool {
            name: name.to_string().into(),
            title: None,
            description: Some("desc".to_string().into()),
            input_schema: std::sync::Arc::new(rmcp::model::object(serde_json::json!({
                "type": "object",
                "properties": {},
            }))),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn from_native_uses_tool_def_native_names() {
        let defs = all_tool_defs();
        let names = ResearchToolNames::from_native(&defs);
        assert_eq!(names.paper_search, "paper_search");
        assert_eq!(names.zotero_search, "zotero_search");
    }

    #[test]
    fn from_mcp_tools_matches_qualified_tool_suffix() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([(
            "mcp__my_paper_search__search_papers".to_string(),
            mcp_tool("search_papers"),
        )]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.paper_search, "mcp__my_paper_search__search_papers");
        assert_eq!(names.zotero_search, "zotero_search");
    }

    #[test]
    fn from_mcp_tools_uses_deterministic_first_match_when_ambiguous() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([
            (
                "mcp__a__search_papers".to_string(),
                mcp_tool("search_papers"),
            ),
            (
                "mcp__z__search_papers".to_string(),
                mcp_tool("search_papers"),
            ),
        ]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.paper_search, "mcp__a__search_papers");
    }

    #[test]
    fn from_mcp_tools_matches_tool_name_when_qualified_name_is_truncated() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([(
            "mcp__very_long_server9f8e7d6c5b4a32100112233445566778899aabb".to_string(),
            mcp_tool("search_papers"),
        )]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(
            names.paper_search,
            "mcp__very_long_server9f8e7d6c5b4a32100112233445566778899aabb"
        );
    }

    #[test]
    fn should_suppress_research_mcp_tool_follows_feature_flags_for_all_categories() {
        let cases = [
            (
                "mcp__paper_server__paper_search",
                "mcp__paper_server__search_papers",
                Feature::ResearchPaperSearch,
            ),
            (
                "mcp__zotero_server__zotero_search",
                "mcp__zotero_server__zotero_search",
                Feature::ResearchZotero,
            ),
            (
                "mcp__hn_server__hn_search",
                "mcp__hn_server__search_hackernews",
                Feature::ResearchHackerNews,
            ),
            (
                "mcp__patent_server__patent_search",
                "mcp__patent_server__search_patents",
                Feature::ResearchPatents,
            ),
            (
                "mcp__repo_server__repo_clone_and_summarize",
                "mcp__repo_server__clone_and_summarize",
                Feature::ResearchRepoAnalysis,
            ),
        ];

        for (prefix_name, alias_name, feature) in cases {
            let mut features = Features::with_defaults();

            features.disable(feature);
            assert!(
                should_suppress_research_mcp_tool(prefix_name, &features),
                "{prefix_name} should be suppressed when {feature:?} is disabled"
            );
            assert!(
                should_suppress_research_mcp_tool(alias_name, &features),
                "{alias_name} should be suppressed when {feature:?} is disabled"
            );

            features.enable(feature);
            assert!(
                !should_suppress_research_mcp_tool(prefix_name, &features),
                "{prefix_name} should be allowed when {feature:?} is enabled"
            );
            assert!(
                !should_suppress_research_mcp_tool(alias_name, &features),
                "{alias_name} should be allowed when {feature:?} is enabled"
            );
        }
    }

    #[test]
    fn should_suppress_research_mcp_tool_leaves_unknown_names_unsuppressed() {
        let features = Features::with_defaults();
        assert!(!should_suppress_research_mcp_tool(
            "mcp__server__custom_tool",
            &features
        ));
    }

    #[test]
    fn should_suppress_research_mcp_tool_respects_master_research_toggle() {
        let cases = [
            "mcp__paper_server__search_papers",
            "mcp__zotero_server__zotero_search",
            "mcp__hn_server__search_hackernews",
            "mcp__patent_server__search_patents",
            "mcp__repo_server__clone_and_summarize",
        ];
        let mut features = Features::with_defaults();
        features.enable(Feature::Research);

        for name in cases {
            assert!(
                !should_suppress_research_mcp_tool(name, &features),
                "{name} should be allowed when Research is enabled"
            );
        }
    }
}

#[cfg(test)]
mod always_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn from_available_native_uses_native_defaults() {
        let names = ResearchToolNames::from_available_native();
        assert_eq!(names, ResearchToolNames::default());
    }

    #[test]
    fn native_tool_availability_matches_build_features() {
        let availability = native_tool_availability();
        assert!(availability.has_paper_search);
        assert!(availability.has_zotero);
    }
}
