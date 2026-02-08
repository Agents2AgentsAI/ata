#[cfg(feature = "paper_search")]
pub(crate) mod arxiv;
#[cfg(feature = "paper_search")]
pub(crate) mod openalex;
#[cfg(feature = "paper_search")]
pub(crate) mod papers_with_code;
#[cfg(feature = "paper_search")]
pub(crate) mod semantic_scholar;

#[cfg(feature = "repo_analysis")]
pub(crate) mod github;

#[cfg(feature = "zotero")]
pub(crate) mod zotero;

use crate::types::Paper;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SearchPage {
    pub papers: Vec<Paper>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}
