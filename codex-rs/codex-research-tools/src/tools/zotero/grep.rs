use std::collections::HashMap;
use std::collections::HashSet;

use futures::StreamExt;
use futures::stream;

use crate::ResearchToolkit;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroChildrenRequest;
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

use super::DEFAULT_CHILDREN_LIMIT;
use super::DEFAULT_FULLTEXT_MAX_CHARS;
use super::NormalizedScope;
use super::content_collector::collect_grep_segments;
use super::match_engine::build_grep_matcher;
use super::match_engine::build_snippet;
use super::normalize_optional_string;
use super::resolve_scope;
use super::to_normalized_scope;
use super::to_scope;
use super::zotero_config;

const DEFAULT_GREP_LIMIT_ITEMS: u32 = 50;
const DEFAULT_GREP_LIMIT_MATCHES: u32 = 100;
const DEFAULT_GREP_MAX_MATCHES_PER_ITEM: u32 = 50;
const DEFAULT_GREP_CONTEXT_CHARS: u32 = 120;
const DEFAULT_GREP_FETCH_CONCURRENCY: usize = 6;
const DEFAULT_GREP_ANNOTATION_PREFETCH_MAX_PAGES: usize = 8;
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
    pub(super) parent_item_key: Option<String>,
    pub(super) source_meta: Option<SourceMeta>,
}

#[derive(Debug, Default)]
pub(super) struct LibraryAnnotationPrefetch {
    pub(super) by_parent: HashMap<String, Vec<zotero::ZoteroAnnotation>>,
    pub(super) complete: bool,
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
        append_truncation_warnings(&normalized, &mut warnings);
        if normalized.candidate_strategy == ZoteroGrepCandidateStrategy::RecentModified {
            warnings.push(
                "candidate_strategy=recent_modified scans only a bounded recent set; results may be incomplete"
                    .to_string(),
            );
        }

