use chrono::Utc;
use serde::Deserialize;

use crate::clients::SearchPage;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::paper_id::PaperIdResolver;
use crate::rate_limiter::ResearchApi;
use crate::types::Paper;
use crate::types::SourceMeta;

const SOURCE_SEMANTIC_SCHOLAR: &str = "semantic_scholar";
const PAPER_FIELDS_WITH_ABSTRACT: &str =
    "paperId,title,abstract,year,citationCount,url,externalIds,openAccessPdf,authors,venue";
const PAPER_FIELDS_NO_ABSTRACT: &str =
    "paperId,title,year,citationCount,url,externalIds,openAccessPdf,authors,venue";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaperRelation {
    Citations,
    References,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticScholarConfig<'a> {
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticScholarSearchRequest<'a> {
    pub query: &'a str,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub fields_of_study: Option<&'a [String]>,
    pub offset: u32,
    pub limit: u32,
    pub include_abstract: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticScholarRelationRequest<'a> {
    pub paper_id: &'a str,
    pub relation: PaperRelation,
    pub offset: u32,
    pub limit: u32,
    pub include_abstract: bool,
}

pub(crate) async fn search(
    http: &HttpClient,
    config: SemanticScholarConfig<'_>,
    request: &SemanticScholarSearchRequest<'_>,
) -> Result<SearchPage> {
    let fields = paper_fields(request.include_abstract);

    let mut url = format!(
        "{base_url}/paper/search?query={query}&offset={offset}&limit={limit}&fields={fields}",
        base_url = config.base_url,
        query = urlencoding::encode(request.query),
        offset = request.offset,
        limit = request.limit,
        fields = urlencoding::encode(fields),
    );

    if let (Some(from), Some(to)) = (request.year_from, request.year_to) {
        url.push_str(&format!("&year={from}-{to}"));
    } else if let Some(from) = request.year_from {
        url.push_str(&format!("&year={from}-"));
    } else if let Some(to) = request.year_to {
        url.push_str(&format!("&year=-{to}"));
    }

    if let Some(fields_of_study) = request.fields_of_study
        && !fields_of_study.is_empty()
    {
        url.push_str(&format!(
            "&fieldsOfStudy={fields}",
            fields = urlencoding::encode(&fields_of_study.join(",")),
        ));
    }

    let response: SemanticScholarSearchResponse = http
        .execute_json(ResearchApi::SemanticScholar, || {
            let mut request = http.client().get(&url);
            if let Some(api_key) = config.api_key {
                request = request.header("x-api-key", api_key);
            }
            request
        })
        .await?;

    let papers = response
        .data
        .into_iter()
        .map(|paper| map_paper(paper, &url, SOURCE_SEMANTIC_SCHOLAR))
        .collect();

    let has_more = response.next.is_some()
        || response
            .total
            .is_some_and(|total| total > u64::from(request.offset) + u64::from(request.limit));

    Ok(SearchPage {
        papers,
        total_available: response.total,
        has_more,
    })
}

pub(crate) async fn get_paper(
    http: &HttpClient,
    config: SemanticScholarConfig<'_>,
    paper_id: &str,
    include_abstract: bool,
) -> Result<Paper> {
    let fields = paper_fields(include_abstract);

    let url = format!(
        "{base_url}/paper/{paper_id}?fields={fields}",
        base_url = config.base_url,
        paper_id = urlencoding::encode(paper_id),
        fields = urlencoding::encode(fields),
    );

    let response: SemanticScholarPaper = http
        .execute_json(ResearchApi::SemanticScholar, || {
            let mut request = http.client().get(&url);
            if let Some(api_key) = config.api_key {
                request = request.header("x-api-key", api_key);
            }
            request
        })
        .await?;

    Ok(map_paper(response, &url, SOURCE_SEMANTIC_SCHOLAR))
}

pub(crate) async fn get_relations(
    http: &HttpClient,
    config: SemanticScholarConfig<'_>,
    request: &SemanticScholarRelationRequest<'_>,
) -> Result<SearchPage> {
    let relation_path = match request.relation {
        PaperRelation::Citations => "citations",
        PaperRelation::References => "references",
    };

    let fields = paper_fields(request.include_abstract);

    let url = format!(
        "{base_url}/paper/{paper_id}/{relation_path}?offset={offset}&limit={limit}&fields={fields}",
        base_url = config.base_url,
        paper_id = urlencoding::encode(request.paper_id),
        offset = request.offset,
        limit = request.limit,
        fields = urlencoding::encode(fields),
    );

    let response: SemanticScholarRelationResponse = http
        .execute_json(ResearchApi::SemanticScholar, || {
            let mut request = http.client().get(&url);
            if let Some(api_key) = config.api_key {
                request = request.header("x-api-key", api_key);
            }
            request
        })
        .await?;

    let papers = response
        .data
        .into_iter()
        .filter_map(|item| {
            item.citing_paper
                .or(item.cited_paper)
                .or_else(|| item.direct.into_paper())
        })
        .map(|paper| map_paper(paper, &url, SOURCE_SEMANTIC_SCHOLAR))
        .collect::<Vec<_>>();

    let has_more = response.next.is_some()
        || response
            .total
            .is_some_and(|total| total > u64::from(request.offset) + u64::from(request.limit));

    Ok(SearchPage {
        papers,
        total_available: response.total,
        has_more,
    })
}

