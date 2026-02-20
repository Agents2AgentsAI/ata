use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;

use crate::ResearchToolkit;
use crate::types::DocumentResolution;
use crate::types::DocumentSourceKind;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroItemDetail;

use super::ARXIV_PDF_BASE_URL;
use super::ZOTERO_STORAGE_DIR_ENV;

pub(super) async fn resolve_document_sources(
    toolkit: &ResearchToolkit,
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    has_indexed_content: Option<bool>,
    trace: Vec<String>,
) -> DocumentResolution {
    resolve_document_sources_with_storage_root(
        item,
        attachments,
        toolkit.config().zotero_storage_dir.as_deref(),
        has_indexed_content,
        trace,
    )
    .await
}

pub(super) async fn resolve_document_sources_with_storage_root(
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    storage_root: Option<&str>,
    has_indexed_content: Option<bool>,
    mut trace: Vec<String>,
) -> DocumentResolution {
    let attachment_urls = attachment_pdf_urls(attachments, &mut trace);

    if let Some(item) = item
        && let Some(arxiv_id) = extract_arxiv_id(item, &mut trace)
    {
        let arxiv_pdf = format!("{ARXIV_PDF_BASE_URL}/{arxiv_id}.pdf");
        trace.push("using arXiv PDF".to_string());
        let mut fallback_urls = Vec::new();
        for url in attachment_urls {
            if url != arxiv_pdf {
                push_unique_url(&mut fallback_urls, url);
            }
        }
        return DocumentResolution {
            source_kind: DocumentSourceKind::ArxivPdf,
            preferred_url: Some(arxiv_pdf),
            fallback_urls,
            local_path: None,
            trace,
        };
    }

    if let Some(preferred_url) = attachment_urls.first().cloned() {
        let fallback_urls = attachment_urls.into_iter().skip(1).collect::<Vec<_>>();
        trace.push("using attachment PDF URL".to_string());
        return DocumentResolution {
            source_kind: DocumentSourceKind::AttachmentPdfUrl,
            preferred_url: Some(preferred_url),
            fallback_urls,
            local_path: None,
            trace,
        };
    }

    if let Some(local_path) = resolve_local_pdf_path(storage_root, attachments, &mut trace) {
        return DocumentResolution {
            source_kind: DocumentSourceKind::LocalPdfPath,
            preferred_url: None,
            fallback_urls: Vec::new(),
            local_path: Some(local_path),
            trace,
        };
    }

    match has_indexed_content {
        Some(true) => {
            trace.push(
                "no canonical document source resolved; using indexed fulltext fallback"
                    .to_string(),
            );
        }
        Some(false) => {
            trace.push(
                "no canonical document source resolved and indexed fulltext is empty".to_string(),
            );
        }
        None => {
            trace.push(
                "no canonical document source resolved; indexed fulltext availability unknown"
                    .to_string(),
            );
        }
    }

    DocumentResolution {
        source_kind: DocumentSourceKind::IndexedFulltext,
        preferred_url: None,
        fallback_urls: Vec::new(),
        local_path: None,
        trace,
    }
}

fn attachment_pdf_urls(attachments: &[ZoteroAttachment], trace: &mut Vec<String>) -> Vec<String> {
    let mut urls = Vec::new();
    for attachment in attachments {
        let Some(raw_url) = attachment.url.as_deref() else {
            continue;
        };
        let Some(url) = sanitize_http_url(raw_url) else {
            trace.push(format!(
                "ignored non-http attachment URL for attachment {}",
                attachment.key
            ));
            continue;
        };
        if attachment_looks_like_pdf(attachment, Some(url.as_str()), attachment.path.as_deref()) {
            push_unique_url(&mut urls, url);
        }
    }
    urls
}

fn attachment_looks_like_pdf(
    attachment: &ZoteroAttachment,
    url: Option<&str>,
    path_hint: Option<&str>,
) -> bool {
    if attachment
        .content_type
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/pdf"))
    {
        return true;
    }
    if attachment.filename.as_deref().is_some_and(has_pdf_suffix) {
        return true;
    }
    if attachment.title.as_deref().is_some_and(has_pdf_suffix) {
        return true;
    }
    if url.is_some_and(is_pdf_url) {
        return true;
    }
    path_hint.is_some_and(has_pdf_suffix)
}

fn has_pdf_suffix(value: &str) -> bool {
    value.trim().to_ascii_lowercase().ends_with(".pdf")
}

fn is_pdf_url(url: &str) -> bool {
    if let Ok(parsed) = reqwest::Url::parse(url) {
        return parsed.path().to_ascii_lowercase().ends_with(".pdf");
    }

    has_pdf_suffix(url)
}

