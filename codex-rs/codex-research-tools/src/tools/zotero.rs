use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use futures::StreamExt;
use futures::stream;
use serde::Serialize;

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
#[path = "zotero/document_resolution.rs"]
mod document_resolution;
#[path = "zotero/grep.rs"]
mod grep;
#[path = "zotero/mappers.rs"]
mod mappers;
#[path = "zotero/match_engine.rs"]
mod match_engine;
#[path = "zotero/search_notes.rs"]
mod search_notes;

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
use document_resolution::ar5iv_probe_candidates;
#[cfg(test)]
use document_resolution::normalize_arxiv_id;
#[cfg(test)]
use document_resolution::resolve_document_sources_with_probe;

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
const AR5IV_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const AR5IV_PROBE_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const AR5IV_PROBE_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const AR5IV_PROBE_MAX_BODY_BYTES: usize = 8 * 1024;
const AR5IV_BASE_URL: &str = "https://ar5iv.labs.arxiv.org/html";
const ARXIV_PDF_BASE_URL: &str = "https://arxiv.org/pdf";
const ZOTERO_STORAGE_DIR_ENV: &str = "ZOTERO_STORAGE_DIR";

#[derive(Debug, Clone, Serialize)]
struct NormalizedScope {
    library_type: String,
    library_id: String,
}

#[derive(Debug, Clone, Serialize)]
enum ResolvedScopes {
    Single(ZoteroLibraryScope),
    All(Vec<ZoteroLibraryScope>),
}

#[derive(Debug, Clone, Serialize)]
enum NormalizedResolvedScopes {
    Single(NormalizedScope),
    All(Vec<NormalizedScope>),
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedSearchParams {
    query: String,
    scopes: NormalizedResolvedScopes,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedTagsParams {
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedRecentParams {
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    sort_by: ZoteroRecentSortBy,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedItemParams {
    item_key: String,
    scopes: NormalizedResolvedScopes,
    max_chars_per_item: Option<u32>,
    include_attachments: bool,
    include_fulltext_resolution: bool,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedCitationParams {
    item_key: String,
    scopes: NormalizedResolvedScopes,
    format: ZoteroCitationFormat,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedAnnotationsParams {
    item_key: Option<String>,
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    include_parent_context: bool,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedTagSearchParams {
    tags: Vec<String>,
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedCollectionsParams {
    scopes: NormalizedResolvedScopes,
    offset: u32,
    limit: u32,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedListGroupsParams {
    user_id: String,
    offset: u32,
    limit: u32,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedCollectionItemsParams {
    collection_key: String,
    scopes: NormalizedResolvedScopes,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
}

pub(crate) async fn zotero_search(
    toolkit: &ResearchToolkit,
    params: ZoteroSearchParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_search_params(toolkit, params, "zotero_search").await?;
    let key = CacheKey {
        tool_name: "zotero_search",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            search_items_across_scopes(
                toolkit,
                config,
                &scopes,
                &ZoteroSearchRequest {
                    query: Some(&normalized.query),
                    tag: None,
                    offset: normalized.offset,
                    limit: normalized.limit,
                    item_type: normalized.item_type.as_deref(),
                    sort: None,
                    direction: None,
                },
                normalized.limit,
            )
            .await
        },
    )
    .await?;

    apply_items_budget(&mut result.items, normalized.max_chars_per_item);
    Ok(result)
}

pub(crate) async fn zotero_get_tags(
    toolkit: &ResearchToolkit,
    params: ZoteroTagsParams,
) -> Result<ZoteroTagsResult> {
    let normalized = normalize_tags_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_get_tags",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_tags(
                    toolkit.http(),
                    config,
                    &scope,
                    ZoteroTagsRequest {
                        offset: normalized.offset,
                        limit: normalized.limit,
                    },
                )
                .await
            }
        },
    )
    .await
}

pub(crate) async fn zotero_get_recent(
    toolkit: &ResearchToolkit,
    params: ZoteroRecentParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_recent_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_get_recent",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::search_items(
                    toolkit.http(),
                    config,
                    &scope,
                    &ZoteroSearchRequest {
                        query: None,
                        tag: None,
                        offset: normalized.offset,
                        limit: normalized.limit,
                        item_type: normalized.item_type.as_deref(),
                        sort: Some(recent_sort_field(&normalized.sort_by)),
                        direction: Some("desc"),
                    },
                )
                .await
            }
        },
    )
    .await?;

    apply_items_budget(&mut result.items, normalized.max_chars_per_item);
    Ok(result)
}

pub(crate) async fn zotero_advanced_search(
    toolkit: &ResearchToolkit,
    params: ZoteroAdvancedSearchParams,
) -> Result<ZoteroAdvancedSearchResult> {
    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_advanced_search",
    )
    .await?;

