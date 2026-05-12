use serde_json::Value;

use super::*;
use crate::types::ZoteroAddItemsToCollectionParams;
use crate::types::ZoteroCreateAttachmentImportUrlParams;
use crate::types::ZoteroCreateAttachmentLinkParams;
use crate::types::ZoteroCreateCollectionParams;
use crate::types::ZoteroCreateCollectionResult;
use crate::types::ZoteroCreateItemsParams;
use crate::types::ZoteroFindOrCreateCollectionParams;
use crate::types::ZoteroItemUpdatePayload;
use crate::types::ZoteroMutationRecord;
use crate::types::ZoteroMutationResult;
use crate::types::ZoteroUpdateItemsParams;

pub(crate) async fn zotero_create_collection(
    toolkit: &ResearchToolkit,
    params: ZoteroCreateCollectionParams,
) -> Result<ZoteroCreateCollectionResult> {
    toolkit.ensure_zotero_running().await?;
    let normalized =
        normalize_create_collection_params(toolkit, params, "zotero_create_collection")?;
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let collection = zotero::create_collection(
        toolkit.http(),
        config,
        &scope,
        &normalized.name,
        normalized.parent_collection_key.as_deref(),
    )
    .await?;
    toolkit.cache().clear().await;
    Ok(ZoteroCreateCollectionResult {
        collection,
        created: true,
        warnings: Vec::new(),
    })
}

pub(crate) async fn zotero_find_or_create_collection(
    toolkit: &ResearchToolkit,
    params: ZoteroFindOrCreateCollectionParams,
) -> Result<ZoteroCreateCollectionResult> {
    toolkit.ensure_zotero_running().await?;
    let normalized = normalize_find_or_create_collection_params(
        toolkit,
        params,
        "zotero_find_or_create_collection",
    )?;
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    if let Some(collection) = find_existing_collection(
        toolkit,
        config,
        &scope,
        &normalized.name,
        normalized.parent_collection_key.as_deref(),
    )
    .await?
    {
        return Ok(ZoteroCreateCollectionResult {
            collection,
            created: false,
            warnings: Vec::new(),
        });
    }

    zotero_create_collection(
        toolkit,
        ZoteroCreateCollectionParams {
            name: normalized.name,
            parent_collection_key: normalized.parent_collection_key,
            library_type: Some(normalized.scope.library_type),
            library_id: Some(normalized.scope.library_id),
        },
    )
    .await
}

async fn find_existing_collection(
    toolkit: &ResearchToolkit,
    config: zotero::ZoteroConfig<'_>,
    scope: &zotero::ZoteroLibraryScope,
    name: &str,
    parent_collection_key: Option<&str>,
) -> Result<Option<crate::types::ZoteroCollection>> {
    let mut offset = 0;
    loop {
        let collections = zotero::get_collections(
            toolkit.http(),
            config,
            scope,
            ZoteroCollectionsRequest {
                offset,
                limit: DEFAULT_COLLECTIONS_LIMIT,
            },
        )
        .await?;

        if let Some(collection) = collections.collections.into_iter().find(|collection| {
            collection.name == name
                && collection.parent_collection.as_deref() == parent_collection_key
        }) {
            return Ok(Some(collection));
        }

        if !collections.has_more {
            return Ok(None);
        }

        offset += DEFAULT_COLLECTIONS_LIMIT;
    }
}

pub(crate) async fn zotero_create_items(
    toolkit: &ResearchToolkit,
    params: ZoteroCreateItemsParams,
) -> Result<ZoteroMutationResult> {
    toolkit.ensure_zotero_running().await?;
    let normalized = normalize_create_items_params(toolkit, params, "zotero_create_items")?;
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut records = Vec::with_capacity(normalized.items.len());
    for item in &normalized.items {
        records.push(zotero::create_item(toolkit.http(), config, &scope, item).await?);
    }
    toolkit.cache().clear().await;
    Ok(ZoteroMutationResult {
        records,
        warnings: Vec::new(),
    })
}

