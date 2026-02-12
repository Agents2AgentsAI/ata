use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::ResearchError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::text_utils::truncate_chars;
use crate::types::SourceMeta;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroAttachmentsResult;
use crate::types::ZoteroCollection;
use crate::types::ZoteroCollectionsResult;
use crate::types::ZoteroFullTextResult;
use crate::types::ZoteroGroup;
use crate::types::ZoteroGroupsResult;
use crate::types::ZoteroItem;
use crate::types::ZoteroItemDetail;
use crate::types::ZoteroNote;
use crate::types::ZoteroNotesResult;
use crate::types::ZoteroSearchResult;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoteroConfig<'a> {
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ZoteroLibraryScope {
    User(String),
    Group(String),
}

impl ZoteroLibraryScope {
    fn root_url(&self, base_url: &str) -> String {
        let base_url = base_url.trim_end_matches('/');
        match self {
            Self::User(user_id) => format!("{base_url}/users/{user_id}"),
            Self::Group(group_id) => format!("{base_url}/groups/{group_id}"),
        }
    }

    fn canonical_item_id(&self, item_key: &str) -> String {
        match self {
            Self::User(user_id) => format!("zotero:user/{user_id}/{item_key}"),
            Self::Group(group_id) => format!("zotero:group/{group_id}/{item_key}"),
        }
    }