fn sanitize_http_url(raw_url: &str) -> Option<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parsed = reqwest::Url::parse(trimmed).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

fn resolve_local_pdf_path(
    storage_root: Option<&str>,
    attachments: &[ZoteroAttachment],
    trace: &mut Vec<String>,
) -> Option<String> {
    let Some(storage_root) = storage_root
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    else {
        trace.push(format!(
            "local path fallback unavailable: {ZOTERO_STORAGE_DIR_ENV} is not set"
        ));
        return None;
    };

    if !storage_root.is_absolute() {
        trace.push(format!(
            "local path fallback unavailable: {ZOTERO_STORAGE_DIR_ENV} must be an absolute path"
        ));
        return None;
    }

    for attachment in attachments {
        let Some(path_hint) = attachment
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        if !attachment_looks_like_pdf(attachment, attachment.url.as_deref(), Some(path_hint)) {
            continue;
        }

        let Some(storage_relative) = path_hint.strip_prefix("storage:") else {
            trace.push(format!(
                "ignored attachment path for {} because it does not use storage: prefix",
                attachment.key
            ));
            continue;
        };

        if !is_safe_storage_component(attachment.key.as_str()) {
            trace.push(format!(
                "ignored attachment path for {} because attachment key is not path-safe",
                attachment.key
            ));
            continue;
        }

        let relative_path = Path::new(storage_relative.trim_start_matches('/'));
        if !is_safe_relative_path(relative_path) {
            trace.push(format!(
                "ignored attachment path for {} due to unsafe relative path",
                attachment.key
            ));
            continue;
        }

        let local_path = storage_root.join(&attachment.key).join(relative_path);
        let Some(local_path) = local_path.to_str() else {
            trace.push(format!(
                "ignored local path for {} because the resolved path is non-UTF-8",
                attachment.key
            ));
            continue;
        };
        trace.push(format!(
            "resolved local PDF path from attachment {}",
            attachment.key
        ));
        return Some(local_path.to_string());
    }

    None
}

fn is_safe_storage_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_safe_relative_path(path: &Path) -> bool {
    let mut has_component = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_component = true,
            Component::CurDir => {}
            _ => return false,
        }
    }
    has_component
}

fn extract_arxiv_id(item: &ZoteroItemDetail, trace: &mut Vec<String>) -> Option<String> {
    if let Some(url) = item.url.as_deref()
        && let Some(arxiv_id) = extract_arxiv_id_from_url(url)
    {
        trace.push(format!("detected arXiv id from item URL: {arxiv_id}"));
        return Some(arxiv_id);
    }

    if let Some(extra) = item.extra.as_deref()
        && let Some(arxiv_id) = extract_arxiv_id_from_extra(extra)
    {
        trace.push(format!("detected arXiv id from item extra: {arxiv_id}"));
        return Some(arxiv_id);
    }

    None
}

fn extract_arxiv_id_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !(host == "arxiv.org" || host.ends_with(".arxiv.org")) {
        return None;
    }

    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    let (prefix, suffix_segments) = segments.split_first()?;

    let candidate = if *prefix == "abs" || *prefix == "pdf" {
        suffix_segments.join("/")
    } else {
        return None;
    };

    let candidate = candidate.strip_suffix(".pdf").unwrap_or(candidate.as_str());
    normalize_arxiv_id(candidate)
}

fn extract_arxiv_id_from_extra(extra: &str) -> Option<String> {
    arxiv_id_regex()
        .captures(extra)
        .and_then(|capture| capture.get(1))
        .map(|match_| match_.as_str().to_string())
}

pub(super) fn normalize_arxiv_id(raw_value: &str) -> Option<String> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    let mut prefix = String::new();
    for _ in 0..6 {
        if let Some(ch) = chars.next() {
            prefix.push(ch);
        } else {
            prefix.clear();
            break;
        }
    }
    let without_prefix = if prefix.eq_ignore_ascii_case("arxiv:") {
        chars.as_str()
    } else {
        trimmed
    };
    let cleaned = without_prefix.trim().trim_end_matches(".pdf").trim();

    arxiv_id_regex()
        .captures(cleaned)
        .and_then(|capture| capture.get(1))
        .map(|match_| match_.as_str().to_string())
}

fn arxiv_id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b((?:\d{4}\.\d{4,5}|[a-z][a-z.\-]+/\d{7})(?:v\d+)?)\b")
            .unwrap_or_else(|err| panic!("arXiv id regex must compile: {err}"))
    })
}

fn push_unique_url(urls: &mut Vec<String>, candidate: String) {
    if !urls.iter().any(|existing| existing == &candidate) {
        urls.push(candidate);
    }
}
