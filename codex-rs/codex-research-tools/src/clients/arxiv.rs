use chrono::Utc;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::clients::SearchPage;
use crate::error::ResearchError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::paper_id::PaperIdResolver;
use crate::rate_limiter::ResearchApi;
use crate::types::Paper;
use crate::types::SourceMeta;

pub(crate) async fn search(
    http: &HttpClient,
    base_url: &str,
    request: &ArxivSearchRequest<'_>,
) -> Result<SearchPage> {
    let url = format!(
        "{base_url}/api/query?search_query=all:{query}&start={offset}&max_results={limit}&sortBy=relevance&sortOrder=descending",
        query = urlencoding::encode(request.query),
        offset = request.offset,
        limit = request.limit,
    );

    let xml = http
        .execute_text(ResearchApi::Arxiv, || http.client().get(&url))
        .await?;

    let parsed = parse_feed(&xml, &url, request.include_abstract)?;
    let papers = parsed
        .papers
        .into_iter()
        .filter(|paper| {
            let Some(year) = paper.year else {
                return true;
            };

            request.year_from.is_none_or(|from| year >= from)
                && request.year_to.is_none_or(|to| year <= to)
        })
        .collect::<Vec<_>>();

    let has_more = parsed.total_available.is_some_and(|total| {
        total > u64::from(request.offset) + u64::try_from(papers.len()).unwrap_or(0)
    });

    Ok(SearchPage {
        papers,
        total_available: parsed.total_available,
        has_more,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArxivSearchRequest<'a> {
    pub query: &'a str,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub offset: u32,
    pub limit: u32,
    pub include_abstract: bool,
}

#[derive(Debug, Default)]
struct ParsedFeed {
    papers: Vec<Paper>,
    total_available: Option<u64>,
}

#[derive(Debug, Default)]
struct ArxivEntry {
    id: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    published: Option<String>,
    authors: Vec<String>,
    url: Option<String>,
    pdf_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextTarget {
    Id,
    Title,
    Summary,
    Published,
    AuthorName,
    TotalResults,
}

fn parse_feed(xml: &str, api_url: &str, include_abstract: bool) -> Result<ParsedFeed> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut parsed = ParsedFeed::default();
    let mut current_entry: Option<ArxivEntry> = None;
    let mut current_target: Option<TextTarget> = None;
    let mut in_author = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "entry" => current_entry = Some(ArxivEntry::default()),
                    "author" if current_entry.is_some() => in_author = true,
                    "link" if current_entry.is_some() => {
                        if let Some(entry) = current_entry.as_mut() {
                            apply_link(event.attributes(), entry);
                        }
                    }
                    "id" if current_entry.is_some() => current_target = Some(TextTarget::Id),
                    "title" if current_entry.is_some() => current_target = Some(TextTarget::Title),
                    "summary" if current_entry.is_some() => {
                        current_target = Some(TextTarget::Summary)
                    }
                    "published" if current_entry.is_some() => {
                        current_target = Some(TextTarget::Published)
                    }
                    "name" if current_entry.is_some() && in_author => {
                        current_target = Some(TextTarget::AuthorName)
                    }
                    "totalResults" => current_target = Some(TextTarget::TotalResults),
                    _ => {}
                }
            }
            Ok(Event::End(event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "entry" => {
                        if let Some(entry) = current_entry.take()
                            && let Some(paper) = entry.into_paper(api_url, include_abstract)
                        {
                            parsed.papers.push(paper);
                        }
                    }
                    "author" => in_author = false,
                    "id" | "title" | "summary" | "published" | "name" | "totalResults" => {
                        current_target = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(text)) => {
                let value = clean_text(std::str::from_utf8(text.as_ref()).unwrap_or_default());
                apply_text(
                    &mut current_entry,
                    &mut parsed,
                    current_target,
                    in_author,
                    &value,
                );
            }
            Ok(Event::CData(text)) => {
                let value = clean_text(std::str::from_utf8(text.as_ref()).unwrap_or_default());
                apply_text(
                    &mut current_entry,
                    &mut parsed,
                    current_target,
                    in_author,
                    &value,
                );
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(ResearchError::Parse {
                    api: ResearchApi::Arxiv,
                    message: err.to_string(),
                });
            }
            _ => {}
        }
    }

    Ok(parsed)
}