fn paper_fields(include_abstract: bool) -> &'static str {
    if include_abstract {
        PAPER_FIELDS_WITH_ABSTRACT
    } else {
        PAPER_FIELDS_NO_ABSTRACT
    }
}

fn map_paper(paper: SemanticScholarPaper, api_url: &str, source: &str) -> Paper {
    let arxiv_id = paper
        .external_ids
        .as_ref()
        .and_then(|ids| ids.arxiv_id.clone());

    let mut pdf_url = paper.open_access_pdf.and_then(|pdf| pdf.url);
    if pdf_url.is_none()
        && let Some(arxiv_id) = arxiv_id.as_deref()
    {
        let normalized = arxiv_id
            .trim()
            .trim_start_matches("arXiv:")
            .trim_start_matches("ARXIV:");
        if !normalized.is_empty() {
            pdf_url = Some(format!("https://arxiv.org/pdf/{normalized}.pdf"));
        }
    }

    let mut mapped = Paper {
        title: paper.title,
        authors: paper
            .authors
            .unwrap_or_default()
            .into_iter()
            .map(|author| author.name)
            .collect::<Vec<_>>()
            .join(", "),
        year: paper.year,
        venue: paper.venue,
        citation_count: paper.citation_count,
        abstract_text: paper.abstract_text,
        doi: paper.external_ids.as_ref().and_then(|ids| ids.doi.clone()),
        arxiv_id,
        s2_paper_id: paper.paper_id,
        openalex_id: None,
        url: paper.url,
        pdf_url,
        code_url: None,
        source_meta: None,
    };

    let canonical_id = PaperIdResolver::canonical_from_fields(
        mapped.doi.as_deref(),
        mapped.arxiv_id.as_deref(),
        mapped.s2_paper_id.as_deref(),
        None,
    )
    .map(|paper_id| paper_id.to_string());

    mapped.source_meta = Some(SourceMeta {
        source: source.to_string(),
        api_url: api_url.to_string(),
        fetched_at: Utc::now(),
        canonical_id,
    });

    mapped
}

#[derive(Debug, Deserialize)]
struct SemanticScholarSearchResponse {
    data: Vec<SemanticScholarPaper>,
    total: Option<u64>,
    next: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarRelationResponse {
    data: Vec<SemanticScholarRelationItem>,
    total: Option<u64>,
    next: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarRelationItem {
    #[serde(rename = "citingPaper")]
    citing_paper: Option<SemanticScholarPaper>,
    #[serde(rename = "citedPaper")]
    cited_paper: Option<SemanticScholarPaper>,
    #[serde(flatten)]
    direct: SemanticScholarPaperLoose,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarPaper {
    #[serde(rename = "paperId")]
    paper_id: Option<String>,
    title: String,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    year: Option<u32>,
    #[serde(rename = "citationCount")]
    citation_count: Option<u32>,
    venue: Option<String>,
    url: Option<String>,
    #[serde(rename = "openAccessPdf")]
    open_access_pdf: Option<SemanticScholarOpenAccessPdf>,
    #[serde(rename = "externalIds")]
    external_ids: Option<SemanticScholarExternalIds>,
    authors: Option<Vec<SemanticScholarAuthor>>,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarPaperLoose {
    #[serde(rename = "paperId")]
    paper_id: Option<String>,
    title: Option<String>,
    #[serde(rename = "abstract")]
    abstract_text: Option<String>,
    year: Option<u32>,
    #[serde(rename = "citationCount")]
    citation_count: Option<u32>,
    venue: Option<String>,
    url: Option<String>,
    #[serde(rename = "openAccessPdf")]
    open_access_pdf: Option<SemanticScholarOpenAccessPdf>,
    #[serde(rename = "externalIds")]
    external_ids: Option<SemanticScholarExternalIds>,
    authors: Option<Vec<SemanticScholarAuthor>>,
}

impl SemanticScholarPaperLoose {
    fn into_paper(self) -> Option<SemanticScholarPaper> {
        Some(SemanticScholarPaper {
            title: self.title?,
            paper_id: self.paper_id,
            abstract_text: self.abstract_text,
            year: self.year,
            citation_count: self.citation_count,
            venue: self.venue,
            url: self.url,
            open_access_pdf: self.open_access_pdf,
            external_ids: self.external_ids,
            authors: self.authors,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SemanticScholarOpenAccessPdf {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarExternalIds {
    #[serde(rename = "DOI", alias = "doi")]
    doi: Option<String>,
    #[serde(rename = "ArXiv", alias = "arxiv")]
    arxiv_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SemanticScholarAuthor {
    name: String,
}
