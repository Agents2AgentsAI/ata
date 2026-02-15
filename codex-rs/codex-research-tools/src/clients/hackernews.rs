use chrono::DateTime;
use chrono::Utc;

use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::types::HnComment;
use crate::types::HnGetThreadParams;
use crate::types::HnItem;
use crate::types::HnSearchParams;
use crate::types::HnSearchResult;
use crate::types::HnThread;

// ---------------------------------------------------------------------------
// Algolia API response types (private)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlgoliaResponse {
    hits: Vec<AlgoliaHit>,
    nb_hits: u64,
    #[serde(default)]
    nb_pages: u64,
    #[serde(default)]
    page: u64,
}

#[derive(Debug, serde::Deserialize)]
struct AlgoliaHit {
    #[serde(rename = "objectID")]
    object_id: String,
    title: Option<String>,
    url: Option<String>,
    author: Option<String>,
    points: Option<u32>,
    num_comments: Option<u32>,
    created_at: Option<String>,
    #[serde(rename = "story_text")]
    story_text: Option<String>,
    comment_text: Option<String>,
    story_title: Option<String>,
    story_url: Option<String>,
    #[serde(rename = "_tags")]
    tags: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct AlgoliaItem {
    id: u64,
    title: Option<String>,
    url: Option<String>,
    author: Option<String>,
    points: Option<u32>,
    created_at: Option<String>,
    text: Option<String>,
    #[serde(rename = "type")]
    item_type: Option<String>,
    #[serde(default)]
    children: Vec<AlgoliaItem>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub(crate) async fn search(
    http: &HttpClient,
    base_url: &str,
    params: &HnSearchParams,
) -> Result<HnSearchResult> {
    let endpoint = if params.sort_by.as_deref() == Some("date") {
        "search_by_date"
    } else {
        "search"
    };

    let mut url = format!(
        "{base_url}/{endpoint}?query={}&hitsPerPage={}&page={}",
        urlencoding::encode(&params.query),
        params.limit.unwrap_or(20),
        params.offset.unwrap_or(0),
    );

    // Build tags filter.
    let mut tags = Vec::new();
    match params.content_type.as_deref() {
        Some("story") => tags.push("story".to_string()),
        Some("comment") => tags.push("comment".to_string()),
        Some("show_hn") => tags.push("show_hn".to_string()),
        Some("ask_hn") => tags.push("ask_hn".to_string()),
        _ => {}
    }
    if let Some(author) = &params.author {
        tags.push(format!("author_{author}"));
    }
    if let Some(story_id) = params.story_id {
        tags.push(format!("story_{story_id}"));
    }
    if !tags.is_empty() {
        let tags_param = tags.join(",");
        url.push_str(&format!("&tags={}", urlencoding::encode(&tags_param)));
    }

    // Build numeric filters.
    let mut numeric_filters = Vec::new();
    if let Some(min_points) = params.min_points {
        numeric_filters.push(format!("points>={min_points}"));
    }
    if let Some(min_comments) = params.min_comments {
        numeric_filters.push(format!("num_comments>={min_comments}"));
    }
    if let Some(date_from) = params.date_from {
        let ts = date_from
            .and_hms_opt(0, 0, 0)
            .map(|dt| dt.and_utc().timestamp())
            .unwrap_or(0);
        numeric_filters.push(format!("created_at_i>={ts}"));
    }
    if let Some(date_to) = params.date_to {
        let ts = date_to
            .and_hms_opt(23, 59, 59)
            .map(|dt| dt.and_utc().timestamp())
            .unwrap_or(0);
        numeric_filters.push(format!("created_at_i<={ts}"));
    }
    if !numeric_filters.is_empty() {
        let nf = numeric_filters.join(",");
        url.push_str(&format!("&numericFilters={}", urlencoding::encode(&nf)));
    }

    let response: AlgoliaResponse = http
        .execute_json(ResearchApi::HackerNews, || http.client().get(&url))
        .await?;

    let items = response.hits.into_iter().map(algolia_hit_to_item).collect();
    let total_pages = response.nb_pages;
    let current_page = response.page;

    Ok(HnSearchResult {
        items,
        total_hits: response.nb_hits,
        has_more: current_page + 1 < total_pages,
    })
}

pub(crate) async fn get_item(
    http: &HttpClient,
    base_url: &str,
    params: &HnGetThreadParams,
) -> Result<HnThread> {
    let url = format!("{base_url}/items/{}", params.item_id);

    let item: AlgoliaItem = http
        .execute_json(ResearchApi::HackerNews, || http.client().get(&url))
        .await?;

    let story = algolia_item_to_hn_item(&item);
    let max_depth = params.max_depth.unwrap_or(5).clamp(1, 20);
    let max_comments = params.max_comments.unwrap_or(200).clamp(1, 500);

    let mut comment_count: u32 = 0;
    let comments = build_comment_tree(
        &item.children,
        1,
        max_depth,
        max_comments,
        &mut comment_count,
    );

    Ok(HnThread {
        story,
        comments,
        total_comments: comment_count,
    })
}

// ---------------------------------------------------------------------------
// Mappers
// ---------------------------------------------------------------------------

fn algolia_hit_to_item(hit: AlgoliaHit) -> HnItem {
    let id = hit.object_id.parse::<u64>().unwrap_or(0);
    let item_type = hit
        .tags
        .as_ref()
        .and_then(|tags| {
            if tags.iter().any(|t| t == "story") {
                Some("story")
            } else if tags.iter().any(|t| t == "comment") {
                Some("comment")
            } else {
                None
            }
        })
        .unwrap_or("story")
        .to_string();

    let text = hit.comment_text.or(hit.story_text);
    let created_at = hit
        .created_at
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());

    HnItem {
        id,
        title: hit.title,
        url: hit.url,
        author: hit.author,
        points: hit.points,
        num_comments: hit.num_comments,
        created_at,
        item_type,
        text,
        story_title: hit.story_title,
        story_url: hit.story_url,
        hn_url: format!("https://news.ycombinator.com/item?id={id}"),
    }
}

fn algolia_item_to_hn_item(item: &AlgoliaItem) -> HnItem {
    let created_at = item
        .created_at
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());