    match resolved {
        ResolvedScopes::Single(_) => advanced_search::zotero_advanced_search(toolkit, params).await,
        ResolvedScopes::All(scopes) => {
            let requested_offset = params.offset.unwrap_or(0) as usize;
            let requested_limit = params.limit.unwrap_or(25).clamp(1, 100) as usize;
            let requested_window = params
                .offset
                .unwrap_or(0)
                .saturating_add(params.limit.unwrap_or(25).clamp(1, 100));
            let per_scope_fetch_limit = requested_window.clamp(1, 100);
            let sort_by = params
                .sort_by
                .clone()
                .unwrap_or(ZoteroAdvancedSortBy::Relevance);
            let sort_direction = params
                .sort_direction
                .clone()
                .unwrap_or(ZoteroSortDirection::Asc);

            let mut merged_items: Vec<ZoteroItem> = Vec::new();
            let mut total_scanned: usize = 0;
            let mut summed_total_available: u64 = 0;
            let mut total_available_known = true;
            let mut successful_scopes = 0usize;
            let mut any_scope_has_more = false;
            let mut all_warnings: Vec<String> = Vec::new();
            let mut all_hints: Vec<String> = Vec::new();
            let mut worst_completeness = ZoteroAdvancedCompleteness::Exact;
            let mut first_strategy = None;

            if requested_window > per_scope_fetch_limit {
                worst_completeness = ZoteroAdvancedCompleteness::Approximate;
                all_warnings.push(
                    "multi-scope advanced search fetch window is capped at 100 items per scope; large offset/limit requests may be incomplete"
                        .to_string(),
                );
            }

            for scope in &scopes {
                let (lib_type, lib_id) = match scope {
                    ZoteroLibraryScope::User(id) => ("user".to_string(), id.clone()),
                    ZoteroLibraryScope::Group(id) => ("group".to_string(), id.clone()),
                };
                let scoped_params = ZoteroAdvancedSearchParams {
                    library_type: Some(lib_type.clone()),
                    library_id: Some(lib_id.clone()),
                    offset: Some(0),
                    limit: Some(per_scope_fetch_limit),
                    ..params.clone()
                };
                match advanced_search::zotero_advanced_search(toolkit, scoped_params).await {
                    Ok(result) => {
                        successful_scopes = successful_scopes.saturating_add(1);
                        total_scanned += result.scanned_items;
                        all_warnings.extend(result.warnings);
                        all_hints.extend(result.hints);
                        if let Some(scope_total) = result.results.total_available {
                            summed_total_available =
                                summed_total_available.saturating_add(scope_total);
                        } else {
                            total_available_known = false;
                        }
                        any_scope_has_more = any_scope_has_more || result.results.has_more;
                        if first_strategy.is_none() {
                            first_strategy = Some(result.candidate_strategy);
                        }
                        if matches!(result.completeness, ZoteroAdvancedCompleteness::Approximate) {
                            worst_completeness = ZoteroAdvancedCompleteness::Approximate;
                        }
                        merged_items.extend(result.results.items);
                    }
                    Err(error) => {
                        worst_completeness = ZoteroAdvancedCompleteness::Approximate;
                        all_warnings.push(format!(
                            "advanced search failed for scope {lib_type}/{lib_id}: {error}"
                        ));
                    }
                }
            }

            advanced_search::apply_advanced_sort(
                merged_items.as_mut_slice(),
                &sort_by,
                &sort_direction,
                &mut all_warnings,
            );

            let after_offset = merged_items
                .into_iter()
                .skip(requested_offset)
                .collect::<Vec<_>>();
            let truncated = after_offset.len() > requested_limit;
            let merged_items = after_offset
                .into_iter()
                .take(requested_limit)
                .collect::<Vec<_>>();
            all_warnings.sort();
            all_warnings.dedup();

            let total_available = if successful_scopes == 0 || !total_available_known {
                None
            } else {
                Some(summed_total_available)
            };

            let has_more = if let Some(total) = total_available {
                let offset = u64::try_from(requested_offset).unwrap_or(u64::MAX);
                let count = u64::try_from(merged_items.len()).unwrap_or(u64::MAX);
                offset.saturating_add(count) < total
            } else {
                any_scope_has_more || truncated
            };

            if total_available == Some(0) {
                all_hints.sort();
                all_hints.dedup();
            } else {
                all_hints.clear();
            }

            Ok(ZoteroAdvancedSearchResult {
                completeness: worst_completeness,
                candidate_strategy: first_strategy
                    .unwrap_or(ZoteroAdvancedCandidateStrategy::RecentModifiedFallback),
                scanned_items: total_scanned,
                warnings: all_warnings,
                hints: all_hints,
                results: ZoteroSearchResult {
                    items: merged_items,
                    total_available,
                    has_more,
                },
            })
        }
    }
}

pub(crate) async fn zotero_grep_text(
    toolkit: &ResearchToolkit,
    params: ZoteroGrepParams,
) -> Result<ZoteroGrepResult> {
    grep::zotero_grep_text(toolkit, params).await
}

pub(crate) async fn zotero_search_notes(
    toolkit: &ResearchToolkit,
    params: ZoteroSearchNotesParams,
) -> Result<ZoteroSearchNotesResult> {
    search_notes::zotero_search_notes(toolkit, params).await
}

