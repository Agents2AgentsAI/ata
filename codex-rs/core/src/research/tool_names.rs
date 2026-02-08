#[cfg(feature = "research")]
use rmcp::model::Tool;
#[cfg(feature = "research")]
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchToolNames {
    pub paper_search: String,
    pub paper_get: String,
    pub paper_citations: String,
    pub paper_references: String,
    pub paper_search_sota: String,
    pub paper_find_repos: String,
    pub zotero_search: String,
    pub zotero_get_item: String,
    pub zotero_get_fulltext: String,
    pub zotero_get_notes: String,
    pub zotero_get_attachments: String,
    pub zotero_search_by_tag: String,
    pub zotero_get_collections: String,
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
}

impl Default for ResearchToolNames {
    fn default() -> Self {
        Self {
            paper_search: "paper_search".to_string(),
            paper_get: "paper_get".to_string(),
            paper_citations: "paper_citations".to_string(),
            paper_references: "paper_references".to_string(),
            paper_search_sota: "paper_search_sota".to_string(),
            paper_find_repos: "paper_find_repos".to_string(),
            zotero_search: "zotero_search".to_string(),
            zotero_get_item: "zotero_get_item".to_string(),
            zotero_get_fulltext: "zotero_get_fulltext".to_string(),
            zotero_get_notes: "zotero_get_notes".to_string(),
            zotero_get_attachments: "zotero_get_attachments".to_string(),
            zotero_search_by_tag: "zotero_search_by_tag".to_string(),
            zotero_get_collections: "zotero_get_collections".to_string(),
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
        }
    }
}

impl ResearchToolNames {
    #[cfg(feature = "research")]
    pub fn from_native(defs: &[codex_research_tools::tool_specs::ToolDef]) -> Self {
        let mut names = Self::default();
        for def in defs {
            names.set_name_for_id(def.id, def.native_name.to_string());
        }
        names
    }

    #[cfg(feature = "research")]
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

    #[cfg(feature = "research")]
    fn set_name_for_id(&mut self, id: &str, resolved_name: String) {
        match id {
            "paper_search" => self.paper_search = resolved_name,
            "paper_get" => self.paper_get = resolved_name,
            "paper_citations" => self.paper_citations = resolved_name,
            "paper_references" => self.paper_references = resolved_name,
            "paper_search_sota" => self.paper_search_sota = resolved_name,
            "paper_find_repos" => self.paper_find_repos = resolved_name,
            "zotero_search" => self.zotero_search = resolved_name,
            "zotero_get_item" => self.zotero_get_item = resolved_name,
            "zotero_get_fulltext" => self.zotero_get_fulltext = resolved_name,
            "zotero_get_notes" => self.zotero_get_notes = resolved_name,
            "zotero_get_attachments" => self.zotero_get_attachments = resolved_name,
            "zotero_search_by_tag" => self.zotero_search_by_tag = resolved_name,
            "zotero_get_collections" => self.zotero_get_collections = resolved_name,
            "zotero_get_collection_items" => self.zotero_get_collection_items = resolved_name,
            "repo_clone_and_summarize" => self.repo_clone_and_summarize = resolved_name,
            "repo_find_models" => self.repo_find_models = resolved_name,
            "repo_extract_requirements" => self.repo_extract_requirements = resolved_name,
            "repo_find_entrypoints" => self.repo_find_entrypoints = resolved_name,
            "repo_extract_io_shapes" => self.repo_extract_io_shapes = resolved_name,
            "repo_get_health" => self.repo_get_health = resolved_name,
            "repo_find_export_paths" => self.repo_find_export_paths = resolved_name,
            "repo_extract_config_schema" => self.repo_extract_config_schema = resolved_name,
            "repo_diff_requirements" => self.repo_diff_requirements = resolved_name,
            _ => {}
        }
    }
}

#[cfg(feature = "research")]
pub(crate) fn find_mcp_tool_matches(
    mcp_name: &str,
    mcp_tools: &BTreeMap<String, Tool>,
) -> Vec<String> {
    mcp_tools
        .keys()
        .filter(|qualified| qualified.split("__").last() == Some(mcp_name))
        .cloned()
        .collect()
}

#[cfg(all(test, feature = "research"))]
mod tests {
    use super::*;
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
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn from_native_uses_tool_def_native_names() {
        let defs = all_tool_defs();
        let names = ResearchToolNames::from_native(&defs);
        assert_eq!(names.paper_search, "paper_search");
        assert_eq!(names.paper_find_repos, "paper_find_repos");
        assert_eq!(names.zotero_search, "zotero_search");
    }

    #[test]
    fn from_mcp_tools_matches_qualified_tool_suffix() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([
            (
                "mcp__my_paper_search__search_papers".to_string(),
                mcp_tool("search_papers"),
            ),
            (
                "mcp__my_zotero__search_library".to_string(),
                mcp_tool("search_library"),
            ),
            (
                "mcp__my_paper_search__find_code_repos".to_string(),
                mcp_tool("find_code_repos"),
            ),
        ]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.paper_search, "mcp__my_paper_search__search_papers");
        assert_eq!(names.zotero_search, "mcp__my_zotero__search_library");
        assert_eq!(
            names.paper_find_repos,
            "mcp__my_paper_search__find_code_repos"
        );
    }

    #[test]
    fn from_mcp_tools_uses_deterministic_first_match_when_ambiguous() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([
            (
                "mcp__a__search_library".to_string(),
                mcp_tool("search_library"),
            ),
            (
                "mcp__z__search_library".to_string(),
                mcp_tool("search_library"),
            ),
        ]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.zotero_search, "mcp__a__search_library");
    }
}
