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

impl Default for ResearchToolContext {
    fn default() -> Self {
        Self {
            names: ResearchToolNames::default(),
            availability: native_tool_availability(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchToolNames {
    pub paper_search: String,
    pub paper_get: String,
    pub paper_citations: String,
    pub paper_references: String,
    pub paper_recommendations: String,
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
    let has_repo_analysis = defs.iter().any(|def| def.id == "repo_find_entrypoints");
    let has_hackernews = defs.iter().any(|def| def.id == "hn_search");
    ResearchToolAvailability {
        has_paper_search,
        has_zotero: false,
        has_repo_analysis,
        has_hackernews,
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

#[cfg(test)]
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
        assert_eq!(names.repo_find_entrypoints, "repo_find_entrypoints");
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
        assert_eq!(names.repo_find_entrypoints, "repo_find_entrypoints");
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
        assert!(!availability.has_zotero);
    }
}
