use quick_xml::Reader;
use quick_xml::events::Event;

use super::epo_auth::EpoAuthManager;
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
// Public API
// ---------------------------------------------------------------------------

pub(crate) async fn search(
    http: &HttpClient,
    base_url: &str,
    auth: &EpoAuthManager,
    params: &PatentSearchParams,
) -> Result<PatentSearchResult> {
    let token = auth.get_token(http.client()).await?;

    let cql = build_cql(params);
    let size = params.size.unwrap_or(25).clamp(10, 100);

    // EPO uses 1-based Range header for pagination.
    let begin: u32 = params
        .after
        .as_deref()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1);
    let end = begin.saturating_add(size).saturating_sub(1);

    let url = format!(
        "{base_url}/rest-services/published-data/search/biblio?q={cql}&Range={begin}-{end}",
        cql = urlencoding::encode(&cql),
    );

    let xml = http
        .execute_text(ResearchApi::Patents, || {
            http.client()
                .get(&url)
                .header("Accept", "application/exchange+xml")
                .bearer_auth(&token)
        })
        .await?;

    let (patents, total_hits) = parse_search_xml(&xml)?;

    let has_more = (begin + u32::try_from(patents.len()).unwrap_or(0))
        <= u32::try_from(total_hits).unwrap_or(u32::MAX);
    let next_cursor = if has_more {
        Some((end + 1).to_string())
    } else {
        None
    };

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
    auth: &EpoAuthManager,
    params: &PatentGetParams,
) -> Result<PatentDetail> {
    let token = auth.get_token(http.client()).await?;
    let id = &params.patent_id;

    // Fetch bibliographic data.
    let biblio_url = format!(
        "{base_url}/rest-services/published-data/publication/epodoc/{id}/biblio",
        id = urlencoding::encode(id),
    );
    let biblio_xml = http
        .execute_text(ResearchApi::Patents, || {
            http.client()
                .get(&biblio_url)
                .header("Accept", "application/exchange+xml")
                .bearer_auth(&token)
        })
        .await?;

    let mut detail = parse_biblio_detail_xml(&biblio_xml, id)?;

    // Fetch claims (non-fatal — not all patents have machine-readable claims).
    let claims_url = format!(
        "{base_url}/rest-services/published-data/publication/epodoc/{id}/claims",
        id = urlencoding::encode(id),
    );
    match http
        .execute_text(ResearchApi::Patents, || {
            http.client()
                .get(&claims_url)
                .header("Accept", "application/fulltext+xml")
                .bearer_auth(&token)
        })
        .await
    {
        Ok(claims_xml) => {
            detail.claims_text = parse_claims_xml(&claims_xml);
        }
        Err(err) => {
            tracing::debug!("claims fetch for {id} failed (non-fatal): {err}");
        }
    }

    Ok(detail)
}

// ---------------------------------------------------------------------------
// CQL query builder
// ---------------------------------------------------------------------------

fn build_cql(params: &PatentSearchParams) -> String {
    let mut parts = Vec::new();

    // Title + abstract search — only add if query is non-trivial.
    // When the user filters by assignee/inventor/cpc alone, omitting ta=
    // avoids requiring the company name to appear in the title/abstract.
    let query_trimmed = params.query.trim();
    if !query_trimmed.is_empty() && query_trimmed != "*" {
        parts.push(format!("ta=\"{}\"", escape_cql(query_trimmed)));
    }

    if let Some(inventor) = &params.inventor {
        parts.push(format!("in=\"{}\"", escape_cql(inventor)));
    }
    if let Some(assignee) = &params.assignee {
        parts.push(format!("pa=\"{}\"", escape_cql(assignee)));
    }
    if let Some(cpc) = &params.cpc_code {
        parts.push(format!("cpc={}", escape_cql(cpc)));
    }

    // Date range: pd within "YYYYMMDD YYYYMMDD".
    match (&params.date_from, &params.date_to) {
        (Some(from), Some(to)) => {
            let from_clean = from.replace('-', "");
            let to_clean = to.replace('-', "");
            parts.push(format!("pd within \"{from_clean} {to_clean}\""));
        }
        (Some(from), None) => {
            let from_clean = from.replace('-', "");
            parts.push(format!("pd>={from_clean}"));
        }
        (None, Some(to)) => {
            let to_clean = to.replace('-', "");
            parts.push(format!("pd<={to_clean}"));
        }
        (None, None) => {}
    }

    // If no CQL terms were produced, fall back to ta= with the original query
    // so EPO doesn't get an empty query string.
    if parts.is_empty() {
        let fallback = params.query.trim();
        if fallback.is_empty() || fallback == "*" {
            return "ta=patent".to_string();
        }
        return format!("ta=\"{}\"", escape_cql(fallback));
    }

    parts.join(" and ")
}

