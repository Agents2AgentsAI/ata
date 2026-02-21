#[cfg(feature = "data")]
use rmcp::model::Tool;
#[cfg(feature = "data")]
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataToolNames {
    pub dataset_search: String,
    pub dataset_get: String,
    pub dataset_list_files: String,
    pub dataset_download: String,
    pub hf_dataset_info: String,
    pub kaggle_dataset_info: String,
    pub kaggle_competitions: String,
    pub kaggle_competition_list_files: String,
    pub kaggle_competition_download: String,
}

impl Default for DataToolNames {
    fn default() -> Self {
        Self {
            dataset_search: "dataset_search".to_string(),
            dataset_get: "dataset_get".to_string(),
            dataset_list_files: "dataset_list_files".to_string(),
            dataset_download: "dataset_download".to_string(),
            hf_dataset_info: "hf_dataset_info".to_string(),
            kaggle_dataset_info: "kaggle_dataset_info".to_string(),
            kaggle_competitions: "kaggle_competitions".to_string(),
            kaggle_competition_list_files: "kaggle_competition_list_files".to_string(),
            kaggle_competition_download: "kaggle_competition_download".to_string(),
        }
    }
}

impl DataToolNames {
    #[cfg(feature = "data")]
    pub fn from_native(defs: &[codex_data_tools::tool_specs::ToolDef]) -> Self {
        let mut names = Self::default();
        for def in defs {
            names.set_name_for_id(def.id, def.native_name.to_string());
        }
        names
    }

    #[cfg(feature = "data")]
    pub fn from_mcp_tools(
        defs: &[codex_data_tools::tool_specs::ToolDef],
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
                    "multiple MCP tools match data tool; using first match",
                );
            }
        }
        names
    }

    #[cfg(feature = "data")]
    fn set_name_for_id(&mut self, id: &str, resolved_name: String) {
        match id {
            "dataset_search" => self.dataset_search = resolved_name,
            "dataset_get" => self.dataset_get = resolved_name,
            "dataset_list_files" => self.dataset_list_files = resolved_name,
            "dataset_download" => self.dataset_download = resolved_name,
            "hf_dataset_info" => self.hf_dataset_info = resolved_name,
            "kaggle_dataset_info" => self.kaggle_dataset_info = resolved_name,
            "kaggle_competitions" => self.kaggle_competitions = resolved_name,
            "kaggle_competition_list_files" => self.kaggle_competition_list_files = resolved_name,
            "kaggle_competition_download" => self.kaggle_competition_download = resolved_name,
            _ => {}
        }
    }
}

#[cfg(feature = "data")]
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

#[cfg(all(test, feature = "data"))]
mod tests {
    use super::*;
    use codex_data_tools::tool_specs::all_tool_defs;
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
            execution: None,
            meta: None,
        }
    }

    #[test]
    fn from_native_uses_tool_def_native_names() {
        let defs = all_tool_defs();
        let names = DataToolNames::from_native(&defs);
        assert_eq!(names.dataset_search, "dataset_search");
        assert_eq!(names.dataset_get, "dataset_get");
        assert_eq!(
            names.kaggle_competition_list_files,
            "kaggle_competition_list_files"
        );
        assert_eq!(
            names.kaggle_competition_download,
            "kaggle_competition_download"
        );
    }

    #[test]
    fn from_mcp_tools_matches_qualified_tool_suffix() {
        let defs = all_tool_defs();
        let mcp_tools = BTreeMap::from([
            (
                "mcp__data_server__search_datasets".to_string(),
                mcp_tool("search_datasets"),
            ),
            (
                "mcp__data_server__get_dataset".to_string(),
                mcp_tool("get_dataset"),
            ),
            (
                "mcp__data_server__list_kaggle_competition_files".to_string(),
                mcp_tool("list_kaggle_competition_files"),
            ),
            (
                "mcp__data_server__download_kaggle_competition_data".to_string(),
                mcp_tool("download_kaggle_competition_data"),
            ),
        ]);

        let names = DataToolNames::from_mcp_tools(&defs, &mcp_tools);
        assert_eq!(names.dataset_search, "mcp__data_server__search_datasets");
        assert_eq!(names.dataset_get, "mcp__data_server__get_dataset");
        assert_eq!(
            names.kaggle_competition_list_files,
            "mcp__data_server__list_kaggle_competition_files"
        );
        assert_eq!(
            names.kaggle_competition_download,
            "mcp__data_server__download_kaggle_competition_data"
        );
    }
}
