use serde_json::Value;

use super::*;
use crate::types::ZoteroAddItemsToCollectionParams;
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
    let normalized = normalize_update_items_params(toolkit, params, "zotero_update_items")?;
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
    let normalized = normalize_add_items_to_collection_params(
        toolkit,
        params,
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
    let normalized =
        normalize_create_attachment_link_params(toolkit, params, "zotero_create_attachment_link")?;
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
