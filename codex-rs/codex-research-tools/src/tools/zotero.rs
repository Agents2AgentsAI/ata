use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroChildrenRequest;
use crate::clients::zotero::ZoteroCollectionItemsRequest;
use crate::clients::zotero::ZoteroCollectionsRequest;
use crate::clients::zotero::ZoteroConfig;
use crate::clients::zotero::ZoteroLibraryScope;
use crate::clients::zotero::ZoteroSearchRequest;
use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroAttachmentsResult;
use crate::types::ZoteroCollectionItemsParams;
use crate::types::ZoteroCollectionsParams;
use crate::types::ZoteroCollectionsResult;
use crate::types::ZoteroFullTextResult;
use crate::types::ZoteroItem;
use crate::types::ZoteroItemDetail;
use crate::types::ZoteroItemParams;
use crate::types::ZoteroNote;
use crate::types::ZoteroNotesResult;
use crate::types::ZoteroSearchParams;
use crate::types::ZoteroSearchResult;
use crate::types::ZoteroTagSearchParams;

const DEFAULT_SEARCH_LIMIT: u32 = 25;
const DEFAULT_COLLECTIONS_LIMIT: u32 = 100;
const DEFAULT_CHILDREN_LIMIT: u32 = 50;
const DEFAULT_FULLTEXT_MAX_CHARS: u32 = 10_000;

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
    let api_key = zotero_api_key(toolkit, "zotero_search")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_search",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::search_items(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
                    &scope,
                    &ZoteroSearchRequest {
                        query: Some(&normalized.query),
                        tag: None,
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

pub(crate) async fn zotero_get_item(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroItemDetail> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_item")?;
    let api_key = zotero_api_key(toolkit, "zotero_get_item")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_item",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut item = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_item(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
                    &scope,
                    &normalized.item_key,
                )
                .await
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
    let api_key = zotero_api_key(toolkit, "zotero_get_fulltext")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_fulltext",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut fulltext = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_fulltext(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
                    &scope,
                    &normalized.item_key,
                )
                .await
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
    let api_key = zotero_api_key(toolkit, "zotero_get_notes")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_notes",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut notes = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_notes(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
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

