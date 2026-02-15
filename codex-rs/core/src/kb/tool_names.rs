#[cfg(feature = "kb")]
use rmcp::model::Tool;
#[cfg(feature = "kb")]
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbToolNames {
    pub kb_write_card: String,
    pub kb_write_overview: String,
    pub kb_suggest_tags: String,
    pub kb_status: String,
    pub kb_search: String,
    pub kb_read_card: String,
    pub kb_delete_card: String,
    pub kb_write_file: String,
    pub kb_read_file: String,
    pub kb_reset: String,
    pub kb_list_cards: String,
}

impl Default for KbToolNames {
    fn default() -> Self {
        Self {
            kb_write_card: "kb_write_card".to_string(),
            kb_write_overview: "kb_write_overview".to_string(),
            kb_suggest_tags: "kb_suggest_tags".to_string(),
            kb_status: "kb_status".to_string(),
            kb_search: "kb_search".to_string(),
            kb_read_card: "kb_read_card".to_string(),
            kb_delete_card: "kb_delete_card".to_string(),
            kb_write_file: "kb_write_file".to_string(),
            kb_read_file: "kb_read_file".to_string(),
            kb_reset: "kb_reset".to_string(),
            kb_list_cards: "kb_list_cards".to_string(),
        }
    }
}

impl KbToolNames {
    #[cfg(feature = "kb")]
    pub fn from_native(defs: &[codex_kb::tool_specs::ToolDef]) -> Self {
        let mut names = Self::default();
        for def in defs {
            names.set_name_for_id(def.id, def.native_name.to_string());
        }
        names
    }

    #[cfg(feature = "kb")]
    pub fn from_mcp_tools(
        defs: &[codex_kb::tool_specs::ToolDef],
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
                    "multiple MCP tools match KB tool; using first match",
                );
            }
        }
        names
    }

    #[cfg(feature = "kb")]
    fn set_name_for_id(&mut self, id: &str, resolved_name: String) {
        match id {
            "kb_write_card" => self.kb_write_card = resolved_name,
            "kb_write_overview" => self.kb_write_overview = resolved_name,
            "kb_suggest_tags" => self.kb_suggest_tags = resolved_name,
            "kb_status" => self.kb_status = resolved_name,
            "kb_search" => self.kb_search = resolved_name,
            "kb_read_card" => self.kb_read_card = resolved_name,
            "kb_delete_card" => self.kb_delete_card = resolved_name,
            "kb_write_file" => self.kb_write_file = resolved_name,
            "kb_read_file" => self.kb_read_file = resolved_name,
            "kb_reset" => self.kb_reset = resolved_name,
            "kb_list_cards" => self.kb_list_cards = resolved_name,
            _ => {}
        }
    }
}

#[cfg(feature = "kb")]
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
