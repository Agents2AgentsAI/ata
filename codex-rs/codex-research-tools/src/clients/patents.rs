use crate::error::ResearchError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::types::PatentDetail;
use crate::types::PatentGetParams;
use crate::types::PatentItem;
use crate::types::PatentSearchParams;
use crate::types::PatentSearchResult;

// ---------------------------------------------------------------------------
// SerpApi Google Patents response types (private)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct SerpSearchResponse {
    #[serde(default)]
    organic_results: Vec<SerpOrganicResult>,
    search_information: Option<SerpSearchInfo>,
    serpapi_pagination: Option<SerpPagination>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpSearchInfo {
    total_results: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpPagination {
    next: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpOrganicResult {
    patent_id: Option<String>,
    title: Option<String>,
    snippet: Option<String>,
    publication_number: Option<String>,
    grant_date: Option<String>,
    publication_date: Option<String>,
    inventor: Option<String>,
    assignee: Option<String>,
    patent_link: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpDetailResponse {
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    publication_number: Option<String>,
    priority_date: Option<String>,
    filing_date: Option<String>,
    publication_date: Option<String>,
    #[serde(default)]
    claims: Option<Vec<String>>,
    #[serde(default)]
    inventors: Option<Vec<SerpInventor>>,
    #[serde(default)]
    assignees: Option<Vec<String>>,
    #[serde(default)]
    classifications: Option<Vec<SerpClassification>>,
    #[serde(default)]
    patent_citations: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    cited_by: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpInventor {
    name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SerpClassification {
    code: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub(crate) async fn search(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    params: &PatentSearchParams,
) -> Result<PatentSearchResult> {
    let api_key = api_key.ok_or_else(|| {
        ResearchError::Internal(
            "SERPAPI_API_KEY is not set. Get a key at https://serpapi.com".to_string(),
        )
    })?;

    let num = params.size.unwrap_or(25).clamp(10, 100);

    // SerpApi Google Patents uses dedicated URL params for filters.
    let mut url = format!(
        "{base_url}/search.json?engine=google_patents&q={query}&num={num}",
        query = urlencoding::encode(&params.query),
    );

    if let Some(inventor) = &params.inventor {
        url.push_str(&format!("&inventor={}", urlencoding::encode(inventor)));
    }
    if let Some(assignee) = &params.assignee {
        url.push_str(&format!("&assignee={}", urlencoding::encode(assignee)));
    }
    if let Some(cpc) = &params.cpc_code {
        // CPC codes go in the query with semicolon syntax.
        url.push_str(&format!(";({})", urlencoding::encode(cpc)));
    }

    if let Some("date") = params.sort_by.as_deref() {
        url.push_str("&sort=new");
    }

    // Date filters: after/before with publication:YYYYMMDD format.
    if let Some(date_from) = &params.date_from {
        let cleaned = date_from.replace('-', "");
        url.push_str(&format!(
            "&after=publication:{}",
            urlencoding::encode(&cleaned)
        ));
    }
    if let Some(date_to) = &params.date_to {
        let cleaned = date_to.replace('-', "");
        url.push_str(&format!(
            "&before=publication:{}",
            urlencoding::encode(&cleaned)
        ));
    }

    // Pagination via page number (SerpApi uses 1-based pages).
    if let Some(after) = &params.after {
        url.push_str(&format!("&page={after}"));
    }

    url.push_str(&format!("&api_key={}", urlencoding::encode(api_key)));

    let response: SerpSearchResponse = http
        .execute_json(ResearchApi::Patents, || http.client().get(&url))
        .await?;

    let total_hits = response
        .search_information
        .as_ref()
        .and_then(|si| si.total_results)
        .unwrap_or(0);

    let has_more = response
        .serpapi_pagination
        .as_ref()
        .and_then(|p| p.next.as_ref())
        .is_some();

    // Derive next page number from pagination URL or current after param.
    let next_cursor = if has_more {
        let current_page: u32 = params
            .after
            .as_deref()
            .and_then(|a| a.parse().ok())
            .unwrap_or(1);
        Some((current_page + 1).to_string())
    } else {
        None
    };

    let patents = response
        .organic_results
        .into_iter()
        .map(serp_result_to_item)
        .collect();

    Ok(PatentSearchResult {
        patents,
        total_hits,
        has_more,
        next_cursor,
    })
}

pub(crate) async fn get_patent(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    params: &PatentGetParams,
) -> Result<PatentDetail> {
    let api_key = api_key.ok_or_else(|| {
        ResearchError::Internal(
            "SERPAPI_API_KEY is not set. Get a key at https://serpapi.com".to_string(),
        )
    })?;

    // SerpApi expects patent_id in the format "patent/US11234567B2/en".
    // If the user provides just a number or publication number, wrap it.
    let patent_id_param = if params.patent_id.starts_with("patent/") {
        params.patent_id.clone()
    } else {
        // Try common formats: bare number -> US patent
        let id = &params.patent_id;
        if id.starts_with("US") {
            format!("patent/{id}/en")
        } else {
            format!("patent/US{id}/en")
        }
    };

    let url = format!(
        "{base_url}/search.json?engine=google_patents_details&patent_id={patent_id}&api_key={api_key}",
        patent_id = urlencoding::encode(&patent_id_param),
        api_key = urlencoding::encode(api_key),
    );

    let response: SerpDetailResponse = http
        .execute_json(ResearchApi::Patents, || http.client().get(&url))
        .await?;

    Ok(serp_detail_to_patent(response, &params.patent_id))
}

// ---------------------------------------------------------------------------
// Mappers
// ---------------------------------------------------------------------------

fn serp_result_to_item(r: SerpOrganicResult) -> PatentItem {
    let patent_id = r
        .publication_number
        .clone()
        .or(r.patent_id.clone())
        .unwrap_or_default();
    let url = r
        .patent_link
        .unwrap_or_else(|| format!("https://patents.google.com/patent/{patent_id}"));

    // Grant date falls back to publication date.
    let grant_date = r.grant_date.or(r.publication_date);

    PatentItem {
        patent_id,
        title: r.title.unwrap_or_default(),
        abstract_text: r.snippet,
        grant_date,
        inventors: split_csv(r.inventor.as_deref()),
        assignees: split_csv(r.assignee.as_deref()),
        cpc_codes: Vec::new(), // search results don't include CPC
        citation_count: None,
        url,
    }
}

fn serp_detail_to_patent(d: SerpDetailResponse, original_id: &str) -> PatentDetail {
    let patent_id = d
        .publication_number
        .clone()
        .unwrap_or_else(|| original_id.to_string());
    let url = format!("https://patents.google.com/patent/{patent_id}");

    let inventors = d
        .inventors
        .unwrap_or_default()
        .into_iter()
        .filter_map(|inv| inv.name)
        .collect();

    let assignees = d.assignees.unwrap_or_default();

    let cpc_codes = d
        .classifications
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.code)
        .collect();

    let claims_text = d.claims.map(|claims| claims.join("\n\n"));

    let cited_by_count = d
        .cited_by
        .as_ref()
        .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX));
    let citation_count = d
        .patent_citations
        .as_ref()
        .map(|v| u32::try_from(v.len()).unwrap_or(u32::MAX));

    PatentDetail {
        patent_id,
        title: d.title.unwrap_or_default(),
        abstract_text: d.abstract_text,
        grant_date: d.publication_date,
        filing_date: d.filing_date.or(d.priority_date),
        inventors,
        assignees,
        cpc_codes,
        claims_text,
        citation_count,
        cited_by_count,
        url,
    }
}

fn split_csv(value: Option<&str>) -> Vec<String> {
    value
        .map(|s| {
            s.split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default()
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
            patents_base_url: base_url.to_string(),
            patents_api_key: Some("test-key".to_string()),
            ..ResearchConfig::default()
        };
        let client = reqwest::Client::new();
        ResearchToolkit::new(client, config)
    }

    #[tokio::test]
    async fn search_parses_serpapi_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "search_information": {
                    "total_results": 1500
                },
                "organic_results": [
                    {
                        "patent_id": "patent/US11234567B2/en",
                        "title": "Test Patent Title",
                        "snippet": "A test abstract.",
                        "publication_number": "US11234567B2",
                        "grant_date": "2023-06-15",
                        "filing_date": "2021-01-10",
                        "inventor": "John Doe, Jane Smith",
                        "assignee": "Acme Corp",
                        "patent_link": "https://patents.google.com/patent/US11234567B2/en"
                    }
                ],
                "serpapi_pagination": {
                    "current": 1,
                    "next": "https://serpapi.com/search.json?page=2"
                }
            })))
            .mount(&server)
            .await;

        let toolkit = test_toolkit(&server.uri());
        let params = PatentSearchParams {
            query: "machine learning".to_string(),
            inventor: None,
            assignee: None,
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        let result = search(toolkit.http(), &server.uri(), Some("test-key"), &params)
            .await
            .expect("search should succeed");

        assert_eq!(result.total_hits, 1500);
        assert_eq!(result.has_more, true);
        assert_eq!(result.next_cursor, Some("2".to_string()));
        assert_eq!(result.patents.len(), 1);
        assert_eq!(result.patents[0].patent_id, "US11234567B2");
        assert_eq!(result.patents[0].title, "Test Patent Title");
        assert_eq!(result.patents[0].inventors, vec!["John Doe", "Jane Smith"]);
        assert_eq!(result.patents[0].assignees, vec!["Acme Corp"]);
        assert_eq!(result.patents[0].grant_date, Some("2023-06-15".to_string()));
    }

    #[tokio::test]
    async fn get_patent_parses_detail_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": "Detailed Patent",
                "abstract": "A detailed abstract.",
                "publication_number": "US11234567B2",
                "publication_date": "2023-06-15",
                "filing_date": "2021-01-10",
                "priority_date": "2020-12-01",
                "inventors": [
                    { "name": "Jane Smith" }
                ],
                "assignees": ["BigCo"],
                "classifications": [
                    { "code": "G06N3/08", "description": "Learning methods" }
                ],
                "claims": ["Claim 1 text.", "Claim 2 text."],
                "patent_citations": [{"patent_id": "US1"}, {"patent_id": "US2"}],
                "cited_by": [{"patent_id": "US3"}]
            })))
            .mount(&server)
            .await;

        let toolkit = test_toolkit(&server.uri());
        let params = PatentGetParams {
            patent_id: "US11234567B2".to_string(),
        };

        let result = get_patent(toolkit.http(), &server.uri(), Some("test-key"), &params)
            .await
            .expect("get_patent should succeed");

        assert_eq!(result.patent_id, "US11234567B2");
        assert_eq!(result.title, "Detailed Patent");
        assert_eq!(
            result.abstract_text,
            Some("A detailed abstract.".to_string())
        );
        assert_eq!(result.filing_date, Some("2021-01-10".to_string()));
        assert_eq!(result.inventors, vec!["Jane Smith"]);
        assert_eq!(result.assignees, vec!["BigCo"]);
        assert_eq!(result.cpc_codes, vec!["G06N3/08"]);
        assert_eq!(
            result.claims_text,
            Some("Claim 1 text.\n\nClaim 2 text.".to_string())
        );
        assert_eq!(result.citation_count, Some(2));
        assert_eq!(result.cited_by_count, Some(1));
    }

    #[tokio::test]
    async fn search_without_api_key_returns_error() {
        let params = PatentSearchParams {
            query: "test".to_string(),
            inventor: None,
            assignee: None,
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        let config = ResearchConfig::default();
        let client = reqwest::Client::new();
        let toolkit = ResearchToolkit::new(client, config);

        let err = search(toolkit.http(), "http://unused", None, &params)
            .await
            .expect_err("should fail without API key");

        let msg = err.to_string();
        assert!(msg.contains("SERPAPI_API_KEY"), "unexpected error: {msg}");
    }
}
