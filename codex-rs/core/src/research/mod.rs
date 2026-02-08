pub mod prompt;
pub mod researcher_prompt;
pub mod tool_names;

pub use prompt::ResearchPromptParams;
pub use prompt::build_research_prompt;
pub use researcher_prompt::RESEARCHER_SYSTEM_PROMPT;
pub use tool_names::ResearchToolNames;

#[cfg(feature = "research")]
pub(crate) type SharedResearchToolkit = codex_research_tools::ResearchToolkit;
#[cfg(not(feature = "research"))]
pub(crate) type SharedResearchToolkit = ();