pub(crate) async fn zotero_get_item(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroItemDetail> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_item").await?;
    let key = CacheKey {
        tool_name: "zotero_get_item",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let (matched_scope, mut item) =
                fetch_item_across_scopes(toolkit, config, &scopes, &normalized.item_key).await?;

            if let Some(max_chars) = normalized.max_chars_per_item {
                truncate_optional_string(&mut item.abstract_text, max_chars as usize);
                truncate_optional_string(&mut item.extra, max_chars as usize);
            }

            let need_attachments =
                normalized.include_attachments || normalized.include_fulltext_resolution;
            if need_attachments {
                let mut attachment_scopes = vec![matched_scope.clone()];
                attachment_scopes
                    .extend(scopes.into_iter().filter(|scope| scope != &matched_scope));

                let mut last_attachment_err = None;
                let mut attachments = None;
                for scope in &attachment_scopes {
                    match zotero::get_attachments(
                        toolkit.http(),
                        config,
                        scope,
                        &ZoteroChildrenRequest {
                            item_key: normalized.item_key.as_str(),
                            offset: 0,
                            limit: DEFAULT_CHILDREN_LIMIT,
                        },
                    )
                    .await
                    {
                        Ok(result) => {
                            attachments = Some(result.attachments);
                            break;
                        }
                        Err(err) => last_attachment_err = Some(err),
                    }
                }

                if let Some(attachments) = attachments {
                    if normalized.include_fulltext_resolution {
                        item.document_resolution = Some(
                            resolve_document_sources(
                                toolkit,
                                Some(&item),
                                &attachments,
                                None,
                                Vec::new(),
                            )
                            .await,
                        );
                    }
                    if normalized.include_attachments {
                        let mut response_attachments = attachments;
                        if let Some(max_chars) = normalized.max_chars_per_item {
                            apply_attachments_budget(&mut response_attachments, max_chars as usize);
                        }
                        item.attachments = Some(response_attachments);
                    }
                } else if let Some(err) = last_attachment_err {
                    tracing::warn!(
                        item_key = %normalized.item_key,
                        %err,
                        "zotero_get_item attachment enrichment failed; returning base item"
                    );
                }
            }

            Ok(item)
        },
    )
    .await
}

pub(crate) async fn zotero_get_item_citation(
    toolkit: &ResearchToolkit,
    params: ZoteroCitationParams,
) -> Result<ZoteroCitationResult> {
    let normalized = normalize_citation_params(toolkit, params).await?;
    let key = CacheKey {
        tool_name: "zotero_get_item_citation",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let (_, item) =
                fetch_item_across_scopes(toolkit, config, &scopes, &normalized.item_key).await?;

            let (citation, citation_key) = fallback_citation_for_item(&item, normalized.format);
            Ok(ZoteroCitationResult {
                item_key: item.key,
                format: normalized.format,
                citation,
                citation_key,
                generator: ZoteroCitationGenerator::FallbackFormatter,
                warnings: Vec::new(),
                source_meta: item.source_meta,
            })
        },
    )
    .await
}

pub(crate) async fn zotero_get_fulltext(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroFullTextResult> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_fulltext").await?;
    let key = CacheKey {
        tool_name: "zotero_get_fulltext",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut fulltext = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let mut last_err = None;
            for scope in &scopes {
                match zotero::get_fulltext(toolkit.http(), config, scope, &normalized.item_key)
                    .await
                {
                    Ok(mut result) => {
                        let mut resolution_trace = Vec::new();

                        let children_request = ZoteroChildrenRequest {
                            item_key: &normalized.item_key,
                            offset: 0,
                            limit: DEFAULT_CHILDREN_LIMIT,
                        };
                        let (item_result, attachments_result) = tokio::join!(
                            zotero::get_item(toolkit.http(), config, scope, &normalized.item_key),
                            zotero::get_attachments(
                                toolkit.http(),
                                config,
                                scope,
                                &children_request
                            )
                        );

                        let item = match item_result {
                            Ok(item) => Some(item),
                            Err(err) => {
                                resolution_trace
                                    .push(format!("item metadata lookup failed: {err}"));
                                None
                            }
                        };

                        let attachments = match attachments_result {
                            Ok(result) => result.attachments,
                            Err(err) => {
                                resolution_trace.push(format!("attachment lookup failed: {err}"));
                                Vec::new()
                            }
                        };

                        let has_indexed_content = !result.content.trim().is_empty();
                        result.resolution = Some(
                            resolve_document_sources(
                                toolkit,
                                item.as_ref(),
                                &attachments,
                                Some(has_indexed_content),
                                resolution_trace,
                            )
                            .await,
                        );
                        return Ok(result);
                    }
                    Err(err) => last_err = Some(err),
                }
            }
            Err(last_err
                .unwrap_or_else(|| ResearchError::Internal("no scopes to search".to_string())))
        },
    )
    .await?;

    let max_chars = normalized
        .max_chars_per_item
        .unwrap_or(DEFAULT_FULLTEXT_MAX_CHARS) as usize;
    fulltext.content = truncate_chars(&fulltext.content, max_chars);

    Ok(fulltext)
}

pub(crate) async fn zotero_get_notes(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroNotesResult> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_notes").await?;
    let key = CacheKey {
        tool_name: "zotero_get_notes",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut notes = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let mut last_err = None;
            for scope in &scopes {
                match zotero::get_notes(
                    toolkit.http(),
                    config,
                    scope,
                    &ZoteroChildrenRequest {
                        item_key: &normalized.item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    },
                )
                .await
                {
                    Ok(result) => return Ok(result),
                    Err(err) => last_err = Some(err),
                }
            }
            Err(last_err
                .unwrap_or_else(|| ResearchError::Internal("no scopes to search".to_string())))
        },
    )
    .await?;

    if let Some(max_chars) = normalized.max_chars_per_item {
        apply_notes_budget(&mut notes.notes, max_chars as usize);
    }

    Ok(notes)
}

