use std::collections::HashMap;
use std::collections::HashSet;

use futures::StreamExt;
use futures::stream;

use crate::ResearchToolkit;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroChildrenItemsRequest;
use crate::clients::zotero::ZoteroSearchRequest;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::types::SourceMeta;
use crate::types::ZoteroGrepCandidateStrategy;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepMatch;
use crate::types::ZoteroGrepMatchMode;
use crate::types::ZoteroGrepParams;
use crate::types::ZoteroGrepResult;
use crate::types::ZoteroItem;
use crate::types::ZoteroSearchResult;

use super::NormalizedScope;
use super::content_collector::collect_grep_segments;
use super::match_engine::build_grep_matcher;
use super::to_normalized_scope;
use super::to_scope;
use super::zotero_config;

const DEFAULT_GREP_ANNOTATION_PREFETCH_MAX_PAGES: usize = 8;
const DEFAULT_GREP_LIMIT_ITEMS: u32 = 50;
const DEFAULT_GREP_LIMIT_MATCHES: u32 = 100;
const DEFAULT_GREP_MAX_MATCHES_PER_ITEM: u32 = 50;
const DEFAULT_GREP_CONTEXT_CHARS: u32 = 120;
const DEFAULT_GREP_FETCH_CONCURRENCY: usize = 6;
const DEFAULT_GREP_FIELDS: [ZoteroGrepField; 7] = [
    ZoteroGrepField::Title,
    ZoteroGrepField::Abstract,
    ZoteroGrepField::Extra,
    ZoteroGrepField::Note,
    ZoteroGrepField::Annotation,
    ZoteroGrepField::Fulltext,
    ZoteroGrepField::Tag,
];

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct NormalizedGrepParams {
    pub(super) pattern: String,
    pub(super) match_mode: ZoteroGrepMatchMode,
    pub(super) case_sensitive: bool,
    pub(super) scope_explicit: bool,
    pub(super) scope: NormalizedScope,
    pub(super) parent_item_key: Option<String>,
    pub(super) query_hint: Option<String>,
    pub(super) item_type: Option<String>,
    pub(super) fields: Vec<ZoteroGrepField>,
    pub(super) limit_items: u32,
    pub(super) limit_matches: u32,
    pub(super) max_matches_per_item: u32,
    pub(super) context_chars: u32,
    pub(super) max_chars_per_item: Option<u32>,
    pub(super) candidate_strategy: ZoteroGrepCandidateStrategy,
}

#[derive(Debug, Clone)]
pub(super) struct GrepCandidate {
    pub(super) key: String,
    pub(super) title: String,
    pub(super) item_type: String,
    pub(super) tags: Vec<String>,
    pub(super) source_meta: Option<SourceMeta>,
}

#[derive(Debug, Default)]
pub(super) struct LibraryAnnotationPrefetch {
    pub(super) by_parent: HashMap<String, Vec<zotero::ZoteroAnnotation>>,
    pub(super) complete: bool,
}

#[derive(Debug)]
struct CollectedCandidateSegments {
    index: usize,
    candidate: GrepCandidate,
    warnings: Vec<String>,
    segments: Vec<super::content_collector::GrepTextSegment>,
}

pub(super) fn field_enabled(fields: &[ZoteroGrepField], field: ZoteroGrepField) -> bool {
    fields.contains(&field)
}