    HnItem {
        id: item.id,
        title: item.title.clone(),
        url: item.url.clone(),
        author: item.author.clone(),
        points: item.points,
        num_comments: None,
        created_at,
        item_type: item
            .item_type
            .clone()
            .unwrap_or_else(|| "story".to_string()),
        text: item.text.clone(),
        story_title: None,
        story_url: None,
        hn_url: format!("https://news.ycombinator.com/item?id={}", item.id),
    }
}

fn build_comment_tree(
    children: &[AlgoliaItem],
    depth: u32,
    max_depth: u32,
    max_comments: u32,
    count: &mut u32,
) -> Vec<HnComment> {
    if depth > max_depth {
        return Vec::new();
    }

    let mut comments = Vec::new();
    for child in children {
        if *count >= max_comments {
            break;
        }

        let created_at = child
            .created_at
            .as_deref()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok());

        *count += 1;
        let nested = build_comment_tree(&child.children, depth + 1, max_depth, max_comments, count);

        comments.push(HnComment {
            id: child.id,
            author: child.author.clone(),
            text: child.text.clone(),
            created_at,
            children: nested,
        });
    }
    comments
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    use super::*;
    use crate::ResearchToolkit;
    use crate::config::ResearchConfig;

    fn test_toolkit(base_url: &str) -> ResearchToolkit {
        let config = ResearchConfig {
            hn_base_url: base_url.to_string(),
            ..ResearchConfig::default()
        };
        let client = reqwest::Client::new();
        ResearchToolkit::new(client, config)
    }

    #[tokio::test]
    async fn search_parses_algolia_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": [
                    {
                        "objectID": "12345",
                        "title": "Show HN: My Project",
                        "url": "https://example.com",
                        "author": "testuser",
                        "points": 42,
                        "num_comments": 10,
                        "created_at": "2024-01-15T10:30:00.000Z",
                        "story_text": null,
                        "comment_text": null,
                        "story_title": null,
                        "story_url": null,
                        "_tags": ["story", "author_testuser", "story_12345"]
                    }
                ],
                "nbHits": 1,
                "page": 0,
                "nbPages": 1,
                "hitsPerPage": 20
            })))
            .mount(&server)
            .await;

        let toolkit = test_toolkit(&server.uri());
        let params = HnSearchParams {
            query: "my project".to_string(),
            content_type: None,
            sort_by: None,
            min_points: None,
            min_comments: None,
            date_from: None,
            date_to: None,
            author: None,
            story_id: None,
            offset: None,
            limit: None,
        };

        let result = search(toolkit.http(), &server.uri(), &params)
            .await
            .expect("search should succeed");
        assert_eq!(result.total_hits, 1);
        assert_eq!(result.has_more, false);
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].id, 12345);
        assert_eq!(
            result.items[0].title.as_deref(),
            Some("Show HN: My Project")
        );
        assert_eq!(result.items[0].points, Some(42));
        assert_eq!(result.items[0].item_type, "story");
    }

    #[tokio::test]
    async fn get_item_parses_nested_comments() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/items/100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 100,
                "title": "Test Story",
                "url": "https://example.com",
                "author": "author1",
                "points": 50,
                "created_at": "2024-01-15T10:30:00.000Z",
                "text": null,
                "type": "story",
                "children": [
                    {
                        "id": 101,
                        "author": "commenter1",
                        "text": "Great article!",
                        "created_at": "2024-01-15T11:00:00.000Z",
                        "type": "comment",
                        "children": [
                            {
                                "id": 102,
                                "author": "commenter2",
                                "text": "I agree!",
                                "created_at": "2024-01-15T11:30:00.000Z",
                                "type": "comment",
                                "children": []
                            }
                        ]
                    }
                ]
            })))
            .mount(&server)
            .await;

        let toolkit = test_toolkit(&server.uri());
        let params = HnGetThreadParams {
            item_id: 100,
            max_depth: Some(5),
            max_comments: Some(200),
        };

        let result = get_item(toolkit.http(), &server.uri(), &params)
            .await
            .expect("get_item should succeed");
        assert_eq!(result.story.id, 100);
        assert_eq!(result.story.title.as_deref(), Some("Test Story"));
        assert_eq!(result.comments.len(), 1);
        assert_eq!(result.comments[0].id, 101);
        assert_eq!(result.comments[0].text.as_deref(), Some("Great article!"));
        assert_eq!(result.comments[0].children.len(), 1);
        assert_eq!(result.comments[0].children[0].id, 102);
        assert_eq!(result.total_comments, 2);
    }

    #[tokio::test]
    async fn get_item_respects_max_depth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/items/200"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 200,
                "title": "Deep Thread",
                "author": "author1",
                "points": 10,
                "created_at": "2024-01-15T10:30:00.000Z",
                "type": "story",
                "children": [{
                    "id": 201, "author": "a", "text": "L1",
                    "created_at": "2024-01-15T11:00:00.000Z", "type": "comment",
                    "children": [{
                        "id": 202, "author": "b", "text": "L2",
                        "created_at": "2024-01-15T12:00:00.000Z", "type": "comment",
                        "children": [{
                            "id": 203, "author": "c", "text": "L3",
                            "created_at": "2024-01-15T13:00:00.000Z", "type": "comment",
                            "children": []
                        }]
                    }]
                }]
            })))
            .mount(&server)
            .await;

        let toolkit = test_toolkit(&server.uri());
        let params = HnGetThreadParams {
            item_id: 200,
            max_depth: Some(2),
            max_comments: Some(200),
        };

        let result = get_item(toolkit.http(), &server.uri(), &params)
            .await
            .expect("get_item should succeed");
        assert_eq!(result.comments.len(), 1);
        assert_eq!(result.comments[0].children.len(), 1);
        // Depth 3 should be cut off (max_depth=2)
        assert_eq!(result.comments[0].children[0].children.len(), 0);
        assert_eq!(result.total_comments, 2);
    }
}
