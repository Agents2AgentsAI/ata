use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use futures::stream;
use regex::Regex;
use serde::Serialize;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::cache::FetchOutput;
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
use crate::types::DocumentResolution;
use crate::types::DocumentSourceKind;
use crate::types::ZoteroAdvancedCandidateStrategy;
use crate::types::ZoteroAdvancedCompleteness;
use crate::types::ZoteroAdvancedSearchParams;
use crate::types::ZoteroAdvancedSearchResult;
use crate::types::ZoteroAdvancedSortBy;
use crate::types::ZoteroAnnotation;
use crate::types::ZoteroAnnotationsParams;
use crate::types::ZoteroAnnotationsResult;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroAttachmentsResult;
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
use crate::types::ZoteroNote;
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
#[path = "zotero/content_collector.rs"]
mod content_collector;
#[path = "zotero/grep.rs"]
mod grep;
#[path = "zotero/match_engine.rs"]
mod match_engine;
#[path = "zotero/search_notes.rs"]
mod search_notes;

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

    let mut item = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || async move {
            let scopes = resolved_scopes_to_vec(&normalized.scopes);
            let mut last_err = None;
            for scope in &scopes {
                match zotero::get_item(toolkit.http(), config, scope, &normalized.item_key).await {
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
        truncate_optional_string(&mut item.abstract_text, max_chars as usize);
        truncate_optional_string(&mut item.extra, max_chars as usize);
    }

    Ok(item)
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
                                has_indexed_content,
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

async fn resolve_document_sources(
    toolkit: &ResearchToolkit,
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    has_indexed_content: bool,
    trace: Vec<String>,
) -> DocumentResolution {
    resolve_document_sources_with_probe(
        item,
        attachments,
        toolkit.config().zotero_storage_dir.as_deref(),
        has_indexed_content,
        trace,
        |arxiv_id, html_url| async move {
            ar5iv_available_cached(toolkit, arxiv_id.as_str(), html_url.as_str()).await
        },
    )
    .await
}

async fn resolve_document_sources_with_probe<F, Fut>(
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    storage_root: Option<&str>,
    has_indexed_content: bool,
    mut trace: Vec<String>,
    mut probe_ar5iv: F,
) -> DocumentResolution
where
    F: FnMut(String, String) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let attachment_urls = attachment_pdf_urls(attachments, &mut trace);

    if let Some(item) = item
        && let Some(arxiv_id) = extract_arxiv_id(item, &mut trace)
    {
        let arxiv_pdf = format!("{ARXIV_PDF_BASE_URL}/{arxiv_id}.pdf");
        for probe_id in ar5iv_probe_candidates(arxiv_id.as_str()) {
            let html_url = format!("{AR5IV_BASE_URL}/{probe_id}");
            if probe_ar5iv(probe_id.clone(), html_url.clone()).await {
                trace.push(format!("ar5iv probe succeeded for {probe_id}"));
                let mut fallback_urls = Vec::new();
                push_unique_url(&mut fallback_urls, arxiv_pdf.clone());
                for url in &attachment_urls {
                    push_unique_url(&mut fallback_urls, url.clone());
                }
                return DocumentResolution {
                    source_kind: DocumentSourceKind::Ar5ivHtml,
                    preferred_url: Some(html_url),
                    fallback_urls,
                    local_path: None,
                    trace,
                };
            }
        }

        trace.push("ar5iv probe unavailable; using arXiv PDF".to_string());
        let mut fallback_urls = Vec::new();
        for url in attachment_urls {
            if url != arxiv_pdf {
                push_unique_url(&mut fallback_urls, url);
            }
        }
        return DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some(arxiv_pdf),
            fallback_urls,
            local_path: None,
            trace,
        };
    }

    if let Some(preferred_url) = attachment_urls.first().cloned() {
        let fallback_urls = attachment_urls.into_iter().skip(1).collect::<Vec<_>>();
        trace.push("using attachment PDF URL".to_string());
        return DocumentResolution {
            source_kind: DocumentSourceKind::AttachmentPdfUrl,
            preferred_url: Some(preferred_url),
            fallback_urls,
            local_path: None,
            trace,
        };
    }

    if let Some(local_path) = resolve_local_pdf_path(storage_root, attachments, &mut trace) {
        return DocumentResolution {
            source_kind: DocumentSourceKind::LocalPdfPath,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: Some(local_path),
            trace,
        };
    }

    if has_indexed_content {
        trace.push(
            "no canonical document source resolved; using indexed fulltext fallback".to_string(),
        );
    } else {
        trace.push(
            "no canonical document source resolved and indexed fulltext is empty".to_string(),
        );
    }

    DocumentResolution {
        source_kind: DocumentSourceKind::IndexedFulltext,
        preferred_url: None,
        fallback_urls: Vec::new(),
        local_path: None,
        trace,
    }
}

fn attachment_pdf_urls(attachments: &[ZoteroAttachment], trace: &mut Vec<String>) -> Vec<String> {
    let mut urls = Vec::new();
    for attachment in attachments {
        let Some(raw_url) = attachment.url.as_deref() else {
            continue;
        };
        let Some(url) = sanitize_http_url(raw_url) else {
            trace.push(format!(
                "ignored non-http attachment URL for attachment {}",
                attachment.key
            ));
            continue;
        };
        if attachment_looks_like_pdf(attachment, Some(url.as_str()), attachment.path.as_deref()) {
            push_unique_url(&mut urls, url);
        }
    }
    urls
}

fn attachment_looks_like_pdf(
    attachment: &ZoteroAttachment,
    url: Option<&str>,
    path_hint: Option<&str>,
) -> bool {
    if attachment
        .content_type
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/pdf"))
    {
        return true;
    }
    if attachment.filename.as_deref().is_some_and(has_pdf_suffix) {
        return true;
    }
    if attachment.title.as_deref().is_some_and(has_pdf_suffix) {
        return true;
    }
    if url.is_some_and(is_pdf_url) {
        return true;
    }
    path_hint.is_some_and(has_pdf_suffix)
}

fn has_pdf_suffix(value: &str) -> bool {
    value.trim().to_ascii_lowercase().ends_with(".pdf")
}

fn is_pdf_url(url: &str) -> bool {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        return parsed.path().to_ascii_lowercase().ends_with(".pdf");
    }

    has_pdf_suffix(url)
}

fn sanitize_http_url(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parsed = reqwest::Url::parse(trimmed).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

fn resolve_local_pdf_path(
    storage_root: Option<&str>,
    attachments: &[ZoteroAttachment],
    trace: &mut Vec<String>,
) -> Option<String> {
    let Some(storage_root) = storage_root
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        trace.push(format!(
            "local path fallback unavailable: {ZOTERO_STORAGE_DIR_ENV} is not set"
        ));
        return None;
    };

    if !storage_root.is_absolute() {
        trace.push(format!(
            "local path fallback unavailable: {ZOTERO_STORAGE_DIR_ENV} must be an absolute path"
        ));
        return None;
    }

    for attachment in attachments {
        let Some(path_hint) = attachment
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        if !attachment_looks_like_pdf(attachment, attachment.url.as_deref(), Some(path_hint)) {
            continue;
        }

        let Some(storage_relative) = path_hint.strip_prefix("storage:") else {
            trace.push(format!(
                "ignored attachment path for {} because it does not use storage: prefix",
                attachment.key
            ));
            continue;
        };

        if !is_safe_storage_component(attachment.key.as_str()) {
            trace.push(format!(
                "ignored attachment path for {} because attachment key is not path-safe",
                attachment.key
            ));
            continue;
        }

        let relative_path = Path::new(storage_relative.trim_start_matches('/'));
        if !is_safe_relative_path(relative_path) {
            trace.push(format!(
                "ignored attachment path for {} due to unsafe relative path",
                attachment.key
            ));
            continue;
        }

        let local_path = storage_root.join(&attachment.key).join(relative_path);
        let Some(local_path) = local_path.to_str() else {
            trace.push(format!(
                "ignored local path for {} because the resolved path is non-UTF-8",
                attachment.key
            ));
            continue;
        };
        trace.push(format!(
            "resolved local PDF path from attachment {}",
            attachment.key
        ));
        return Some(local_path.to_string());
    }

    None
}

