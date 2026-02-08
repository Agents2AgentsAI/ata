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
    value
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .to_ascii_lowercase()
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
        let mut deduped: HashMap<String, Paper> = HashMap::new();

        for paper in papers {
            let key = self
                .canonical_id_for_paper(&paper)
                .map_or_else(|| paper.title.to_lowercase(), |id| id.to_string());

            if let Some(existing) = deduped.get_mut(&key) {
                merge_paper(existing, paper);
            } else {
                deduped.insert(key, paper);
            }
        }

        deduped.into_values().collect()
    }
}

fn merge_paper(existing: &mut Paper, incoming: Paper) {
    if existing.abstract_text.is_none() {
        existing.abstract_text = incoming.abstract_text.clone();
    }
    if existing.url.is_none() {
        existing.url = incoming.url.clone();
    }
    if existing.pdf_url.is_none() {
        existing.pdf_url = incoming.pdf_url.clone();
    }
    if existing.code_url.is_none() {
        existing.code_url = incoming.code_url.clone();
    }
    if existing.venue.is_none() {
        existing.venue = incoming.venue.clone();
    }
    if existing.openalex_id.is_none() {
        existing.openalex_id = incoming.openalex_id.clone();
    }
    if existing.citation_count.unwrap_or(0) < incoming.citation_count.unwrap_or(0) {
        existing.citation_count = incoming.citation_count;
    }
    if existing.source_meta.is_none() {
        existing.source_meta = incoming.source_meta;
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use crate::paper_id::PaperId;

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
}