pub(crate) async fn zotero_get_annotations(
    toolkit: &ResearchToolkit,
    params: ZoteroAnnotationsParams,
) -> Result<ZoteroAnnotationsResult> {
    let normalized = normalize_annotations_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_get_annotations",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut annotations = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                let mut annotations = if let Some(item_key) = normalized.item_key.as_deref() {
                    let response = zotero::get_annotations(
                        toolkit.http(),
                        config,
                        &scope,
                        &ZoteroChildrenRequest {
                            item_key,
                            offset: normalized.offset,
                            limit: normalized.limit,
                        },
                    )
                    .await?;
                    ZoteroAnnotationsResult {
                        item_key: Some(response.item_key),
                        annotations: response
                            .annotations
                            .into_iter()
                            .map(map_zotero_annotation)
                            .collect(),
                        total_available: response.total_available,
                        has_more: response.has_more,
                    }
                } else {
                    let response = zotero::get_library_annotations(
                        toolkit.http(),
                        config,
                        &scope,
                        ZoteroLibraryAnnotationsRequest {
                            offset: normalized.offset,
                            limit: normalized.limit,
                        },
                    )
                    .await?;

                    ZoteroAnnotationsResult {
                        item_key: None,
                        annotations: response
                            .annotations
                            .into_iter()
                            .map(map_zotero_annotation)
                            .collect(),
                        total_available: response.total_available,
                        has_more: response.has_more,
                    }
                };

                if normalized.include_parent_context {
                    let parent_item_keys = annotations
                        .annotations
                        .iter()
                        .filter_map(|annotation| annotation.parent_item.clone())
                        .collect::<HashSet<_>>();

                    if !parent_item_keys.is_empty() {
                        let parent_titles = stream::iter(parent_item_keys.into_iter())
                            .map(|parent_item_key| {
                                let scope = scope.clone();
                                async move {
                                    match zotero::get_item(
                                        toolkit.http(),
                                        config,
                                        &scope,
                                        parent_item_key.as_str(),
                                    )
                                    .await
                                    {
                                        Ok(parent_item) => {
                                            Some((parent_item_key, parent_item.title))
                                        }
                                        Err(error) => {
                                            tracing::warn!(
                                                parent_item_key = %parent_item_key,
                                                %error,
                                                "failed to resolve annotation parent item title"
                                            );
                                            None
                                        }
                                    }
                                }
                            })
                            .buffer_unordered(DEFAULT_ANNOTATION_PARENT_FETCH_CONCURRENCY)
                            .collect::<Vec<_>>()
                            .await
                            .into_iter()
                            .flatten()
                            .collect::<HashMap<_, _>>();

                        for annotation in &mut annotations.annotations {
                            if let Some(parent_item) = annotation.parent_item.as_deref()
                                && let Some(parent_item_title) = parent_titles.get(parent_item)
                            {
                                annotation.parent_item_title = Some(parent_item_title.clone());
                            }
                        }
                    }
                }

                Ok(annotations)
            }
        },
    )
    .await?;

    if let Some(max_chars) = normalized.max_chars_per_item {
        apply_annotations_budget(&mut annotations.annotations, max_chars as usize);
    }

    Ok(annotations)
}

pub(crate) async fn zotero_get_attachments(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroAttachmentsResult> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_attachments").await?;
    let key = CacheKey {
        tool_name: "zotero_get_attachments",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut attachments = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let mut last_err = None;
            for scope in &scopes {
                match zotero::get_attachments(
                    toolkit.http(),
                    config,
                    scope,
                    &ZoteroChildrenRequest {
                        item_key: &normalized.item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    },
                )
                .await
                {
                    Ok(result) => return Ok(result),
                    Err(err) => last_err = Some(err),
                }
            }
            Err(last_err
                .unwrap_or_else(|| ResearchError::Internal("no scopes to search".to_string())))
        },
    )
    .await?;

    if let Some(max_chars) = normalized.max_chars_per_item {
        apply_attachments_budget(&mut attachments.attachments, max_chars as usize);
    }

    Ok(attachments)
}