fn is_safe_storage_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_safe_relative_path(path: &Path) -> bool {
    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_component = true,
            Component::CurDir => {}
            _ => return false,
        }
    }
    has_component
}

fn extract_arxiv_id(item: &ZoteroItemDetail, trace: &mut Vec<String>) -> Option<String> {
    if let Some(url) = item.url.as_deref()
        && let Some(arxiv_id) = extract_arxiv_id_from_url(url)
    {
        trace.push(format!("detected arXiv id from item URL: {arxiv_id}"));
        return Some(arxiv_id);
    }

    if let Some(extra) = item.extra.as_deref()
        && let Some(arxiv_id) = extract_arxiv_id_from_extra(extra)
    {
        trace.push(format!("detected arXiv id from item extra: {arxiv_id}"));
        return Some(arxiv_id);
    }

    None
}

fn extract_arxiv_id_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !(host == "arxiv.org" || host.ends_with(".arxiv.org")) {
        return None;
    }

    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    let (prefix, suffix_segments) = segments.split_first()?;

    let candidate = if *prefix == "abs" || *prefix == "pdf" {
        suffix_segments.join("/")
    } else {
        return None;
    };

    let candidate = candidate.strip_suffix(".pdf").unwrap_or(candidate.as_str());
    normalize_arxiv_id(candidate)
}

fn extract_arxiv_id_from_extra(extra: &str) -> Option<String> {
    arxiv_id_regex()
        .captures(extra)
        .and_then(|capture| capture.get(1))
        .map(|match_| match_.as_str().to_string())
}

fn normalize_arxiv_id(raw_value: &str) -> Option<String> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    let mut prefix = String::new();
    for _ in 0..6 {
        if let Some(ch) = chars.next() {
            prefix.push(ch);
        } else {
            prefix.clear();
            break;
        }
    }
    let without_prefix = if prefix.eq_ignore_ascii_case("arxiv:") {
        chars.as_str()
    } else {
        trimmed
    };
    let cleaned = without_prefix.trim().trim_end_matches(".pdf").trim();

    arxiv_id_regex()
        .captures(cleaned)
        .and_then(|capture| capture.get(1))
        .map(|match_| match_.as_str().to_string())
}

fn arxiv_id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b((?:\d{4}\.\d{4,5}|[a-z][a-z.\-]+/\d{7})(?:v\d+)?)\b")
            .unwrap_or_else(|err| panic!("arXiv id regex must compile: {err}"))
    })
}

fn ar5iv_probe_candidates(arxiv_id: &str) -> Vec<String> {
    let mut candidates = vec![arxiv_id.to_string()];
    if let Some((without_version, version_suffix)) = arxiv_id.rsplit_once('v')
        && !without_version.is_empty()
        && !version_suffix.is_empty()
        && version_suffix.chars().all(|ch| ch.is_ascii_digit())
        && without_version != arxiv_id
    {
        candidates.push(without_version.to_string());
    }
    candidates
}

fn push_unique_url(urls: &mut Vec<String>, candidate: String) {
    if !urls.iter().any(|existing| existing == &candidate) {
        urls.push(candidate);
    }
}

async fn ar5iv_available_cached(toolkit: &ResearchToolkit, arxiv_id: &str, html_url: &str) -> bool {
    let params_hash = match hash_cache_payload(&arxiv_id) {
        Ok(hash) => hash,
        Err(err) => {
            tracing::warn!(%err, arxiv_id, "failed to hash ar5iv probe cache key");
            return false;
        }
    };

    let key = CacheKey {
        tool_name: "zotero_ar5iv_probe",
        params_hash,
    };

    match toolkit
        .cache()
        .get_or_fetch_with_meta_ttls(
            key,
            AR5IV_PROBE_CACHE_TTL,
            AR5IV_PROBE_NEGATIVE_CACHE_TTL,
            || async move {
                let probe_result = probe_ar5iv_html(toolkit, html_url).await;
                let data = serde_json::to_value(probe_result).map_err(|err| {
                    ResearchError::Internal(format!(
                        "failed to serialize ar5iv probe result: {err}"
                    ))
                })?;
                if probe_result {
                    Ok(FetchOutput::positive(data))
                } else {
                    Ok(FetchOutput::negative(data))
                }
            },
        )
        .await
    {
        Ok(output) => match serde_json::from_value::<bool>(output.data) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(%err, arxiv_id, "failed to deserialize ar5iv probe result");
                false
            }
        },
        Err(err) => {
            tracing::warn!(%err, arxiv_id, "ar5iv probe failed");
            false
        }
    }
}

async fn probe_ar5iv_html(toolkit: &ResearchToolkit, html_url: &str) -> bool {
    let response = match tokio::time::timeout(
        AR5IV_PROBE_TIMEOUT,
        toolkit
            .http()
            .execute_response(ResearchApi::Arxiv, || toolkit.http().client().get(html_url)),
    )
    .await
    {
        Ok(Ok(response)) => response,
        _ => return false,
    };

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_ascii_lowercase);
    if !content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return false;
    }

    let html_prefix = match tokio::time::timeout(
        AR5IV_PROBE_TIMEOUT,
        read_response_prefix(response, AR5IV_PROBE_MAX_BODY_BYTES),
    )
    .await
    {
        Ok(Some(prefix)) => prefix,
        _ => return false,
    };

    let normalized_html = html_prefix.to_ascii_lowercase();
    ![
        "no paper found for arxiv id",
        "this paper is not yet available",
        "unable to fetch source",
        "ar5iv is temporarily unavailable",
    ]
    .iter()
    .any(|needle| normalized_html.contains(needle))
}

async fn read_response_prefix(mut response: reqwest::Response, max_bytes: usize) -> Option<String> {
    let mut bytes = Vec::with_capacity(max_bytes);
    while bytes.len() < max_bytes {
        let chunk = response.chunk().await.ok()??;
        let remaining = max_bytes.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Some(String::from_utf8_lossy(&bytes).to_string())
}

fn map_zotero_annotation(annotation: zotero::ZoteroAnnotation) -> ZoteroAnnotation {
    let annotation_type = annotation
        .annotation_type
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| {
            matches!(
                value.as_str(),
                "highlight" | "note" | "image" | "underline" | "strikethrough" | "ink"
            )
        })
        .unwrap_or_else(|| "unknown".to_string());

    ZoteroAnnotation {
        key: annotation.key,
        parent_item: normalize_optional_string(annotation.parent_item),
        annotation_type,
        annotation_text: annotation
            .annotation_text
            .map(|text| match_engine::strip_html_to_text(text.as_str())),
        annotation_comment: annotation
            .annotation_comment
            .map(|comment| match_engine::strip_html_to_text(comment.as_str())),
        annotation_color: normalize_optional_string(annotation.annotation_color),
        annotation_page_label: normalize_optional_string(annotation.annotation_page_label),
        annotation_sort_index: normalize_optional_string(annotation.annotation_sort_index),
        parent_item_title: None,
        source_meta: annotation.source_meta,
    }
}