pub(super) async fn zotero_grep_text(
    toolkit: &ResearchToolkit,
    params: ZoteroGrepParams,
) -> Result<ZoteroGrepResult> {
    let normalized = normalize_grep_params(toolkit, params)?;
    let matcher = build_grep_matcher(
        normalized.pattern.as_str(),
        &normalized.match_mode,
        normalized.case_sensitive,
    )?;
    let tool_timeout = toolkit.config().tool_timeout;
    let timeout_ms = u64::try_from(tool_timeout.as_millis()).unwrap_or(u64::MAX);

    tokio::time::timeout(tool_timeout, async move {
        let (candidates, mut warnings) = load_grep_candidates(toolkit, &normalized).await?;
        let scanned_items = candidates.len();

        if normalized.parent_item_key.is_some() && normalized.query_hint.is_some() {
            warnings.push("query_hint is ignored when parent_item_key is provided".to_string());
        }

        let (library_annotation_prefetch, prefetch_warnings) = if normalized.candidate_strategy
            != ZoteroGrepCandidateStrategy::ParentScoped
            && field_enabled(&normalized.fields, ZoteroGrepField::Annotation)
        {
            prefetch_library_annotations(toolkit, &normalized).await
        } else {
            (LibraryAnnotationPrefetch::default(), Vec::new())
        };
        warnings.extend(prefetch_warnings);

        let parent_scoped_notes = HashMap::new();
        let parent_scoped_annotations = HashMap::new();
        let normalized_ref = &normalized;
        let parent_scoped_notes_ref = &parent_scoped_notes;
        let parent_scoped_annotations_ref = &parent_scoped_annotations;
        let library_annotation_prefetch_ref = &library_annotation_prefetch;
        let mut collected = stream::iter(candidates.into_iter().enumerate())
            .map(|(index, candidate)| async move {
                let collection = collect_grep_segments(
                    toolkit,
                    normalized_ref,
                    &candidate,
                    parent_scoped_notes_ref,
                    parent_scoped_annotations_ref,
                    library_annotation_prefetch_ref,
                )
                .await;
                CollectedCandidateSegments {
                    index,
                    candidate,
                    warnings: collection.warnings,
                    segments: collection.segments,
                }
            })
            .buffer_unordered(DEFAULT_GREP_FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        collected.sort_by_key(|entry| entry.index);

        let mut matches = Vec::new();
        let mut truncated = false;
        let parent_item_key =
            if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::ParentScoped {
                normalized.parent_item_key.clone()
            } else {
                None
            };

        'candidates: for entry in collected {
            warnings.extend(entry.warnings);
            let mut matches_for_item = 0usize;
            for segment in entry.segments {
                if matches_for_item >= normalized.max_matches_per_item as usize {
                    break;
                }
                for hit in matcher.regex.find_iter(segment.text.as_str()) {
                    let match_text = segment.text[hit.start()..hit.end()].to_string();
                    let snippet = build_match_snippet(
                        segment.text.as_str(),
                        hit.start(),
                        hit.end(),
                        normalized.context_chars,
                    );
                    matches.push(ZoteroGrepMatch {
                        item_key: entry.candidate.key.clone(),
                        field: segment.field.to_string(),
                        match_text,
                        snippet,
                        parent_item_key: parent_item_key.clone(),
                        source_meta: entry.candidate.source_meta.clone(),
                    });
                    matches_for_item = matches_for_item.saturating_add(1);
                    if matches.len() >= normalized.limit_matches as usize {
                        truncated = true;
                        break 'candidates;
                    }
                    if matches_for_item >= normalized.max_matches_per_item as usize {
                        break;
                    }
                }
            }
        }

        let hints = if matches.is_empty() {
            build_empty_grep_hints(&normalized)
        } else {
            Vec::new()
        };

        Ok(ZoteroGrepResult {
            candidate_strategy: normalized.candidate_strategy,
            scanned_items,
            returned_matches: matches.len(),
            truncated,
            warnings,
            hints,
            matches,
        })
    })
    .await
    .map_err(|_| ResearchError::Timeout {
        api: ResearchApi::Zotero,
        timeout_ms,
    })?
}

