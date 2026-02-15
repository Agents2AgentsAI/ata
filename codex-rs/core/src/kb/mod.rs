pub mod tool_names;

pub use tool_names::KbToolNames;

#[cfg(feature = "kb")]
pub(crate) type SharedKbToolkit = codex_kb::KnowledgeBase;
#[cfg(not(feature = "kb"))]
pub(crate) type SharedKbToolkit = ();
