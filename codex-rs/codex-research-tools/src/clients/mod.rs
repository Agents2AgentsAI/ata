pub(crate) mod arxiv;
pub(crate) mod openalex;
pub(crate) mod semantic_scholar;

pub(crate) mod github;

pub(crate) mod hackernews;

pub(crate) mod epo_auth;
pub(crate) mod patents;

pub(crate) mod zotero;

use crate::types::Paper;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SearchPage {
    pub papers: Vec<Paper>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}