fn escape_cql(value: &str) -> String {
    value.replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// XML parsing: search results
// ---------------------------------------------------------------------------

fn parse_search_xml(xml: &str) -> Result<(Vec<PatentItem>, u64)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut patents = Vec::new();
    let mut total_hits: u64 = 0;
    let mut current_doc: Option<EpoDocument> = None;
    let mut text_target: Option<SearchTextTarget> = None;
    let mut in_applicant = false;
    let mut in_inventor = false;
    let mut in_classification = false;
    let mut title_lang: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref event)) | Ok(Event::Empty(ref event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "biblio-search" => {
                        total_hits = attr_value(event.attributes(), "total-result-count")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                    }
                    "exchange-document" => {
                        let country = attr_value(event.attributes(), "country").unwrap_or_default();
                        let doc_number =
                            attr_value(event.attributes(), "doc-number").unwrap_or_default();
                        let kind = attr_value(event.attributes(), "kind").unwrap_or_default();
                        current_doc = Some(EpoDocument {
                            country,
                            doc_number,
                            kind,
                            ..EpoDocument::default()
                        });
                    }
                    "invention-title" if current_doc.is_some() => {
                        title_lang = attr_value(event.attributes(), "lang");
                        text_target = Some(SearchTextTarget::Title);
                    }
                    "applicant" if current_doc.is_some() => in_applicant = true,
                    "inventor" if current_doc.is_some() => in_inventor = true,
                    "name" if current_doc.is_some() && (in_applicant || in_inventor) => {
                        text_target = if in_applicant {
                            Some(SearchTextTarget::ApplicantName)
                        } else {
                            Some(SearchTextTarget::InventorName)
                        };
                    }
                    "p" if current_doc.is_some() => {
                        text_target = Some(SearchTextTarget::Abstract);
                    }
                    "patent-classification" if current_doc.is_some() => {
                        in_classification = true;
                    }
                    "section" | "class" | "subclass" | "main-group" | "subgroup"
                        if in_classification && current_doc.is_some() =>
                    {
                        text_target = Some(SearchTextTarget::ClassPart);
                    }
                    "date" if current_doc.is_some() => {
                        text_target = Some(SearchTextTarget::Date);
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref event)) => {
                let name = local_name(event.name().as_ref());
                match name.as_str() {
                    "exchange-document" => {
                        if let Some(doc) = current_doc.take() {
                            patents.push(doc.into_item());
                        }
                    }
                    "applicant" => in_applicant = false,
                    "inventor" => in_inventor = false,
                    "patent-classification" => {
                        if in_classification {
                            if let Some(doc) = current_doc.as_mut() {
                                doc.flush_classification();
                            }
                            in_classification = false;
                        }
                    }
                    "invention-title" | "name" | "p" | "date" | "section" | "class"
                    | "subclass" | "main-group" | "subgroup" => {
                        text_target = None;
                        title_lang = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref text)) => {
                let value = clean_text(std::str::from_utf8(text.as_ref()).unwrap_or_default());
                if !value.is_empty()
                    && let Some(doc) = current_doc.as_mut()
                {
                    apply_search_text(doc, &text_target, title_lang.as_deref(), &value);
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(ResearchError::Parse {
                    api: ResearchApi::Patents,
                    message: format!("EPO XML parse error: {err}"),
                });
            }
            _ => {}
        }
    }

    Ok((patents, total_hits))
}

#[derive(Debug, Clone)]
enum SearchTextTarget {
    Title,
    Abstract,
    ApplicantName,
    InventorName,
    Date,
    ClassPart,
}

