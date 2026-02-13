use std::sync::OnceLock;

use regex::Regex;

use crate::types::ZoteroCitationFormat;
use crate::types::ZoteroItemDetail;

pub(super) fn fallback_citation_for_item(
    item: &ZoteroItemDetail,
    format: ZoteroCitationFormat,
) -> (String, Option<String>) {
    match format {
        ZoteroCitationFormat::Bibtex => {
            let year = extract_year_from_date(item.date.as_deref());
            let citation_key = fallback_citation_key(item, year.as_deref());
            let citation = fallback_bibtex_citation(item, citation_key.as_str(), year.as_deref());
            (citation, Some(citation_key))
        }
        ZoteroCitationFormat::CslJson => (fallback_csl_json_citation(item), None),
        ZoteroCitationFormat::Apa => (fallback_apa_citation(item), None),
    }
}

fn fallback_bibtex_citation(
    item: &ZoteroItemDetail,
    citation_key: &str,
    year: Option<&str>,
) -> String {
    let entry_type = fallback_bibtex_entry_type(item.item_type.as_str());
    let mut fields = Vec::new();

    let title = item.title.trim();
    if !title.is_empty() {
        fields.push(("title", escape_bibtex_value(title)));
    }

    if !item.authors.is_empty() {
        let authors = item
            .authors
            .iter()
            .map(|author| escape_bibtex_value(author.trim()))
            .collect::<Vec<_>>()
            .join(" and ");
        if !authors.trim().is_empty() {
            fields.push(("author", authors));
        }
    }

    if let Some(year) = year {
        fields.push(("year", year.to_string()));
    }

    if let Some(publication) = item
        .publication
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match entry_type {
            "article" => fields.push(("journal", escape_bibtex_value(publication))),
            "inproceedings" => fields.push(("booktitle", escape_bibtex_value(publication))),
            _ => {}
        }
    }

    if let Some(doi) = item
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        fields.push(("doi", escape_bibtex_value(doi)));
    }

    if let Some(url) = item
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        fields.push(("url", escape_bibtex_value(url)));
    }

    let mut lines = vec![format!("@{entry_type}{{{citation_key},")];
    for (name, value) in fields {
        lines.push(format!("  {name} = {{{value}}},"));
    }
    lines.push("}".to_string());
    lines.join("\n")
}

fn fallback_csl_json_citation(item: &ZoteroItemDetail) -> String {
    let mut map = serde_json::Map::new();
    map.insert(
        "id".to_string(),
        serde_json::Value::String(item.key.clone()),
    );
    map.insert(
        "type".to_string(),
        serde_json::Value::String(fallback_csl_item_type(item.item_type.as_str()).to_string()),
    );

    let title = item.title.trim();
    if !title.is_empty() {
        map.insert(
            "title".to_string(),
            serde_json::Value::String(title.to_string()),
        );
    }

    if !item.authors.is_empty() {
        let authors = item
            .authors
            .iter()
            .map(|author| author.trim())
            .filter(|author| !author.is_empty())
            .map(|author| serde_json::json!({ "literal": author }))
            .collect::<Vec<_>>();
        if !authors.is_empty() {
            map.insert("author".to_string(), serde_json::Value::Array(authors));
        }
    }

    if let Some(year) = extract_year_from_date(item.date.as_deref())
        && let Ok(year_value) = year.parse::<u32>()
    {
        map.insert(
            "issued".to_string(),
            serde_json::json!({ "date-parts": [[year_value]] }),
        );
    }

    if let Some(publication) = item
        .publication
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        map.insert(
            "container-title".to_string(),
            serde_json::Value::String(publication.to_string()),
        );
    }

    if let Some(doi) = item
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        map.insert(
            "DOI".to_string(),
            serde_json::Value::String(doi.to_string()),
        );
    }

    if let Some(url) = item
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        map.insert(
            "URL".to_string(),
            serde_json::Value::String(url.to_string()),
        );
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .unwrap_or_else(|err| format!("{{\"serialization_error\":\"{err}\"}}"))
}

pub(super) fn fallback_apa_citation(item: &ZoteroItemDetail) -> String {
    let author_text = item
        .authors
        .iter()
        .map(|author| author.trim())
        .filter(|author| !author.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let author_text = if author_text.is_empty() {
        "Unknown author".to_string()
    } else {
        author_text
    };
    let year = extract_year_from_date(item.date.as_deref()).unwrap_or_else(|| "n.d.".to_string());
    let title = if item.title.trim().is_empty() {
        "Untitled item"
    } else {
        item.title.trim()
    };

    let mut citation = format!("{author_text} ({year}). {title}.");
    if let Some(publication) = item
        .publication
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        citation.push(' ');
        citation.push_str(publication);
        if !publication.ends_with('.') {
            citation.push('.');
        }
    }

    if let Some(doi) = item
        .doi
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        citation.push(' ');
        if doi.starts_with("10.") {
            citation.push_str(&format!("https://doi.org/{doi}"));
        } else {
            citation.push_str(doi);
        }
    } else if let Some(url) = item
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        citation.push(' ');
        citation.push_str(url);
    }

    citation
}

fn fallback_bibtex_entry_type(item_type: &str) -> &'static str {
    match item_type.trim().to_ascii_lowercase().as_str() {
        "journalarticle" => "article",
        "book" => "book",
        "conferencepaper" => "inproceedings",
        _ => "misc",
    }
}

fn fallback_csl_item_type(item_type: &str) -> &'static str {
    match item_type.trim().to_ascii_lowercase().as_str() {
        "journalarticle" => "article-journal",
        "book" => "book",
        "conferencepaper" => "paper-conference",
        _ => "article",
    }
}

fn fallback_citation_key(item: &ZoteroItemDetail, year: Option<&str>) -> String {
    let suffix = sanitize_citation_key_component(item.key.as_str());
    if let Some(author) = first_author_key_component(item.authors.as_slice()) {
        if let Some(year) = year {
            return format!("{author}{year}{suffix}");
        }
        return format!("{author}{suffix}");
    }

    if let Some(year) = year {
        return format!("item{year}{suffix}");
    }

    format!("item{suffix}")
}

fn first_author_key_component(authors: &[String]) -> Option<String> {
    let first_author = authors.first()?.trim();
    if first_author.is_empty() {
        return None;
    }

    let surname = first_author
        .split(',')
        .next()
        .unwrap_or(first_author)
        .split_whitespace()
        .last()
        .unwrap_or(first_author);
    let sanitized = surname
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if sanitized.is_empty() {
        return None;
    }

    Some(sanitized)
}

fn sanitize_citation_key_component(raw: &str) -> String {
    let sanitized = raw
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    if sanitized.is_empty() {
        return "item".to_string();
    }

    sanitized
}

pub(super) fn escape_bibtex_value(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => escaped.push_str("\\&"),
            '%' => escaped.push_str("\\%"),
            '#' => escaped.push_str("\\#"),
            '_' => escaped.push_str("\\_"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '~' => escaped.push_str("\\textasciitilde{}"),
            '^' => escaped.push_str("\\textasciicircum{}"),
            '\\' => escaped.push_str("\\textbackslash{}"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

fn extract_year_from_date(date: Option<&str>) -> Option<String> {
    static YEAR_RE: OnceLock<Regex> = OnceLock::new();
    let year_re = YEAR_RE.get_or_init(|| {
        Regex::new(r"(19|20)\d{2}")
            .unwrap_or_else(|err| panic!("year extraction regex must compile: {err}"))
    });

    let raw = date?.trim();
    if raw.is_empty() {
        return None;
    }

    year_re.find(raw).map(|match_| match_.as_str().to_string())
}