fn apply_text(
    current_entry: &mut Option<ArxivEntry>,
    parsed: &mut ParsedFeed,
    current_target: Option<TextTarget>,
    in_author: bool,
    value: &str,
) {
    if value.is_empty() {
        return;
    }

    match current_target {
        Some(TextTarget::TotalResults) => {
            if let Ok(total) = value.parse::<u64>() {
                parsed.total_available = Some(total);
            }
        }
        Some(TextTarget::Id) => {
            if let Some(entry) = current_entry.as_mut() {
                entry.id = Some(value.to_string());
            }
        }
        Some(TextTarget::Title) => {
            if let Some(entry) = current_entry.as_mut() {
                entry.title = Some(value.to_string());
            }
        }
        Some(TextTarget::Summary) => {
            if let Some(entry) = current_entry.as_mut() {
                entry.summary = Some(value.to_string());
            }
        }
        Some(TextTarget::Published) => {
            if let Some(entry) = current_entry.as_mut() {
                entry.published = Some(value.to_string());
            }
        }
        Some(TextTarget::AuthorName) if in_author => {
            if let Some(entry) = current_entry.as_mut() {
                entry.authors.push(value.to_string());
            }
        }
        _ => {}
    }
}

fn apply_link<'a>(
    attributes: quick_xml::events::attributes::Attributes<'a>,
    entry: &mut ArxivEntry,
) {
    let mut href: Option<String> = None;
    let mut rel: Option<String> = None;
    let mut link_type: Option<String> = None;
    let mut title: Option<String> = None;

    for attribute in attributes.flatten() {
        let key = local_name(attribute.key.as_ref());
        let value = clean_text(std::str::from_utf8(attribute.value.as_ref()).unwrap_or_default());
        match key.as_str() {
            "href" => href = Some(value),
            "rel" => rel = Some(value),
            "type" => link_type = Some(value),
            "title" => title = Some(value),
            _ => {}
        }
    }

    if let Some(href) = href {
        let is_pdf = title.as_deref() == Some("pdf")
            || link_type.as_deref() == Some("application/pdf")
            || href.contains("/pdf/");

        if is_pdf {
            entry.pdf_url = Some(href);
            return;
        }

        if rel.as_deref() == Some("alternate") || href.contains("/abs/") {
            entry.url = Some(href);
        }
    }
}

impl ArxivEntry {
    fn into_paper(self, api_url: &str, include_abstract: bool) -> Option<Paper> {
        let title = self.title.map(|title| normalize_whitespace(&title))?;
        let arxiv_id = self
            .id
            .as_deref()
            .and_then(extract_arxiv_id)
            .or_else(|| self.url.as_deref().and_then(extract_arxiv_id));

        let year = self
            .published
            .as_deref()
            .and_then(|published| published.get(0..4))
            .and_then(|year| year.parse::<u32>().ok());

        let canonical_id =
            PaperIdResolver::canonical_from_fields(None, arxiv_id.as_deref(), None, None)
                .map(|paper_id| paper_id.to_string());

        Some(Paper {
            title,
            authors: self.authors.join(", "),
            year,
            venue: Some("arXiv".to_string()),
            citation_count: None,
            abstract_text: if include_abstract {
                self.summary.map(|summary| normalize_whitespace(&summary))
            } else {
                None
            },
            doi: None,
            arxiv_id,
            s2_paper_id: None,
            openalex_id: None,
            url: self.url.or(self.id),
            pdf_url: self.pdf_url,
            code_url: None,
            source_meta: Some(SourceMeta {
                source: "arxiv".to_string(),
                api_url: api_url.to_string(),
                fetched_at: Utc::now(),
                canonical_id,
            }),
        })
    }
}

fn extract_arxiv_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = if let Some((_, tail)) = trimmed.rsplit_once("/abs/") {
        tail
    } else if let Some(stripped) = trimmed.strip_prefix("arXiv:") {
        stripped
    } else {
        trimmed
    };

    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

fn local_name(name: &[u8]) -> String {
    let raw = std::str::from_utf8(name).unwrap_or_default();
    raw.rsplit(':').next().unwrap_or(raw).to_string()
}

fn clean_text(value: &str) -> String {
    value.trim().to_string()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
