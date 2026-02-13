use serde::Serialize;

use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedScope {
    pub(super) library_type: String,
    pub(super) library_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) enum ResolvedScopes {
    Single(ZoteroLibraryScope),
    All(Vec<ZoteroLibraryScope>),
}

#[derive(Debug, Clone, Serialize)]
pub(super) enum NormalizedResolvedScopes {
    Single(NormalizedScope),
    All(Vec<NormalizedScope>),
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedSearchParams {
    pub(super) query: String,
    pub(super) scopes: NormalizedResolvedScopes,
    pub(super) offset: u32,
    pub(super) limit: u32,
    pub(super) item_type: Option<String>,
    pub(super) max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedTagsParams {
    pub(super) scope: NormalizedScope,
    pub(super) offset: u32,
    pub(super) limit: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedRecentParams {
    pub(super) scope: NormalizedScope,
    pub(super) offset: u32,
    pub(super) limit: u32,
    pub(super) item_type: Option<String>,
    pub(super) sort_by: ZoteroRecentSortBy,
    pub(super) max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedItemParams {
    pub(super) item_key: String,
    pub(super) scopes: NormalizedResolvedScopes,
    pub(super) max_chars_per_item: Option<u32>,
    pub(super) include_attachments: bool,
    pub(super) include_fulltext_resolution: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedCitationParams {
    pub(super) item_key: String,
    pub(super) scopes: NormalizedResolvedScopes,
    pub(super) format: ZoteroCitationFormat,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedAnnotationsParams {
    pub(super) item_key: Option<String>,
    pub(super) scope: NormalizedScope,
    pub(super) offset: u32,
    pub(super) limit: u32,
    pub(super) include_parent_context: bool,
    pub(super) max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedTagSearchParams {
    pub(super) tags: Vec<String>,
    pub(super) scope: NormalizedScope,
    pub(super) offset: u32,
    pub(super) limit: u32,
    pub(super) item_type: Option<String>,
    pub(super) max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedCollectionsParams {
    pub(super) scopes: NormalizedResolvedScopes,
    pub(super) offset: u32,
    pub(super) limit: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedListGroupsParams {
    pub(super) user_id: String,
    pub(super) offset: u32,
    pub(super) limit: u32,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedCollectionItemsParams {
    pub(super) collection_key: String,
    pub(super) scopes: NormalizedResolvedScopes,
    pub(super) offset: u32,
    pub(super) limit: u32,
    pub(super) item_type: Option<String>,
    pub(super) max_chars_per_item: Option<u32>,
}
pub(super) fn normalize_tags_params(
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

pub(super) fn normalize_recent_params(
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

pub(super) async fn normalize_search_params(
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

pub(super) async fn normalize_item_params(
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

pub(super) async fn normalize_citation_params(
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

pub(super) fn normalize_annotations_params(
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

pub(super) fn normalize_tag_search_params(
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

pub(super) async fn normalize_collections_params(
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

pub(super) fn normalize_list_groups_params(
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

pub(super) async fn normalize_collection_items_params(
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

pub(super) fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(super) fn zotero_config(toolkit: &ResearchToolkit) -> ZoteroConfig<'_> {
    ZoteroConfig {
        base_url: &toolkit.config().zotero_base_url,
        api_key: toolkit.config().zotero_api_key.as_deref(),
    }
}

pub(super) fn uses_local_zotero_api(toolkit: &ResearchToolkit) -> bool {
    toolkit.config().uses_local_zotero_api()
}

pub(super) fn resolve_scope(
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

pub(super) fn to_normalized_scope(scope: &ZoteroLibraryScope) -> NormalizedScope {
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

pub(super) fn to_scope(scope: &NormalizedScope) -> ZoteroLibraryScope {
    if scope.library_type == "group" {
        return ZoteroLibraryScope::Group(scope.library_id.clone());
    }

    ZoteroLibraryScope::User(scope.library_id.clone())
}

pub(super) fn to_normalized_resolved_scopes(resolved: &ResolvedScopes) -> NormalizedResolvedScopes {
    match resolved {
        ResolvedScopes::Single(scope) => {
            NormalizedResolvedScopes::Single(to_normalized_scope(scope))
        }
        ResolvedScopes::All(scopes) => {
            NormalizedResolvedScopes::All(scopes.iter().map(to_normalized_scope).collect())
        }
    }
}

pub(super) fn resolved_scopes_to_vec(scopes: &NormalizedResolvedScopes) -> Vec<ZoteroLibraryScope> {
    match scopes {
        NormalizedResolvedScopes::Single(scope) => vec![to_scope(scope)],
        NormalizedResolvedScopes::All(scopes) => scopes.iter().map(to_scope).collect(),
    }
}

pub(super) async fn resolve_scopes(
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

pub(super) async fn discover_all_scopes(
    toolkit: &ResearchToolkit,
) -> Result<Vec<ZoteroLibraryScope>> {
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
pub(super) async fn search_items_across_scopes(
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
pub(super) async fn collection_items_across_scopes(
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
pub(super) async fn merge_collections_across_scopes(
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

pub(super) fn recent_sort_field(sort_by: &ZoteroRecentSortBy) -> &'static str {
    match sort_by {
        ZoteroRecentSortBy::DateAdded => "dateAdded",
        ZoteroRecentSortBy::DateModified => "dateModified",
    }
}

pub(super) async fn fetch_item_across_scopes(
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
