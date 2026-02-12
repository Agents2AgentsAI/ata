use std::collections::HashMap;
use std::collections::HashSet;

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
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::text_utils::truncate_chars;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::ZoteroAdvancedSearchParams;
use crate::types::ZoteroAdvancedSearchResult;
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
use crate::types::ZoteroSearchNotesParams;
use crate::types::ZoteroSearchNotesResult;
use crate::types::ZoteroSearchParams;
use crate::types::ZoteroSearchResult;
use crate::types::ZoteroTagSearchParams;

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
const DEFAULT_COLLECTIONS_LIMIT: u32 = 100;
const DEFAULT_GROUPS_LIMIT: u32 = 100;
const DEFAULT_CHILDREN_LIMIT: u32 = 50;
const DEFAULT_ANNOTATIONS_LIMIT: u32 = 50;
const DEFAULT_FULLTEXT_MAX_CHARS: u32 = 10_000;
const ZOTERO_MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_LOCAL_USER_LIBRARY_ID: &str = "0";
const DEFAULT_ANNOTATION_PARENT_FETCH_CONCURRENCY: usize = 6;

#[derive(Debug, Clone, Serialize)]
struct NormalizedScope {
    library_type: String,
    library_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedSearchParams {
    query: String,
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedItemParams {
    item_key: String,
    scope: NormalizedScope,
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
    scope: NormalizedScope,
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
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
}

pub(crate) async fn zotero_search(
    toolkit: &ResearchToolkit,
    params: ZoteroSearchParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_search_params(toolkit, params, "zotero_search")?;
    let key = CacheKey {
        tool_name: "zotero_search",
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
                        query: Some(&normalized.query),
                        tag: None,
                        offset: normalized.offset,
                        limit: normalized.limit,
                        item_type: normalized.item_type.as_deref(),
                        sort: None,
                        direction: None,
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

pub(crate) async fn zotero_advanced_search(
    toolkit: &ResearchToolkit,
    params: ZoteroAdvancedSearchParams,
) -> Result<ZoteroAdvancedSearchResult> {
    advanced_search::zotero_advanced_search(toolkit, params).await
}

pub(crate) async fn zotero_get_item(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroItemDetail> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_item")?;
    let key = CacheKey {
        tool_name: "zotero_get_item",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut item =
        get_or_fetch_typed(
            toolkit,
            key,
            toolkit.config().cache_ttls.zotero_items,
            || {
                let scope = to_scope(&normalized.scope);
                async move {
                    zotero::get_item(toolkit.http(), config, &scope, &normalized.item_key).await
                }
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
    let normalized = normalize_item_params(toolkit, params, "zotero_get_fulltext")?;
    let key = CacheKey {
        tool_name: "zotero_get_fulltext",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut fulltext = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_fulltext(toolkit.http(), config, &scope, &normalized.item_key).await
            }
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
    let normalized = normalize_item_params(toolkit, params, "zotero_get_notes")?;
    let key = CacheKey {
        tool_name: "zotero_get_notes",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut notes = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_notes(
                    toolkit.http(),
                    config,
                    &scope,
                    &ZoteroChildrenRequest {
                        item_key: &normalized.item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    },
                )
                .await
            }
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
                if let Some(item_key) = normalized.item_key.as_deref() {
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
                    return Ok(ZoteroAnnotationsResult {
                        item_key: Some(response.item_key),
                        annotations: response
                            .annotations
                            .into_iter()
                            .map(map_zotero_annotation)
                            .collect(),
                        total_available: response.total_available,
                        has_more: response.has_more,
                    });
                }

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

                Ok(ZoteroAnnotationsResult {
                    item_key: None,
                    annotations: response
                        .annotations
                        .into_iter()
                        .map(map_zotero_annotation)
                        .collect(),
                    total_available: response.total_available,
                    has_more: response.has_more,
                })
            }
        },
    )
    .await?;

    if normalized.include_parent_context {
        let parent_item_keys = annotations
            .annotations
            .iter()
            .filter_map(|annotation| annotation.parent_item.clone())
            .collect::<HashSet<_>>();

        if !parent_item_keys.is_empty() {
            let scope = to_scope(&normalized.scope);
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
                            Ok(parent_item) => Some((parent_item_key, parent_item.title)),
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

    if let Some(max_chars) = normalized.max_chars_per_item {
        apply_annotations_budget(&mut annotations.annotations, max_chars as usize);
    }

    Ok(annotations)
}

pub(crate) async fn zotero_get_attachments(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroAttachmentsResult> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_attachments")?;
    let key = CacheKey {
        tool_name: "zotero_get_attachments",
        params_hash: hash_cache_payload(&normalized)?,
    };
    let config = zotero_config(toolkit);

    let mut attachments = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_attachments(
                    toolkit.http(),
                    config,
                    &scope,
                    &ZoteroChildrenRequest {
                        item_key: &normalized.item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    },
                )
                .await
            }
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
    let normalized = normalize_collections_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_get_collections",
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
                zotero::get_collections(
                    toolkit.http(),
                    config,
                    &scope,
                    ZoteroCollectionsRequest {
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
    let normalized = normalize_collection_items_params(toolkit, params)?;
    let key = CacheKey {
        tool_name: "zotero_get_collection_items",
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
                zotero::get_collection_items(
                    toolkit.http(),
                    config,
                    &scope,
                    &ZoteroCollectionItemsRequest {
                        collection_key: &normalized.collection_key,
                        offset: normalized.offset,
                        limit: normalized.limit,
                        item_type: normalized.item_type.as_deref(),
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

fn normalize_search_params(
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

    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        tool_name,
    )?;

    Ok(NormalizedSearchParams {
        query,
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).clamp(1, 100),
        item_type: normalize_optional_string(params.item_type),
        max_chars_per_item: params.max_chars_per_item,
    })
}

fn normalize_item_params(
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

    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        tool_name,
    )?;

    Ok(NormalizedItemParams {
        item_key,
        scope: to_normalized_scope(&scope),
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

fn normalize_collections_params(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionsParams,
) -> Result<NormalizedCollectionsParams> {
    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_collections",
    )?;

    Ok(NormalizedCollectionsParams {
        scope: to_normalized_scope(&scope),
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

fn normalize_collection_items_params(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionItemsParams,
) -> Result<NormalizedCollectionItemsParams> {
    let collection_key = params.collection_key.trim().to_string();
    if collection_key.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_get_collection_items collection_key must not be empty".to_string(),
        ));
    }

    let scope = resolve_scope(
        toolkit,
        params.library_type.as_deref(),
        params.library_id.as_deref(),
        "zotero_get_collection_items",
    )?;

    Ok(NormalizedCollectionItemsParams {
        collection_key,
        scope: to_normalized_scope(&scope),
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

fn map_zotero_annotation(annotation: zotero::ZoteroAnnotation) -> ZoteroAnnotation {
    let annotation_type = annotation
        .annotation_type
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| matches!(value.as_str(), "highlight" | "note" | "image"))
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
    use wiremock::matchers::query_param_is_missing;

    use crate::ResearchToolkit;
    use crate::config::ResearchConfig;
    use crate::error::ResearchError;
    use crate::tools::test_helpers::build_test_toolkit_with_config;
    use crate::types::ZoteroAdvancedCandidateStrategy;
    use crate::types::ZoteroAdvancedCompleteness;
    use crate::types::ZoteroAdvancedSearchParams;
    use crate::types::ZoteroAnnotation;
    use crate::types::ZoteroAnnotationsParams;
    use crate::types::ZoteroCollectionItemsParams;
    use crate::types::ZoteroCollectionsParams;
    use crate::types::ZoteroGrepCandidateStrategy;
    use crate::types::ZoteroGrepField;
    use crate::types::ZoteroGrepMatchMode;
    use crate::types::ZoteroGrepParams;
    use crate::types::ZoteroItemParams;
    use crate::types::ZoteroListGroupsParams;
    use crate::types::ZoteroSearchCondition;
    use crate::types::ZoteroSearchConditionField;
    use crate::types::ZoteroSearchConditionOperation;
    use crate::types::ZoteroSearchNotesParams;
    use crate::types::ZoteroSearchParams;
    use crate::types::ZoteroTagSearchParams;

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
                annotation_type: "unknown".to_string(),
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
    async fn zotero_get_annotations_can_enrich_parent_item_title() {
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
    async fn zotero_grep_text_query_filtered_matches_title_literal() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "vision"))
            .and(query_param("sort", "relevance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Diffusion Models in Vision",
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
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: Some("vision".to_string()),
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(10),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::QueryFiltered
        );
        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.truncated, false);
        assert_eq!(result.matches[0].item_key, "ITEM1");
        assert_eq!(result.matches[0].field, "title".to_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_recent_strategy_reports_truncation() {
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
                        "title": "Item Alpha",
                        "creators": []
                    }
                },
                {
                    "key": "ITEM2",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Item Beta",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "item".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: None,
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(10),
                limit_matches: Some(1),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::RecentModified
        );
        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.truncated, true);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("candidate_strategy=recent_modified"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_parent_scoped_strips_html_in_note_and_annotation() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("start", "0"))
            .and(query_param("limit", "20"))
            .and(query_param_is_missing("itemType"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "NOTE1",
                    "data": {
                        "itemType": "note",
                        "title": "Note Item",
                        "parentItem": "ITEM1"
                    }
                },
                {
                    "key": "ANNO1",
                    "data": {
                        "itemType": "annotation",
                        "title": "Annotation Item",
                        "parentItem": "ITEM1"
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
                        "note": "<p>Key <b>evidence</b> in note</p>",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/children"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO1",
                    "data": {
                        "itemType": "annotation",
                        "annotationText": "<span>Supporting evidence</span>",
                        "annotationComment": "<p>not relevant</p>",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "evidence".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: Some("ITEM1".to_string()),
                query_hint: None,
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Note, ZoteroGrepField::Annotation]),
                limit_items: Some(20),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(
            result.candidate_strategy,
            ZoteroGrepCandidateStrategy::ParentScoped
        );
        assert_eq!(result.scanned_items, 2);
        assert_eq!(result.returned_matches, 2);
        assert!(
            result
                .matches
                .iter()
                .all(|matched| matched.parent_item_key.as_deref() == Some("ITEM1"))
        );
        let mut fields = result
            .matches
            .iter()
            .map(|matched| matched.field.clone())
            .collect::<Vec<_>>();
        fields.sort();
        assert_eq!(
            fields,
            vec!["annotation_text".to_string(), "note".to_string()]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_rejects_invalid_regex() {
        let toolkit = build_test_toolkit("http://localhost".to_string());
        let err = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "[".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Regex),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: Some("x".to_string()),
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(5),
                limit_matches: Some(5),
                max_matches_per_item: Some(5),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect_err("invalid regex should fail");

        assert!(matches!(err, ResearchError::InvalidInput(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_empty_results_include_hints() {
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
                        "title": "Completely different title",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "one two three four".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: None,
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(10),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(result.returned_matches, 0);
        assert!(result.matches.is_empty());
        assert!(
            result
                .hints
                .iter()
                .any(|hint| hint.contains("No explicit library scope"))
        );
        assert!(
            result
                .hints
                .iter()
                .any(|hint| hint.contains("Try fewer or broader search terms"))
        );
        assert!(
            result
                .hints
                .iter()
                .any(|hint| hint.contains("Try broadening the fields list"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_annotation_fetch_limit_independent_from_limit_items() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "topic"))
            .and(query_param("sort", "relevance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Topic Paper",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO1",
                    "data": {
                        "itemType": "annotation",
                        "annotationText": "first evidence",
                        "parentItem": "ITEM1"
                    }
                },
                {
                    "key": "ANNO2",
                    "data": {
                        "itemType": "annotation",
                        "annotationText": "second evidence",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "evidence".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: Some("topic".to_string()),
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Annotation]),
                limit_items: Some(1),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert_eq!(result.scanned_items, 1);
        assert_eq!(result.returned_matches, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_warns_when_max_chars_per_item_truncates_segments() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "topic"))
            .and(query_param("sort", "relevance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Topic Paper",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "top".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: Some("topic".to_string()),
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Title]),
                limit_items: Some(10),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: Some(3),
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("max_chars_per_item=3 truncates text segments"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_grep_text_warns_on_default_fulltext_cap() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "topic"))
            .and(query_param("sort", "relevance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Topic Paper",
                        "creators": []
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items/ITEM1/fulltext"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "evidence appears in fulltext"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_grep_text(ZoteroGrepParams {
                pattern: "evidence".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                query_hint: Some("topic".to_string()),
                item_type: None,
                fields: Some(vec![ZoteroGrepField::Fulltext]),
                limit_items: Some(10),
                limit_matches: Some(10),
                max_matches_per_item: Some(10),
                context_chars: Some(30),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_grep_text should succeed");

        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("default 10000 character cap"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zotero_search_notes_reuses_note_and_annotation_matching() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("q", "evidence"))
            .and(query_param("sort", "relevance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ITEM1",
                    "data": {
                        "itemType": "journalArticle",
                        "title": "Paper One",
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
                        "note": "<p>Primary <b>evidence</b> excerpt</p>",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/users/123/items"))
            .and(query_param("itemType", "annotation"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "key": "ANNO1",
                    "data": {
                        "itemType": "annotation",
                        "annotationComment": "<p>secondary evidence note</p>",
                        "parentItem": "ITEM1"
                    }
                }
            ])))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri());
        let result = toolkit
            .zotero_search_notes(ZoteroSearchNotesParams {
                query: "evidence".to_string(),
                match_mode: Some(ZoteroGrepMatchMode::Literal),
                case_sensitive: Some(false),
                library_type: None,
                library_id: None,
                parent_item_key: None,
                include_annotations: Some(true),
                limit: Some(10),
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_search_notes should succeed");

        assert_eq!(result.query, "evidence");
        assert_eq!(result.notes.len(), 2);
        assert_eq!(result.has_more, false);
        let mut fields = result
            .notes
            .iter()
            .map(|note| note.field.clone())
            .collect::<Vec<_>>();
        fields.sort();
        assert_eq!(
            fields,
            vec!["annotation_comment".to_string(), "note".to_string()]
        );
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