        let parent_scoped_notes = if field_enabled(&normalized.fields, ZoteroGrepField::Note) {
            if let Some(parent_item_key) = normalized.parent_item_key.as_deref() {
                let result = toolkit
                    .zotero_get_notes(crate::types::ZoteroItemParams {
                        item_key: parent_item_key.to_string(),
                        library_type: Some(normalized.scope.library_type.clone()),
                        library_id: Some(normalized.scope.library_id.clone()),
                        max_chars_per_item: normalized.max_chars_per_item,
                    })
                    .await;
                match result {
                    Ok(notes) => notes
                        .notes
                        .into_iter()
                        .map(|note| (note.key.clone(), note))
                        .collect::<HashMap<_, _>>(),
                    Err(err) => {
                        warnings.push(format!(
                            "failed to load parent-scoped notes for {parent_item_key}: {err}"
                        ));
                        HashMap::new()
                    }
                }
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let parent_scoped_annotations = if field_enabled(&normalized.fields, ZoteroGrepField::Annotation)
            && normalized.candidate_strategy == ZoteroGrepCandidateStrategy::ParentScoped
        {
            if let Some(parent_item_key) = normalized.parent_item_key.as_deref() {
                let config = zotero_config(toolkit);
                let scope = to_scope(&normalized.scope);
                let result = zotero::get_annotations(
                    toolkit.http(),
                    config,
                    &scope,
                    &ZoteroChildrenRequest {
                        item_key: parent_item_key,
                        offset: 0,
                        limit: DEFAULT_CHILDREN_LIMIT,
                    },
                )
                .await;
                match result {
                    Ok(annotations) => annotations
                        .annotations
                        .into_iter()
                        .map(|annotation| (annotation.key.clone(), annotation))
                        .collect::<HashMap<_, _>>(),
                    Err(err) => {
                        warnings.push(format!(
                            "failed to load parent-scoped annotations for {parent_item_key}: {err}"
                        ));
                        HashMap::new()
                    }
                }
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let (library_annotation_prefetch, prefetch_warnings) = if field_enabled(
            &normalized.fields,
            ZoteroGrepField::Annotation,
        ) && normalized.candidate_strategy != ZoteroGrepCandidateStrategy::ParentScoped
        {
            prefetch_library_annotations(toolkit, &normalized).await
        } else {
            (LibraryAnnotationPrefetch::default(), Vec::new())
        };
        warnings.extend(prefetch_warnings);

        let normalized_ref = &normalized;
        let parent_scoped_notes_ref = &parent_scoped_notes;
        let parent_scoped_annotations_ref = &parent_scoped_annotations;
        let library_annotation_prefetch_ref = &library_annotation_prefetch;
        let mut collected_by_candidate = stream::iter(candidates.iter().cloned().enumerate())
            .map(|(idx, candidate)| async move {
                let collected = collect_grep_segments(
                    toolkit,
                    normalized_ref,
                    &candidate,
                    parent_scoped_notes_ref,
                    parent_scoped_annotations_ref,
                    library_annotation_prefetch_ref,
                )
                .await;
                (idx, collected)
            })
            .buffer_unordered(DEFAULT_GREP_FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        collected_by_candidate.sort_by_key(|(idx, _)| *idx);

        let mut collected_matches = Vec::new();
        let mut truncated = false;

        for (_, collected) in collected_by_candidate {
            warnings.extend(collected.warnings);

            let mut matches_for_item = 0usize;
            for segment in collected.segments {
                for hit in matcher.regex.find_iter(segment.text.as_str()) {
                    collected_matches.push(ZoteroGrepMatch {
                        item_key: segment.item_key.clone(),
                        field: segment.field.to_string(),
                        match_text: segment.text[hit.start()..hit.end()].to_string(),
                        snippet: build_snippet(
                            segment.text.as_str(),
                            hit.start(),
                            hit.end(),
                            normalized.context_chars as usize,
                        ),
                        parent_item_key: segment.parent_item_key.clone(),
                        source_meta: segment.source_meta.clone(),
                    });
                    matches_for_item = matches_for_item.saturating_add(1);

                    if matches_for_item >= normalized.max_matches_per_item as usize {
                        break;
                    }
                    if collected_matches.len() >= normalized.limit_matches as usize {
                        truncated = true;
                        break;
                    }
                }

                if matches_for_item >= normalized.max_matches_per_item as usize
                    || collected_matches.len() >= normalized.limit_matches as usize
                {
                    break;
                }
            }

            if collected_matches.len() >= normalized.limit_matches as usize {
                break;
            }
        }

        let returned_matches = collected_matches.len();
        let hints = if returned_matches == 0 {
            build_empty_grep_hints(&normalized)
        } else {
            Vec::new()
        };

        Ok(ZoteroGrepResult {
            candidate_strategy: normalized.candidate_strategy,
            scanned_items: candidates.len(),
            returned_matches,
            truncated,
            warnings,
            hints,
            matches: collected_matches,
        })
    })
    .await
    .map_err(|_| ResearchError::Timeout {
        api: ResearchApi::Zotero,
        timeout_ms,
    })?
}

pub(super) fn field_enabled(fields: &[ZoteroGrepField], field: ZoteroGrepField) -> bool {
    fields.contains(&field)
}

fn normalize_grep_params(
    toolkit: &ResearchToolkit,
    params: ZoteroGrepParams,
) -> Result<NormalizedGrepParams> {
    let library_type = normalize_optional_string(params.library_type);
    let library_id = normalize_optional_string(params.library_id);

    let pattern = params.pattern.trim().to_string();
    if pattern.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_grep_text pattern must not be empty".to_string(),
        ));
    }

    let scope = resolve_scope(
        toolkit,
        library_type.as_deref(),
        library_id.as_deref(),
        "zotero_grep_text",
    )?;

    let parent_item_key = normalize_optional_string(params.parent_item_key);
    let query_hint = normalize_optional_string(params.query_hint);

    let fields = normalize_grep_fields(params.fields)?;
    let candidate_strategy = if parent_item_key.is_some() {
        ZoteroGrepCandidateStrategy::ParentScoped
    } else if query_hint.is_some() {
        ZoteroGrepCandidateStrategy::QueryFiltered
    } else {
        ZoteroGrepCandidateStrategy::RecentModified
    };

    Ok(NormalizedGrepParams {
        pattern,
        match_mode: params.match_mode.unwrap_or(ZoteroGrepMatchMode::Literal),
        case_sensitive: params.case_sensitive.unwrap_or(false),
        scope_explicit: library_type.is_some() && library_id.is_some(),
        scope: to_normalized_scope(&scope),
        parent_item_key,
        query_hint,
        item_type: normalize_optional_string(params.item_type),
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

fn normalize_grep_fields(fields: Option<Vec<ZoteroGrepField>>) -> Result<Vec<ZoteroGrepField>> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();

    let requested = fields.unwrap_or_else(|| DEFAULT_GREP_FIELDS.to_vec());

    for field in requested {
        if seen.insert(field) {
            deduped.push(field);
        }
    }

    if deduped.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_grep_text requires at least one field".to_string(),
        ));
    }

    Ok(deduped)
}

