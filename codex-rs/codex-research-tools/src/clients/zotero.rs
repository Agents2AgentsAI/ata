use chrono::Utc;
use serde::Deserialize;

use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::types::SourceMeta;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroAttachmentsResult;
use crate::types::ZoteroCollection;
use crate::types::ZoteroCollectionsResult;
use crate::types::ZoteroFullTextResult;
use crate::types::ZoteroItem;
use crate::types::ZoteroItemDetail;
use crate::types::ZoteroNote;
use crate::types::ZoteroNotesResult;
use crate::types::ZoteroSearchResult;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ZoteroConfig<'a> {
    pub base_url: &'a str,
    pub api_key: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ZoteroLibraryScope {
    User(String),
    Group(String),
}

impl ZoteroLibraryScope {
    fn root_url(&self, base_url: &str) -> String {
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
pub(crate) struct ZoteroCollectionItemsRequest<'a> {
    pub collection_key: &'a str,
    pub offset: u32,
    pub limit: u32,
    pub item_type: Option<&'a str>,
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
        url.push_str("&sort=relevance");
    }

    if let Some(tag) = request.tag {
        url.push_str(&format!("&tag={}", urlencoding::encode(tag)));
    }

    if let Some(item_type) = request.item_type {
        url.push_str(&format!("&itemType={}", urlencoding::encode(item_type)));
    }

    let response: Vec<ZoteroApiItem> = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    let items = response
        .into_iter()
        .map(|item| map_item_summary(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroSearchResult {
        has_more: u32::try_from(items.len()).unwrap_or(0) == request.limit,
        total_available: None,
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

    let response: Vec<ZoteroApiItem> = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    let notes = response
        .into_iter()
        .filter_map(|item| map_note(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroNotesResult {
        item_key: request.item_key.to_string(),
        has_more: u32::try_from(notes.len()).unwrap_or(0) == request.limit,
        total_available: None,
        notes,
    })
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

    let response: Vec<ZoteroApiItem> = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    let attachments = response
        .into_iter()
        .filter_map(|item| map_attachment(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroAttachmentsResult {
        item_key: request.item_key.to_string(),
        has_more: u32::try_from(attachments.len()).unwrap_or(0) == request.limit,
        total_available: None,
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

    let response: Vec<ZoteroApiCollection> = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    let collections = response
        .into_iter()
        .map(|collection| map_collection(collection, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroCollectionsResult {
        has_more: u32::try_from(collections.len()).unwrap_or(0) == request.limit,
        total_available: None,
        collections,
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

    let response: Vec<ZoteroApiItem> = http
        .execute_json(ResearchApi::Zotero, || zotero_request(http, config, &url))
        .await?;

    let items = response
        .into_iter()
        .map(|item| map_item_summary(item, &url, scope))
        .collect::<Vec<_>>();

    Ok(ZoteroSearchResult {
        has_more: u32::try_from(items.len()).unwrap_or(0) == request.limit,
        total_available: None,
        items,
    })
}

fn zotero_request(
    http: &HttpClient,
    config: ZoteroConfig<'_>,
    url: &str,
) -> reqwest::RequestBuilder {
    http.client()
        .get(url)
        .header("Zotero-API-Key", config.api_key)
        .header("Zotero-API-Version", "3")
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

    let digits = raw.chars().filter(char::is_ascii_digit).collect::<String>();

    if digits.len() < 4 {
        return None;
    }

    Some(digits.chars().take(4).collect())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
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