pub(crate) async fn zotero_search_by_tag(
    toolkit: &ResearchToolkit,
    params: ZoteroTagSearchParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_tag_search_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_search_by_tag",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let tool_timeout = toolkit.config().tool_timeout;
    let timeout_ms = u64::try_from(tool_timeout.as_millis()).unwrap_or(u64::MAX);
    let mut result = tokio::time::timeout(
        tool_timeout,
        get_or_fetch_typed(
            toolkit,
            key,
            toolkit.config().cache_ttls.zotero_items,
            || {
                let scope = to_scope(&normalized.scope);
                async move {
                    let mut by_key: HashMap<String, (ZoteroItem, HashSet<String>)> = HashMap::new();

                    // Fetch each tag independently and keep only items that appear for all tags.
                    for tag in &normalized.tags {
                        let tag_key = tag.to_ascii_lowercase();
                        let mut offset = 0;

                        loop {
                            let page = zotero::search_items(
                                toolkit.http(),
                                config,
                                &scope,
                                &ZoteroSearchRequest {
                                    query: None,
                                    tag: Some(tag),
                                    offset,
                                    limit: ZOTERO_MAX_PAGE_SIZE,
                                    item_type: normalized.item_type.as_deref(),
                                    sort: None,
                                    direction: None,
                                },
                            )
                            .await?;

                            let fetched = u32::try_from(page.items.len()).unwrap_or(0);
                            for item in page.items {
                                let entry = by_key
                                    .entry(item.key.clone())
                                    .or_insert_with(|| (item, HashSet::new()));
                                entry.1.insert(tag_key.clone());
                            }

                            if !page.has_more || fetched == 0 {
                                break;
                            }

                            offset = offset.saturating_add(fetched);
                        }
                    }

                    let required = normalized.tags.len();
                    let mut matched = by_key
                        .into_values()
                        .filter_map(|(item, seen_tags)| {
                            if seen_tags.len() == required {
                                Some(item)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    matched.sort_by(|left, right| {
                        left.title
                            .cmp(&right.title)
                            .then_with(|| left.key.cmp(&right.key))
                    });

                    let total = matched.len();
                    let offset = normalized.offset as usize;
                    let limit = normalized.limit as usize;
                    let items = matched
                        .into_iter()
                        .skip(offset)
                        .take(limit)
                        .collect::<Vec<_>>();

                    Ok(ZoteroSearchResult {
                        has_more: offset + items.len() < total,
                        total_available: Some(u64::try_from(total).unwrap_or(u64::MAX)),
                        items,
                    })
                }
            },
        ),
    )
    .await
    .map_err(|_| ResearchError::Timeout {
        api: ResearchApi::Zotero,
        timeout_ms,
    })??;

    apply_items_budget(&mut result.items, normalized.max_chars_per_item);
    Ok(result)
}

pub(crate) async fn zotero_get_collections(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionsParams,
) -> Result<ZoteroCollectionsResult> {
    let normalized = normalize_collections_params(toolkit, params).await?;
    let key = CacheKey {
        tool_name: "zotero_get_collections",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            merge_collections_across_scopes(
                toolkit,
                config,
                &scopes,
                normalized.offset,
                normalized.limit,
            )
            .await
        },
    )
    .await
}

pub(crate) async fn zotero_list_groups(
    toolkit: &ResearchToolkit,
    params: ZoteroListGroupsParams,
) -> Result<ZoteroGroupsResult> {
    let normalized = normalize_list_groups_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_list_groups",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            zotero::list_groups(
                toolkit.http(),
                config,
                &normalized.user_id,
                ZoteroListGroupsRequest {
                    offset: normalized.offset,
                    limit: normalized.limit,
                },
            )
            .await
        },
    )
    .await
}

pub(crate) async fn zotero_get_collection_items(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionItemsParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_collection_items_params(toolkit, params).await?;
    let key = CacheKey {
        tool_name: "zotero_get_collection_items",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            collection_items_across_scopes(
                toolkit,
                config,
                &scopes,
                &ZoteroCollectionItemsRequest {
                    collection_key: &normalized.collection_key,
                    offset: normalized.offset,
                    limit: normalized.limit,
                    item_type: normalized.item_type.as_deref(),
                },
                normalized.limit,
            )
            .await
        },
    )
    .await?;

    apply_items_budget(&mut result.items, normalized.max_chars_per_item);
    Ok(result)
}

fn normalize_tags_params(
    toolkit: &ResearchToolkit,
    params: ZoteroTagsParams,
) -> Result<NormalizedTagsParams> {
    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_tags",
    )?;

    Ok(NormalizedTagsParams {
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_TAGS_LIMIT).clamp(1, 200),
    })
}

fn normalize_recent_params(
    toolkit: &ResearchToolkit,
    params: ZoteroRecentParams,
) -> Result<NormalizedRecentParams> {
    // Recent is intentionally single-scope. Merging recency-ordered streams
    // across scopes would require an explicit cross-scope merge policy.
    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_recent",
    )?;

    Ok(NormalizedRecentParams {
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 100),
        item_type: normalize_optional_string(params.item_type),
        sort_by: params.sort_by.unwrap_or(ZoteroRecentSortBy::DateAdded),
        max_chars_per_item: params.max_chars_per_item,
    })
}

async fn normalize_search_params(
    toolkit: &ResearchToolkit,
    params: ZoteroSearchParams,
    tool_name: &'static str,
) -> Result<NormalizedSearchParams> {
    let query = params.query.trim().to_string();
    if query.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_search query must not be empty".to_string(),
        ));
    }

    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        tool_name,
    )
    .await?;

    Ok(NormalizedSearchParams {
        query,
        scopes: to_normalized_resolved_scopes(&resolved),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 100),
        item_type: normalize_optional_string(params.item_type),
        max_chars_per_item: params.max_chars_per_item,
    })
}

async fn normalize_item_params(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
    tool_name: &'static str,
) -> Result<NormalizedItemParams> {
    let item_key = params.item_key.trim().to_string();
    if item_key.is_empty() {
        return Err(ResearchError::InvalidInput(format!(
            "{tool_name} item_key must not be empty"
        )));
    }

    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        tool_name,
    )
    .await?;

    Ok(NormalizedItemParams {
        item_key,
        scopes: to_normalized_resolved_scopes(&resolved),
        max_chars_per_item: params.max_chars_per_item,
        include_attachments: params.include_attachments.unwrap_or(false),
        include_fulltext_resolution: params.include_fulltext_resolution.unwrap_or(false),
    })
}

