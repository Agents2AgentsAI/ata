use serde::Serialize;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::clients::SearchPage;
use crate::clients::arxiv;
use crate::clients::arxiv::ArxivSearchRequest;
use crate::clients::openalex;
use crate::clients::openalex::OpenAlexSearchRequest;
use crate::clients::semantic_scholar;
use crate::clients::semantic_scholar::PaperRelation;
use crate::clients::semantic_scholar::SemanticScholarConfig;
use crate::clients::semantic_scholar::SemanticScholarRelationRequest;
use crate::clients::semantic_scholar::SemanticScholarSearchRequest;
use crate::error::ResearchError;
use crate::error::Result;
use crate::paper_id::PaperId;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::CitationResult;
use crate::types::PaginationParams;
use crate::types::Paper;
use crate::types::PaperDetail;
use crate::types::PaperSearchParams;
use crate::types::SearchResult;
use crate::types::SourceResult;
use crate::types::SourceStatus;

const SOURCE_SEMANTIC_SCHOLAR: &str = "semantic_scholar";
const SOURCE_ARXIV: &str = "arxiv";
const SOURCE_OPENALEX: &str = "openalex";
const MAX_MULTI_SOURCE_FETCH_LIMIT: u32 = 200;

#[derive(Debug, Clone, Serialize)]
struct NormalizedPaperSearchParams {
    query: String,
    year_from: Option<u32>,
    year_to: Option<u32>,
    fields_of_study: Option<Vec<String>>,
    source: Option<String>,
    sort_by: PaperSortBy,
    offset: u32,
    limit: u32,
    include_abstract: bool,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PaperSortBy {
    Relevance,
    CitationCount,
    Year,
}

#[derive(Debug, Clone, Serialize)]
struct NormalizedPaginationParams {
    offset: u32,
    limit: u32,
    include_abstract: bool,
    max_chars_per_item: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedSources {
    semantic_scholar: bool,
    arxiv: bool,
    openalex: bool,
}

#[derive(Debug, Default)]
struct AggregationState {
    all_papers: Vec<Paper>,
    per_source_status: Vec<SourceStatus>,
    warnings: Vec<String>,
    total_available: u64,
    has_more: bool,
    any_success: bool,
}

pub(crate) async fn paper_search(
    toolkit: &ResearchToolkit,
    params: PaperSearchParams,
) -> Result<SearchResult> {
    let normalized = normalize_paper_search_params(params)?;
    let selected_sources = resolve_sources(normalized.source.as_deref())?;
    let (source_fetch_params, pagination_warning) =
        source_fetch_params(&normalized, selected_sources);

    let semantic_future = async {
        if selected_sources.semantic_scholar {
            Some(search_semantic_scholar(toolkit, &source_fetch_params).await)
        } else {
            None
        }
    };

    let arxiv_future = async {
        if selected_sources.arxiv {
            Some(search_arxiv(toolkit, &source_fetch_params).await)
        } else {
            None
        }
    };

    let openalex_future = async {
        if selected_sources.openalex {
            Some(search_openalex(toolkit, &source_fetch_params).await)
        } else {
            None
        }
    };

    let (semantic_result, arxiv_result, openalex_result) =
        tokio::join!(semantic_future, arxiv_future, openalex_future);

    let mut aggregation = AggregationState::default();
    if let Some(warning) = pagination_warning {
        aggregation.warnings.push(warning);
    }
    aggregation.record_source_result(SOURCE_SEMANTIC_SCHOLAR, semantic_result);
    aggregation.record_source_result(SOURCE_ARXIV, arxiv_result);
    aggregation.record_source_result(SOURCE_OPENALEX, openalex_result);

    let mut deduped = toolkit.paper_ids().dedup_papers(aggregation.all_papers);
    sort_papers(&mut deduped, normalized.sort_by);

    let offset = if selected_sources.enabled_count() <= 1 {
        0
    } else {
        normalized.offset as usize
    };
    let limit = normalized.limit as usize;
    let total_deduped = deduped.len();
    let mut papers = deduped
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();

    apply_output_budget(
        &mut papers,
        normalized.include_abstract,
        normalized.max_chars_per_item,
    );

    if offset + papers.len() < total_deduped {
        aggregation.has_more = true;
    }

    Ok(SearchResult {
        papers,
        per_source_status: aggregation.per_source_status,
        warnings: aggregation.warnings,
        total_available: if aggregation.any_success {
            Some(aggregation.total_available)
        } else {
            None
        },
        has_more: aggregation.has_more,
    })
}

pub(crate) async fn paper_get(toolkit: &ResearchToolkit, paper_id: &str) -> Result<PaperDetail> {
    let include_abstract = true;
    let lookup_id = to_semantic_scholar_id(paper_id)?;
    let params_hash = hash_cache_payload(&("paper_get", &lookup_id, include_abstract))?;
    let key = CacheKey {
        tool_name: "paper_get",
        params_hash,
    };

    let paper: Paper = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.paper_search,
        || async {
            semantic_scholar::get_paper(
                toolkit.http(),
                SemanticScholarConfig {
                    base_url: &toolkit.config().semantic_scholar_base_url,
                    api_key: toolkit.config().semantic_scholar_api_key.as_deref(),
                },
                &lookup_id,
                include_abstract,
            )
            .await
        },
    )
    .await?;

