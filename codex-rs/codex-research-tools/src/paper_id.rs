use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::types::Paper;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperId {
    Doi(String),
    ArxivId(String),
    S2Id(String),
    OpenAlexId(String),
}

impl fmt::Display for PaperId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Doi(value) => write!(f, "doi:{value}"),
            Self::ArxivId(value) => write!(f, "arxiv:{value}"),
            Self::S2Id(value) => write!(f, "s2:{value}"),
            Self::OpenAlexId(value) => write!(f, "openalex:{value}"),
        }
    }
}

impl FromStr for PaperId {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let raw = input.trim();
        if raw.is_empty() {
            return Err("paper id cannot be empty".to_string());
        }

        if let Some((prefix, id)) = raw.split_once(':') {
            return match prefix.to_ascii_lowercase().as_str() {
                "doi" => Ok(Self::Doi(normalize_doi(id))),
                "arxiv" => Ok(Self::ArxivId(normalize_arxiv(id))),
                "s2" => Ok(Self::S2Id(id.trim().to_string())),
                "openalex" => Ok(Self::OpenAlexId(normalize_openalex(id))),
                _ => Err(format!("unknown paper id prefix: {prefix}")),
            };
        }

        if raw.starts_with("10.") {
            return Ok(Self::Doi(normalize_doi(raw)));
        }
        if looks_like_arxiv(raw) {
            return Ok(Self::ArxivId(normalize_arxiv(raw)));
        }
        if looks_like_openalex(raw) {
            return Ok(Self::OpenAlexId(normalize_openalex(raw)));
        }
        if !raw.contains('/') {
            return Ok(Self::S2Id(raw.to_string()));
        }

        Err(format!("could not infer paper id type from '{raw}'"))
    }
}

fn normalize_doi(value: &str) -> String {
    let lowered = value.trim().to_ascii_lowercase();
    lowered
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("https://dx.doi.org/")
        .trim_start_matches("http://dx.doi.org/")
        .trim_start_matches("doi:")
        .trim()
        .to_string()
}

fn normalize_arxiv(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("arXiv:")
        .trim_start_matches("ARXIV:")
        .to_string()
}

fn normalize_openalex(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('W') {
        trimmed.to_string()
    } else {
        format!("W{}", trimmed.trim_start_matches('w'))
    }
}

fn looks_like_arxiv(value: &str) -> bool {
    let mut split = value.split('.');
    let Some(left) = split.next() else {
        return false;
    };
    let Some(right) = split.next() else {
        return false;
    };
    if split.next().is_some() {
        return false;
    }

    let right_no_version = right.split('v').next().unwrap_or(right);
    left.len() == 4
        && (right_no_version.len() == 4 || right_no_version.len() == 5)
        && left.chars().all(|ch| ch.is_ascii_digit())
        && right_no_version.chars().all(|ch| ch.is_ascii_digit())
}

fn looks_like_openalex(value: &str) -> bool {
    if value.len() < 2 {
        return false;
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == 'W' || first == 'w') && chars.all(|ch| ch.is_ascii_digit())
}

fn normalize_optional_doi(value: &mut Option<String>) {
    if let Some(doi) = value {
        let normalized = normalize_doi(doi);
        if normalized.is_empty() {
            *value = None;
        } else {
            *value = Some(normalized);
        }
    }
}

fn normalize_optional_arxiv(value: &mut Option<String>) {
    if let Some(arxiv_id) = value {
        let normalized = normalize_arxiv(arxiv_id);
        if normalized.trim().is_empty() {
            *value = None;
        } else {
            *value = Some(normalized);
        }
    }
}

fn normalize_optional_openalex(value: &mut Option<String>) {
    if let Some(openalex_id) = value {
        let normalized = normalize_openalex(openalex_id);
        if normalized.trim().is_empty() {
            *value = None;
        } else {
            *value = Some(normalized);
        }
    }
}

fn normalize_optional_trimmed(value: &mut Option<String>) {
    if let Some(raw) = value {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            *value = None;
        } else {
            *value = Some(trimmed);
        }
    }
}