async fn normalize_citation_params(
    toolkit: &ResearchToolkit,
    params: ZoteroCitationParams,
) -> Result<NormalizedCitationParams> {
    let item_key = params.item_key.trim().to_string();
    if item_key.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_get_item_citation item_key must not be empty".to_string(),
        ));
    }

    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_item_citation",
    )
    .await?;

    Ok(NormalizedCitationParams {
        item_key,
        scopes: to_normalized_resolved_scopes(&resolved),
        format: params.format.unwrap_or(ZoteroCitationFormat::Bibtex),
    })
}

fn normalize_annotations_params(
    toolkit: &ResearchToolkit,
    params: ZoteroAnnotationsParams,
) -> Result<NormalizedAnnotationsParams> {
    let item_key = normalize_optional_string(params.item_key);
    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_annotations",
    )?;

    Ok(NormalizedAnnotationsParams {
        item_key,
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params
            .limit
            .unwrap_or(DEFAULT_ANNOTATIONS_LIMIT)
            .clamp(1, 100),
        include_parent_context: params.include_parent_context.unwrap_or(false),
        max_chars_per_item: params.max_chars_per_item,
    })
}

fn normalize_tag_search_params(
    toolkit: &ResearchToolkit,
    params: ZoteroTagSearchParams,
) -> Result<NormalizedTagSearchParams> {
    let mut tags = params
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();

    tags.sort();
    tags.dedup();

    if tags.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_search_by_tag requires at least one non-empty tag".to_string(),
        ));
    }

    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_search_by_tag",
    )?;

    Ok(NormalizedTagSearchParams {
        tags,
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 100),
        item_type: normalize_optional_string(params.item_type),
        max_chars_per_item: params.max_chars_per_item,
    })
}

async fn normalize_collections_params(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionsParams,
) -> Result<NormalizedCollectionsParams> {
    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_collections",
    )
    .await?;

    Ok(NormalizedCollectionsParams {
        scopes: to_normalized_resolved_scopes(&resolved),
        offset: params.offset.unwrap_or(0),
        limit: params
            .limit
            .unwrap_or(DEFAULT_COLLECTIONS_LIMIT)
            .clamp(1, 100),
    })
}

fn normalize_list_groups_params(
    toolkit: &ResearchToolkit,
    params: ZoteroListGroupsParams,
) -> Result<NormalizedListGroupsParams> {
    let scope = resolve_scope(
        toolkit,
        Some("user"),
        params.user_id.as_deref(),
        "zotero_list_groups",
    )?;

    let user_id = match scope {
        ZoteroLibraryScope::User(user_id) => user_id,
        ZoteroLibraryScope::Group(_) => {
            return Err(ResearchError::Internal(
                "zotero_list_groups resolved a non-user scope".to_string(),
            ));
        }
    };

    Ok(NormalizedListGroupsParams {
        user_id,
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_GROUPS_LIMIT).clamp(1, 100),
    })
}

async fn normalize_collection_items_params(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionItemsParams,
) -> Result<NormalizedCollectionItemsParams> {
    let collection_key = params.collection_key.trim().to_string();
    if collection_key.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_get_collection_items collection_key must not be empty".to_string(),
        ));
    }

    let resolved = resolve_scopes(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_collection_items",
    )
    .await?;

    Ok(NormalizedCollectionItemsParams {
        collection_key,
        scopes: to_normalized_resolved_scopes(&resolved),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 100),
        item_type: normalize_optional_string(params.item_type),
        max_chars_per_item: params.max_chars_per_item,
    })
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn zotero_config(toolkit: &ResearchToolkit) -> ZoteroConfig<'_> {
    ZoteroConfig {
        base_url: &toolkit.config().zotero_base_url,
        api_key: toolkit.config().zotero_api_key.as_deref(),
    }
}

fn uses_local_zotero_api(toolkit: &ResearchToolkit) -> bool {
    toolkit.config().uses_local_zotero_api()
}