fn apply_search_text(
    doc: &mut EpoDocument,
    target: &Option<SearchTextTarget>,
    title_lang: Option<&str>,
    value: &str,
) {
    let Some(target) = target else { return };
    match target {
        SearchTextTarget::Title => {
            // Prefer English title; accept first if no English one yet.
            if doc.title.is_empty() || title_lang == Some("en") {
                doc.title = value.to_string();
            }
        }
        SearchTextTarget::Abstract => {
            if !doc.abstract_text.is_empty() {
                doc.abstract_text.push(' ');
            }
            doc.abstract_text.push_str(value);
        }
        SearchTextTarget::ApplicantName => {
            doc.applicants.push(value.to_string());
        }
        SearchTextTarget::InventorName => {
            doc.inventors.push(value.to_string());
        }
        SearchTextTarget::Date => {
            if doc.pub_date.is_none() {
                doc.pub_date = Some(value.to_string());
            }
        }
        SearchTextTarget::ClassPart => {
            doc.class_parts.push(value.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// XML parsing: biblio detail
// ---------------------------------------------------------------------------

fn parse_biblio_detail_xml(xml: &str, original_id: &str) -> Result<PatentDetail> {
    let (patents, _) = parse_search_xml(xml)?;
    let doc = patents
        .into_iter()
        .next()
        .ok_or_else(|| ResearchError::Parse {
            api: ResearchApi::Patents,
            message: format!("no exchange-document found for {original_id}"),
        })?;

    // Promote PatentItem to PatentDetail — claims filled later.
    Ok(PatentDetail {
        patent_id: doc.patent_id,
        title: doc.title,
        abstract_text: doc.abstract_text,
        grant_date: doc.grant_date,
        filing_date: None,
        inventors: doc.inventors,
        assignees: doc.assignees,
        cpc_codes: doc.cpc_codes,
        claims_text: None,
        citation_count: None,
        cited_by_count: None,
        url: doc.url,
    })
}

// ---------------------------------------------------------------------------
// XML parsing: claims
// ---------------------------------------------------------------------------

fn parse_claims_xml(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut claims = Vec::new();
    let mut in_claim_text = false;
    let mut current_claim = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref event)) => {
                let name = local_name(event.name().as_ref());
                if name == "claim-text" {
                    in_claim_text = true;
                    current_claim.clear();
                }
            }
            Ok(Event::End(ref event)) => {
                let name = local_name(event.name().as_ref());
                if name == "claim-text" {
                    in_claim_text = false;
                    let trimmed = current_claim.trim().to_string();
                    if !trimmed.is_empty() {
                        claims.push(trimmed);
                    }
                }
            }
            Ok(Event::Text(ref text)) => {
                if in_claim_text {
                    let value = clean_text(std::str::from_utf8(text.as_ref()).unwrap_or_default());
                    if !current_claim.is_empty() && !value.is_empty() {
                        current_claim.push(' ');
                    }
                    current_claim.push_str(&value);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
    }

    if claims.is_empty() {
        None
    } else {
        Some(claims.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// Internal document struct
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct EpoDocument {
    country: String,
    doc_number: String,
    kind: String,
    title: String,
    abstract_text: String,
    inventors: Vec<String>,
    applicants: Vec<String>,
    cpc_codes: Vec<String>,
    pub_date: Option<String>,
    /// Accumulates classification part texts for the current classification.
    class_parts: Vec<String>,
}

impl EpoDocument {
    fn patent_id(&self) -> String {
        format!("{}{}{}", self.country, self.doc_number, self.kind)
    }

    fn formatted_date(&self) -> Option<String> {
        self.pub_date.as_ref().map(|d| {
            if d.len() == 8 {
                format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
            } else {
                d.clone()
            }
        })
    }

    fn flush_classification(&mut self) {
        if !self.class_parts.is_empty() {
            let code = self.class_parts.join("");
            if !code.is_empty() {
                self.cpc_codes.push(code);
            }
            self.class_parts.clear();
        }
    }

    fn into_item(self) -> PatentItem {
        let patent_id = self.patent_id();
        let grant_date = self.formatted_date();
        let url = format!("https://worldwide.espacenet.com/patent/search?q=pn%3D{patent_id}");
        let abstract_text = if self.abstract_text.is_empty() {
            None
        } else {
            Some(self.abstract_text)
        };
        PatentItem {
            patent_id,
            title: self.title,
            abstract_text,
            grant_date,
            inventors: self.inventors,
            assignees: self.applicants,
            cpc_codes: self.cpc_codes,
            citation_count: None,
            url,
        }
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn local_name(name: &[u8]) -> String {
    let raw = std::str::from_utf8(name).unwrap_or_default();
    raw.rsplit(':').next().unwrap_or(raw).to_string()
}

fn clean_text(value: &str) -> String {
    value.trim().to_string()
}

fn attr_value(
    attributes: quick_xml::events::attributes::Attributes<'_>,
    target: &str,
) -> Option<String> {
    for attr in attributes.flatten() {
        let key = local_name(attr.key.as_ref());
        if key == target {
            return Some(clean_text(
                std::str::from_utf8(attr.value.as_ref()).unwrap_or_default(),
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path_regex;

    use super::*;
    use crate::ResearchToolkit;
    use crate::clients::epo_auth::EpoAuthManager;
    use crate::config::ResearchConfig;

    fn test_toolkit(base_url: &str) -> (ResearchToolkit, EpoAuthManager) {
        let config = ResearchConfig {
            patents_base_url: base_url.to_string(),
            epo_consumer_key: Some("test-key".to_string()),
            epo_consumer_secret: Some("test-secret".to_string()),
            ..ResearchConfig::default()
        };
        let client = reqwest::Client::new();
        let auth = EpoAuthManager::new("test-key".to_string(), "test-secret".to_string(), base_url);
        (ResearchToolkit::new(client, config), auth)
    }

    async fn mount_auth(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path_regex("/auth/accesstoken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "test-bearer-token",
                "token_type": "Bearer",
                "expires_in": 1200
            })))
            .mount(server)
            .await;
    }

    const SEARCH_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ops:world-patent-data xmlns:ops="http://ops.epo.org" xmlns:exch="http://www.epo.org/exchange">
  <ops:biblio-search total-result-count="1500">
    <ops:search-result>
      <ops:publication-reference>
        <exch:exchange-documents>
          <exch:exchange-document country="EP" doc-number="1000000" kind="A1" system="ops.epo.org">
            <exch:bibliographic-data>
              <exch:invention-title lang="en">Test Patent Title</exch:invention-title>
              <exch:invention-title lang="de">Testpatenttitel</exch:invention-title>
              <exch:parties>
                <exch:applicants>
                  <exch:applicant>
                    <exch:applicant-name><exch:name>Acme Corp</exch:name></exch:applicant-name>
                  </exch:applicant>
                </exch:applicants>
                <exch:inventors>
                  <exch:inventor>
                    <exch:inventor-name><exch:name>John Doe</exch:name></exch:inventor-name>
                  </exch:inventor>
                  <exch:inventor>
                    <exch:inventor-name><exch:name>Jane Smith</exch:name></exch:inventor-name>
                  </exch:inventor>
                </exch:inventors>
              </exch:parties>
              <exch:abstract>
                <exch:p>A test abstract about transformers.</exch:p>
              </exch:abstract>
              <exch:publication-reference>
                <exch:document-id>
                  <exch:date>20230615</exch:date>
                </exch:document-id>
              </exch:publication-reference>
              <exch:patent-classifications>
                <exch:patent-classification>
                  <exch:section>G</exch:section>
                  <exch:class>06</exch:class>
                  <exch:subclass>N</exch:subclass>
                  <exch:main-group>3</exch:main-group>
                  <exch:subgroup>08</exch:subgroup>
                </exch:patent-classification>
              </exch:patent-classifications>
            </exch:bibliographic-data>
          </exch:exchange-document>
        </exch:exchange-documents>
      </ops:publication-reference>
    </ops:search-result>
  </ops:biblio-search>
</ops:world-patent-data>"#;

    #[tokio::test]
    async fn search_parses_epo_xml_response() {
        let server = MockServer::start().await;
        mount_auth(&server).await;
        Mock::given(method("GET"))
            .and(path_regex("/rest-services/published-data/search/biblio"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(SEARCH_XML, "application/exchange+xml"),
            )
            .mount(&server)
            .await;

        let (toolkit, auth) = test_toolkit(&server.uri());
        let params = PatentSearchParams {
            query: "transformer neural network".to_string(),
            inventor: None,
            assignee: None,
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        let result = search(toolkit.http(), &server.uri(), &auth, &params)
            .await
            .expect("search should succeed");

        assert_eq!(result.total_hits, 1500);
        assert!(result.has_more);
        assert_eq!(result.patents.len(), 1);

        let patent = &result.patents[0];
        assert_eq!(patent.patent_id, "EP1000000A1");
        assert_eq!(patent.title, "Test Patent Title");
        assert_eq!(
            patent.abstract_text,
            Some("A test abstract about transformers.".to_string())
        );
        assert_eq!(patent.inventors, vec!["John Doe", "Jane Smith"]);
        assert_eq!(patent.assignees, vec!["Acme Corp"]);
        assert_eq!(patent.grant_date, Some("2023-06-15".to_string()));
        assert_eq!(patent.cpc_codes, vec!["G06N308"]);
        assert!(patent.url.contains("EP1000000A1"));
    }

    #[tokio::test]
    async fn get_patent_parses_detail_with_claims() {
        let server = MockServer::start().await;
        mount_auth(&server).await;

        // Biblio endpoint returns the same XML structure as search.
        Mock::given(method("GET"))
            .and(path_regex(
                "/rest-services/published-data/publication/epodoc/.*/biblio",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(SEARCH_XML, "application/exchange+xml"),
            )
            .mount(&server)
            .await;

        let claims_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ftxt:fulltext-documents xmlns:ftxt="http://www.epo.org/fulltext">
  <ftxt:fulltext-document>
    <ftxt:claims>
      <ftxt:claim>
        <ftxt:claim-text>A method for training a transformer model.</ftxt:claim-text>
      </ftxt:claim>
      <ftxt:claim>
        <ftxt:claim-text>The method of claim 1, further comprising attention layers.</ftxt:claim-text>
      </ftxt:claim>
    </ftxt:claims>
  </ftxt:fulltext-document>
</ftxt:fulltext-documents>"#;

        Mock::given(method("GET"))
            .and(path_regex(
                "/rest-services/published-data/publication/epodoc/.*/claims",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(claims_xml, "application/fulltext+xml"),
            )
            .mount(&server)
            .await;

        let (toolkit, auth) = test_toolkit(&server.uri());
        let params = PatentGetParams {
            patent_id: "EP1000000A1".to_string(),
        };

        let result = get_patent(toolkit.http(), &server.uri(), &auth, &params)
            .await
            .expect("get_patent should succeed");

        assert_eq!(result.patent_id, "EP1000000A1");
        assert_eq!(result.title, "Test Patent Title");
        assert_eq!(result.inventors, vec!["John Doe", "Jane Smith"]);
        assert_eq!(result.assignees, vec!["Acme Corp"]);
        assert_eq!(
            result.claims_text,
            Some("A method for training a transformer model.\n\nThe method of claim 1, further comprising attention layers.".to_string())
        );
    }

    #[tokio::test]
    async fn get_patent_handles_missing_claims_gracefully() {
        let server = MockServer::start().await;
        mount_auth(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(
                "/rest-services/published-data/publication/epodoc/.*/biblio",
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(SEARCH_XML, "application/exchange+xml"),
            )
            .mount(&server)
            .await;

        // Claims endpoint returns 404.
        Mock::given(method("GET"))
            .and(path_regex(
                "/rest-services/published-data/publication/epodoc/.*/claims",
            ))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let (toolkit, auth) = test_toolkit(&server.uri());
        let params = PatentGetParams {
            patent_id: "EP1000000A1".to_string(),
        };

        let result = get_patent(toolkit.http(), &server.uri(), &auth, &params)
            .await
            .expect("get_patent should succeed even without claims");

        assert_eq!(result.patent_id, "EP1000000A1");
        assert!(result.claims_text.is_none());
    }

    #[test]
    fn build_cql_generates_correct_query() {
        let params = PatentSearchParams {
            query: "neural network".to_string(),
            inventor: Some("Smith".to_string()),
            assignee: Some("Google".to_string()),
            cpc_code: Some("G06N".to_string()),
            date_from: Some("2020-01-01".to_string()),
            date_to: Some("2023-12-31".to_string()),
            sort_by: None,
            size: None,
            after: None,
        };

        let cql = build_cql(&params);
        assert_eq!(
            cql,
            "ta=\"neural network\" and in=\"Smith\" and pa=\"Google\" and cpc=G06N and pd within \"20200101 20231231\""
        );
    }

    #[test]
    fn build_cql_handles_minimal_params() {
        let params = PatentSearchParams {
            query: "transformer".to_string(),
            inventor: None,
            assignee: None,
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        assert_eq!(build_cql(&params), "ta=\"transformer\"");
    }

    #[test]
    fn build_cql_wildcard_with_assignee_skips_ta() {
        let params = PatentSearchParams {
            query: "*".to_string(),
            inventor: None,
            assignee: Some("Apple Inc".to_string()),
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        assert_eq!(build_cql(&params), "pa=\"Apple Inc\"");
    }

    #[test]
    fn build_cql_empty_query_with_inventor_and_date() {
        let params = PatentSearchParams {
            query: "".to_string(),
            inventor: Some("Smith".to_string()),
            assignee: None,
            cpc_code: None,
            date_from: Some("2024-01-01".to_string()),
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        assert_eq!(build_cql(&params), "in=\"Smith\" and pd>=20240101");
    }

    #[test]
    fn build_cql_wildcard_alone_falls_back() {
        let params = PatentSearchParams {
            query: "*".to_string(),
            inventor: None,
            assignee: None,
            cpc_code: None,
            date_from: None,
            date_to: None,
            sort_by: None,
            size: None,
            after: None,
        };

        // Degenerate case: wildcard with no filters → fallback query.
        assert_eq!(build_cql(&params), "ta=patent");
    }
}