pub(crate) async fn zotero_update_items(
    toolkit: &ResearchToolkit,
    params: ZoteroUpdateItemsParams,
) -> Result<ZoteroMutationResult> {
    toolkit.ensure_zotero_running().await?;
    let inferred_scope = if params.library_type.is_none() && params.library_id.is_none() {
        Some(
            infer_scope_from_item_keys(
                toolkit,
                params
                    .items
                    .iter()
                    .map(|item| item.item_key.as_str())
                    .collect::<Vec<_>>()
                    .as_slice(),
                "zotero_update_items",
            )
            .await?,
        )
    } else {
        None
    };
    let normalized = normalize_update_items_params(
        toolkit,
        ZoteroUpdateItemsParams {
            items: params.items,
            library_type: inferred_scope
                .as_ref()
                .map(|scope| scope.library_type.clone())
                .or(params.library_type),
            library_id: inferred_scope
                .as_ref()
                .map(|scope| scope.library_id.clone())
                .or(params.library_id),
        },
        "zotero_update_items",
    )?;
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut records = Vec::with_capacity(normalized.items.len());

    for item in normalized.items {
        let raw_item = zotero::get_item_raw(toolkit.http(), config, &scope, &item.item_key).await?;
        let version = raw_item
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ResearchError::Parse {
                api: ResearchApi::Zotero,
                message: format!("zotero item {} missing version", item.item_key),
            })?;
        let mut data = raw_item
            .get("data")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| ResearchError::Parse {
                api: ResearchApi::Zotero,
                message: format!("zotero item {} missing data object", item.item_key),
            })?;
        let patch = item.patch.as_object().ok_or_else(|| {
            ResearchError::InvalidInput("zotero_update_items patch must be an object".to_string())
        })?;
        for (key, value) in patch {
            data.insert(key.clone(), value.clone());
        }
        let updated_version = zotero::update_item_data(
            toolkit.http(),
            config,
            &scope,
            &item.item_key,
            version,
            &Value::Object(data.clone()),
        )
        .await?;
        records.push(ZoteroMutationRecord {
            key: item.item_key,
            version: updated_version,
            title: data
                .get("title")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            item_type: data
                .get("itemType")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            url: data
                .get("url")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            parent_item: data
                .get("parentItem")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            collection_keys: data
                .get("collections")
                .and_then(Value::as_array)
                .map(|collections| {
                    collections
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        });
    }

    toolkit.cache().clear().await;
    Ok(ZoteroMutationResult {
        records,
        warnings: Vec::new(),
    })
}

pub(crate) async fn zotero_add_items_to_collection(
    toolkit: &ResearchToolkit,
    params: ZoteroAddItemsToCollectionParams,
) -> Result<ZoteroMutationResult> {
    toolkit.ensure_zotero_running().await?;
    let inferred_scope = if params.library_type.is_none() && params.library_id.is_none() {
        Some(
            infer_scope_from_collection_key(
                toolkit,
                &params.collection_key,
                "zotero_add_items_to_collection",
            )
            .await?,
        )
    } else {
        None
    };
    let normalized = normalize_add_items_to_collection_params(
        toolkit,
        ZoteroAddItemsToCollectionParams {
            collection_key: params.collection_key,
            item_keys: params.item_keys,
            library_type: inferred_scope
                .as_ref()
                .map(|scope| scope.library_type.clone())
                .or(params.library_type),
            library_id: inferred_scope
                .as_ref()
                .map(|scope| scope.library_id.clone())
                .or(params.library_id),
        },
        "zotero_add_items_to_collection",
    )?;
    let update_items = normalized
        .item_keys
        .iter()
        .map(|item_key| ZoteroItemUpdatePayload {
            item_key: item_key.clone(),
            patch: serde_json::json!({
                "collections": [normalized.collection_key.clone()]
            }),
        })
        .collect::<Vec<_>>();

    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut records = Vec::with_capacity(update_items.len());
    for item in update_items {
        let raw_item = zotero::get_item_raw(toolkit.http(), config, &scope, &item.item_key).await?;
        let version = raw_item
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ResearchError::Parse {
                api: ResearchApi::Zotero,
                message: format!("zotero item {} missing version", item.item_key),
            })?;
        let mut data = raw_item
            .get("data")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| ResearchError::Parse {
                api: ResearchApi::Zotero,
                message: format!("zotero item {} missing data object", item.item_key),
            })?;
        let mut collections = data
            .get("collections")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        if !collections.contains(&normalized.collection_key) {
            collections.push(normalized.collection_key.clone());
        }
        data.insert(
            "collections".to_string(),
            Value::Array(collections.iter().cloned().map(Value::String).collect()),
        );
        let updated_version = zotero::update_item_data(
            toolkit.http(),
            config,
            &scope,
            &item.item_key,
            version,
            &Value::Object(data.clone()),
        )
        .await?;
        records.push(ZoteroMutationRecord {
            key: item.item_key,
            version: updated_version,
            title: data
                .get("title")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            item_type: data
                .get("itemType")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            url: data
                .get("url")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            parent_item: data
                .get("parentItem")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            collection_keys: collections,
        });
    }

    toolkit.cache().clear().await;
    Ok(ZoteroMutationResult {
        records,
        warnings: Vec::new(),
    })
}

