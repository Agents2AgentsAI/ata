use super::*;

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