pub(crate) fn normalize_paper_identifiers(paper: &mut Paper) {
    normalize_optional_doi(&mut paper.doi);
    normalize_optional_arxiv(&mut paper.arxiv_id);
    normalize_optional_trimmed(&mut paper.s2_paper_id);
    normalize_optional_openalex(&mut paper.openalex_id);
}

#[derive(Debug, Default, Clone)]
pub struct PaperIdResolver;

impl PaperIdResolver {
    #[must_use]
    pub fn canonical_from_fields(
        doi: Option<&str>,
        arxiv_id: Option<&str>,
        s2_id: Option<&str>,
        openalex_id: Option<&str>,
    ) -> Option<PaperId> {
        if let Some(value) = doi.filter(|value| !value.trim().is_empty()) {
            return Some(PaperId::Doi(normalize_doi(value)));
        }
        if let Some(value) = arxiv_id.filter(|value| !value.trim().is_empty()) {
            return Some(PaperId::ArxivId(normalize_arxiv(value)));
        }
        if let Some(value) = s2_id.filter(|value| !value.trim().is_empty()) {
            return Some(PaperId::S2Id(value.trim().to_string()));
        }
        if let Some(value) = openalex_id.filter(|value| !value.trim().is_empty()) {
            return Some(PaperId::OpenAlexId(normalize_openalex(value)));
        }

        None
    }

    #[must_use]
    pub fn canonical_id_for_paper(&self, paper: &Paper) -> Option<PaperId> {
        Self::canonical_from_fields(
            paper.doi.as_deref(),
            paper.arxiv_id.as_deref(),
            paper.s2_paper_id.as_deref(),
            paper.openalex_id.as_deref(),
        )
    }

    #[must_use]
    pub fn dedup_papers(&self, papers: Vec<Paper>) -> Vec<Paper> {
        let mut key_to_index: HashMap<String, usize> = HashMap::new();
        let mut deduped: Vec<Paper> = Vec::new();

        for mut paper in papers {
            normalize_paper_identifiers(&mut paper);
            let key = self
                .canonical_id_for_paper(&paper)
                .map_or_else(|| paper.title.to_lowercase(), |id| id.to_string());

            if let Some(existing_index) = key_to_index.get(&key).copied() {
                if let Some(existing) = deduped.get_mut(existing_index) {
                    merge_paper(existing, paper);
                } else {
                    key_to_index.insert(key, deduped.len());
                    deduped.push(paper);
                }
            } else {
                key_to_index.insert(key, deduped.len());
                deduped.push(paper);
            }
        }

        deduped
    }
}