fn normalize_grep_params(
    toolkit: &ResearchToolkit,
    params: ZoteroGrepParams,
) -> Result<NormalizedGrepParams> {
    let pattern = params.pattern.trim().to_string();
    if pattern.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_grep_text pattern must not be empty".to_string(),
        ));
    }

    let library_type = super::normalize_optional_string(params.library_type);
    let library_id = super::normalize_optional_string(params.library_id);
    let scope = super::resolve_scope(
        toolkit,
        library_type.as_deref(),
        library_id.as_deref(),
        "zotero_grep_text",
    )?;

    let parent_item_key = super::normalize_optional_string(params.parent_item_key);
    let query_hint = super::normalize_optional_string(params.query_hint);
    let item_type = super::normalize_optional_string(params.item_type);
    let candidate_strategy = if parent_item_key.is_some() {
        ZoteroGrepCandidateStrategy::ParentScoped
    } else if query_hint.is_some() {
        ZoteroGrepCandidateStrategy::QueryFiltered
    } else {
        ZoteroGrepCandidateStrategy::RecentModified
    };

    let raw_fields = params
        .fields
        .unwrap_or_else(|| DEFAULT_GREP_FIELDS.to_vec());
    let mut fields = Vec::new();
    let mut seen_fields = HashSet::new();
    for field in raw_fields {
        if seen_fields.insert(field) {
            fields.push(field);
        }
    }
    if fields.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_grep_text fields must contain at least one field".to_string(),
        ));
    }

    Ok(NormalizedGrepParams {
        pattern,
        match_mode: params.match_mode.unwrap_or(ZoteroGrepMatchMode::Literal),
        case_sensitive: params.case_sensitive.unwrap_or(false),
        scope_explicit: library_type.is_some() && library_id.is_some(),
        scope: to_normalized_scope(&scope),
        parent_item_key,
        query_hint,
        item_type,
        fields,
        limit_items: params
            .limit_items
            .unwrap_or(DEFAULT_GREP_LIMIT_ITEMS)
            .clamp(1, 200),
        limit_matches: params
            .limit_matches
            .unwrap_or(DEFAULT_GREP_LIMIT_MATCHES)
            .clamp(1, 500),
        max_matches_per_item: params
            .max_matches_per_item
            .unwrap_or(DEFAULT_GREP_MAX_MATCHES_PER_ITEM)
            .clamp(1, 200),
        context_chars: params
            .context_chars
            .unwrap_or(DEFAULT_GREP_CONTEXT_CHARS)
            .clamp(20, 400),
        max_chars_per_item: params.max_chars_per_item,
        candidate_strategy,
    })
}