pub(crate) async fn zotero_create_attachment_link(
    toolkit: &ResearchToolkit,
    params: ZoteroCreateAttachmentLinkParams,
) -> Result<ZoteroMutationResult> {
    toolkit.ensure_zotero_running().await?;
    let inferred_scope = if params.library_type.is_none() && params.library_id.is_none() {
        Some(
            infer_scope_from_parent_item(
                toolkit,
                &params.parent_item_key,
                "zotero_create_attachment_link",
            )
            .await?,
        )
    } else {
        None
    };
    let normalized = normalize_create_attachment_link_params(
        toolkit,
        ZoteroCreateAttachmentLinkParams {
            parent_item_key: params.parent_item_key,
            title: params.title,
            url: params.url,
            content_type: params.content_type,
            collections: params.collections,
            tags: params.tags,
            library_type: inferred_scope
                .as_ref()
                .map(|scope| scope.library_type.clone())
                .or(params.library_type),
            library_id: inferred_scope
                .as_ref()
                .map(|scope| scope.library_id.clone())
                .or(params.library_id),
        },
        "zotero_create_attachment_link",
    )?;
    let item = serde_json::json!({
        "itemType": "attachment",
        "parentItem": normalized.parent_item_key,
        "linkMode": "linked_url",
        "title": normalized.title,
        "url": normalized.url,
        "contentType": normalized.content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
        "collections": normalized.collections,
        "tags": normalized
            .tags
            .into_iter()
            .map(|tag| serde_json::json!({ "tag": tag }))
            .collect::<Vec<_>>(),
    });
    zotero_create_items(
        toolkit,
        ZoteroCreateItemsParams {
            items: vec![item],
            library_type: Some(normalized.scope.library_type),
            library_id: Some(normalized.scope.library_id),
        },
    )
    .await
}

pub(crate) async fn zotero_create_attachment_import_url(
    toolkit: &ResearchToolkit,
    params: ZoteroCreateAttachmentImportUrlParams,
) -> Result<ZoteroMutationResult> {
    toolkit.ensure_zotero_running().await?;
    let inferred_scope = if params.library_type.is_none() && params.library_id.is_none() {
        Some(
            infer_scope_from_parent_item(
                toolkit,
                &params.parent_item_key,
                "zotero_create_attachment_import_url",
            )
            .await?,
        )
    } else {
        None
    };
    let normalized = normalize_create_attachment_import_url_params(
        toolkit,
        ZoteroCreateAttachmentImportUrlParams {
            parent_item_key: params.parent_item_key,
            title: params.title,
            url: params.url,
            content_type: params.content_type,
            filename: params.filename,
            tags: params.tags,
            library_type: inferred_scope
                .as_ref()
                .map(|scope| scope.library_type.clone())
                .or(params.library_type),
            library_id: inferred_scope
                .as_ref()
                .map(|scope| scope.library_id.clone())
                .or(params.library_id),
        },
        "zotero_create_attachment_import_url",
    )?;
    let content_type = normalized
        .content_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let filename = normalized
        .filename
        .clone()
        .unwrap_or_else(|| "attachment.bin".to_string());
    let file_bytes = zotero::download_attachment_url(toolkit.http(), &normalized.url).await?;
    let item = serde_json::json!({
        "itemType": "attachment",
        "parentItem": normalized.parent_item_key,
        "linkMode": "imported_url",
        "title": normalized.title,
        "url": normalized.url,
        "contentType": content_type,
        "filename": filename,
        "tags": normalized
            .tags
            .into_iter()
            .map(|tag| serde_json::json!({ "tag": tag }))
            .collect::<Vec<_>>(),
    });
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let record = zotero::create_item(toolkit.http(), config, &scope, &item).await?;
    zotero::upload_attachment_file(
        toolkit.http(),
        config,
        &scope,
        &zotero::ZoteroFileUploadRequest {
            item_key: &record.key,
            filename: filename.as_str(),
            content_type: content_type.as_str(),
            file_bytes,
        },
    )
    .await?;
    toolkit.cache().clear().await;
    Ok(ZoteroMutationResult {
        records: vec![record],
        warnings: Vec::new(),
    })
}

