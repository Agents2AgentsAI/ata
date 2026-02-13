use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::cache::FetchOutput;
use crate::error::ResearchError;
use crate::rate_limiter::ResearchApi;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::DocumentResolution;
use crate::types::DocumentSourceKind;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroItemDetail;

use super::AR5IV_BASE_URL;
use super::AR5IV_PROBE_CACHE_TTL;
use super::AR5IV_PROBE_MAX_BODY_BYTES;
use super::AR5IV_PROBE_NEGATIVE_CACHE_TTL;
use super::AR5IV_PROBE_TIMEOUT;
use super::ARXIV_PDF_BASE_URL;
use super::ZOTERO_STORAGE_DIR_ENV;

pub(super) async fn resolve_document_sources(
    toolkit: &ResearchToolkit,
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    has_indexed_content: Option<bool>,
    trace: Vec<String>,
) -> DocumentResolution {
    resolve_document_sources_with_probe(
        item,
        attachments,
        toolkit.config().zotero_storage_dir.as_deref(),
        has_indexed_content,
        trace,
        |arxiv_id, html_url| async move {
            ar5iv_available_cached(toolkit, arxiv_id.as_str(), html_url.as_str()).await
        },
    )
    .await
}

pub(super) async fn resolve_document_sources_with_probe<F, Fut>(
    item: Option<&ZoteroItemDetail>,
    attachments: &[ZoteroAttachment],
    storage_root: Option<&str>,
    has_indexed_content: Option<bool>,
    mut trace: Vec<String>,
    mut probe_ar5iv: F,
) -> DocumentResolution
where
    F: FnMut(String, String) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let attachment_urls = attachment_pdf_urls(attachments, &mut trace);

    if let Some(item) = item
        && let Some(arxiv_id) = extract_arxiv_id(item, &mut trace)
    {
        let arxiv_pdf = format!("{ARXIV_PDF_BASE_URL}/{arxiv_id}.pdf");
        for probe_id in ar5iv_probe_candidates(arxiv_id.as_str()) {
            let html_url = format!("{AR5IV_BASE_URL}/{probe_id}");
            if probe_ar5iv(probe_id.clone(), html_url.clone()).await {
                trace.push(format!("ar5iv probe succeeded for {probe_id}"));
                let mut fallback_urls = Vec::new();
                push_unique_url(&mut fallback_urls, arxiv_pdf.clone());
                for url in &attachment_urls {
                    push_unique_url(&mut fallback_urls, url.clone());
                }
                return DocumentResolution {
                    source_kind: DocumentSourceKind::Ar5ivHtml,
                    preferred_url: Some(html_url),
                    fallback_urls,
                    local_path: None,
                    trace,
                };
            }
        }

        trace.push("ar5iv probe unavailable; using arXiv PDF".to_string());
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

pub(super) fn ar5iv_probe_candidates(arxiv_id: &str) -> Vec<String> {
    let mut candidates = vec![arxiv_id.to_string()];
    if let Some((without_version, version_suffix)) = arxiv_id.rsplit_once('v')
        && !without_version.is_empty()
        && !version_suffix.is_empty()
        && version_suffix.chars().all(|ch| ch.is_ascii_digit())
        && without_version != arxiv_id
    {
        candidates.push(without_version.to_string());
    }
    candidates
}

fn push_unique_url(urls: &mut Vec<String>, candidate: String) {
    if !urls.iter().any(|existing| existing == &candidate) {
        urls.push(candidate);
    }
}

async fn ar5iv_available_cached(toolkit: &ResearchToolkit, arxiv_id: &str, html_url: &str) -> bool {
    let params_hash = match hash_cache_payload(&arxiv_id) {
        Ok(hash) => hash,
        Err(err) => {
            tracing::warn!(%err, arxiv_id, "failed to hash ar5iv probe cache key");
            return false;
        }
    };

    let key = CacheKey {
        tool_name: "zotero_ar5iv_probe",
        params_hash,
    };

    match toolkit
        .cache()
        .get_or_fetch_with_meta_ttls(
            key,
            AR5IV_PROBE_CACHE_TTL,
            AR5IV_PROBE_NEGATIVE_CACHE_TTL,
            || async move {
                let probe_result = probe_ar5iv_html(toolkit, html_url).await;
                let data = serde_json::to_value(probe_result).map_err(|err| {
                    ResearchError::Internal(format!(
                        "failed to serialize ar5iv probe result: {err}"
                    ))
                })?;
                if probe_result {
                    Ok(FetchOutput::positive(data))
                } else {
                    Ok(FetchOutput::negative(data))
                }
            },
        )
        .await
    {
        Ok(output) => match serde_json::from_value::<bool>(output.data) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(%err, arxiv_id, "failed to deserialize ar5iv probe result");
                false
            }
        },
        Err(err) => {
            tracing::warn!(%err, arxiv_id, "ar5iv probe failed");
            false
        }
    }
}

async fn probe_ar5iv_html(toolkit: &ResearchToolkit, html_url: &str) -> bool {
    let response = match tokio::time::timeout(
        AR5IV_PROBE_TIMEOUT,
        toolkit
            .http()
            .execute_response(ResearchApi::Arxiv, || toolkit.http().client().get(html_url)),
    )
    .await
    {
        Ok(Ok(response)) => response,
        _ => return false,
    };

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_ascii_lowercase);
    if !content_type
        .as_deref()
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return false;
    }

    let html_prefix = match tokio::time::timeout(
        AR5IV_PROBE_TIMEOUT,
        read_response_prefix(response, AR5IV_PROBE_MAX_BODY_BYTES),
    )
    .await
    {
        Ok(Some(prefix)) => prefix,
        _ => return false,
    };

    let normalized_html = html_prefix.to_ascii_lowercase();
    ![
        "no paper found for arxiv id",
        "this paper is not yet available",
        "unable to fetch source",
        "ar5iv is temporarily unavailable",
    ]
    .iter()
    .any(|needle| normalized_html.contains(needle))
}

async fn read_response_prefix(mut response: reqwest::Response, max_bytes: usize) -> Option<String> {
    let mut bytes = Vec::with_capacity(max_bytes);
    while bytes.len() < max_bytes {
        let chunk = response.chunk().await.ok()??;
        let remaining = max_bytes.saturating_sub(bytes.len());
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Some(String::from_utf8_lossy(&bytes).to_string())
}