fn merge_paper(existing: &mut Paper, incoming: Paper) {
    let Paper {
        abstract_text,
        url,
        pdf_url,
        code_url,
        venue,
        citation_count,
        doi,
        arxiv_id,
        s2_paper_id,
        openalex_id,
        source_meta,
        ..
    } = incoming;

    if existing.abstract_text.is_none() {
        existing.abstract_text = abstract_text;
    }
    if existing.url.is_none() {
        existing.url = url;
    }
    if existing.pdf_url.is_none() {
        existing.pdf_url = pdf_url;
    }
    if existing.code_url.is_none() {
        existing.code_url = code_url;
    }
    if existing.venue.is_none() {
        existing.venue = venue;
    }
    if existing.doi.is_none() {
        existing.doi = doi;
    }
    if existing.arxiv_id.is_none() {
        existing.arxiv_id = arxiv_id;
    }
    if existing.s2_paper_id.is_none() {
        existing.s2_paper_id = s2_paper_id;
    }
    if existing.openalex_id.is_none() {
        existing.openalex_id = openalex_id;
    }
    if existing.citation_count.unwrap_or(0) < citation_count.unwrap_or(0) {
        existing.citation_count = citation_count;
    }
    if existing.source_meta.is_none() {
        existing.source_meta = source_meta;
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use crate::paper_id::PaperId;
    use crate::paper_id::PaperIdResolver;
    use crate::types::Paper;

    #[test]
    fn parses_and_formats_all_supported_prefixes() {
        let cases = [
            ("doi:10.1000/ABC", "doi:10.1000/abc"),
            ("arxiv:2301.12345v2", "arxiv:2301.12345v2"),
            ("s2:abc123", "s2:abc123"),
            ("openalex:W123456", "openalex:W123456"),
        ];

        for (input, expected) in cases {
            let id = PaperId::from_str(input).expect("paper id should parse");
            assert_eq!(id.to_string(), expected);
            let round_trip = PaperId::from_str(&id.to_string()).expect("round trip should parse");
            assert_eq!(round_trip, id);
        }
    }

    #[test]
    fn infers_ids_without_prefix() {
        assert_eq!(
            PaperId::from_str("10.1145/123.456")
                .expect("doi infer")
                .to_string(),
            "doi:10.1145/123.456"
        );
        assert_eq!(
            PaperId::from_str("2301.12345v1")
                .expect("arxiv infer")
                .to_string(),
            "arxiv:2301.12345v1"
        );
        assert_eq!(
            PaperId::from_str("W4398752345")
                .expect("openalex infer")
                .to_string(),
            "openalex:W4398752345"
        );
    }

    #[test]
    fn normalizes_common_doi_url_variants() {
        let cases = [
            ("doi:10.1000/ABC", "doi:10.1000/abc"),
            ("doi:https://doi.org/10.1000/ABC", "doi:10.1000/abc"),
            ("doi:http://doi.org/10.1000/ABC", "doi:10.1000/abc"),
            ("doi:https://dx.doi.org/10.1000/ABC", "doi:10.1000/abc"),
            ("doi:http://dx.doi.org/10.1000/ABC", "doi:10.1000/abc"),
        ];

        for (input, expected) in cases {
            let parsed = PaperId::from_str(input).expect("doi variant should parse");
            assert_eq!(parsed.to_string(), expected);
        }
    }

    #[test]
    fn dedup_merges_identifier_fields_from_secondary_records() {
        let resolver = PaperIdResolver;
        let papers = vec![
            make_paper(
                "Paper",
                Some("10.1000/shared"),
                None,
                Some("s2-1"),
                None,
                Some(5),
            ),
            make_paper(
                "Paper",
                Some("https://doi.org/10.1000/shared"),
                Some("2301.12345v1"),
                None,
                Some("W123456"),
                Some(10),
            ),
        ];

        let deduped = resolver.dedup_papers(papers);
        assert_eq!(deduped.len(), 1);

        let paper = &deduped[0];
        assert_eq!(paper.doi.as_deref(), Some("10.1000/shared"));
        assert_eq!(paper.arxiv_id.as_deref(), Some("2301.12345v1"));
        assert_eq!(paper.s2_paper_id.as_deref(), Some("s2-1"));
        assert_eq!(paper.openalex_id.as_deref(), Some("W123456"));
        assert_eq!(paper.citation_count, Some(10));
    }

    #[test]
    fn dedup_preserves_first_seen_order_for_relevance() {
        let resolver = PaperIdResolver;
        let papers = vec![
            make_paper("First", Some("10.1000/first"), None, None, None, Some(1)),
            make_paper(
                "Second",
                Some("10.1000/second"),
                None,
                None,
                None,
                Some(100),
            ),
            make_paper(
                "First duplicate",
                Some("https://doi.org/10.1000/first"),
                None,
                None,
                None,
                Some(2),
            ),
        ];

        let deduped = resolver.dedup_papers(papers);
        let titles = deduped
            .iter()
            .map(|paper| paper.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["First", "Second"]);
    }

    fn make_paper(
        title: &str,
        doi: Option<&str>,
        arxiv_id: Option<&str>,
        s2_paper_id: Option<&str>,
        openalex_id: Option<&str>,
        citation_count: Option<u32>,
    ) -> Paper {
        Paper {
            title: title.to_string(),
            authors: String::new(),
            year: None,
            venue: None,
            citation_count,
            abstract_text: None,
            doi: doi.map(ToString::to_string),
            arxiv_id: arxiv_id.map(ToString::to_string),
            s2_paper_id: s2_paper_id.map(ToString::to_string),
            openalex_id: openalex_id.map(ToString::to_string),
            url: None,
            pdf_url: None,
            code_url: None,
            source_meta: None,
        }
    }
}
