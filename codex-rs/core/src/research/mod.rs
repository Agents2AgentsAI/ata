pub mod output_schema;
pub mod prompt;
pub mod researcher_prompt;
pub mod tool_names;
pub mod types;

// TODO(ata): re-enable tools::handlers::research::build_research_config + the
// `configured_native_tool_context` / `should_suppress_research_mcp_tool` helpers
// in tool_names.rs once codex-features and codex-core::config::ResearchToolsToml
// are restored. For now tool_names exposes only the data types.
// pub use crate::tools::handlers::research::build_research_config;
pub use output_schema::research_output_schema;
pub use prompt::ResearchPromptParams;
pub use prompt::build_research_prompt;
pub use researcher_prompt::RESEARCHER_SYSTEM_PROMPT;
pub use types::ResearchOutput;

pub(crate) type SharedResearchToolkit = codex_research_tools::ResearchToolkit;
