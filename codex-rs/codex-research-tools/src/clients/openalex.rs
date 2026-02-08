use std::collections::BTreeMap;
use std::collections::HashMap;

use chrono::Utc;
use serde::Deserialize;

use crate::clients::SearchPage;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::paper_id::PaperIdResolver;
use crate::rate_limiter::ResearchApi;
use crate::types::Paper;
use crate::types::SourceMeta;

pub(crate) async fn search(
    http: &HttpClient,
    base_url: &str,
    request: &OpenAlexSearchRequest<'_>,
) -> Result<SearchPage> {
    let per_page = request.limit.clamp(1, 200);
    let page = request.offset / per_page + 1;
    let page_offset = request.offset % per_page;

    let mut url = format!(
        "{base_url}/works?search={query}&per-page={per_page}&page={page}",
        query = urlencoding::encode(request.query),
    );

    let mut filters = Vec::new();
    if let Some(year_from) = request.year_from {
        filters.push(format!("from_publication_date:{year_from}-01-01"));
    }
    if let Some(year_to) = request.year_to {
        filters.push(format!("to_publication_date:{year_to}-12-31"));
    }
    if !filters.is_empty() {
        url.push_str(&format!(
            "&filter={}",
            urlencoding::encode(&filters.join(","))
        ));
    }

    if let Some(polite_email) = request.polite_email {
        url.push_str(&format!("&mailto={}", urlencoding::encode(polite_email)));
    }

    let response: OpenAlexResponse = http
        .execute_json(ResearchApi::OpenAlex, || http.client().get(&url))
        .await?;

    let papers = response
        .results
        .into_iter()
        .skip(page_offset as usize)
        .map(|work| map_work(work, &url, request.include_abstract))
        .collect::<Vec<_>>();

    let has_more = response.meta.count.is_some_and(|total| {
        total > u64::from(request.offset) + u64::try_from(papers.len()).unwrap_or(0)
    });

    Ok(SearchPage {
        papers,
        total_available: response.meta.count,
        has_more,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenAlexSearchRequest<'a> {
    pub polite_email: Option<&'a str>,
    pub query: &'a str,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub offset: u32,
    pub limit: u32,
    pub include_abstract: bool,
}

fn map_work(work: OpenAlexWork, api_url: &str, include_abstract: bool) -> Paper {
    let openalex_id = work
        .ids
        .as_ref()
        .and_then(|ids| ids.openalex.as_deref())
        .and_then(extract_openalex_id)
        .or_else(|| extract_openalex_id(&work.id));

    let doi = work
        .doi
        .clone()
        .or_else(|| work.ids.as_ref().and_then(|ids| ids.doi.clone()));

    let arxiv_id = work
        .ids
        .as_ref()
        .and_then(|ids| ids.arxiv.as_deref())
        .and_then(extract_arxiv_id);

    let authors = work
        .authorships
        .as_ref()
        .map(|authorships| {
            authorships
                .iter()
                .filter_map(|authorship| {
                    authorship
                        .author
                        .as_ref()
                        .and_then(|author| author.display_name.clone())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");

    let (url, pdf_url, venue) = extract_location(&work);

    let abstract_text = if include_abstract {
        work.abstract_inverted_index
            .as_ref()
            .and_then(reconstruct_abstract)
    } else {
        None
    };

    let canonical_id = PaperIdResolver::canonical_from_fields(
        doi.as_deref(),
        arxiv_id.as_deref(),
        None,
        openalex_id.as_deref(),
    )
    .map(|paper_id| paper_id.to_string());

    Paper {
        title: work.display_name,
        authors,
        year: work.publication_year,
        venue,
        citation_count: work.cited_by_count,
        abstract_text,
        doi,
        arxiv_id,
        s2_paper_id: None,
        openalex_id,
        url,
        pdf_url,
        code_url: None,
        source_meta: Some(SourceMeta {
            source: "openalex".to_string(),
            api_url: api_url.to_string(),
            fetched_at: Utc::now(),
            canonical_id,
        }),
    }
}

fn extract_location(work: &OpenAlexWork) -> (Option<String>, Option<String>, Option<String>) {
    let primary = work.primary_location.as_ref();
    let best_oa = work.best_oa_location.as_ref();

    let url = primary
        .and_then(|location| location.landing_page_url.clone())
        .or_else(|| best_oa.and_then(|location| location.landing_page_url.clone()))
        .or_else(|| Some(work.id.clone()));

    let pdf_url = best_oa
        .and_then(|location| location.pdf_url.clone())
        .or_else(|| primary.and_then(|location| location.pdf_url.clone()));

    let venue = primary
        .and_then(|location| location.source.as_ref())
        .and_then(|source| source.display_name.clone())
        .or_else(|| {
            best_oa
                .and_then(|location| location.source.as_ref())
                .and_then(|source| source.display_name.clone())
        });

    (url, pdf_url, venue)
}

fn extract_openalex_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let id = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if id.starts_with('W') {
        Some(id.to_string())
    } else {
        None
    }
}

fn extract_arxiv_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn reconstruct_abstract(index: &HashMap<String, Vec<usize>>) -> Option<String> {
    let mut words_by_position = BTreeMap::new();
    for (word, positions) in index {
        for &position in positions {
            words_by_position.insert(position, word.as_str());
        }
    }

    if words_by_position.is_empty() {
        return None;
    }

    let abstract_text = words_by_position
        .into_values()
        .collect::<Vec<_>>()
        .join(" ");

    if abstract_text.is_empty() {
        None
    } else {
        Some(abstract_text)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;

    use super::reconstruct_abstract;

    #[test]
    fn reconstruct_abstract_orders_words_by_position() {
        let mut index = HashMap::new();
        index.insert("world".to_string(), vec![1]);
        index.insert("hello".to_string(), vec![0]);

        assert_eq!(
            reconstruct_abstract(&index),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn reconstruct_abstract_handles_sparse_large_positions_without_dense_buffer() {
        let mut index = HashMap::new();
        index.insert("sparse".to_string(), vec![999_999]);
        index.insert("index".to_string(), vec![1_000_000]);

        assert_eq!(
            reconstruct_abstract(&index),
            Some("sparse index".to_string())
        );
    }
}

#[derive(Debug, Deserialize)]
struct OpenAlexResponse {
    meta: OpenAlexMeta,
    results: Vec<OpenAlexWork>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexMeta {
    count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexWork {
    id: String,
    #[serde(rename = "display_name")]
    display_name: String,
    publication_year: Option<u32>,
    cited_by_count: Option<u32>,
    doi: Option<String>,
    ids: Option<OpenAlexIds>,
    authorships: Option<Vec<OpenAlexAuthorship>>,
    abstract_inverted_index: Option<HashMap<String, Vec<usize>>>,
    primary_location: Option<OpenAlexLocation>,
    best_oa_location: Option<OpenAlexLocation>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexIds {
    openalex: Option<String>,
    doi: Option<String>,
    arxiv: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexAuthorship {
    author: Option<OpenAlexAuthor>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexAuthor {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexLocation {
    landing_page_url: Option<String>,
    pdf_url: Option<String>,
    source: Option<OpenAlexSource>,
}

#[derive(Debug, Deserialize)]
struct OpenAlexSource {
    display_name: Option<String>,
}