fn append_truncation_warnings(normalized: &NormalizedGrepParams, warnings: &mut Vec<String>) {
    if let Some(max_chars) = normalized.max_chars_per_item {
        warnings.push(format!(
            "max_chars_per_item={max_chars} truncates text segments before matching; matches beyond this boundary may be missed"
        ));
        return;
    }

    if field_enabled(&normalized.fields, ZoteroGrepField::Fulltext) {
        warnings.push(format!(
            "fulltext search uses default {DEFAULT_FULLTEXT_MAX_CHARS} character cap per item; matches beyond this cap may be missed. Set max_chars_per_item to adjust."
        ));
    }
}

fn build_empty_grep_hints(normalized: &NormalizedGrepParams) -> Vec<String> {
    let mut hints = Vec::new();

    if !normalized.scope_explicit {
        hints.push(
            "No explicit library scope provided. Verify with zotero_list_groups or zotero_get_collections."
                .to_string(),
        );
    }
    if normalized.pattern.split_whitespace().count() > 3 {
        hints.push("Try fewer or broader search terms.".to_string());
    }
    if grep_uses_restricted_fields(normalized.fields.as_slice()) {
        hints.push("Try broadening the fields list (e.g. add 'title', 'abstract').".to_string());
    }

    hints
}

fn grep_uses_restricted_fields(fields: &[ZoteroGrepField]) -> bool {
    fields.len() != DEFAULT_GREP_FIELDS.len()
        || DEFAULT_GREP_FIELDS
            .iter()
            .any(|default_field| !fields.contains(default_field))
}

async fn load_grep_candidates(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
) -> Result<(Vec<GrepCandidate>, Vec<String>)> {
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);

    let (raw_items, parent_override) = match normalized.candidate_strategy {
        ZoteroGrepCandidateStrategy::ParentScoped => {
            let parent_item_key = normalized.parent_item_key.as_deref().ok_or_else(|| {
                ResearchError::Internal(
                    "zotero_grep_text missing parent item key for parent scoped search".to_string(),
                )
            })?;
            let children = zotero::get_children_items(
                toolkit.http(),
                config,
                &scope,
                &ZoteroChildrenRequest {
                    item_key: parent_item_key,
                    offset: 0,
                    limit: normalized.limit_items,
                },
                normalized.item_type.as_deref(),
            )
            .await?;
            (children.items, Some(parent_item_key.to_string()))
        }
        ZoteroGrepCandidateStrategy::QueryFiltered => {
            let query = normalized.query_hint.as_deref().ok_or_else(|| {
                ResearchError::Internal(
                    "zotero_grep_text missing query hint for query filtered search".to_string(),
                )
            })?;
            let result = zotero::search_items(
                toolkit.http(),
                config,
                &scope,
                &ZoteroSearchRequest {
                    query: Some(query),
                    tag: None,
                    offset: 0,
                    limit: normalized.limit_items,
                    item_type: normalized.item_type.as_deref(),
                    sort: None,
                    direction: None,
                },
            )
            .await?;
            (result.items, None)
        }
        ZoteroGrepCandidateStrategy::RecentModified => {
            let result = zotero::search_items(
                toolkit.http(),
                config,
                &scope,
                &ZoteroSearchRequest {
                    query: None,
                    tag: None,
                    offset: 0,
                    limit: normalized.limit_items,
                    item_type: normalized.item_type.as_deref(),
                    sort: Some("dateModified"),
                    direction: Some("desc"),
                },
            )
            .await?;
            (result.items, None)
        }
    };

    let candidates = raw_items
        .into_iter()
        .map(|item| GrepCandidate {
            key: item.key,
            title: item.title,
            item_type: item.item_type,
            tags: item.tags,
            parent_item_key: parent_override.clone(),
            source_meta: item.source_meta,
        })
        .collect::<Vec<_>>();

    Ok((candidates, Vec::new()))
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
                limit: DEFAULT_CHILDREN_LIMIT,
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