    let references_page = relation_search(
        toolkit,
        &lookup_id,
        PaperRelation::References,
        NormalizedPaginationParams {
            offset: 0,
            limit: 5,
            include_abstract: false,
            max_chars_per_item: None,
        },
    )
    .await?;

    Ok(PaperDetail {
        paper,
        references: references_page.papers,
    })
}

pub(crate) async fn paper_citations(
    toolkit: &ResearchToolkit,
    paper_id: &str,
    params: PaginationParams,
) -> Result<CitationResult> {
    let lookup_id = to_semantic_scholar_id(paper_id)?;
    let normalized = normalize_pagination(params, 50);
    let page = relation_search(toolkit, &lookup_id, PaperRelation::Citations, normalized).await?;

    Ok(CitationResult {
        papers: page.papers,
        total_available: page.total_available,
        has_more: page.has_more,
    })
}

pub(crate) async fn paper_references(
    toolkit: &ResearchToolkit,
    paper_id: &str,
    params: PaginationParams,
) -> Result<CitationResult> {
    let lookup_id = to_semantic_scholar_id(paper_id)?;
    let normalized = normalize_pagination(params, 50);
    let page = relation_search(toolkit, &lookup_id, PaperRelation::References, normalized).await?;

    Ok(CitationResult {
        papers: page.papers,
        total_available: page.total_available,
        has_more: page.has_more,
    })
}

fn normalize_paper_search_params(params: PaperSearchParams) -> Result<NormalizedPaperSearchParams> {
    let query = params.query.trim().to_string();
    if query.is_empty() {
        return Err(ResearchError::InvalidInput(
            "paper_search query must not be empty".to_string(),
        ));
    }

    if let (Some(from), Some(to)) = (params.year_from, params.year_to)
        && from > to
    {
        return Err(ResearchError::InvalidInput(format!(
            "year_from ({from}) cannot be greater than year_to ({to})"
        )));
    }

    let include_abstract = should_include_abstract(params.include_abstract, params.fields.as_ref());

    Ok(NormalizedPaperSearchParams {
        query,
        year_from: params.year_from,
        year_to: params.year_to,
        fields_of_study: params.fields_of_study,
        source: params.source.map(|source| source.to_ascii_lowercase()),
        sort_by: normalize_sort_by(params.sort_by)?,
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(20).clamp(1, 50),
        include_abstract,
        max_chars_per_item: params.max_chars_per_item,
    })
}

fn normalize_sort_by(sort_by: Option<String>) -> Result<PaperSortBy> {
    let Some(raw) = sort_by else {
        return Ok(PaperSortBy::Relevance);
    };

    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "relevance" => Ok(PaperSortBy::Relevance),
        "citation_count" | "citations" => Ok(PaperSortBy::CitationCount),
        "year" | "publication_year" | "newest" => Ok(PaperSortBy::Year),
        other => Err(ResearchError::InvalidInput(format!(
            "unsupported sort_by '{other}' (expected one of: relevance, citation_count, year)"
        ))),
    }
}

