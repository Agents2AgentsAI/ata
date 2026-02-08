pub mod output_schema;
pub mod prompt;
pub mod researcher_prompt;
pub mod tool_names;
pub mod types;

pub use output_schema::research_output_schema;
pub use prompt::ResearchPromptParams;
pub use prompt::build_research_prompt;
pub use researcher_prompt::RESEARCHER_SYSTEM_PROMPT;
pub use tool_names::ResearchToolAvailability;
pub use tool_names::ResearchToolContext;
pub use tool_names::ResearchToolNames;
pub use tool_names::configured_native_tool_context;
pub use tool_names::native_tool_availability;
pub use types::ResearchOutput;

#[cfg(feature = "research")]
pub(crate) type SharedResearchToolkit = codex_research_tools::ResearchToolkit;
#[cfg(not(feature = "research"))]
pub(crate) type SharedResearchToolkit = ();