    fn canonical_collection_id(&self, collection_key: &str) -> String {
        match self {
            Self::User(user_id) => {
                format!("zotero:user/{user_id}/collection/{collection_key}")
            }
            Self::Group(group_id) => {
                format!("zotero:group/{group_id}/collection/{collection_key}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ZoteroSearchRequest<'a> {
    pub query: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub offset: u32,
    pub limit: u32,
    pub item_type: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub direction: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub(crate) struct ZoteroChildrenRequest<'a> {
    pub item_key: &'a str,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ZoteroCollectionsRequest {
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ZoteroListGroupsRequest {
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ZoteroCollectionItemsRequest<'a> {
    pub collection_key: &'a str,
    pub offset: u32,
    pub limit: u32,
    pub item_type: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoteroLibraryAnnotationsRequest {
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct ZoteroAnnotation {
    pub key: String,
    pub parent_item: Option<String>,
    pub annotation_type: Option<String>,
    pub annotation_text: Option<String>,
    pub annotation_comment: Option<String>,
    pub annotation_color: Option<String>,
    pub annotation_page_label: Option<String>,
    pub annotation_sort_index: Option<String>,
    pub source_meta: Option<SourceMeta>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct ZoteroAnnotationsResult {
    pub item_key: String,
    pub annotations: Vec<ZoteroAnnotation>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct ZoteroLibraryAnnotationsPage {
    pub annotations: Vec<ZoteroAnnotation>,
    pub total_available: Option<u64>,
    pub has_more: bool,
}

pub(crate) async fn search_items(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroSearchRequest<'_>,
) -> Result<ZoteroSearchResult> {
    let mut url = format!(
        "{root}/items?format=json&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        offset = request.offset,
        limit = request.limit,
    );

    if let Some(query) = request.query {
        url.push_str(&format!("&q={}", urlencoding::encode(query)));
    }

    if let Some(tag) = request.tag {
        url.push_str(&format!("&tag={}", urlencoding::encode(tag)));
    }

    if let Some(item_type) = request.item_type {
        url.push_str(&format!("&itemType={}", urlencoding::encode(item_type)));
    }

    if let Some(sort) = request.sort {
        url.push_str(&format!("&sort={}", urlencoding::encode(sort)));
    } else if request.query.is_some() {
        url.push_str("&sort=relevance");
    }

    if let Some(direction) = request.direction {
        url.push_str(&format!("&direction={}", urlencoding::encode(direction)));
    }

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let items = response
        .into_iter()
        .map(|item| map_item_summary(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroSearchResult {
        has_more: compute_has_more(request.offset, items.len(), request.limit, total_available),
        total_available,
        items,
    })
}

pub(crate) async fn get_item(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    item_key: &str,
) -> Result<ZoteroItemDetail> {
    let url = format!(
        "{root}/items/{item_key}?format=json",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(item_key),
    );

    let response: ZoteroApiItem = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    Ok(map_item_detail(response, &url, scope))
}

pub(crate) async fn get_fulltext(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    item_key: &str,
) -> Result<ZoteroFullTextResult> {
    let url = format!(
        "{root}/items/{item_key}/fulltext",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(item_key),
    );

    let response: ZoteroApiFullText = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    Ok(ZoteroFullTextResult {
        item_key: item_key.to_string(),
        content: response.content.unwrap_or_default(),
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: url,
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(item_key)),
        }),
    })
}

pub(crate) async fn get_notes(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroChildrenRequest<'_>,
) -> Result<ZoteroNotesResult> {
    let url = format!(
        "{root}/items/{item_key}/children?format=json&itemType=note&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(request.item_key),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let notes = response
        .into_iter()
        .filter_map(|item| map_note(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroNotesResult {
        item_key: request.item_key.to_string(),
        has_more: compute_has_more(request.offset, notes.len(), request.limit, total_available),
        total_available,
        notes,
    })
}

pub(crate) async fn get_annotations(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroChildrenRequest<'_>,
) -> Result<ZoteroAnnotationsResult> {
    let url = format!(
        "{root}/items/{item_key}/children?format=json&itemType=annotation&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(request.item_key),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let annotations = response
        .into_iter()
        .filter_map(|item| map_annotation(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroAnnotationsResult {
        item_key: request.item_key.to_string(),
        has_more: compute_has_more(
            request.offset,
            annotations.len(),
            request.limit,
            total_available,
        ),
        total_available,
        annotations,
    })
}

pub(crate) async fn get_library_annotations(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: ZoteroLibraryAnnotationsRequest,
) -> Result<ZoteroLibraryAnnotationsPage> {
    let url = format!(
        "{root}/items?format=json&itemType=annotation&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let annotations = response
        .into_iter()
        .filter_map(|item| map_annotation(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroLibraryAnnotationsPage {
        has_more: compute_has_more(
            request.offset,
            annotations.len(),
            request.limit,
            total_available,
        ),
        total_available,
        annotations,
    })
}

pub(crate) async fn get_children_items(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroChildrenRequest<'_>,
    item_type: Option<&str>,
) -> Result<ZoteroSearchResult> {
    let mut url = format!(
        "{root}/items/{item_key}/children?format=json&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(request.item_key),
        offset = request.offset,
        limit = request.limit,
    );

    if let Some(item_type) = item_type {
        url.push_str(&format!("&itemType={}", urlencoding::encode(item_type)));
    }

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let items = response
        .into_iter()
        .map(|item| map_item_summary(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroSearchResult {
        has_more: compute_has_more(request.offset, items.len(), request.limit, total_available),
        total_available,
        items,
    })
}

pub(crate) async fn get_note_item(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    item_key: &str,
) -> Result<Option<ZoteroNote>> {
    let url = format!(
        "{root}/items/{item_key}?format=json",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(item_key),
    );

    let response: ZoteroApiItem = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    Ok(map_note(response, &url, scope))
}

pub(crate) async fn get_annotation_item(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    item_key: &str,
) -> Result<Option<ZoteroAnnotation>> {
    let url = format!(
        "{root}/items/{item_key}?format=json",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(item_key),
    );

    let response: ZoteroApiItem = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    Ok(map_annotation(response, &url, scope))
}

pub(crate) async fn get_attachments(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroChildrenRequest<'_>,
) -> Result<ZoteroAttachmentsResult> {
    let url = format!(
        "{root}/items/{item_key}/children?format=json&itemType=attachment&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        item_key = urlencoding::encode(request.item_key),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let attachments = response
        .into_iter()
        .filter_map(|item| map_attachment(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroAttachmentsResult {
        item_key: request.item_key.to_string(),
        has_more: compute_has_more(
            request.offset,
            attachments.len(),
            request.limit,
            total_available,
        ),
        total_available,
        attachments,
    })
}

pub(crate) async fn get_collections(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: ZoteroCollectionsRequest,
) -> Result<ZoteroCollectionsResult> {
    let url = format!(
        "{root}/collections?format=json&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiCollection>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let collections = response
        .into_iter()
        .map(|collection| map_collection(collection, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroCollectionsResult {
        has_more: compute_has_more(
            request.offset,
            collections.len(),
            request.limit,
            total_available,
        ),
        total_available,
        collections,
    })
}

pub(crate) async fn list_groups(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    user_id: &str,
    request: ZoteroListGroupsRequest,
) -> Result<ZoteroGroupsResult> {
    let scope = ZoteroLibraryScope::User(user_id.to_string());
    let url = format!(
        "{root}/groups?format=json&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        offset = request.offset,
        limit = request.limit,
    );

    let (response, total_available): (Vec<ZoteroApiGroup>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let groups = response
        .into_iter()
        .map(|group| map_group(group, &url))
        .collect::<Vec<_>>();

    Ok(ZoteroGroupsResult {
        has_more: compute_has_more(request.offset, groups.len(), request.limit, total_available),
        total_available,
        groups,
    })
}

pub(crate) async fn get_collection_items(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    scope: &ZoteroLibraryScope,
    request: &ZoteroCollectionItemsRequest<'_>,
) -> Result<ZoteroSearchResult> {
    let mut url = format!(
        "{root}/collections/{collection_key}/items?format=json&start={offset}&limit={limit}",
        root = scope.root_url(config.base_url),
        collection_key = urlencoding::encode(request.collection_key),
        offset = request.offset,
        limit = request.limit,
    );

    if let Some(item_type) = request.item_type {
        url.push_str(&format!("&itemType={}", urlencoding::encode(item_type)));
    }

    let (response, total_available): (Vec<ZoteroApiItem>, Option<u64>) =
        execute_json_with_total_results(http, config, &url).await?;

    let items = response
        .into_iter()
        .map(|item| map_item_summary(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroSearchResult {
        has_more: compute_has_more(request.offset, items.len(), request.limit, total_available),
        total_available,
        items,
    })
}

fn zotero_request(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    url: &str,
) -> reqwest::RequestBuilder {
    let mut request = http.client().get(url).header("Zotero-API-Version", "3");
    if let Some(api_key) = config.api_key.filter(|value| !value.trim().is_empty()) {
        request = request.header("Zotero-API-Key", api_key);
    } else {
        // Local Zotero API accepts this header for non-browser clients.
        request = request.header("Zotero-Allowed-Request", "1");
    }
    request
}

async fn execute_json_with_total_results<T>(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    url: &str,
) -> Result<(T, Option<u64>)>
where
    T: DeserializeOwned,
{
    let response = http
        .execute_response(ResearchApi::Zotero, || zotero_request(http, config, url))
        .await?;
    let total_available = response
        .headers()
        .get("Total-Results")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<u64>().ok());

    let payload = response
        .json::<T>()
        .await
        .map_err(|err| ResearchError::Parse {
            api: ResearchApi::Zotero,
            message: err.to_string(),
        })?;
    Ok((payload, total_available))
}

fn compute_has_more(
    offset: u32,
    page_len: usize,
    limit: u32,
    total_available: Option<u64>,
) -> bool {
    if let Some(total) = total_available {
        let offset = u64::from(offset);
        let page_len = u64::try_from(page_len).unwrap_or(u64::MAX);
        return offset.saturating_add(page_len) < total;
    }

    u32::try_from(page_len).unwrap_or(0) == limit
}

fn map_item_summary(item: ZoteroApiItem, api_url: &str, scope: &ZoteroLibraryScope) -> ZoteroItem {
    let authors = creator_display_names(&item.data.creators);
    let tags = item
        .data
        .tags
        .iter()
        .map(|tag| tag.tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();

    ZoteroItem {
        key: item.key.clone(),
        title: item
            .data
            .title
            .unwrap_or_else(|| "Untitled Zotero Item".to_string()),
        authors: authors.join(", "),
        year: year_from_date(item.data.date.as_deref()),
        item_type: item.data.item_type.unwrap_or_else(|| "unknown".to_string()),
        doi: item.data.doi,
        abstract_snippet: item
            .data
            .abstract_note
            .as_deref()
            .map(|text| truncate_chars(text, 200)),
        tags,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(&item.key)),
        }),
    }
}

fn map_item_detail(
    item: ZoteroApiItem,
    api_url: &str,
    scope: &ZoteroLibraryScope,
) -> ZoteroItemDetail {
    ZoteroItemDetail {
        key: item.key.clone(),
        title: item
            .data
            .title
            .unwrap_or_else(|| "Untitled Zotero Item".to_string()),
        authors: creator_display_names(&item.data.creators),
        abstract_text: item.data.abstract_note,
        date: item.data.date,
        doi: item.data.doi,
        url: item.data.url,
        publication: item.data.publication_title,
        item_type: item.data.item_type.unwrap_or_else(|| "unknown".to_string()),
        tags: item
            .data
            .tags
            .iter()
            .map(|tag| tag.tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect(),
        extra: item.data.extra,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(&item.key)),
        }),
    }
}

fn map_note(item: ZoteroApiItem, api_url: &str, scope: &ZoteroLibraryScope) -> Option<ZoteroNote> {
    if item.data.item_type.as_deref() != Some("note") {
        return None;
    }

    Some(ZoteroNote {
        key: item.key.clone(),
        title: item.data.title,
        note: item.data.note,
        parent_item: item.data.parent_item,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(&item.key)),
        }),
    })
}

fn map_annotation(
    item: ZoteroApiItem,
    api_url: &str,
    scope: &ZoteroLibraryScope,
) -> Option<ZoteroAnnotation> {
    if item.data.item_type.as_deref() != Some("annotation") {
        return None;
    }

    Some(ZoteroAnnotation {
        key: item.key.clone(),
        parent_item: item.data.parent_item,
        annotation_type: item.data.annotation_type,
        annotation_text: item.data.annotation_text,
        annotation_comment: item.data.annotation_comment,
        annotation_color: item.data.annotation_color,
        annotation_page_label: item.data.annotation_page_label,
        annotation_sort_index: item.data.annotation_sort_index,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(&item.key)),
        }),
    })
}

fn map_attachment(
    item: ZoteroApiItem,
    api_url: &str,
    scope: &ZoteroLibraryScope,
) -> Option<ZoteroAttachment> {
    if item.data.item_type.as_deref() != Some("attachment") {
        return None;
    }

    Some(ZoteroAttachment {
        key: item.key.clone(),
        title: item.data.title,
        filename: item.data.filename,
        content_type: item.data.content_type,
        link_mode: item.data.link_mode,
        url: item.data.url,
        parent_item: item.data.parent_item,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_item_id(&item.key)),
        }),
    })
}

fn map_collection(
    collection: ZoteroApiCollection,
    api_url: &str,
    scope: &ZoteroLibraryScope,
) -> ZoteroCollection {
    let key = collection.key;
    ZoteroCollection {
        key: key.clone(),
        name: collection
            .data
            .name
            .unwrap_or_else(|| "Untitled Collection".to_string()),
        parent_collection: collection.data.parent_collection,
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(scope.canonical_collection_id(&key)),
        }),
    }
}

fn map_group(group: ZoteroApiGroup, api_url: &str) -> ZoteroGroup {
    let ZoteroApiGroup { id, data } = group;
    let ZoteroApiGroupData {
        id: data_id,
        name,
        description,
    } = data;

    let id = id
        .or(data_id)
        .map(ZoteroApiGroupId::into_string)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    ZoteroGroup {
        id: id.clone(),
        name: name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Untitled Group".to_string()),
        description: description
            .map(|description| description.trim().to_string())
            .filter(|description| !description.is_empty()),
        source_meta: Some(SourceMeta {
            source: "zotero".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id: Some(format!("zotero:group/{id}")),
        }),
    }
}

fn creator_display_names(creators: &[ZoteroApiCreator]) -> Vec<String> {
    creators
        .iter()
        .filter_map(|creator| {
            if let Some(name) = creator.name.as_deref() {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }

            let first = creator.first_name.as_deref().unwrap_or("").trim();
            let last = creator.last_name.as_deref().unwrap_or("").trim();
            let full = format!("{first} {last}").trim().to_string();
            if full.is_empty() { None } else { Some(full) }
        })
        .collect()
}

fn year_from_date(date: Option<&str>) -> Option<String> {
    let raw = date?.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.len() >= 4 {
        let prefix = &raw[..4];
        if prefix.chars().all(|char| char.is_ascii_digit())
            && let Ok(year) = prefix.parse::<u32>()
            && (1900..=2099).contains(&year)
        {
            return Some(year.to_string());
        }
    }

    let mut run = String::new();
    for char in raw.chars() {
        if char.is_ascii_digit() {
            run.push(char);
            continue;
        }

        if let Some(year) = parse_year_run(&run) {
            return Some(year);
        }
        run.clear();
    }

    parse_year_run(&run)
}

fn parse_year_run(run: &str) -> Option<String> {
    if run.len() < 4 {
        return None;
    }

    for idx in 0..=run.len().saturating_sub(4) {
        let candidate = run.get(idx..idx + 4)?;
        let year = candidate.parse::<u32>().ok()?;
        if (1900..=2099).contains(&year) {
            return Some(year.to_string());
        }
    }

    None
}

#[derive(Debug, Deserialize)]
struct ZoteroApiItem {
    key: String,
    #[serde(default)]
    data: ZoteroApiItemData,
}

#[derive(Debug, Deserialize, Default)]
struct ZoteroApiItemData {
    #[serde(rename = "itemType")]
    item_type: Option<String>,
    title: Option<String>,
    #[serde(default)]
    creators: Vec<ZoteroApiCreator>,
    date: Option<String>,
    #[serde(rename = "DOI", alias = "doi")]
    doi: Option<String>,
    #[serde(rename = "abstractNote")]
    abstract_note: Option<String>,
    url: Option<String>,
    #[serde(rename = "publicationTitle")]
    publication_title: Option<String>,
    #[serde(default)]
    tags: Vec<ZoteroApiTag>,
    extra: Option<String>,
    note: Option<String>,
    #[serde(rename = "parentItem")]
    parent_item: Option<String>,
    #[serde(rename = "annotationType")]
    annotation_type: Option<String>,
    #[serde(rename = "annotationText")]
    annotation_text: Option<String>,
    #[serde(rename = "annotationComment")]
    annotation_comment: Option<String>,
    #[serde(rename = "annotationColor")]
    annotation_color: Option<String>,
    #[serde(rename = "annotationPageLabel")]
    annotation_page_label: Option<String>,
    #[serde(rename = "annotationSortIndex")]
    annotation_sort_index: Option<String>,
    filename: Option<String>,
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    #[serde(rename = "linkMode")]
    link_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZoteroApiCreator {
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZoteroApiTag {
    tag: String,
}

#[cfg(test)]
mod tests {
    use super::year_from_date;
    use pretty_assertions::assert_eq;

    #[test]
    fn year_from_date_prefers_prefix_year_when_present() {
        assert_eq!(year_from_date(Some("2023-06-01")), Some("2023".to_string()));
    }

    #[test]
    fn year_from_date_finds_four_digit_year_tokens() {
        assert_eq!(
            year_from_date(Some("v1.2.3 2023 revision")),
            Some("2023".to_string())
        );
        assert_eq!(
            year_from_date(Some("Project 2021: Ancient History")),
            Some("2021".to_string())
        );
    }

    #[test]
    fn year_from_date_rejects_out_of_range_runs() {
        assert_eq!(year_from_date(Some("v1.2.3 1234 build")), None);
        assert_eq!(year_from_date(Some("")), None);
        assert_eq!(year_from_date(None), None);
    }
}

#[derive(Debug, Deserialize)]
struct ZoteroApiFullText {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZoteroApiCollection {
    key: String,
    #[serde(default)]
    data: ZoteroApiCollectionData,
}

#[derive(Debug, Deserialize, Default)]
struct ZoteroApiCollectionData {
    name: Option<String>,
    #[serde(rename = "parentCollection")]
    parent_collection: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ZoteroApiGroupId {
    Numeric(u64),
    Text(String),
}

impl ZoteroApiGroupId {
    fn into_string(self) -> String {
        match self {
            Self::Numeric(id) => id.to_string(),
            Self::Text(id) => id.trim().to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ZoteroApiGroup {
    id: Option<ZoteroApiGroupId>,
    #[serde(default)]
    data: ZoteroApiGroupData,
}

#[derive(Debug, Deserialize, Default)]
struct ZoteroApiGroupData {
    id: Option<ZoteroApiGroupId>,
    name: Option<String>,
    description: Option<String>,
}