fn sort_papers(papers: &mut [Paper], sort_by: PaperSortBy) {
    match sort_by {
        PaperSortBy::Relevance => {}
        PaperSortBy::CitationCount => {
            papers.sort_by(|left, right| {
                right
                    .citation_count
                    .unwrap_or(0)
                    .cmp(&left.citation_count.unwrap_or(0))
                    .then_with(|| right.year.unwrap_or(0).cmp(&left.year.unwrap_or(0)))
                    .then_with(|| left.title.cmp(&right.title))
            });
        }
        PaperSortBy::Year => {
            papers.sort_by(|left, right| {
                right
                    .year
                    .unwrap_or(0)
                    .cmp(&left.year.unwrap_or(0))
                    .then_with(|| {
                        right
                            .citation_count
                            .unwrap_or(0)
                            .cmp(&left.citation_count.unwrap_or(0))
                    })
                    .then_with(|| left.title.cmp(&right.title))
            });
        }
    }
}

fn normalize_pagination(
    params: PaginationParams,
    default_limit: u32,
) -> NormalizedPaginationParams {
    let include_abstract = should_include_abstract(Some(false), params.fields.as_ref());

    NormalizedPaginationParams {
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(default_limit).clamp(1, 100),
        include_abstract,
        max_chars_per_item: params.max_chars_per_item,
    }
}

fn should_include_abstract(include_abstract: Option<bool>, fields: Option<&Vec<String>>) -> bool {
    if include_abstract == Some(false) {
        return false;
    }

    if let Some(fields) = fields
        && !fields.is_empty()
    {
        return fields.iter().any(|field| {
            let lowered = field.to_ascii_lowercase();
            lowered == "abstract" || lowered == "abstract_text"
        });
    }

    true
}

fn resolve_sources(source: Option<&str>) -> Result<SelectedSources> {
    match source {
        None | Some("all") => Ok(SelectedSources {
            semantic_scholar: true,
            arxiv: true,
            openalex: true,
        }),
        Some(SOURCE_SEMANTIC_SCHOLAR) => Ok(SelectedSources {
            semantic_scholar: true,
            arxiv: false,
            openalex: false,
        }),
        Some(SOURCE_ARXIV) => Ok(SelectedSources {
            semantic_scholar: false,
            arxiv: true,
            openalex: false,
        }),
        Some(SOURCE_OPENALEX) => Ok(SelectedSources {
            semantic_scholar: false,
            arxiv: false,
            openalex: true,
        }),
        Some(other) => Err(ResearchError::InvalidInput(format!(
            "unknown source '{other}' (expected one of: all, semantic_scholar, arxiv, openalex)"
        ))),
    }
}

fn source_fetch_params(
    params: &NormalizedPaperSearchParams,
    selected_sources: SelectedSources,
) -> (NormalizedPaperSearchParams, Option<String>) {
    if selected_sources.enabled_count() <= 1 {
        return (params.clone(), None);
    }

    let requested_window = params.offset.saturating_add(params.limit);
    let effective_limit = requested_window.min(MAX_MULTI_SOURCE_FETCH_LIMIT);
    let mut normalized = params.clone();
    normalized.offset = 0;
    normalized.limit = effective_limit;

    let warning = if effective_limit < requested_window {
        Some(format!(
            "multi-source pagination window capped at {MAX_MULTI_SOURCE_FETCH_LIMIT}; large offsets may return incomplete results"
        ))
    } else {
        None
    };

    (normalized, warning)
}

impl SelectedSources {
    fn enabled_count(self) -> u32 {
        u32::from(self.semantic_scholar) + u32::from(self.arxiv) + u32::from(self.openalex)
    }
}