fn resolve_scope(
    toolkit: &ResearchToolkit,
    library_type: Option<&str>,
    library_id: Option<&str>,
    tool_name: &'static str,
) -> Result<ZoteroLibraryScope> {
    let requested_type = library_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let requested_id = library_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let configured_type = toolkit
        .config()
        .zotero_library_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    match requested_type.as_deref().or(configured_type.as_deref()) {
        Some("user") => {
            let id = requested_id
                .or_else(|| toolkit.config().zotero_user_id.clone())
                .or_else(|| {
                    if uses_local_zotero_api(toolkit) {
                        Some(DEFAULT_LOCAL_USER_LIBRARY_ID.to_string())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| ResearchError::NotConfigured {
                    tool: tool_name,
                    reason: "missing Zotero user library id (set `zotero_user_id` or provide `library_id`)".to_string(),
                })?;
            Ok(ZoteroLibraryScope::User(id))
        }
        Some("group") => {
            let id = requested_id
                .or_else(|| toolkit.config().zotero_group_id.clone())
                .ok_or_else(|| ResearchError::NotConfigured {
                    tool: tool_name,
                    reason: "missing Zotero group library id (set `zotero_group_id` or provide `library_id`)".to_string(),
                })?;
            Ok(ZoteroLibraryScope::Group(id))
        }
        Some(other) => Err(ResearchError::InvalidInput(format!(
            "invalid library_type '{other}' (expected 'user' or 'group')"
        ))),
        None => {
            if let Some(user_id) = toolkit.config().zotero_user_id.clone() {
                return Ok(ZoteroLibraryScope::User(user_id));
            }
            if let Some(group_id) = toolkit.config().zotero_group_id.clone() {
                return Ok(ZoteroLibraryScope::Group(group_id));
            }
            if uses_local_zotero_api(toolkit) {
                return Ok(ZoteroLibraryScope::User(
                    DEFAULT_LOCAL_USER_LIBRARY_ID.to_string(),
                ));
            }

            Err(ResearchError::NotConfigured {
                tool: tool_name,
                reason: "no Zotero library configured (set `zotero_user_id` or `zotero_group_id`)"
                    .to_string(),
            })
        }
    }
}

fn to_normalized_scope(scope: &ZoteroLibraryScope) -> NormalizedScope {
    match scope {
        ZoteroLibraryScope::User(user_id) => NormalizedScope {
            library_type: "user".to_string(),
            library_id: user_id.clone(),
        },
        ZoteroLibraryScope::Group(group_id) => NormalizedScope {
            library_type: "group".to_string(),
            library_id: group_id.clone(),
        },
    }
}

fn to_scope(scope: &NormalizedScope) -> ZoteroLibraryScope {
    if scope.library_type == "group" {
        return ZoteroLibraryScope::Group(scope.library_id.clone());
    }

    ZoteroLibraryScope::User(scope.library_id.clone())
}

fn to_normalized_resolved_scopes(resolved: &ResolvedScopes) -> NormalizedResolvedScopes {
    match resolved {
        ResolvedScopes::Single(scope) => {
            NormalizedResolvedScopes::Single(to_normalized_scope(scope))
        }
        ResolvedScopes::All(scopes) => {
            NormalizedResolvedScopes::All(scopes.iter().map(to_normalized_scope).collect())
        }
    }
}

fn resolved_scopes_to_vec(scopes: &NormalizedResolvedScopes) -> Vec<ZoteroLibraryScope> {
    match scopes {
        NormalizedResolvedScopes::Single(scope) => vec![to_scope(scope)],
        NormalizedResolvedScopes::All(scopes) => scopes.iter().map(to_scope).collect(),
    }
}

async fn resolve_scopes(
    toolkit: &ResearchToolkit,
    library_type: Option<&str>,
    library_id: Option<&str>,
    tool_name: &'static str,
) -> Result<ResolvedScopes> {
    let has_explicit_type = library_type
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_explicit_id = library_id
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());

    // If caller provides explicit scope params, use single scope.
    if has_explicit_type || has_explicit_id {
        return Ok(ResolvedScopes::Single(resolve_scope(
            toolkit,
            library_type,
            library_id,
            tool_name,
        )?));
    }

    // If config has explicit library_type, use single scope.
    let configured_type = toolkit
        .config()
        .zotero_library_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if configured_type.is_some() {
        return Ok(ResolvedScopes::Single(resolve_scope(
            toolkit,
            library_type,
            library_id,
            tool_name,
        )?));
    }

    // Otherwise, discover all scopes (user library + group libraries).
    let mut scopes = discover_all_scopes(toolkit).await?;
    if scopes.len() == 1 {
        return Ok(ResolvedScopes::Single(scopes.swap_remove(0)));
    }
    Ok(ResolvedScopes::All(scopes))
}

async fn discover_all_scopes(toolkit: &ResearchToolkit) -> Result<Vec<ZoteroLibraryScope>> {
    let mut scopes = Vec::new();
    let config = zotero_config(toolkit);

    // Determine user ID.
    let user_id = toolkit
        .config()
        .zotero_user_id
        .clone()
        .or_else(|| {
            if uses_local_zotero_api(toolkit) {
                Some(DEFAULT_LOCAL_USER_LIBRARY_ID.to_string())
            } else {
                None
            }
        })
        .filter(|id| !id.trim().is_empty());

    if let Some(ref user_id) = user_id {
        scopes.push(ZoteroLibraryScope::User(user_id.clone()));

        // Discover groups via cached API call. Errors are silently ignored so that
        // we gracefully fall back to the configured scopes.
        let groups_key = CacheKey {
            tool_name: "zotero_discover_groups",
            params_hash: hash_cache_payload(user_id)?,
        };

        let groups_result: std::result::Result<ZoteroGroupsResult, _> = get_or_fetch_typed(
            toolkit,
            groups_key,
            toolkit.config().cache_ttls.zotero_items,
            || {
                let user_id = user_id.clone();
                async move {
                    zotero::list_groups(
                        toolkit.http(),
                        config,
                        &user_id,
                        ZoteroListGroupsRequest {
                            offset: 0,
                            limit: DEFAULT_GROUPS_LIMIT,
                        },
                    )
                    .await
                }
            },
        )
        .await;

        match groups_result {
            Ok(result) => {
                for group in result.groups {
                    scopes.push(ZoteroLibraryScope::Group(group.id));
                }
            }
            Err(err) => {
                tracing::warn!(%err, "zotero group discovery failed; using configured scopes only");
            }
        }
    }

    // Add configured group_id if not already discovered.
    if let Some(group_id) = toolkit.config().zotero_group_id.clone()
        && !scopes
            .iter()
            .any(|s| matches!(s, ZoteroLibraryScope::Group(id) if id == &group_id))
    {
        scopes.push(ZoteroLibraryScope::Group(group_id));
    }

    if scopes.is_empty() {
        return Err(ResearchError::NotConfigured {
            tool: "zotero",
            reason: "no Zotero library configured (set `zotero_user_id` or `zotero_group_id`)"
                .to_string(),
        });
    }

    Ok(scopes)
}

/// Searches items across multiple scopes. Errors from individual scopes are
/// silently skipped for graceful degradation.
async fn search_items_across_scopes(
    toolkit: &ResearchToolkit,
    config: ZoteroConfig<'_>,
    scopes: &[ZoteroLibraryScope],
    request: &ZoteroSearchRequest<'_>,
    limit: u32,
) -> Result<ZoteroSearchResult> {
    let mut all_items = Vec::new();
    let mut total: u64 = 0;
    let mut any_has_more = false;

    for scope in scopes {
        match zotero::search_items(toolkit.http(), config, scope, request).await {
            Ok(result) => {
                total = total.saturating_add(result.total_available.unwrap_or(0));
                any_has_more = any_has_more || result.has_more;
                all_items.extend(result.items);
            }
            Err(err) => {
                tracing::warn!(?scope, %err, "zotero search_items failed for scope; skipping");
            }
        }
    }

    let truncated = all_items.len() > limit as usize;
    all_items.truncate(limit as usize);

    Ok(ZoteroSearchResult {
        has_more: any_has_more || truncated,
        total_available: Some(total),
        items: all_items,
    })
}

/// Gets collection items across multiple scopes.
async fn collection_items_across_scopes(
    toolkit: &ResearchToolkit,
    config: ZoteroConfig<'_>,
    scopes: &[ZoteroLibraryScope],
    request: &ZoteroCollectionItemsRequest<'_>,
    limit: u32,
) -> Result<ZoteroSearchResult> {
    let mut all_items = Vec::new();
    let mut total: u64 = 0;
    let mut any_has_more = false;

    for scope in scopes {
        match zotero::get_collection_items(toolkit.http(), config, scope, request).await {
            Ok(result) => {
                total = total.saturating_add(result.total_available.unwrap_or(0));
                any_has_more = any_has_more || result.has_more;
                all_items.extend(result.items);
            }
            Err(err) => {
                tracing::warn!(?scope, %err, "zotero get_collection_items failed for scope; skipping");
            }
        }
    }

    let truncated = all_items.len() > limit as usize;
    all_items.truncate(limit as usize);

    Ok(ZoteroSearchResult {
        has_more: any_has_more || truncated,
        total_available: Some(total),
        items: all_items,
    })
}

/// Merges `ZoteroCollectionsResult` from multiple scopes.
async fn merge_collections_across_scopes(
    toolkit: &ResearchToolkit,
    config: ZoteroConfig<'_>,
    scopes: &[ZoteroLibraryScope],
    offset: u32,
    limit: u32,
) -> Result<ZoteroCollectionsResult> {
    let mut all_collections = Vec::new();
    let mut total: u64 = 0;
    let mut any_has_more = false;

    for scope in scopes {
        match zotero::get_collections(
            toolkit.http(),
            config,
            scope,
            ZoteroCollectionsRequest { offset: 0, limit },
        )
        .await
        {
            Ok(result) => {
                total = total.saturating_add(result.total_available.unwrap_or(0));
                any_has_more = any_has_more || result.has_more;
                all_collections.extend(result.collections);
            }
            Err(err) => {
                tracing::warn!(?scope, %err, "zotero get_collections failed for scope; skipping");
            }
        }
    }

    let offset = offset as usize;
    let limit = limit as usize;
    let after_offset: Vec<_> = all_collections.into_iter().skip(offset).collect();
    let truncated = after_offset.len() > limit;
    let items: Vec<_> = after_offset.into_iter().take(limit).collect();

    Ok(ZoteroCollectionsResult {
        has_more: any_has_more || truncated,
        total_available: Some(total),
        collections: items,
    })
}

fn recent_sort_field(sort_by: &ZoteroRecentSortBy) -> &'static str {
    match sort_by {
        ZoteroRecentSortBy::DateAdded => "dateAdded",
        ZoteroRecentSortBy::DateModified => "dateModified",
    }
}

async fn fetch_item_across_scopes(
    toolkit: &ResearchToolkit,
    config: ZoteroConfig<'_>,
    scopes: &[ZoteroLibraryScope],
    item_key: &str,
) -> Result<(ZoteroLibraryScope, ZoteroItemDetail)> {
    let mut last_err = None;
    for scope in scopes {
        match zotero::get_item(toolkit.http(), config, scope, item_key).await {
            Ok(result) => return Ok((scope.clone(), result)),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| ResearchError::Internal("no scopes to search".to_string())))
}

#[cfg(test)]
#[path = "zotero/tests.rs"]
mod tests;
