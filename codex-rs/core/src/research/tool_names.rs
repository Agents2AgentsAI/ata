use std::path::Path;

use crate::config::ResearchToolsToml;

#[cfg(feature = "research")]
use rmcp::model::Tool;
#[cfg(feature = "research")]
use std::collections::BTreeMap;

#[cfg(feature = "research")]
macro_rules! set_tool_name_for_id {
    ($self:ident, $id:ident, $resolved_name:ident) => {
        match $id {
            "paper_search" => $self.paper_search = $resolved_name,
            "paper_get" => $self.paper_get = $resolved_name,
            "paper_citations" => $self.paper_citations = $resolved_name,
            "paper_references" => $self.paper_references = $resolved_name,
            "zotero_search" => $self.zotero_search = $resolved_name,
            "zotero_advanced_search" => $self.zotero_advanced_search = $resolved_name,
            "zotero_get_item" => $self.zotero_get_item = $resolved_name,
            "zotero_get_fulltext" => $self.zotero_get_fulltext = $resolved_name,
            "zotero_get_notes" => $self.zotero_get_notes = $resolved_name,
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
            "pdf_extract_figures" => $self.pdf_extract_figures = $resolved_name,
            _ => {}
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResearchToolAvailability {
    pub has_paper_search: bool,
    pub has_zotero: bool,
    pub has_repo_analysis: bool,
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
    pub zotero_search: String,
    pub zotero_advanced_search: String,
    pub zotero_get_item: String,
    pub zotero_get_fulltext: String,
    pub zotero_get_notes: String,
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
    pub pdf_extract_figures: String,
}

impl Default for ResearchToolNames {
    fn default() -> Self {
        Self {
            paper_search: "paper_search".to_string(),
            paper_get: "paper_get".to_string(),
            paper_citations: "paper_citations".to_string(),
            paper_references: "paper_references".to_string(),
            zotero_search: "zotero_search".to_string(),
            zotero_advanced_search: "zotero_advanced_search".to_string(),
            zotero_get_item: "zotero_get_item".to_string(),
            zotero_get_fulltext: "zotero_get_fulltext".to_string(),
            zotero_get_notes: "zotero_get_notes".to_string(),
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
            pdf_extract_figures: "pdf_extract_figures".to_string(),
        }
    }
}

impl ResearchToolNames {
    #[must_use]
    pub fn from_available_native() -> Self {
        #[cfg(feature = "research")]
        {
            let defs = codex_research_tools::tool_specs::all_tool_defs();
            Self::from_native(&defs)
        }
        #[cfg(not(feature = "research"))]
        {
            Self::default()
        }
    }

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
        set_tool_name_for_id!(self, id, resolved_name);
    }
}

#[must_use]
pub fn native_tool_availability() -> ResearchToolAvailability {
    #[cfg(feature = "research")]
    {
        let defs = codex_research_tools::tool_specs::all_tool_defs();
        let has_paper_search = defs.iter().any(|def| def.id == "paper_search");
        let has_zotero = defs.iter().any(|def| def.id == "zotero_search");
        let has_repo_analysis = defs.iter().any(|def| def.id == "repo_find_entrypoints");
        ResearchToolAvailability {
            has_paper_search,
            has_zotero,
            has_repo_analysis,
        }
    }
    #[cfg(not(feature = "research"))]
    {
        ResearchToolAvailability::default()
    }
}

#[must_use]
pub fn configured_native_tool_context(
    research_toml: Option<&ResearchToolsToml>,
    codex_home: &Path,
    cwd: &Path,
) -> ResearchToolContext {
    #[cfg(feature = "research")]
    {
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

            names.set_name_for_id(def.id, def.native_name.to_string());
            match def.id {
                "paper_search" => availability.has_paper_search = true,
                "zotero_search" => availability.has_zotero = true,
                "repo_find_entrypoints" => availability.has_repo_analysis = true,
                _ => {}
            }
        }

        ResearchToolContext {
            names,
            availability,
        }
    }
    #[cfg(not(feature = "research"))]
    {
        let _ = research_toml;
        let _ = codex_home;
        let _ = cwd;
        ResearchToolContext {
            names: ResearchToolNames::from_available_native(),
            availability: native_tool_availability(),
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
        ]);

        let names = ResearchToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.paper_search, "mcp__my_paper_search__search_papers");
        assert_eq!(names.zotero_search, "mcp__my_zotero__search_library");
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
        #[cfg(feature = "research")]
        {
            assert!(availability.has_paper_search);
            assert!(availability.has_zotero);
        }
        #[cfg(not(feature = "research"))]
        {
            assert_eq!(availability, ResearchToolAvailability::default());
        }
    }
}