async fn infer_scope_from_parent_item(
    toolkit: &ResearchToolkit,
    parent_item_key: &str,
    tool_name: &'static str,
) -> Result<NormalizedScope> {
    let config = zotero_config(toolkit);
    let scopes = discover_all_scopes(toolkit).await?;
    let (scope, _) = fetch_item_across_scopes(toolkit, config, &scopes, parent_item_key)
        .await
        .map_err(|err| {
            ResearchError::InvalidInput(format!(
                "{tool_name} could not resolve parent item `{parent_item_key}` across accessible libraries: {err}"
            ))
        })?;
    Ok(to_normalized_scope(&scope))
}

async fn infer_scope_from_item_keys(
    toolkit: &ResearchToolkit,
    item_keys: &[&str],
    tool_name: &'static str,
) -> Result<NormalizedScope> {
    let config = zotero_config(toolkit);
    let scopes = discover_all_scopes(toolkit).await?;
    let mut matched_scope = None;
    for item_key in item_keys {
        let (scope, _) = fetch_item_across_scopes(toolkit, config, &scopes, item_key)
            .await
            .map_err(|err| {
                ResearchError::InvalidInput(format!(
                    "{tool_name} could not resolve item `{item_key}` across accessible libraries: {err}"
                ))
            })?;
        let normalized = to_normalized_scope(&scope);
        if let Some(current) = matched_scope.as_ref()
            && current != &normalized
        {
            return Err(ResearchError::InvalidInput(format!(
                "{tool_name} items span multiple Zotero libraries; pass --library-type/--library-id explicitly"
            )));
        }
        matched_scope = Some(normalized);
    }
    matched_scope.ok_or_else(|| {
        ResearchError::InvalidInput(format!(
            "{tool_name} requires at least one item key to infer the Zotero library"
        ))
    })
}

async fn infer_scope_from_collection_key(
    toolkit: &ResearchToolkit,
    collection_key: &str,
    tool_name: &'static str,
) -> Result<NormalizedScope> {
    let config = zotero_config(toolkit);
    let scopes = discover_all_scopes(toolkit).await?;
    let mut matches = Vec::new();
    for scope in scopes {
        let mut offset = 0;
        loop {
            let page = zotero::get_collections(
                toolkit.http(),
                config,
                &scope,
                ZoteroCollectionsRequest {
                    offset,
                    limit: DEFAULT_COLLECTIONS_LIMIT,
                },
            )
            .await;
            let Ok(page) = page else {
                break;
            };
            if page
                .collections
                .iter()
                .any(|collection| collection.key == collection_key)
            {
                matches.push(to_normalized_scope(&scope));
                break;
            }
            if !page.has_more {
                break;
            }
            offset += DEFAULT_COLLECTIONS_LIMIT;
        }
    }

    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(ResearchError::InvalidInput(format!(
            "{tool_name} could not resolve collection `{collection_key}` across accessible libraries"
        ))),
        _ => Err(ResearchError::InvalidInput(format!(
            "{tool_name} found collection `{collection_key}` in multiple Zotero libraries; pass --library-type/--library-id explicitly"
        ))),
    }
}