pub(crate) async fn zotero_get_attachments(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroAttachmentsResult> {
    let normalized = normalize_item_params(toolkit, params, "zotero_get_attachments")?;
    let api_key = zotero_api_key(toolkit, "zotero_get_attachments")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_attachments",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut attachments = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_attachments(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
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
    let api_key = zotero_api_key(toolkit, "zotero_search_by_tag")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_search_by_tag",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                let mut by_key: HashMap<String, (ZoteroItem, HashSet<String>)> = HashMap::new();

                // Fetch each tag independently and keep only items that appear for all tags.
                for tag in &normalized.tags {
                    let page = zotero::search_items(
                        toolkit.http(),
                        ZoteroConfig {
                            base_url: &toolkit.config().zotero_base_url,
                            api_key: &api_key,
                        },
                        &scope,
                        &ZoteroSearchRequest {
                            query: None,
                            tag: Some(tag),
                            offset: 0,
                            limit: normalized.limit.clamp(1, 100),
                            item_type: normalized.item_type.as_deref(),
                        },
                    )
                    .await?;

                    let tag_key = tag.to_ascii_lowercase();
                    for item in page.items {
                        let entry = by_key
                            .entry(item.key.clone())
                            .or_insert_with(|| (item, HashSet::new()));
                        entry.1.insert(tag_key.clone());
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
    )
    .await?;

    apply_items_budget(&mut result.items, normalized.max_chars_per_item);
    Ok(result)
}

pub(crate) async fn zotero_get_collections(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionsParams,
) -> Result<ZoteroCollectionsResult> {
    let normalized = normalize_collections_params(toolkit, params)?;
    let api_key = zotero_api_key(toolkit, "zotero_get_collections")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_collections",
        params_hash: hash_cache_payload(&normalized)?,
    };

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_collections(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
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

pub(crate) async fn zotero_get_collection_items(
    toolkit: &ResearchToolkit,
    params: ZoteroCollectionItemsParams,
) -> Result<ZoteroSearchResult> {
    let normalized = normalize_collection_items_params(toolkit, params)?;
    let api_key = zotero_api_key(toolkit, "zotero_get_collection_items")?.to_string();
    let key = CacheKey {
        tool_name: "zotero_get_collection_items",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let mut result = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.zotero_items,
        || {
            let scope = to_scope(&normalized.scope);
            async move {
                zotero::get_collection_items(
                    toolkit.http(),
                    ZoteroConfig {
                        base_url: &toolkit.config().zotero_base_url,
                        api_key: &api_key,
                    },
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

fn normalize_tag_search_params(
    toolkit: &ResearchToolkit,
    params: ZoteroTagSearchParams,
) -> Result<NormalizedTagSearchParams> {
    let mut tags = params
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
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

            Err(ResearchError::NotConfigured {
                tool: tool_name,
                reason: "no Zotero library configured (set `zotero_user_id` or `zotero_group_id`)"
                    .to_string(),
            })
        }
    }
}

fn zotero_api_key<'a>(toolkit: &'a ResearchToolkit, tool_name: &'static str) -> Result<&'a str> {
    toolkit
        .config()
        .zotero_api_key
        .as_deref()
        .ok_or_else(|| ResearchError::NotConfigured {
            tool: tool_name,
            reason: "missing Zotero API key (set ZOTERO_API_KEY)".to_string(),
        })
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

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

async fn get_or_fetch_typed<T, F, Fut>(
    toolkit: &ResearchToolkit,
    key: CacheKey,
    ttl: std::time::Duration,
    fetch: F,
) -> Result<T>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let value = toolkit
        .cache()
        .get_or_fetch(key, ttl, || async {
            let output = fetch().await?;
            serde_json::to_value(output).map_err(|err| {
                ResearchError::Internal(format!("failed to serialize cached value: {err}"))
            })
        })
        .await?;

    serde_json::from_value(value).map_err(|err| {
        ResearchError::Internal(format!("failed to deserialize cached value: {err}"))
    })
}

fn hash_cache_payload<T: Serialize>(payload: &T) -> Result<u64> {
    let serialized = serde_json::to_string(payload)
        .map_err(|err| ResearchError::Internal(format!("failed to serialize cache key: {err}")))?;
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::query_param;

    use crate::ResearchToolkit;
    use crate::config::RateLimitOverrides;
    use crate::config::ResearchConfig;
    use crate::rate_limiter::ApiRateLimit;
    use crate::types::ZoteroCollectionItemsParams;
    use crate::types::ZoteroCollectionsParams;
    use crate::types::ZoteroItemParams;
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
                fields: None,
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
        assert_eq!(fulltext.content, "abcde...");

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
        assert_eq!(notes.notes[0].title, Some("note...".to_string()));

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
        assert_eq!(
            attachments.attachments[0].title,
            Some("paper...".to_string())
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
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect("zotero_get_collection_items should succeed");

        assert_eq!(items.items.len(), 1);
        assert_eq!(items.items[0].key, "ITEM1");
    }

    fn build_test_toolkit(zotero_base_url: String) -> ResearchToolkit {
        let config = ResearchConfig {
            zotero_api_key: Some("test-key".to_string()),
            zotero_user_id: Some("123".to_string()),
            zotero_group_id: Some("456".to_string()),
            zotero_base_url,
            rate_limit_overrides: RateLimitOverrides {
                semantic_scholar: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
                arxiv: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
                openalex: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
                papers_with_code: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
                zotero: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
                github: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
            },
            ..ResearchConfig::default()
        };

        let http_client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("test http client should build");

        ResearchToolkit::new(http_client, config)
    }
}