fn apply_items_budget(items: &mut [ZoteroItem], max_chars_per_item: Option<u32>) {
    let max_chars = max_chars_per_item.map(|value| value as usize);

    for item in items {
        if let Some(max) = max_chars {
            truncate_optional_string(&mut item.abstract_snippet, max);
            item.title = truncate_chars(&item.title, max);
            item.authors = truncate_chars(&item.authors, max);
            item.tags = item
                .tags
                .iter()
                .map(|tag| truncate_chars(tag, max))
                .collect();
        }
    }
}

fn apply_notes_budget(notes: &mut [ZoteroNote], max_chars: usize) {
    for note in notes {
        truncate_optional_string(&mut note.title, max_chars);
        truncate_optional_string(&mut note.note, max_chars);
    }
}

fn apply_annotations_budget(annotations: &mut [ZoteroAnnotation], max_chars: usize) {
    for annotation in annotations {
        truncate_optional_string(&mut annotation.annotation_text, max_chars);
        truncate_optional_string(&mut annotation.annotation_comment, max_chars);
    }
}

fn apply_attachments_budget(attachments: &mut [ZoteroAttachment], max_chars: usize) {
    for attachment in attachments {
        truncate_optional_string(&mut attachment.title, max_chars);
        truncate_optional_string(&mut attachment.filename, max_chars);
        truncate_optional_string(&mut attachment.content_type, max_chars);
        truncate_optional_string(&mut attachment.link_mode, max_chars);
        truncate_optional_string(&mut attachment.url, max_chars);
        truncate_optional_string(&mut attachment.path, max_chars);
    }
}

