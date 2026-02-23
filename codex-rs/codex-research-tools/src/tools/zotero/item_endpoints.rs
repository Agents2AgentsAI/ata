use super::*;

pub(crate) async fn zotero_get_item(
    toolkit: &ResearchToolkit,
    params: ZoteroItemParams,
) -> Result<ZoteroItemDetail> {
    toolkit.ensure_zotero_running().await?;
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
    toolkit.ensure_zotero_running().await?;
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
    toolkit.ensure_zotero_running().await?;
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
                let mut resolution_trace = Vec::new();
                let mut prefetched_attachments = None;
                let mut result =
                    match zotero::get_fulltext(toolkit.http(), config, scope, &normalized.item_key)
                        .await
                    {
                        Ok(result) => result,
                        Err(err) => {
                            if let ResearchError::Upstream { status, .. } = &err
                                && *status == reqwest::StatusCode::NOT_FOUND
                            {
                                let children_request = ZoteroChildrenRequest {
                                    item_key: &normalized.item_key,
                                    offset: 0,
                                    limit: DEFAULT_CHILDREN_LIMIT,
                                };
                                let attachments = match zotero::get_attachments(
                                    toolkit.http(),
                                    config,
                                    scope,
                                    &children_request,
                                )
                                .await
                                {
                                    Ok(result) => result.attachments,
                                    Err(attachment_lookup_err) => {
                                        last_err = Some(attachment_lookup_err);
                                        continue;
                                    }
                                };
                                let mut fallback_result = None;
                                let mut fallback_err = None;
                                for attachment in &attachments {
                                    match zotero::get_fulltext(
                                        toolkit.http(),
                                        config,
                                        scope,
                                        &attachment.key,
                                    )
                                    .await
                                    {
                                        Ok(result) => {
                                            resolution_trace.push(format!(
                                            "indexed fulltext missing for {}; using attachment {}",
                                            normalized.item_key, attachment.key
                                        ));
                                            fallback_result = Some(result);
                                            break;
                                        }
                                        Err(attachment_err) => fallback_err = Some(attachment_err),
                                    }
                                }
                                if let Some(result) = fallback_result {
                                    prefetched_attachments = Some(attachments);
                                    result
                                } else {
                                    last_err = fallback_err.or(Some(err));
                                    continue;
                                }
                            } else {
                                last_err = Some(err);
                                continue;
                            }
                        }
                    };

                let item =
                    match zotero::get_item(toolkit.http(), config, scope, &normalized.item_key)
                        .await
                    {
                        Ok(item) => Some(item),
                        Err(err) => {
                            resolution_trace.push(format!("item metadata lookup failed: {err}"));
                            None
                        }
                    };

                let attachments = if let Some(attachments) = prefetched_attachments {
                    attachments
                } else {
                    let children_request = ZoteroChildrenRequest {
                        item_key: &normalized.item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    };
                    match zotero::get_attachments(toolkit.http(), config, scope, &children_request)
                        .await
                    {
                        Ok(result) => result.attachments,
                        Err(err) => {
                            resolution_trace.push(format!("attachment lookup failed: {err}"));
                            Vec::new()
                        }
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
    toolkit.ensure_zotero_running().await?;
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
    toolkit.ensure_zotero_running().await?;
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
    toolkit.ensure_zotero_running().await?;
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