impl AggregationState {
    fn record_source_result(&mut self, source: &str, source_result: Option<Result<SearchPage>>) {
        let Some(source_result) = source_result else {
            return;
        };

        match source_result {
            Ok(page) => {
                self.any_success = true;
                self.total_available += page
                    .total_available
                    .unwrap_or_else(|| u64::try_from(page.papers.len()).unwrap_or(0));
                self.has_more |= page.has_more;
                self.per_source_status.push(SourceStatus {
                    source: source.to_string(),
                    status: SourceResult::Ok {
                        count: page.papers.len(),
                    },
                });
                self.all_papers.extend(page.papers);
            }
            Err(err) => {
                self.warnings.push(format!("{source} search failed: {err}"));
                self.per_source_status.push(SourceStatus {
                    source: source.to_string(),
                    status: SourceResult::Error {
                        message: err.to_string(),
                        retryable: is_retryable_error(&err),
                    },
                });
            }
        }
    }
}

fn is_retryable_error(err: &ResearchError) -> bool {
    err.is_retryable()
}

fn apply_output_budget(
    papers: &mut [Paper],
    include_abstract: bool,
    max_chars_per_item: Option<u32>,
) {
    let max_chars = usize::try_from(max_chars_per_item.unwrap_or(500)).unwrap_or(500);

    for paper in papers {
        if !include_abstract {
            paper.abstract_text = None;
            continue;
        }

        if let Some(abstract_text) = paper.abstract_text.as_ref()
            && abstract_text.chars().count() > max_chars
        {
            let truncated = abstract_text.chars().take(max_chars).collect::<String>();
            paper.abstract_text = Some(truncated);
        }
    }
}

fn to_semantic_scholar_id(id: &str) -> Result<String> {
    let parsed = id.parse::<PaperId>().map_err(ResearchError::InvalidInput)?;
    let lookup = match parsed {
        PaperId::Doi(doi) => format!("DOI:{doi}"),
        PaperId::ArxivId(arxiv_id) => format!("ARXIV:{arxiv_id}"),
        PaperId::S2Id(s2_id) => s2_id,
        PaperId::OpenAlexId(openalex_id) => {
            return Err(ResearchError::InvalidInput(format!(
                "openalex id '{openalex_id}' is not supported by Semantic Scholar lookup"
            )));
        }
    };

    Ok(lookup)
}

async fn search_semantic_scholar(
    toolkit: &ResearchToolkit,
    params: &NormalizedPaperSearchParams,
) -> Result<SearchPage> {
    let key = CacheKey {
        tool_name: "paper_search_semantic_scholar",
        params_hash: hash_cache_payload(params)?,
    };

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.paper_search,
        || async {
            semantic_scholar::search(
                toolkit.http(),
                SemanticScholarConfig {
                    base_url: &toolkit.config().semantic_scholar_base_url,
                    api_key: toolkit.config().semantic_scholar_api_key.as_deref(),
                },
                &SemanticScholarSearchRequest {
                    query: &params.query,
                    year_from: params.year_from,
                    year_to: params.year_to,
                    fields_of_study: params.fields_of_study.as_deref(),
                    offset: params.offset,
                    limit: params.limit,
                    include_abstract: params.include_abstract,
                },
            )
            .await
        },
    )
    .await
}

async fn search_arxiv(
    toolkit: &ResearchToolkit,
    params: &NormalizedPaperSearchParams,
) -> Result<SearchPage> {
    let key = CacheKey {
        tool_name: "paper_search_arxiv",
        params_hash: hash_cache_payload(params)?,
    };

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.paper_search,
        || async {
            arxiv::search(
                toolkit.http(),
                &toolkit.config().arxiv_base_url,
                &ArxivSearchRequest {
                    query: &params.query,
                    year_from: params.year_from,
                    year_to: params.year_to,
                    offset: params.offset,
                    limit: params.limit,
                    include_abstract: params.include_abstract,
                },
            )
            .await
        },
    )
    .await
}