fn truncate_optional_string(value: &mut Option<String>, max_chars: usize) {
    if let Some(value_ref) = value.as_mut() {
        *value_ref = truncate_chars(value_ref, max_chars);
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::query_param;

    use crate::ResearchToolkit;
    use crate::config::ResearchConfig;
    use crate::error::ResearchError;
    use crate::tools::test_helpers::build_test_toolkit_with_config;
    use crate::types::DocumentResolution;
    use crate::types::DocumentSourceKind;
    use crate::types::ZoteroAdvancedCandidateStrategy;
    use crate::types::ZoteroAdvancedCompleteness;
    use crate::types::ZoteroAdvancedSearchParams;
    use crate::types::ZoteroAdvancedSortBy;
    use crate::types::ZoteroAnnotation;
    use crate::types::ZoteroAnnotationsParams;
    use crate::types::ZoteroAttachment;
    use crate::types::ZoteroCollectionItemsParams;
    use crate::types::ZoteroCollectionsParams;
    use crate::types::ZoteroGrepCandidateStrategy;
    use crate::types::ZoteroGrepField;
    use crate::types::ZoteroGrepMatchMode;
    use crate::types::ZoteroGrepParams;
    use crate::types::ZoteroItemDetail;
    use crate::types::ZoteroItemParams;
    use crate::types::ZoteroListGroupsParams;
    use crate::types::ZoteroRecentParams;
    use crate::types::ZoteroRecentSortBy;
    use crate::types::ZoteroSearchCondition;
    use crate::types::ZoteroSearchConditionField;
    use crate::types::ZoteroSearchConditionOperation;
    use crate::types::ZoteroSearchNotesParams;
    use crate::types::ZoteroSearchParams;
    use crate::types::ZoteroSortDirection;
    use crate::types::ZoteroTagSearchParams;
    use crate::types::ZoteroTagsParams;

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_and_get_item_use_user_scope() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "diffusion"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Diffusion Models in Vision",
                        "creators": [{"firstName": "Alice", "lastName": "Kim"}],
                        "date": "2023-06-01",
                        "DOI": "10.1000/x",
                        "abstractNote": "A long abstract.",
                        "tags": [{"tag": "vision"}, {"tag": "diffusion"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Diffusion Models in Vision",
                    "creators": [{"firstName": "Alice", "lastName": "Kim"}],
                    "date": "2023-06-01",
                    "DOI": "10.1000/x",
                    "abstractNote": "A long abstract.",
                    "publicationTitle": "NeurIPS",
                    "tags": [{"tag": "vision"}, {"tag": "diffusion"}]
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let search = toolkit
            .zotero_search(ZoteroSearchParams {
                query: "diffusion".to_string(),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(20),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search should succeed");

        assert_eq!(search.items.len(), 1);
        assert_eq!(search.items[0].key, "ITEM1");
        assert_eq!(search.items[0].tags, vec!["vision", "diffusion"]);

        let item = toolkit
            .zotero_get_item(ZoteroItemParams {
                item_key: "ITEM1".to_string(),
                library_type: None,
                library_id: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_item should succeed");

        assert_eq!(item.key, "ITEM1");
        assert_eq!(item.publication, Some("NeurIPS".to_string()));
        assert_eq!(item.tags, vec!["vision", "diffusion"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_fulltext_notes_and_attachments_are_supported() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/fulltext"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "abcdefghijklmnopqrstuvwxyz"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "note"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "note title",
                        "note": "This is a very long note body.",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "attachment"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ATT1",
                    "data": {
                        "itemType": "attachment",
                        "title": "paper.pdf",
                        "filename": "paper.pdf",
                        "contentType": "application/pdf",
                        "linkMode": "imported_file",
                        "url": "https://example.com/paper.pdf",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let fulltext = toolkit
            .zotero_get_fulltext(ZoteroItemParams {
                item_key: "ITEM1".to_string(),
                library_type: None,
                library_id: None,
                max_chars_per_item: Some(5),
            })
            .await
            .expect("zotero_get_fulltext should succeed");
        assert_eq!(fulltext.content, "ab...");
        let resolution = fulltext
            .resolution
            .expect("zotero_get_fulltext should include resolution");
        assert_eq!(resolution.source_kind, DocumentSourceKind::AttachmentPdfUrl);
        assert_eq!(
            resolution.preferred_url,
            Some("https://example.com/paper.pdf".to_string())
        );
        assert_eq!(resolution.fallback_urls, Vec::<String>::new());
        assert_eq!(resolution.local_path, None);
        assert!(
            resolution
                .trace
                .iter()
                .any(|entry| entry.contains("item metadata lookup failed")),
            "expected metadata lookup warning in trace: {:?}",
            resolution.trace
        );

        let notes = toolkit
            .zotero_get_notes(ZoteroItemParams {
                item_key: "ITEM1".to_string(),
                library_type: None,
                library_id: None,
                max_chars_per_item: Some(4),
            })
            .await
            .expect("zotero_get_notes should succeed");
        assert_eq!(notes.notes.len(), 1);
        assert_eq!(notes.notes[0].title, Some("n...".to_string()));

        let attachments = toolkit
            .zotero_get_attachments(ZoteroItemParams {
                item_key: "ITEM1".to_string(),
                library_type: None,
                library_id: None,
                max_chars_per_item: Some(5),
            })
            .await
            .expect("zotero_get_attachments should succeed");
        assert_eq!(attachments.attachments.len(), 1);
        assert_eq!(attachments.attachments[0].title, Some("pa...".to_string()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_item_scoped_parses_annotation_fields() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "10"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "1")
                    .set_body_json(serde_json::json!([
                        {
                            "key": "ANNO1",
                            "data": {
                                "itemType": "annotation",
                                "parentItem": "ITEM1",
                                "annotationType": "highlight",
                                "annotationText": "<p>Important <b>finding</b></p>",
                                "annotationComment": "<div>See <i>section 2</i></div>",
                                "annotationColor": "#ffd400",
                                "annotationPageLabel": "12",
                                "annotationSortIndex": "00012|000001|00000"
                            }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM1".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                include_parent_context: Some(false),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(result.item_key, Some("ITEM1".to_string()));
        assert_eq!(result.total_available, Some(1));
        assert_eq!(result.has_more, false);
        assert_eq!(
            result.annotations,
            vec![ZoteroAnnotation {
                key: "ANNO1".to_string(),
                parent_item: Some("ITEM1".to_string()),
                annotation_type: "highlight".to_string(),
                annotation_text: Some("Important finding".to_string()),
                annotation_comment: Some("See section 2".to_string()),
                annotation_color: Some("#ffd400".to_string()),
                annotation_page_label: Some("12".to_string()),
                annotation_sort_index: Some("00012|000001|00000".to_string()),
                parent_item_title: None,
                source_meta: None,
            }]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_library_scope_exposes_pagination_headers() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "1"))
            .and(query_param("limit", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "3")
                    .set_body_json(serde_json::json!([
                        {
                            "key": "ANNO2",
                            "data": {
                                "itemType": "annotation",
                                "parentItem": "ITEM2",
                                "annotationType": "underline",
                                "annotationText": "alpha"
                            }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: None,
                library_type: None,
                library_id: None,
                offset: Some(1),
                limit: Some(1),
                include_parent_context: Some(false),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(result.item_key, None);
        assert_eq!(result.total_available, Some(3));
        assert_eq!(result.has_more, true);
        assert_eq!(
            result.annotations,
            vec![ZoteroAnnotation {
                key: "ANNO2".to_string(),
                parent_item: Some("ITEM2".to_string()),
                annotation_type: "underline".to_string(),
                annotation_text: Some("alpha".to_string()),
                annotation_comment: None,
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: None,
                source_meta: None,
            }]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_preserves_extended_annotation_types() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO_STRIKE",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "ITEM1",
                        "annotationType": "strikethrough",
                        "annotationText": "x"
                    }
                },
                {
                    "key": "ANNO_INK",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "ITEM1",
                        "annotationType": "ink",
                        "annotationComment": "pen"
                    }
                },
                {
                    "key": "ANNO_UNKNOWN",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "ITEM1",
                        "annotationType": "weird-new-type",
                        "annotationComment": "fallback"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: None,
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                include_parent_context: Some(false),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(
            result
                .annotations
                .iter()
                .map(|annotation| annotation.annotation_type.clone())
                .collect::<Vec<_>>(),
            vec![
                "strikethrough".to_string(),
                "ink".to_string(),
                "unknown".to_string()
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_returns_empty_list_when_item_has_no_annotations() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM_EMPTY/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "0")
                    .set_body_json(serde_json::json!([])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM_EMPTY".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(false),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        assert_eq!(result.item_key, Some("ITEM_EMPTY".to_string()));
        assert_eq!(result.total_available, Some(0));
        assert_eq!(result.has_more, false);
        assert_eq!(result.annotations, Vec::<ZoteroAnnotation>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_can_enrich_parent_item_title_and_cache_result() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO3",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "PARENT1",
                        "annotationType": "note",
                        "annotationComment": "memo"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "PARENT1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Parent Item Title",
                    "creators": []
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut first = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM1".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(true),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        let mut second = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM1".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(true),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut first.annotations {
            annotation.source_meta = None;
        }
        for annotation in &mut second.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(
            first.annotations,
            vec![ZoteroAnnotation {
                key: "ANNO3".to_string(),
                parent_item: Some("PARENT1".to_string()),
                annotation_type: "note".to_string(),
                annotation_text: None,
                annotation_comment: Some("memo".to_string()),
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: Some("Parent Item Title".to_string()),
                source_meta: None,
            }]
        );
        assert_eq!(first.annotations, second.annotations);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_parent_enrichment_tolerates_partial_failures() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO_OK",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "PARENT_OK",
                        "annotationType": "highlight",
                        "annotationText": "keep"
                    }
                },
                {
                    "key": "ANNO_MISSING",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "PARENT_MISSING",
                        "annotationType": "note",
                        "annotationComment": "still return"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT_OK"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "PARENT_OK",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Resolvable Parent",
                    "creators": []
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT_MISSING"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM1".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(true),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(
            result.annotations,
            vec![
                ZoteroAnnotation {
                    key: "ANNO_OK".to_string(),
                    parent_item: Some("PARENT_OK".to_string()),
                    annotation_type: "highlight".to_string(),
                    annotation_text: Some("keep".to_string()),
                    annotation_comment: None,
                    annotation_color: None,
                    annotation_page_label: None,
                    annotation_sort_index: None,
                    parent_item_title: Some("Resolvable Parent".to_string()),
                    source_meta: None,
                },
                ZoteroAnnotation {
                    key: "ANNO_MISSING".to_string(),
                    parent_item: Some("PARENT_MISSING".to_string()),
                    annotation_type: "note".to_string(),
                    annotation_text: None,
                    annotation_comment: Some("still return".to_string()),
                    annotation_color: None,
                    annotation_page_label: None,
                    annotation_sort_index: None,
                    parent_item_title: None,
                    source_meta: None,
                }
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_library_scope_can_enrich_parent_context() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO_LIB",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "PARENT_LIB",
                        "annotationType": "note",
                        "annotationComment": "library scoped"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT_LIB"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "PARENT_LIB",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Library Parent",
                    "creators": []
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: None,
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(true),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(
            result.annotations,
            vec![ZoteroAnnotation {
                key: "ANNO_LIB".to_string(),
                parent_item: Some("PARENT_LIB".to_string()),
                annotation_type: "note".to_string(),
                annotation_text: None,
                annotation_comment: Some("library scoped".to_string()),
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: Some("Library Parent".to_string()),
                source_meta: None,
            }]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_annotations_truncates_text_and_comment_after_html_strip() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO4",
                    "data": {
                        "itemType": "annotation",
                        "parentItem": "ITEM1",
                        "annotationType": "highlight",
                        "annotationText": "<p>abcdefghi</p>",
                        "annotationComment": "<div>123456789</div>"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let mut result = toolkit
            .zotero_get_annotations(ZoteroAnnotationsParams {
                item_key: Some("ITEM1".to_string()),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(50),
                include_parent_context: Some(false),
                max_chars_per_item: Some(5),
            })
            .await
            .expect("zotero_get_annotations should succeed");

        for annotation in &mut result.annotations {
            annotation.source_meta = None;
        }

        assert_eq!(
            result.annotations,
            vec![ZoteroAnnotation {
                key: "ANNO4".to_string(),
                parent_item: Some("ITEM1".to_string()),
                annotation_type: "highlight".to_string(),
                annotation_text: Some("ab...".to_string()),
                annotation_comment: Some("12...".to_string()),
                annotation_color: None,
                annotation_page_label: None,
                annotation_sort_index: None,
                parent_item_title: None,
                source_meta: None,
            }]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_by_tag_requires_all_tags() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ml"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item One",
                        "creators": [],
                        "tags": [{"tag": "ml"}, {"tag": "vision"}]
                    }
                },
                {
                    "key": "ITEM2",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item Two",
                        "creators": [],
                        "tags": [{"tag": "ml"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "vision"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item One",
                        "creators": [],
                        "tags": [{"tag": "ml"}, {"tag": "vision"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let result = toolkit
            .zotero_search_by_tag(ZoteroTagSearchParams {
                tags: vec!["ml".to_string(), "vision".to_string()],
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_by_tag should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_by_tag_dedups_tags_case_insensitively() {
        let server = MockServer::start().await;

        let tag_result = serde_json::json!([
            {
                "key": "ITEM1",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Item One",
                    "creators": [],
                    "tags": [{"tag": "ml"}]
                }
            }
        ]);

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ML"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tag_result.clone()))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ml"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tag_result))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let result = toolkit
            .zotero_search_by_tag(ZoteroTagSearchParams {
                tags: vec!["ML".to_string(), "ml".to_string()],
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_by_tag should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_by_tag_paginates_each_tag_before_intersection() {
        let server = MockServer::start().await;

        let first_ml_page = (0..100)
            .map(|idx| {
                serde_json::json!({
                    "key": format!("ML{idx:03}"),
                    "data": {
                        "itemType": "journalArticle",
                        "title": format!("ML Item {idx}"),
                        "creators": [],
                        "tags": [{"tag": "ml"}]
                    }
                })
            })
            .collect::<Vec<_>>();

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ml"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first_ml_page))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ml"))
            .and(query_param("start", "100"))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "TARGET",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Target Item",
                        "creators": [],
                        "tags": [{"tag": "ml"}, {"tag": "vision"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "vision"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "TARGET",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Target Item",
                        "creators": [],
                        "tags": [{"tag": "ml"}, {"tag": "vision"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let result = toolkit
            .zotero_search_by_tag(ZoteroTagSearchParams {
                tags: vec!["ml".to_string(), "vision".to_string()],
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_by_tag should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "TARGET");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_tags_uses_tags_endpoint_and_link_header_for_pagination() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/tags"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "Link",
                        r#"<https://api.zotero.org/users/123/tags?start=100&limit=100>; rel="next""#,
                    )
                    .set_body_json(serde_json::json!([
                        { "tag": "ml" },
                        { "tag": "vision" },
                        { "tag": "   " }
                    ])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_get_tags(ZoteroTagsParams {
                library_type: None,
                library_id: None,
                offset: None,
                limit: None,
            })
            .await
            .expect("zotero_get_tags should succeed");

        assert_eq!(result.tags, vec!["ml".to_string(), "vision".to_string()]);
        assert_eq!(result.has_more, true);
        assert_eq!(result.total_available, 4);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_tags_dedups_case_insensitively_and_uses_raw_page_for_has_more() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/tags"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "tag": "ml" },
                { "tag": "ML" },
                { "tag": "ml" }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_get_tags(ZoteroTagsParams {
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(3),
            })
            .await
            .expect("zotero_get_tags should succeed");

        assert_eq!(result.tags, vec!["ml".to_string()]);
        assert_eq!(result.has_more, true);
        assert_eq!(result.total_available, 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_recent_defaults_to_date_added_descending() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateAdded"))
            .and(query_param("direction", "desc"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "25"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "RECENT1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Recently Added",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_get_recent(ZoteroRecentParams {
                library_type: None,
                library_id: None,
                offset: None,
                limit: None,
                item_type: None,
                sort_by: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_recent should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "RECENT1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_get_recent_supports_item_type_and_date_modified_sort() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "journalArticle"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "RECENT2",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Recently Modified",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_get_recent(ZoteroRecentParams {
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(25),
                item_type: Some("journalArticle".to_string()),
                sort_by: Some(ZoteroRecentSortBy::DateModified),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_recent should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "RECENT2");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_tags_and_recent_limits_are_clamped() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/tags"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "200"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateAdded"))
            .and(query_param("direction", "desc"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let tags = toolkit
            .zotero_get_tags(ZoteroTagsParams {
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(500),
            })
            .await
            .expect("zotero_get_tags should clamp limit");
        assert_eq!(tags.tags, Vec::<String>::new());

        let recent = toolkit
            .zotero_get_recent(ZoteroRecentParams {
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(0),
                item_type: None,
                sort_by: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_recent should clamp limit");
        assert_eq!(recent.items.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_collections_support_group_scope_override() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/groups/999/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "COL1",
                    "data": {"name": "Important"}
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/groups/999/collections/COL1/items"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Group Item",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        let collections = toolkit
            .zotero_get_collections(ZoteroCollectionsParams {
                library_type: Some("group".to_string()),
                library_id: Some("999".to_string()),
                offset: Some(0),
                limit: Some(20),
            })
            .await
            .expect("zotero_get_collections should succeed");

        assert_eq!(collections.collections.len(), 1);
        assert_eq!(collections.collections[0].name, "Important");

        let items = toolkit
            .zotero_get_collection_items(ZoteroCollectionItemsParams {
                collection_key: "COL1".to_string(),
                library_type: Some("group".to_string()),
                library_id: Some("999".to_string()),
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_collection_items should succeed");

        assert_eq!(items.items.len(), 1);
        assert_eq!(items.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_list_groups_uses_local_user_zero_in_local_mode() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/users/0/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "id": 999,
                    "data": {
                        "name": "Research Lab",
                        "description": "Shared papers"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: None,
            zotero_user_id: None,
            zotero_group_id: None,
            zotero_base_url: format!("{}/api/", server.uri()),
            ..ResearchConfig::default()
        });

        let result = toolkit
            .zotero_list_groups(ZoteroListGroupsParams {
                user_id: None,
                offset: Some(0),
                limit: Some(20),
            })
            .await
            .expect("local zotero list groups should succeed without API key");

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].id, "999");
        assert_eq!(result.groups[0].name, "Research Lab");
        assert_eq!(
            result.groups[0].description,
            Some("Shared papers".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_local_mode_defaults_to_user_zero_without_api_key() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/users/0/items"))
            .and(query_param("q", "diffusion"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM_LOCAL",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Local Zotero Item",
                        "creators": [],
                        "tags": [{"tag": "local"}]
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: None,
            zotero_user_id: None,
            zotero_group_id: None,
            zotero_base_url: format!("{}/api/", server.uri()),
            ..ResearchConfig::default()
        });

        let result = toolkit
            .zotero_search(ZoteroSearchParams {
                query: "diffusion".to_string(),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("local zotero search should succeed without API key");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "ITEM_LOCAL");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_uses_collector_for_note_conditions() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item One",
                        "creators": []
                    }
                },
                {
                    "key": "ITEM2",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item Two",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "note"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "note": "<p>Contains target keyword</p>",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM2/children"))
            .and(query_param("itemType", "note"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE2",
                    "data": {
                        "itemType": "note",
                        "note": "<p>No signal here</p>",
                        "parentItem": "ITEM2"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::Note,
                    operation: ZoteroSearchConditionOperation::Contains,
                    value: Some("target keyword".to_string()),
                    case_sensitive: Some(false),
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
        );
        assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Approximate);
        assert_eq!(result.scanned_items, 2);
        assert_eq!(result.results.items.len(), 1);
        assert_eq!(result.results.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_warns_on_default_fulltext_cap() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item One",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/fulltext"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "fulltext signal appears here"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::Fulltext,
                    operation: ZoteroSearchConditionOperation::Contains,
                    value: Some("signal".to_string()),
                    case_sensitive: Some(false),
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("default 10000 character cap"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_does_not_emit_empty_hints_for_pagination_gap() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Transformer Models",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::Title,
                    operation: ZoteroSearchConditionOperation::Contains,
                    value: Some("Transformer".to_string()),
                    case_sensitive: Some(false),
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                offset: Some(10),
                limit: Some(5),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert_eq!(result.results.items.len(), 0);
        assert_eq!(result.results.total_available, Some(1));
        assert_eq!(result.results.has_more, false);
        assert_eq!(result.hints, Vec::<String>::new());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_uses_server_filtered_strategy_for_tag_equals() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("tag", "ml"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "ML Systems",
                        "creators": [],
                        "tags": [{ "tag": "ml" }]
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::Tag,
                    operation: ZoteroSearchConditionOperation::Equals,
                    value: Some("ml".to_string()),
                    case_sensitive: Some(false),
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroAdvancedCandidateStrategy::ServerFiltered
        );
        assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Exact);
        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.results.items.len(), 1);
        assert_eq!(result.results.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_matches_title_literal() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Diffusion Models for Vision",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "diffusion".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: None,
                query_hint: None,
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(10),
                limit_matches: Some(5),
                max_matches_per_item: Some(5),
                context_chars: Some(50),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.matches[0].item_key, "ITEM1");
        assert_eq!(result.matches[0].field, "title");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_uses_parent_scoped_grep_adapter() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT1/children"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "Note One",
                        "parentItem": "PARENT1"
                    }
                },
                {
                    "key": "ANN1",
                    "data": {
                        "itemType": "annotation",
                        "title": "Annotation One",
                        "parentItem": "PARENT1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/NOTE1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "note": "<p>Contains target phrase</p>",
                    "parentItem": "PARENT1"
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ANN1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "ANN1",
                "data": {
                    "itemType": "annotation",
                    "parentItem": "PARENT1",
                    "annotationType": "highlight",
                    "annotationText": "<p>Target evidence</p>"
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "target".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: Some("PARENT1".to_string()),
                include_annotations: Some(true),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(result.query, "target");
        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::ParentScoped
        );
        assert_eq!(result.scanned_items, 2);
        assert_eq!(result.notes.len(), 2);
        assert_eq!(result.total_available, Some(2));
        assert_eq!(result.has_more, false);
        assert_eq!(result.hints, Vec::<String>::new());
        assert_eq!(
            result
                .notes
                .iter()
                .map(|note| note.parent_item.clone())
                .collect::<Vec<_>>(),
            vec![Some("PARENT1".to_string()), Some("PARENT1".to_string())]
        );
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.field == "note" && note.item_key == "NOTE1")
        );
        assert!(
            result
                .notes
                .iter()
                .any(|note| note.field == "annotation_text" && note.item_key == "ANN1")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_reports_note_origin_for_library_scoped_matches() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "target"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Parent Item",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "note"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "parentItem": "ITEM1",
                        "note": "<p>Target in note body</p>"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "target".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: None,
                include_annotations: Some(false),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::QueryFiltered
        );
        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.total_available, Some(1));
        assert_eq!(result.has_more, false);
        assert_eq!(result.hints, Vec::<String>::new());
        assert_eq!(result.notes[0].item_key, "NOTE1");
        assert_eq!(result.notes[0].parent_item.as_deref(), Some("ITEM1"));
        assert_eq!(result.notes[0].field, "note");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_regex_mode_disables_query_prefilter_hint() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("sort", "dateModified"))
            .and(query_param("direction", "desc"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "Note One",
                        "parentItem": "PARENT1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/NOTE1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "note": "<p>Cell cycle evidence</p>",
                    "parentItem": "PARENT1"
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: r"\bcell\b".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Regex),
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: None,
                include_annotations: Some(false),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::RecentModified
        );
        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].item_key, "NOTE1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_parent_scope_respects_include_annotations_false() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT1/children"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "Note One",
                        "parentItem": "PARENT1"
                    }
                },
                {
                    "key": "ANN1",
                    "data": {
                        "itemType": "annotation",
                        "title": "Annotation One",
                        "parentItem": "PARENT1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/NOTE1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "note": "<p>No keyword here</p>",
                    "parentItem": "PARENT1"
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "target".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: Some("PARENT1".to_string()),
                include_annotations: Some(false),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::ParentScoped
        );
        assert_eq!(result.scanned_items, 2);
        assert_eq!(result.notes, Vec::new());
        assert_eq!(result.total_available, Some(0));
        assert_eq!(result.has_more, false);
        assert!(!result.hints.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_respects_max_chars_per_item_bounds() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "needle"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Parent Item",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "note"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "parentItem": "ITEM1",
                        "note": "<p>aaaaaaaaaaaaaaa needle</p>"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "needle".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: None,
                include_annotations: Some(false),
                limit: Some(10),
                max_chars_per_item: Some(10),
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::QueryFiltered
        );
        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.notes, Vec::new());
        assert_eq!(result.total_available, Some(0));
        assert_eq!(result.has_more, false);
        assert!(!result.hints.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_limit_sets_has_more_and_total_available() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/PARENT1/children"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "Note One",
                        "parentItem": "PARENT1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/NOTE1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "NOTE1",
                "data": {
                    "itemType": "note",
                    "note": "<p>target alpha target beta target gamma</p>",
                    "parentItem": "PARENT1"
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "target".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: Some("PARENT1".to_string()),
                include_annotations: Some(false),
                limit: Some(2),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(result.notes.len(), 2);
        assert_eq!(result.total_available, None);
        assert_eq!(result.has_more, true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_rejects_empty_query() {
        let toolkit = build_test_toolkit("http://127.0.0.1".to_string());
        let err = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "   ".to_string(),
                match_mode: None,
                case_sensitive: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                parent_item_key: None,
                include_annotations: Some(true),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect_err("empty queries should be rejected");

        assert!(
            matches!(err, ResearchError::InvalidInput(message) if message.contains("must not be empty"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_multi_scope_resorts_and_computes_pagination_metadata() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "2")
                    .set_body_json(serde_json::json!([
                        {
                            "key": "USER_2020",
                            "data": {
                                "itemType": "journalArticle",
                                "title": "User 2020",
                                "creators": [],
                                "date": "2020"
                            }
                        },
                        {
                            "key": "USER_2023",
                            "data": {
                                "itemType": "journalArticle",
                                "title": "User 2023",
                                "creators": [],
                                "date": "2023"
                            }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/groups/456/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "2")
                    .set_body_json(serde_json::json!([
                        {
                            "key": "GROUP_2019",
                            "data": {
                                "itemType": "journalArticle",
                                "title": "Group 2019",
                                "creators": [],
                                "date": "2019"
                            }
                        },
                        {
                            "key": "GROUP_2021",
                            "data": {
                                "itemType": "journalArticle",
                                "title": "Group 2021",
                                "creators": [],
                                "date": "2021"
                            }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::ItemType,
                    operation: ZoteroSearchConditionOperation::IsNotEmpty,
                    value: None,
                    case_sensitive: None,
                }],
                join_mode: None,
                sort_by: Some(ZoteroAdvancedSortBy::Year),
                sort_direction: Some(ZoteroSortDirection::Asc),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(3),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert_eq!(
            result
                .results
                .items
                .iter()
                .map(|item| item.key.clone())
                .collect::<Vec<_>>(),
            vec![
                "GROUP_2019".to_string(),
                "USER_2020".to_string(),
                "GROUP_2021".to_string()
            ]
        );
        assert_eq!(result.results.total_available, Some(4));
        assert_eq!(result.results.has_more, true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_multi_scope_surfaces_scope_errors_in_warnings() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/groups/456/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Total-Results", "1")
                    .set_body_json(serde_json::json!([
                        {
                            "key": "GROUP_1",
                            "data": {
                                "itemType": "journalArticle",
                                "title": "Group Item",
                                "creators": [],
                                "date": "2021"
                            }
                        }
                    ])),
            )
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::ItemType,
                    operation: ZoteroSearchConditionOperation::IsNotEmpty,
                    value: None,
                    case_sensitive: None,
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_advanced_search should succeed");

        assert_eq!(result.results.items.len(), 1);
        assert_eq!(result.results.items[0].key, "GROUP_1");
        assert_eq!(result.completeness, ZoteroAdvancedCompleteness::Approximate);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("advanced search failed for scope user/123"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_advanced_search_rejects_conflicting_item_type_filters() {
        let server = MockServer::start().await;
        let toolkit = build_test_toolkit(server.uri());

        let error = toolkit
            .zotero_advanced_search(ZoteroAdvancedSearchParams {
                conditions: vec![ZoteroSearchCondition {
                    field: ZoteroSearchConditionField::ItemType,
                    operation: ZoteroSearchConditionOperation::Equals,
                    value: Some("journalArticle".to_string()),
                    case_sensitive: Some(false),
                }],
                join_mode: None,
                sort_by: None,
                sort_direction: None,
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                offset: Some(0),
                limit: Some(10),
                item_type: Some("book".to_string()),
                max_chars_per_item: None,
            })
            .await
            .expect_err("conflicting item_type filters should fail");

        assert!(matches!(error, ResearchError::InvalidInput(_)));
        assert!(error.to_string().contains("conflicts"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multi_scope_search_merges_user_and_group_results() {
        let server = MockServer::start().await;

        // User scope returns one item.
        Mock::given(method("GET"))
            .and(path("/users/42/items"))
            .and(query_param("q", "robots"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "U1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "User Robot Paper",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        // Groups discovery returns group 999.
        Mock::given(method("GET"))
            .and(path("/users/42/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 999, "data": { "name": "Lab" } }
            ])))
            .mount(&server)
            .await;

        // Group scope returns a different item.
        Mock::given(method("GET"))
            .and(path("/groups/999/items"))
            .and(query_param("q", "robots"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "G1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Group Robot Paper",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        // No explicit library_type/library_id -> multi-scope discovery.
        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("42".to_string()),
            zotero_group_id: None,
            zotero_base_url: server.uri(),
            ..ResearchConfig::default()
        });

        let result = toolkit
            .zotero_search(ZoteroSearchParams {
                query: "robots".to_string(),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(20),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("multi-scope search should succeed");

        assert_eq!(result.items.len(), 2);
        let keys: Vec<_> = result.items.iter().map(|i| i.key.as_str()).collect();
        assert!(keys.contains(&"U1"));
        assert!(keys.contains(&"G1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multi_scope_get_item_finds_item_in_group() {
        let server = MockServer::start().await;

        // User scope returns 404.
        Mock::given(method("GET"))
            .and(path("/users/42/items/GRP_ITEM"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        // Groups discovery returns group 999.
        Mock::given(method("GET"))
            .and(path("/users/42/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 999, "data": { "name": "Lab" } }
            ])))
            .mount(&server)
            .await;

        // Group scope has the item.
        Mock::given(method("GET"))
            .and(path("/groups/999/items/GRP_ITEM"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "key": "GRP_ITEM",
                "data": {
                    "itemType": "journalArticle",
                    "title": "Group Only Paper",
                    "creators": [{"firstName": "Bob", "lastName": "Z"}],
                    "tags": []
                }
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("42".to_string()),
            zotero_group_id: None,
            zotero_base_url: server.uri(),
            ..ResearchConfig::default()
        });

        let item = toolkit
            .zotero_get_item(ZoteroItemParams {
                item_key: "GRP_ITEM".to_string(),
                library_type: None,
                library_id: None,
                max_chars_per_item: None,
            })
            .await
            .expect("should find item in group scope");

        assert_eq!(item.key, "GRP_ITEM");
        assert_eq!(item.title, "Group Only Paper");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_scope_queries_only_that_scope() {
        let server = MockServer::start().await;

        // Only mount user scope mock.
        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Explicit User Item",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());

        // Explicit library_type="user" should NOT trigger group discovery.
        let result = toolkit
            .zotero_search(ZoteroSearchParams {
                query: "test".to_string(),
                library_type: Some("user".to_string()),
                library_id: Some("123".to_string()),
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("explicit scope search should succeed");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "ITEM1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn group_discovery_failure_falls_back_to_configured_scopes() {
        let server = MockServer::start().await;

        // User scope search works.
        Mock::given(method("GET"))
            .and(path("/users/42/items"))
            .and(query_param("q", "fallback"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "FB1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Fallback Item",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        // No groups mock -> discovery fails silently.
        // Configured group 777 also has no items mock -> skipped.

        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("42".to_string()),
            zotero_group_id: Some("777".to_string()),
            zotero_base_url: server.uri(),
            ..ResearchConfig::default()
        });

        let result = toolkit
            .zotero_search(ZoteroSearchParams {
                query: "fallback".to_string(),
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(10),
                item_type: None,
                max_chars_per_item: None,
            })
            .await
            .expect("should succeed with fallback to user scope");

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].key, "FB1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multi_scope_collections_merges_across_libraries() {
        let server = MockServer::start().await;

        // User scope has one collection.
        Mock::given(method("GET"))
            .and(path("/users/42/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "key": "COL_U", "data": { "name": "User Collection" } }
            ])))
            .mount(&server)
            .await;

        // Groups discovery.
        Mock::given(method("GET"))
            .and(path("/users/42/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 999, "data": { "name": "Lab" } }
            ])))
            .mount(&server)
            .await;

        // Group scope has another collection.
        Mock::given(method("GET"))
            .and(path("/groups/999/collections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "key": "COL_G", "data": { "name": "Group Collection" } }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("42".to_string()),
            zotero_group_id: None,
            zotero_base_url: server.uri(),
            ..ResearchConfig::default()
        });

        let result = toolkit
            .zotero_get_collections(ZoteroCollectionsParams {
                library_type: None,
                library_id: None,
                offset: Some(0),
                limit: Some(20),
            })
            .await
            .expect("multi-scope collections should succeed");

        assert_eq!(result.collections.len(), 2);
        let names: Vec<_> = result.collections.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"User Collection"));
        assert!(names.contains(&"Group Collection"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_prefers_ar5iv_when_available() {
        let item = sample_item_with_source(Some("https://arxiv.org/abs/2401.12345v2"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/fallback.pdf"),
            None,
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            true,
            Vec::new(),
            |_, _| async { true },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::Ar5ivHtml,
                preferred_url: Some("https://ar5iv.labs.arxiv.org/html/2401.12345v2".to_string()),
                fallback_urls: vec![
                    "https://arxiv.org/pdf/2401.12345v2.pdf".to_string(),
                    "https://example.com/fallback.pdf".to_string()
                ],
                local_path: None,
                trace: vec![
                    "detected arXiv id from item URL: 2401.12345v2".to_string(),
                    "ar5iv probe succeeded for 2401.12345v2".to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_falls_back_to_arxiv_pdf_when_ar5iv_unavailable() {
        let item = sample_item_with_source(Some("https://arxiv.org/abs/2401.12345v2"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/fallback.pdf"),
            None,
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            true,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::ArxivPdf,
                preferred_url: Some("https://arxiv.org/pdf/2401.12345v2.pdf".to_string()),
                fallback_urls: vec!["https://example.com/fallback.pdf".to_string()],
                local_path: None,
                trace: vec![
                    "detected arXiv id from item URL: 2401.12345v2".to_string(),
                    "ar5iv probe unavailable; using arXiv PDF".to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_extracts_arxiv_id_from_extra_when_url_missing() {
        let item = sample_item_with_source(None, Some("Notes\narXiv:2401.54321v3"));
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/fallback.pdf"),
            None,
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            true,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::ArxivPdf,
                preferred_url: Some("https://arxiv.org/pdf/2401.54321v3.pdf".to_string()),
                fallback_urls: vec!["https://example.com/fallback.pdf".to_string()],
                local_path: None,
                trace: vec![
                    "detected arXiv id from item extra: 2401.54321v3".to_string(),
                    "ar5iv probe unavailable; using arXiv PDF".to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_supports_old_style_arxiv_ids() {
        let item = sample_item_with_source(Some("https://arxiv.org/abs/hep-ph/0001234v1"), None);

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &Vec::new(),
            None,
            true,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::ArxivPdf,
                preferred_url: Some("https://arxiv.org/pdf/hep-ph/0001234v1.pdf".to_string()),
                fallback_urls: Vec::new(),
                local_path: None,
                trace: vec![
                    "detected arXiv id from item URL: hep-ph/0001234v1".to_string(),
                    "ar5iv probe unavailable; using arXiv PDF".to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_tries_unversioned_ar5iv_candidate_after_versioned_miss() {
        let item = sample_item_with_source(Some("https://arxiv.org/abs/2401.12345v2"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/fallback.pdf"),
            None,
        )];
        let observed = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            true,
            Vec::new(),
            {
                let observed = observed.clone();
                move |probe_id, _| {
                    let observed = observed.clone();
                    async move {
                        observed.lock().await.push(probe_id.clone());
                        probe_id == "2401.12345"
                    }
                }
            },
        )
        .await;

        assert_eq!(
            *observed.lock().await,
            vec!["2401.12345v2".to_string(), "2401.12345".to_string()]
        );
        assert_eq!(
            resolution.preferred_url,
            Some("https://ar5iv.labs.arxiv.org/html/2401.12345".to_string())
        );
        assert_eq!(resolution.source_kind, DocumentSourceKind::Ar5ivHtml);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_prefers_attachment_pdf_urls_for_non_arxiv_items() {
        let item = sample_item_with_source(Some("https://example.com/paper"), None);
        let attachments = vec![
            sample_attachment(
                "ATT1",
                Some("application/pdf"),
                Some("https://example.com/primary.pdf"),
                None,
            ),
            sample_attachment(
                "ATT2",
                Some("application/pdf"),
                Some("https://example.com/secondary.pdf"),
                None,
            ),
        ];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            false,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::AttachmentPdfUrl,
                preferred_url: Some("https://example.com/primary.pdf".to_string()),
                fallback_urls: vec!["https://example.com/secondary.pdf".to_string()],
                local_path: None,
                trace: vec!["using attachment PDF URL".to_string()],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_handles_missing_item_metadata() {
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/primary.pdf"),
            None,
        )];

        let resolution = super::resolve_document_sources_with_probe(
            None,
            &attachments,
            None,
            false,
            vec!["item metadata lookup failed: missing".to_string()],
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::AttachmentPdfUrl,
                preferred_url: Some("https://example.com/primary.pdf".to_string()),
                fallback_urls: Vec::new(),
                local_path: None,
                trace: vec![
                    "item metadata lookup failed: missing".to_string(),
                    "using attachment PDF URL".to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_uses_indexed_fulltext_trace_when_no_sources_and_no_content() {
        let item = sample_item_with_source(Some("https://example.com/paper"), None);

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &Vec::new(),
            None,
            false,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::IndexedFulltext,
                preferred_url: None,
                fallback_urls: Vec::new(),
                local_path: None,
                trace: vec![
                    "local path fallback unavailable: ZOTERO_STORAGE_DIR is not set".to_string(),
                    "no canonical document source resolved and indexed fulltext is empty"
                        .to_string()
                ],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_skips_malformed_arxiv_urls() {
        let item = sample_item_with_source(Some("https://arxiv.org/foo/2401.12345"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            Some("https://example.com/primary.pdf"),
            None,
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            None,
            true,
            Vec::new(),
            |_, _| async { true },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::AttachmentPdfUrl,
                preferred_url: Some("https://example.com/primary.pdf".to_string()),
                fallback_urls: Vec::new(),
                local_path: None,
                trace: vec!["using attachment PDF URL".to_string()],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn normalize_arxiv_id_strips_prefix_case_insensitively() {
        assert_eq!(
            super::normalize_arxiv_id("ArXiV:2401.12345v1"),
            Some("2401.12345v1".to_string())
        );
        assert_eq!(
            super::normalize_arxiv_id("arxiv:2401.12345v2"),
            Some("2401.12345v2".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ar5iv_probe_candidates_ignores_empty_version_suffix() {
        assert_eq!(
            super::ar5iv_probe_candidates("2401.12345v"),
            vec!["2401.12345v".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_uses_local_storage_path_when_no_pdf_url_exists() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let storage_root = temp_dir.path().to_string_lossy().to_string();
        let item = sample_item_with_source(Some("https://example.com/paper"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            None,
            Some("storage:paper.pdf"),
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            Some(storage_root.as_str()),
            false,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        let expected_path = temp_dir
            .path()
            .join("ATT1")
            .join("paper.pdf")
            .to_string_lossy()
            .to_string();
        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::LocalPdfPath,
                preferred_url: None,
                fallback_urls: Vec::new(),
                local_path: Some(expected_path),
                trace: vec!["resolved local PDF path from attachment ATT1".to_string()],
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_document_sources_rejects_unsafe_local_paths() {
        let temp_dir = tempfile::tempdir().expect("temp directory");
        let storage_root = temp_dir.path().to_string_lossy().to_string();
        let item = sample_item_with_source(Some("https://example.com/paper"), None);
        let attachments = vec![sample_attachment(
            "ATT1",
            Some("application/pdf"),
            None,
            Some("storage:../../escape.pdf"),
        )];

        let resolution = super::resolve_document_sources_with_probe(
            Some(&item),
            &attachments,
            Some(storage_root.as_str()),
            true,
            Vec::new(),
            |_, _| async { false },
        )
        .await;

        assert_eq!(
            resolution,
            DocumentResolution {
                source_kind: DocumentSourceKind::IndexedFulltext,
                preferred_url: None,
                fallback_urls: Vec::new(),
                local_path: None,
                trace: vec![
                    "ignored attachment path for ATT1 due to unsafe relative path".to_string(),
                    "no canonical document source resolved; using indexed fulltext fallback"
                        .to_string()
                ],
            }
        );
    }

    fn sample_item_with_source(url: Option<&str>, extra: Option<&str>) -> ZoteroItemDetail {
        ZoteroItemDetail {
            key: "ITEM1".to_string(),
            title: "Sample Item".to_string(),
            authors: vec!["Alice Example".to_string()],
            abstract_text: None,
            date: Some("2024".to_string()),
            doi: None,
            url: url.map(ToString::to_string),
            publication: None,
            item_type: "journalArticle".to_string(),
            tags: Vec::new(),
            extra: extra.map(ToString::to_string),
            source_meta: None,
        }
    }

    fn sample_attachment(
        key: &str,
        content_type: Option<&str>,
        url: Option<&str>,
        path: Option<&str>,
    ) -> ZoteroAttachment {
        ZoteroAttachment {
            key: key.to_string(),
            title: Some("attachment".to_string()),
            filename: Some("paper.pdf".to_string()),
            content_type: content_type.map(ToString::to_string),
            link_mode: Some("imported_file".to_string()),
            url: url.map(ToString::to_string),
            path: path.map(ToString::to_string),
            parent_item: Some("ITEM1".to_string()),
            source_meta: None,
        }
    }

    fn build_test_toolkit(zotero_base_url: String) -> ResearchToolkit {
        build_test_toolkit_with_config(ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("123".to_string()),
            zotero_group_id: Some("456".to_string()),
            zotero_base_url,
            ..ResearchConfig::default()
        })
    }
}
