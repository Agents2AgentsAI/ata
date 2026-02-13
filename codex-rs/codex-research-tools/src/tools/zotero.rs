use std::collections::HashMap;
use std::collections::HashSet;

use futures::StreamExt;
use futures::stream;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroChildrenRequest;
use crate::clients::zotero::ZoteroCollectionItemsRequest;
use crate::clients::zotero::ZoteroCollectionsRequest;
use crate::clients::zotero::ZoteroConfig;
use crate::clients::zotero::ZoteroLibraryAnnotationsRequest;
use crate::clients::zotero::ZoteroLibraryScope;
use crate::clients::zotero::ZoteroListGroupsRequest;
use crate::clients::zotero::ZoteroSearchRequest;
use crate::clients::zotero::ZoteroTagsRequest;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::text_utils::truncate_chars;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::ZoteroAdvancedCandidateStrategy;
use crate::types::ZoteroAdvancedCompleteness;
use crate::types::ZoteroAdvancedSearchParams;
use crate::types::ZoteroAdvancedSearchResult;
use crate::types::ZoteroAdvancedSortBy;
use crate::types::ZoteroAnnotationsParams;
use crate::types::ZoteroAnnotationsResult;
use crate::types::ZoteroAttachmentsResult;
use crate::types::ZoteroCitationFormat;
use crate::types::ZoteroCitationGenerator;
use crate::types::ZoteroCitationParams;
use crate::types::ZoteroCitationResult;
use crate::types::ZoteroCollectionItemsParams;
use crate::types::ZoteroCollectionsParams;
use crate::types::ZoteroCollectionsResult;
use crate::types::ZoteroFullTextResult;
use crate::types::ZoteroGrepParams;
use crate::types::ZoteroGrepResult;
use crate::types::ZoteroGroupsResult;
use crate::types::ZoteroItem;
use crate::types::ZoteroItemDetail;
use crate::types::ZoteroItemParams;
use crate::types::ZoteroListGroupsParams;
use crate::types::ZoteroNotesResult;
use crate::types::ZoteroRecentParams;
use crate::types::ZoteroRecentSortBy;
use crate::types::ZoteroSearchNotesParams;
use crate::types::ZoteroSearchNotesResult;
use crate::types::ZoteroSearchParams;
use crate::types::ZoteroSearchResult;
use crate::types::ZoteroSortDirection;
use crate::types::ZoteroTagSearchParams;
use crate::types::ZoteroTagsParams;
use crate::types::ZoteroTagsResult;

#[path = "zotero/advanced_search.rs"]
mod advanced_search;
#[path = "zotero/budget.rs"]
mod budget;
#[path = "zotero/citation.rs"]
mod citation;
#[path = "zotero/content_collector.rs"]
mod content_collector;
#[path = "zotero/core.rs"]
mod core;
#[path = "zotero/document_resolution.rs"]
mod document_resolution;
#[path = "zotero/grep.rs"]
mod grep;
#[path = "zotero/item_endpoints.rs"]
mod item_endpoints;
#[path = "zotero/mappers.rs"]
mod mappers;
#[path = "zotero/match_engine.rs"]
mod match_engine;
#[path = "zotero/query_endpoints.rs"]
mod query_endpoints;
#[path = "zotero/search_notes.rs"]
mod search_notes;

use core::*;
pub(crate) use item_endpoints::*;
pub(crate) use query_endpoints::*;

use budget::apply_annotations_budget;
use budget::apply_attachments_budget;
use budget::apply_items_budget;
use budget::apply_notes_budget;
use budget::truncate_optional_string;
use citation::fallback_citation_for_item;
use document_resolution::resolve_document_sources;
use mappers::map_zotero_annotation;

#[cfg(test)]
use citation::escape_bibtex_value;
#[cfg(test)]
use citation::fallback_apa_citation;
#[cfg(test)]
use document_resolution::normalize_arxiv_id;
#[cfg(test)]
use document_resolution::resolve_document_sources_with_storage_root;

const DEFAULT_SEARCH_LIMIT: u32 = 25;
const DEFAULT_TAGS_LIMIT: u32 = 100;
const DEFAULT_COLLECTIONS_LIMIT: u32 = 100;
const DEFAULT_GROUPS_LIMIT: u32 = 100;
const DEFAULT_CHILDREN_LIMIT: u32 = 50;
const DEFAULT_ANNOTATIONS_LIMIT: u32 = 50;
const DEFAULT_FULLTEXT_MAX_CHARS: u32 = 10_000;
const DEFAULT_LOCAL_USER_LIBRARY_ID: &str = "0";
const DEFAULT_ANNOTATION_PARENT_FETCH_CONCURRENCY: usize = 6;
const ZOTERO_MAX_PAGE_SIZE: u32 = 100;
const ARXIV_PDF_BASE_URL: &str = "https://arxiv.org/pdf";
const ZOTERO_STORAGE_DIR_ENV: &str = "ZOTERO_STORAGE_DIR";

#[cfg(test)]
#[path = "zotero/tests.rs"]
mod tests;