async fn load_grep_candidates(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> Result<(Vec<GrepCandidate>, Vec<String>)> {
    match normalized.candidate_strategy {
        ZoteroGrepCandidateStrategy::ParentScoped => {
            load_parent_scoped_candidates(toolkit, normalized).await
        }
        ZoteroGrepCandidateStrategy::QueryFiltered
        | ZoteroGrepCandidateStrategy::RecentModified => {
            load_library_candidates(toolkit, normalized).await
        }
    }
}

async fn load_library_candidates(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> Result<(Vec<GrepCandidate>, Vec<String>)> {
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut offset = 0u32;

    loop {
        let remaining = normalized
            .limit_items
            .saturating_sub(u32::try_from(candidates.len()).unwrap_or(u32::MAX));
        if remaining == 0 {
            break;
        }

        let page_limit = remaining.min(super::ZOTERO_MAX_PAGE_SIZE);
        let request = ZoteroSearchRequest {
            query: if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::QueryFiltered {
                normalized.query_hint.as_deref()
            } else {
                None
            },
            tag: None,
            offset,
            limit: page_limit,
            item_type: normalized.item_type.as_deref(),
            sort: if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::RecentModified {
                Some("dateModified")
            } else {
                None
            },
            direction: if normalized.candidate_strategy
                == ZoteroGrepCandidateStrategy::RecentModified
            {
                Some("desc")
            } else {
                None
            },
        };

        let page = zotero::search_items(toolkit.http(), config, &scope, &request).await?;
        let fetched = page.items.len();
        if fetched == 0 {
            break;
        }

        for item in page.items {
            if seen_keys.insert(item.key.clone()) {
                candidates.push(map_item_to_candidate(item));
            }
        }

        if !page.has_more {
            break;
        }

        offset = offset.saturating_add(u32::try_from(fetched).unwrap_or(0));
        if candidates.len() >= normalized.limit_items as usize {
            break;
        }
    }

    if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::RecentModified {
        warnings.push(
            "candidate_strategy=recent_modified scans only a bounded recent set; results may be incomplete"
                .to_string(),
        );
    }

    Ok((candidates, warnings))
}

async fn load_parent_scoped_candidates(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> Result<(Vec<GrepCandidate>, Vec<String>)> {
    let parent_item_key = normalized.parent_item_key.as_deref().ok_or_else(|| {
        ResearchError::Internal("parent-scoped grep missing parent_item_key".to_string())
    })?;

    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut candidates = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut offset = 0u32;

    loop {
        let remaining = normalized
            .limit_items
            .saturating_sub(u32::try_from(candidates.len()).unwrap_or(u32::MAX));
        if remaining == 0 {
            break;
        }

        let page_limit = remaining.min(super::ZOTERO_MAX_PAGE_SIZE);
        let page: ZoteroSearchResult = zotero::get_children_items(
            toolkit.http(),
            config,
            &scope,
            &ZoteroChildrenItemsRequest {
                item_key: parent_item_key,
                offset,
                limit: page_limit,
                item_type: normalized.item_type.as_deref(),
            },
        )
        .await?;
        let fetched = page.items.len();
        if fetched == 0 {
            break;
        }

        for item in page.items {
            if seen_keys.insert(item.key.clone()) {
                candidates.push(map_item_to_candidate(item));
            }
        }

        if !page.has_more {
            break;
        }

        offset = offset.saturating_add(u32::try_from(fetched).unwrap_or(0));
        if candidates.len() >= normalized.limit_items as usize {
            break;
        }
    }

    Ok((candidates, Vec::new()))
}

fn map_item_to_candidate(item: ZoteroItem) -> GrepCandidate {
    GrepCandidate {
        key: item.key,
        title: item.title,
        item_type: item.item_type,
        tags: item.tags,
        source_meta: item.source_meta,
    }
}

fn build_match_snippet(text: &str, start: usize, end: usize, context_chars: u32) -> String {
    let context_chars = context_chars as usize;
    let prefix_chars = text[..start].chars().count();
    let match_chars = text[start..end].chars().count();
    let total_chars = text.chars().count();
    let snippet_start = prefix_chars.saturating_sub(context_chars);
    let snippet_end = prefix_chars
        .saturating_add(match_chars)
        .saturating_add(context_chars)
        .min(total_chars);
    let core = text
        .chars()
        .skip(snippet_start)
        .take(snippet_end.saturating_sub(snippet_start))
        .collect::<String>();

    let mut snippet = core;
    if snippet_start > 0 {
        snippet = format!("...{snippet}");
    }
    if snippet_end < total_chars {
        snippet = format!("{snippet}...");
    }

    snippet
}

fn build_empty_grep_hints(normalized: &NormalizedGrepParams) -> Vec<String> {
    let mut hints = Vec::new();

    if !normalized.scope_explicit {
        hints.push(
            "No explicit library scope provided. Verify with zotero_list_groups or zotero_get_collections."
                .to_string(),
        );
    }
    if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::RecentModified {
        hints.push(
            "Try setting query_hint to prefilter candidates or increase limit_items.".to_string(),
        );
    }
    if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::ParentScoped {
        hints.push("Verify parent_item_key and that the parent has matching children.".to_string());
    }
    if normalized.fields.len() < DEFAULT_GREP_FIELDS.len() {
        hints.push("Try searching additional fields.".to_string());
    }
    if normalized.match_mode == ZoteroGrepMatchMode::Regex {
        hints.push("Try match_mode='literal' if your regex is too strict.".to_string());
    }
    hints.push("Try broader terms.".to_string());

    hints
}

pub(super) async fn prefetch_library_annotations(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> (LibraryAnnotationPrefetch, Vec<String>) {
    let mut warnings = Vec::new();
    let mut prefetch = LibraryAnnotationPrefetch::default();

    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut offset = 0u32;
    let mut page_count = 0usize;

    loop {
        if page_count >= DEFAULT_GREP_ANNOTATION_PREFETCH_MAX_PAGES {
            warnings.push(
                "annotation prefetch hit page cap; falling back to per-item annotation fetch for uncovered items"
                    .to_string(),
            );
            break;
        }

        let page = match zotero::get_library_annotations(
            toolkit.http(),
            config,
            &scope,
            zotero::ZoteroLibraryAnnotationsRequest {
                offset,
                limit: super::DEFAULT_CHILDREN_LIMIT,
            },
        )
        .await
        {
            Ok(page) => page,
            Err(err) => {
                warnings.push(format!("annotation prefetch failed: {err}"));
                break;
            }
        };

        page_count = page_count.saturating_add(1);
        let fetched_count = page.annotations.len();
        for annotation in page.annotations {
            if let Some(parent_item) = annotation.parent_item.clone() {
                prefetch
                    .by_parent
                    .entry(parent_item)
                    .or_default()
                    .push(annotation);
            }
        }

        if !page.has_more {
            prefetch.complete = true;
            break;
        }
        if fetched_count == 0 {
            warnings.push(
                "annotation prefetch stopped early because upstream returned an empty page with has_more=true"
                    .to_string(),
            );
            break;
        }

        offset = offset.saturating_add(u32::try_from(fetched_count).unwrap_or(0));
    }

    (prefetch, warnings)
}