async fn search_openalex(
    toolkit: &ResearchToolkit,
    params: &NormalizedPaperSearchParams,
) -> Result<SearchPage> {
    let key = CacheKey {
        tool_name: "paper_search_openalex",
        params_hash: hash_cache_payload(params)?,
    };

    get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.paper_search,
        || async {
            openalex::search(
                toolkit.http(),
                &toolkit.config().openalex_base_url,
                &OpenAlexSearchRequest {
                    polite_email: toolkit.config().openalex_email.as_deref(),
                    query: &params.query,
                    year_from: params.year_from,
                    year_to: params.year_to,
                    offset: params.offset,
                    limit: params.limit,
                    include_abstract: params.include_abstract,
                },
            )
            .await
        },
    )
    .await
}

async fn relation_search(
    toolkit: &ResearchToolkit,
    paper_id: &str,
    relation: PaperRelation,
    params: NormalizedPaginationParams,
) -> Result<SearchPage> {
    let (tool_name, relation_name) = match relation {
        PaperRelation::Citations => ("paper_citations", "citations"),
        PaperRelation::References => ("paper_references", "references"),
    };

    let key = CacheKey {
        tool_name,
        params_hash: hash_cache_payload(&(paper_id, relation_name, &params))?,
    };

    let mut page = get_or_fetch_typed(
        toolkit,
        key,
        toolkit.config().cache_ttls.citations,
        || async {
            semantic_scholar::get_relations(
                toolkit.http(),
                SemanticScholarConfig {
                    base_url: &toolkit.config().semantic_scholar_base_url,
                    api_key: toolkit.config().semantic_scholar_api_key.as_deref(),
                },
                &SemanticScholarRelationRequest {
                    paper_id,
                    relation,
                    offset: params.offset,
                    limit: params.limit,
                    include_abstract: params.include_abstract,
                },
            )
            .await
        },
    )
    .await?;

    apply_output_budget(
        &mut page.papers,
        params.include_abstract,
        params.max_chars_per_item,
    );

    Ok(page)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::query_param;

    use crate::ResearchToolkit;
    use crate::config::ResearchConfig;
    use crate::tools::test_helpers::build_test_toolkit_with_config;
    use crate::types::PaginationParams;
    use crate::types::PaperSearchParams;
    use crate::types::SourceResult;

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_aggregates_sources_and_reports_partial_failure() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/paper/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 2,
                "next": null,
                "data": [
                    {
                        "paperId": "s2-paper-1",
                        "title": "A Semantic Paper",
                        "abstract": "Semantic abstract",
                        "year": 2021,
                        "citationCount": 10,
                        "venue": "ICLR",
                        "url": "https://example.org/s2-1",
                        "externalIds": { "DOI": "10.1000/shared" },
                        "authors": [{"name": "Alice"}]
                    }
                ]
            })))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream failure"))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 2},
                "results": [
                    {
                        "id": "https://openalex.org/W123",
                        "display_name": "A Semantic Paper",
                        "publication_year": 2021,
                        "cited_by_count": 99,
                        "doi": "https://doi.org/10.1000/shared",
                        "ids": {
                            "openalex": "https://openalex.org/W123",
                            "doi": "https://doi.org/10.1000/shared"
                        },
                        "authorships": [{"author": {"display_name": "Alice"}}],
                        "primary_location": {
                            "landing_page_url": "https://example.org/oa-1",
                            "pdf_url": "https://example.org/oa-1.pdf",
                            "source": {"display_name": "NeurIPS"}
                        },
                        "best_oa_location": null
                    },
                    {
                        "id": "https://openalex.org/W456",
                        "display_name": "A Distinct OpenAlex Paper",
                        "publication_year": 2023,
                        "cited_by_count": 12,
                        "doi": "https://doi.org/10.1000/other",
                        "ids": {
                            "openalex": "https://openalex.org/W456",
                            "doi": "https://doi.org/10.1000/other"
                        },
                        "authorships": [{"author": {"display_name": "Bob"}}],
                        "primary_location": {
                            "landing_page_url": "https://example.org/oa-2",
                            "pdf_url": null,
                            "source": {"display_name": "CVPR"}
                        },
                        "best_oa_location": null
                    }
                ]
            })))
            .mount(&openalex_server)
            .await;

        let toolkit = build_test_toolkit(
            semantic_server.uri(),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let result = toolkit
            .paper_search(PaperSearchParams {
                query: "grasp detection".to_string(),
                year_from: None,
                year_to: None,
                fields_of_study: None,
                source: Some("all".to_string()),
                sort_by: None,
                offset: Some(0),
                limit: Some(20),
                include_abstract: Some(true),
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect("paper_search should succeed with partial results");

        assert_eq!(result.papers.len(), 2);
        assert_eq!(result.per_source_status.len(), 3);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("arxiv"));
        assert!(
            result
                .papers
                .iter()
                .any(|paper| paper.title == "A Semantic Paper")
        );
        assert!(
            result
                .papers
                .iter()
                .any(|paper| paper.title == "A Distinct OpenAlex Paper")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_multi_source_fetches_zero_based_window_for_global_pagination() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/paper/search"))
            .and(query_param("offset", "0"))
            .and(query_param("limit", "30"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 1,
                "next": null,
                "data": [{
                    "paperId": "s2-paper-1",
                    "title": "A Semantic Paper",
                    "year": 2021,
                    "citationCount": 10,
                    "venue": "ICLR",
                    "url": "https://example.org/s2-1",
                    "externalIds": { "DOI": "10.1000/shared" },
                    "authors": [{"name": "Alice"}]
                }]
            })))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/query"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <feed xmlns="http://www.w3.org/2005/Atom">
                  <entry>
                    <id>http://arxiv.org/abs/2301.12345v1</id>
                    <title>arxiv result</title>
                    <summary>summary</summary>
                    <published>2023-01-01T00:00:00Z</published>
                    <author><name>Alice</name></author>
                  </entry>
                </feed>"#,
                "application/atom+xml",
            ))
            .mount(&arxiv_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/works"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": {"count": 0},
                "results": []
            })))
            .mount(&openalex_server)
            .await;

        let toolkit = build_test_toolkit(
            semantic_server.uri(),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let result = toolkit
            .paper_search(PaperSearchParams {
                query: "grasp detection".to_string(),
                year_from: None,
                year_to: None,
                fields_of_study: None,
                source: Some("all".to_string()),
                sort_by: None,
                offset: Some(20),
                limit: Some(10),
                include_abstract: Some(false),
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect("paper_search should succeed");

        assert!(
            !result
                .warnings
                .iter()
                .any(|warning| warning.contains("semantic_scholar search failed")),
            "warnings: {:?}",
            result.warnings
        );
        assert!(result.per_source_status.iter().any(|status| {
            status.source == "semantic_scholar"
                && matches!(&status.status, SourceResult::Ok { count: 1 })
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_get_and_references_are_fetched_from_semantic_scholar() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2id123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "paperId": "s2id123",
                "title": "Main Paper",
                "abstract": "Main abstract",
                "year": 2020,
                "citationCount": 15,
                "venue": "ICML",
                "url": "https://example.org/main",
                "externalIds": { "DOI": "10.5555/main" },
                "authors": [{"name": "Carol"}]
            })))
            .mount(&semantic_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2id123/references"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 1,
                "next": null,
                "data": [
                    {
                        "citedPaper": {
                            "paperId": "s2id-ref",
                            "title": "Reference Paper",
                            "year": 2019,
                            "citationCount": 5,
                            "venue": "NeurIPS",
                            "url": "https://example.org/ref",
                            "externalIds": { "DOI": "10.5555/ref" },
                            "authors": [{"name": "Dave"}]
                        }
                    }
                ]
            })))
            .mount(&semantic_server)
            .await;

        let toolkit = build_test_toolkit(
            format!("{}/graph/v1", semantic_server.uri()),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let detail = toolkit
            .paper_get("s2:s2id123")
            .await
            .expect("paper_get should succeed");

        assert_eq!(detail.paper.title, "Main Paper");
        assert_eq!(detail.references.len(), 1);
        assert_eq!(detail.references[0].title, "Reference Paper");

        let citations = toolkit
            .paper_citations(
                "s2:s2id123",
                PaginationParams {
                    offset: Some(0),
                    limit: Some(10),
                    fields: None,
                    max_chars_per_item: None,
                },
            )
            .await;

        assert!(citations.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_honors_sort_by_year() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/paper/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 2,
                "next": null,
                "data": [
                    {
                        "paperId": "s2-older",
                        "title": "Older Paper",
                        "year": 2020,
                        "citationCount": 500,
                        "authors": [{"name": "Alice"}]
                    },
                    {
                        "paperId": "s2-newer",
                        "title": "Newer Paper",
                        "year": 2024,
                        "citationCount": 5,
                        "authors": [{"name": "Bob"}]
                    }
                ]
            })))
            .mount(&semantic_server)
            .await;

        let toolkit = build_test_toolkit(
            semantic_server.uri(),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let result = toolkit
            .paper_search(PaperSearchParams {
                query: "sorting".to_string(),
                year_from: None,
                year_to: None,
                fields_of_study: None,
                source: Some("semantic_scholar".to_string()),
                sort_by: Some("year".to_string()),
                offset: Some(0),
                limit: Some(10),
                include_abstract: Some(false),
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect("paper_search should succeed");

        let titles = result
            .papers
            .iter()
            .map(|paper| paper.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Newer Paper", "Older Paper"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_single_source_offset_is_not_double_applied() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/paper/search"))
            .and(query_param("offset", "20"))
            .and(query_param("limit", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total": 200,
                "next": 30,
                "data": [
                    {
                        "paperId": "s2-21",
                        "title": "Paged Paper",
                        "year": 2024,
                        "citationCount": 3,
                        "authors": [{"name": "Alice"}]
                    }
                ]
            })))
            .mount(&semantic_server)
            .await;

        let toolkit = build_test_toolkit(
            semantic_server.uri(),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let result = toolkit
            .paper_search(PaperSearchParams {
                query: "pagination".to_string(),
                year_from: None,
                year_to: None,
                fields_of_study: None,
                source: Some("semantic_scholar".to_string()),
                sort_by: None,
                offset: Some(20),
                limit: Some(10),
                include_abstract: Some(false),
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect("paper_search should succeed");

        assert_eq!(result.papers.len(), 1);
        assert_eq!(result.papers[0].title, "Paged Paper");
        assert_eq!(result.has_more, true);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_rejects_unknown_sort_by_value() {
        let semantic_server = MockServer::start().await;
        let arxiv_server = MockServer::start().await;
        let openalex_server = MockServer::start().await;
        let toolkit = build_test_toolkit(
            semantic_server.uri(),
            arxiv_server.uri(),
            openalex_server.uri(),
        );

        let error = toolkit
            .paper_search(PaperSearchParams {
                query: "sorting".to_string(),
                year_from: None,
                year_to: None,
                fields_of_study: None,
                source: Some("semantic_scholar".to_string()),
                sort_by: Some("randomness".to_string()),
                offset: Some(0),
                limit: Some(10),
                include_abstract: Some(false),
                fields: None,
                max_chars_per_item: None,
            })
            .await
            .expect_err("unknown sort_by should fail");

        assert!(
            matches!(error, crate::error::ResearchError::InvalidInput(_)),
            "unexpected error: {error}"
        );
    }

    fn build_test_toolkit(
        semantic_scholar_base_url: String,
        arxiv_base_url: String,
        openalex_base_url: String,
    ) -> ResearchToolkit {
        build_test_toolkit_with_config(ResearchConfig {
            semantic_scholar_base_url,
            arxiv_base_url,
            openalex_base_url,
            ..ResearchConfig::default()
        })
    }
}
