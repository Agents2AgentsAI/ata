use std::collections::HashMap;
use std::collections::HashSet;

use futures::StreamExt;
use futures::stream;

use crate::ResearchToolkit;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroSearchRequest;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::types::ZoteroAdvancedCandidateStrategy;
use crate::types::ZoteroAdvancedCompleteness;
use crate::types::ZoteroAdvancedJoinMode;
use crate::types::ZoteroAdvancedSearchParams;
use crate::types::ZoteroAdvancedSearchResult;
use crate::types::ZoteroAdvancedSortBy;
use crate::types::ZoteroGrepCandidateStrategy;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepMatchMode;
use crate::types::ZoteroItem;
use crate::types::ZoteroItemParams;
use crate::types::ZoteroSearchCondition;
use crate::types::ZoteroSearchConditionField;
use crate::types::ZoteroSearchConditionOperation;
use crate::types::ZoteroSearchResult;
use crate::types::ZoteroSortDirection;

use super::DEFAULT_FULLTEXT_MAX_CHARS;
use super::NormalizedScope;
use super::apply_items_budget;
use super::content_collector::collect_grep_segments;
use super::grep;
use super::grep::GrepCandidate;
use super::grep::LibraryAnnotationPrefetch;
use super::grep::NormalizedGrepParams;
use super::match_engine::GrepMatcher;
use super::match_engine::build_grep_matcher;
use super::normalize_optional_string;
use super::resolve_scope;
use super::to_normalized_scope;
use super::to_scope;
use super::zotero_config;

const DEFAULT_ADVANCED_LIMIT: u32 = 25;
const DEFAULT_ADVANCED_FETCH_CONCURRENCY: usize = 6;
const DEFAULT_ADVANCED_CANDIDATE_LIMIT: u32 = 500;

#[derive(Debug)]
struct NormalizedAdvancedParams {
    conditions: Vec<NormalizedCondition>,
    join_mode: ZoteroAdvancedJoinMode,
    sort_by: ZoteroAdvancedSortBy,
    sort_direction: ZoteroSortDirection,
    scope_explicit: bool,
    scope: NormalizedScope,
    offset: u32,
    limit: u32,
    item_type: Option<String>,
    max_chars_per_item: Option<u32>,
    candidate_strategy: ZoteroAdvancedCandidateStrategy,
    server_tag_filter: Option<String>,
    candidate_limit: u32,
    collector_fields: Vec<ZoteroGrepField>,
    requires_publication_detail: bool,
    requires_doi_detail: bool,
}

#[derive(Debug)]
struct NormalizedCondition {
    field: ZoteroSearchConditionField,
    operation: ZoteroSearchConditionOperation,
    value: Option<String>,
    case_sensitive: bool,
    matcher: Option<GrepMatcher>,
    year_value: Option<i32>,
}

#[derive(Debug, Clone)]
struct AdvancedCandidate {
    summary: ZoteroItem,
    grep_candidate: GrepCandidate,
}

#[derive(Debug)]
struct EvaluatedCandidate {
    index: usize,
    matched: bool,
    item: ZoteroItem,
    warnings: Vec<String>,
}

#[derive(Debug)]
struct CollectedFieldText {
    notes: Vec<String>,
    annotations: Vec<String>,
    fulltext: Vec<String>,
}

pub(super) async fn zotero_advanced_search(
    toolkit: &ResearchToolkit,
    params: ZoteroAdvancedSearchParams,
) -> Result<ZoteroAdvancedSearchResult> {
    let normalized = normalize_advanced_search_params(toolkit, params)?;
    let tool_timeout = toolkit.config().tool_timeout;
    let timeout_ms = u64::try_from(tool_timeout.as_millis()).unwrap_or(u64::MAX);

    tokio::time::timeout(tool_timeout, async move {
        let (candidate_items, candidate_truncated, mut warnings) =
            load_advanced_candidates(toolkit, &normalized).await?;
        let scanned_items = candidate_items.len();
        append_truncation_warnings(&normalized, &mut warnings);

        if normalized.candidate_strategy == ZoteroAdvancedCandidateStrategy::RecentModifiedFallback {
            warnings.push(
                "candidate_strategy=recent_modified_fallback scans only a bounded recent set; results may be incomplete"
                    .to_string(),
            );
        }
        if candidate_truncated {
            warnings.push(
                "advanced candidate pool reached cap; results may be incomplete".to_string(),
            );
        }

        let collector_params = build_collection_params(&normalized);
        let (library_annotation_prefetch, prefetch_warnings) = if normalized
            .collector_fields
            .contains(&ZoteroGrepField::Annotation)
        {
            grep::prefetch_library_annotations(toolkit, &collector_params).await
        } else {
            (LibraryAnnotationPrefetch::default(), Vec::new())
        };
        warnings.extend(prefetch_warnings);

        let normalized_ref = &normalized;
        let collector_params_ref = &collector_params;
        let library_annotation_prefetch_ref = &library_annotation_prefetch;
        let mut evaluated = stream::iter(
            candidate_items
                .into_iter()
                .map(|summary| AdvancedCandidate {
                    grep_candidate: to_grep_candidate(&summary),
                    summary,
                })
                .enumerate(),
        )
            .map(|(index, candidate)| async move {
                evaluate_candidate(
                    toolkit,
                    normalized_ref,
                    collector_params_ref,
                    library_annotation_prefetch_ref,
                    index,
                    candidate,
                )
                .await
            })
            .buffer_unordered(DEFAULT_ADVANCED_FETCH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        evaluated.sort_by_key(|candidate| candidate.index);

        let mut filtered = Vec::new();
        for evaluated_candidate in evaluated {
            warnings.extend(evaluated_candidate.warnings);
            if evaluated_candidate.matched {
                filtered.push(evaluated_candidate.item);
            }
        }

        apply_advanced_sort(
            filtered.as_mut_slice(),
            &normalized.sort_by,
            &normalized.sort_direction,
            &mut warnings,
        );

        let total = filtered.len();
        let offset = normalized.offset as usize;
        let limit = normalized.limit as usize;
        let mut items = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        apply_items_budget(items.as_mut_slice(), normalized.max_chars_per_item);

        let hints = if total == 0 {
            build_empty_advanced_hints(&normalized)
        } else {
            Vec::new()
        };

        let completeness = if normalized.candidate_strategy
            == ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
            || candidate_truncated
        {
            ZoteroAdvancedCompleteness::Approximate
        } else {
            ZoteroAdvancedCompleteness::Exact
        };

        Ok(ZoteroAdvancedSearchResult {
            completeness,
            candidate_strategy: normalized.candidate_strategy,
            scanned_items,
            warnings,
            hints,
            results: ZoteroSearchResult {
                has_more: offset + items.len() < total,
                total_available: Some(u64::try_from(total).unwrap_or(u64::MAX)),
                items,
            },
        })
    })
    .await
    .map_err(|_| ResearchError::Timeout {
        api: ResearchApi::Zotero,
        timeout_ms,
    })?
}

fn normalize_advanced_search_params(
    toolkit: &ResearchToolkit,
    params: ZoteroAdvancedSearchParams,
) -> Result<NormalizedAdvancedParams> {
    let library_type = normalize_optional_string(params.library_type);
    let library_id = normalize_optional_string(params.library_id);
    let scope = resolve_scope(
        toolkit,
        library_type.as_deref(),
        library_id.as_deref(),
        "zotero_advanced_search",
    )?;
    let join_mode = params.join_mode.unwrap_or(ZoteroAdvancedJoinMode::All);
    let conditions = normalize_advanced_conditions(params.conditions)?;
    let item_type = normalize_optional_string(params.item_type);
    let server_tag_filter = derive_server_tag_filter(&conditions, &join_mode);
    let condition_item_type = derive_server_item_type_filter(&conditions, &join_mode);
    let effective_item_type = item_type.or(condition_item_type);
    let candidate_strategy = if server_tag_filter.is_some() || effective_item_type.is_some() {
        ZoteroAdvancedCandidateStrategy::ServerFiltered
    } else {
        ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
    };

    let collector_fields = derive_collector_fields(&conditions);
    let requires_publication_detail = conditions
        .iter()
        .any(|condition| condition.field == ZoteroSearchConditionField::Publication);
    let requires_doi_detail = conditions
        .iter()
        .any(|condition| condition.field == ZoteroSearchConditionField::Doi);

    Ok(NormalizedAdvancedParams {
        conditions,
        join_mode,
        sort_by: params.sort_by.unwrap_or(ZoteroAdvancedSortBy::Relevance),
        sort_direction: params.sort_direction.unwrap_or(ZoteroSortDirection::Asc),
        scope_explicit: library_type.is_some() && library_id.is_some(),
        scope: to_normalized_scope(&scope),
        offset: params.offset.unwrap_or(0),
        limit: params.limit.unwrap_or(DEFAULT_ADVANCED_LIMIT).clamp(1, 100),
        item_type: effective_item_type,
        max_chars_per_item: params.max_chars_per_item,
        candidate_strategy,
        server_tag_filter,
        candidate_limit: DEFAULT_ADVANCED_CANDIDATE_LIMIT,
        collector_fields,
        requires_publication_detail,
        requires_doi_detail,
    })
}

fn normalize_advanced_conditions(
    conditions: Vec<ZoteroSearchCondition>,
) -> Result<Vec<NormalizedCondition>> {
    if conditions.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_advanced_search requires at least one condition".to_string(),
        ));
    }

    conditions
        .into_iter()
        .map(normalize_advanced_condition)
        .collect()
}

fn normalize_advanced_condition(condition: ZoteroSearchCondition) -> Result<NormalizedCondition> {
    let field = condition.field;
    let operation = condition.operation;
    let value = normalize_optional_string(condition.value);
    let case_sensitive = condition.case_sensitive.unwrap_or(false);

    let requires_value = matches!(
        operation,
        ZoteroSearchConditionOperation::Contains
            | ZoteroSearchConditionOperation::NotContains
            | ZoteroSearchConditionOperation::Equals
            | ZoteroSearchConditionOperation::NotEquals
            | ZoteroSearchConditionOperation::Regex
            | ZoteroSearchConditionOperation::BeforeYear
            | ZoteroSearchConditionOperation::AfterYear
    );
    if requires_value && value.is_none() {
        return Err(ResearchError::InvalidInput(format!(
            "zotero_advanced_search condition {} on field {} requires non-empty value",
            format_operation(&operation),
            format_field(field)
        )));
    }

    if matches!(
        operation,
        ZoteroSearchConditionOperation::BeforeYear | ZoteroSearchConditionOperation::AfterYear
    ) && field != ZoteroSearchConditionField::Year
    {
        return Err(ResearchError::InvalidInput(
            "before_year/after_year operations require field='year'".to_string(),
        ));
    }

    let matcher = match operation {
        ZoteroSearchConditionOperation::Contains | ZoteroSearchConditionOperation::NotContains => {
            let pattern = value.as_deref().ok_or_else(|| {
                ResearchError::Internal(
                    "advanced condition missing value for contains/not_contains".to_string(),
                )
            })?;
            Some(build_grep_matcher(
                pattern,
                &ZoteroGrepMatchMode::Literal,
                case_sensitive,
            )?)
        }
        ZoteroSearchConditionOperation::Regex => {
            let pattern = value.as_deref().ok_or_else(|| {
                ResearchError::Internal("advanced condition missing value for regex".to_string())
            })?;
            Some(build_grep_matcher(
                pattern,
                &ZoteroGrepMatchMode::Regex,
                case_sensitive,
            )?)
        }
        ZoteroSearchConditionOperation::NotEquals
        | ZoteroSearchConditionOperation::Equals
        | ZoteroSearchConditionOperation::BeforeYear
        | ZoteroSearchConditionOperation::AfterYear
        | ZoteroSearchConditionOperation::IsEmpty
        | ZoteroSearchConditionOperation::IsNotEmpty => None,
    };

    let year_value = match operation {
        ZoteroSearchConditionOperation::BeforeYear | ZoteroSearchConditionOperation::AfterYear => {
            let raw = value.as_deref().ok_or_else(|| {
                ResearchError::Internal("advanced year condition missing value".to_string())
            })?;
            let parsed = raw.parse::<i32>().map_err(|error| {
                ResearchError::InvalidInput(format!(
                    "invalid year value '{raw}' for {}: {error}",
                    format_operation(&operation)
                ))
            })?;
            Some(parsed)
        }
        ZoteroSearchConditionOperation::Contains
        | ZoteroSearchConditionOperation::NotContains
        | ZoteroSearchConditionOperation::Equals
        | ZoteroSearchConditionOperation::NotEquals
        | ZoteroSearchConditionOperation::Regex
        | ZoteroSearchConditionOperation::IsEmpty
        | ZoteroSearchConditionOperation::IsNotEmpty => None,
    };

    Ok(NormalizedCondition {
        field,
        operation,
        value,
        case_sensitive,
        matcher,
        year_value,
    })
}

fn derive_server_tag_filter(
    conditions: &[NormalizedCondition],
    join_mode: &ZoteroAdvancedJoinMode,
) -> Option<String> {
    if *join_mode != ZoteroAdvancedJoinMode::All {
        return None;
    }

    conditions
        .iter()
        .find(|condition| {
            condition.field == ZoteroSearchConditionField::Tag
                && condition.operation == ZoteroSearchConditionOperation::Equals
        })
        .and_then(|condition| condition.value.clone())
}

fn derive_server_item_type_filter(
    conditions: &[NormalizedCondition],
    join_mode: &ZoteroAdvancedJoinMode,
) -> Option<String> {
    if *join_mode != ZoteroAdvancedJoinMode::All {
        return None;
    }

    conditions
        .iter()
        .find(|condition| {
            condition.field == ZoteroSearchConditionField::ItemType
                && condition.operation == ZoteroSearchConditionOperation::Equals
        })
        .and_then(|condition| condition.value.clone())
}

fn derive_collector_fields(conditions: &[NormalizedCondition]) -> Vec<ZoteroGrepField> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();
    for condition in conditions {
        let mapped = match condition.field {
            ZoteroSearchConditionField::Note => Some(ZoteroGrepField::Note),
            ZoteroSearchConditionField::Annotation => Some(ZoteroGrepField::Annotation),
            ZoteroSearchConditionField::Fulltext => Some(ZoteroGrepField::Fulltext),
            ZoteroSearchConditionField::Title
            | ZoteroSearchConditionField::Creator
            | ZoteroSearchConditionField::Year
            | ZoteroSearchConditionField::Tag
            | ZoteroSearchConditionField::ItemType
            | ZoteroSearchConditionField::Publication
            | ZoteroSearchConditionField::Doi => None,
        };

        if let Some(mapped_field) = mapped
            && seen.insert(mapped_field)
        {
            fields.push(mapped_field);
        }
    }
    fields
}

fn append_truncation_warnings(normalized: &NormalizedAdvancedParams, warnings: &mut Vec<String>) {
    let truncation_relevant = !normalized.collector_fields.is_empty()
        || normalized.requires_publication_detail
        || normalized.requires_doi_detail;
    if !truncation_relevant {
        return;
    }

    if let Some(max_chars) = normalized.max_chars_per_item {
        warnings.push(format!(
            "max_chars_per_item={max_chars} truncates fetched text before condition evaluation; matches beyond this boundary may be missed"
        ));
        return;
    }

    if normalized
        .collector_fields
        .contains(&ZoteroGrepField::Fulltext)
    {
        warnings.push(format!(
            "fulltext conditions use default {DEFAULT_FULLTEXT_MAX_CHARS} character cap per item; matches beyond this cap may be missed. Set max_chars_per_item to adjust."
        ));
    }
}

async fn load_advanced_candidates(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedAdvancedParams,
) -> Result<(Vec<ZoteroItem>, bool, Vec<String>)> {
    let config = zotero_config(toolkit);
    let scope = to_scope(&normalized.scope);
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut offset = 0u32;
    let mut truncated = false;

    loop {
        let remaining = normalized
            .candidate_limit
            .saturating_sub(u32::try_from(items.len()).unwrap_or(u32::MAX));
        if remaining == 0 {
            truncated = true;
            break;
        }

        let page_limit = remaining.min(100);
        let request = ZoteroSearchRequest {
            query: None,
            tag: normalized.server_tag_filter.as_deref(),
            offset,
            limit: page_limit,
            item_type: normalized.item_type.as_deref(),
            sort: if normalized.candidate_strategy
                == ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
            {
                Some("dateModified")
            } else {
                None
            },
            direction: if normalized.candidate_strategy
                == ZoteroAdvancedCandidateStrategy::RecentModifiedFallback
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
                items.push(item);
            }
        }

        if !page.has_more {
            break;
        }

        offset = offset.saturating_add(u32::try_from(fetched).unwrap_or(0));
        if items.len() >= normalized.candidate_limit as usize {
            truncated = true;
            break;
        }
    }

    if normalized.candidate_strategy == ZoteroAdvancedCandidateStrategy::ServerFiltered
        && normalized.server_tag_filter.is_some()
        && normalized.item_type.is_none()
    {
        warnings.push(
            "server candidate filter uses tag equality; additional non-tag conditions are evaluated client-side"
                .to_string(),
        );
    }

    Ok((items, truncated, warnings))
}

fn build_collection_params(normalized: &NormalizedAdvancedParams) -> NormalizedGrepParams {
    NormalizedGrepParams {
        pattern: String::new(),
        match_mode: ZoteroGrepMatchMode::Literal,
        case_sensitive: false,
        scope_explicit: normalized.scope_explicit,
        scope: normalized.scope.clone(),
        parent_item_key: None,
        query_hint: None,
        item_type: normalized.item_type.clone(),
        fields: normalized.collector_fields.clone(),
        limit_items: normalized.candidate_limit,
        limit_matches: 1,
        max_matches_per_item: 1,
        context_chars: 120,
        max_chars_per_item: normalized.max_chars_per_item,
        candidate_strategy: if normalized.candidate_strategy
            == ZoteroAdvancedCandidateStrategy::ServerFiltered
        {
            ZoteroGrepCandidateStrategy::QueryFiltered
        } else {
            ZoteroGrepCandidateStrategy::RecentModified
        },
    }
}

async fn evaluate_candidate(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedAdvancedParams,
    collection_params: &NormalizedGrepParams,
    library_annotation_prefetch: &LibraryAnnotationPrefetch,
    index: usize,
    candidate: AdvancedCandidate,
) -> EvaluatedCandidate {
    let mut warnings = Vec::new();
    let mut collected_text = CollectedFieldText {
        notes: Vec::new(),
        annotations: Vec::new(),
        fulltext: Vec::new(),
    };

    if !normalized.collector_fields.is_empty() {
        let parent_scoped_notes = HashMap::new();
        let parent_scoped_annotations = HashMap::new();
        let collected = collect_grep_segments(
            toolkit,
            collection_params,
            &candidate.grep_candidate,
            &parent_scoped_notes,
            &parent_scoped_annotations,
            library_annotation_prefetch,
        )
        .await;
        warnings.extend(collected.warnings);
        collected_text = map_segments_to_field_text(collected.segments);
    }

    let needs_detail = normalized.requires_publication_detail
        || (normalized.requires_doi_detail && candidate.summary.doi.is_none());
    let detail = if needs_detail {
        match toolkit
            .zotero_get_item(ZoteroItemParams {
                item_key: candidate.summary.key.clone(),
                library_type: Some(normalized.scope.library_type.clone()),
                library_id: Some(normalized.scope.library_id.clone()),
                max_chars_per_item: normalized.max_chars_per_item,
            })
            .await
        {
            Ok(item) => Some(item),
            Err(error) => {
                warnings.push(format!(
                    "failed to load item detail for {}: {error}",
                    candidate.summary.key
                ));
                None
            }
        }
    } else {
        None
    };

    let publication = detail.as_ref().and_then(|item| item.publication.as_deref());
    let detail_doi = detail.as_ref().and_then(|item| item.doi.as_deref());
    let matched = evaluate_joined_conditions(
        normalized.conditions.as_slice(),
        &normalized.join_mode,
        &candidate.summary,
        publication,
        detail_doi,
        &collected_text,
    );

    let mut item = candidate.summary;
    if item.doi.is_none()
        && let Some(doi) = detail.and_then(|detail| detail.doi)
    {
        item.doi = Some(doi);
    }

    EvaluatedCandidate {
        index,
        matched,
        item,
        warnings,
    }
}

fn evaluate_joined_conditions(
    conditions: &[NormalizedCondition],
    join_mode: &ZoteroAdvancedJoinMode,
    item: &ZoteroItem,
    publication: Option<&str>,
    detail_doi: Option<&str>,
    collected_text: &CollectedFieldText,
) -> bool {
    match join_mode {
        ZoteroAdvancedJoinMode::All => conditions.iter().all(|condition| {
            evaluate_condition(condition, item, publication, detail_doi, collected_text)
        }),
        ZoteroAdvancedJoinMode::Any => conditions.iter().any(|condition| {
            evaluate_condition(condition, item, publication, detail_doi, collected_text)
        }),
    }
}

fn evaluate_condition(
    condition: &NormalizedCondition,
    item: &ZoteroItem,
    publication: Option<&str>,
    detail_doi: Option<&str>,
    collected_text: &CollectedFieldText,
) -> bool {
    let values = values_for_field(
        condition.field,
        item,
        publication,
        detail_doi,
        collected_text,
    );

    match condition.operation {
        ZoteroSearchConditionOperation::Contains => {
            matches_with_matcher(values.as_slice(), condition.matcher.as_ref())
        }
        ZoteroSearchConditionOperation::NotContains => {
            !matches_with_matcher(values.as_slice(), condition.matcher.as_ref())
        }
        ZoteroSearchConditionOperation::Regex => {
            matches_with_matcher(values.as_slice(), condition.matcher.as_ref())
        }
        ZoteroSearchConditionOperation::Equals => values_match_exact(
            values.as_slice(),
            condition.value.as_deref(),
            condition.case_sensitive,
        ),
        ZoteroSearchConditionOperation::NotEquals => !values_match_exact(
            values.as_slice(),
            condition.value.as_deref(),
            condition.case_sensitive,
        ),
        ZoteroSearchConditionOperation::BeforeYear => values
            .iter()
            .filter_map(|value| parse_year(value))
            .any(|year| year < condition.year_value.unwrap_or(i32::MIN)),
        ZoteroSearchConditionOperation::AfterYear => values
            .iter()
            .filter_map(|value| parse_year(value))
            .any(|year| year > condition.year_value.unwrap_or(i32::MAX)),
        ZoteroSearchConditionOperation::IsEmpty => {
            values.is_empty() || values.iter().all(|value| value.trim().is_empty())
        }
        ZoteroSearchConditionOperation::IsNotEmpty => {
            values.iter().any(|value| !value.trim().is_empty())
        }
    }
}

fn values_for_field<'a>(
    field: ZoteroSearchConditionField,
    item: &'a ZoteroItem,
    publication: Option<&'a str>,
    detail_doi: Option<&'a str>,
    collected_text: &'a CollectedFieldText,
) -> Vec<&'a str> {
    match field {
        ZoteroSearchConditionField::Title => vec![item.title.as_str()],
        ZoteroSearchConditionField::Creator => {
            if item.authors.trim().is_empty() {
                Vec::new()
            } else {
                vec![item.authors.as_str()]
            }
        }
        ZoteroSearchConditionField::Year => item.year.as_deref().into_iter().collect(),
        ZoteroSearchConditionField::Tag => item.tags.iter().map(String::as_str).collect(),
        ZoteroSearchConditionField::ItemType => vec![item.item_type.as_str()],
        ZoteroSearchConditionField::Publication => publication.into_iter().collect(),
        ZoteroSearchConditionField::Doi => item.doi.as_deref().or(detail_doi).into_iter().collect(),
        ZoteroSearchConditionField::Note => {
            collected_text.notes.iter().map(String::as_str).collect()
        }
        ZoteroSearchConditionField::Annotation => collected_text
            .annotations
            .iter()
            .map(String::as_str)
            .collect(),
        ZoteroSearchConditionField::Fulltext => {
            collected_text.fulltext.iter().map(String::as_str).collect()
        }
    }
}

fn matches_with_matcher(values: &[&str], matcher: Option<&GrepMatcher>) -> bool {
    let Some(matcher) = matcher else {
        return false;
    };
    values.iter().any(|value| matcher.regex.is_match(value))
}

fn values_match_exact(values: &[&str], expected: Option<&str>, case_sensitive: bool) -> bool {
    let Some(expected) = expected else {
        return false;
    };

    values
        .iter()
        .any(|value| text_equals(value, expected, case_sensitive))
}

fn text_equals(actual: &str, expected: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        return actual == expected;
    }

    actual.to_lowercase() == expected.to_lowercase()
}

fn parse_year(value: &str) -> Option<i32> {
    let digits = value
        .chars()
        .filter(char::is_ascii_digit)
        .take(4)
        .collect::<String>();

    if digits.len() != 4 {
        return None;
    }

    digits.parse::<i32>().ok()
}

fn map_segments_to_field_text(
    segments: Vec<super::content_collector::GrepTextSegment>,
) -> CollectedFieldText {
    let mut out = CollectedFieldText {
        notes: Vec::new(),
        annotations: Vec::new(),
        fulltext: Vec::new(),
    };

    for segment in segments {
        match segment.field {
            "note" => out.notes.push(segment.text),
            "annotation_text" | "annotation_comment" => out.annotations.push(segment.text),
            "fulltext" => out.fulltext.push(segment.text),
            _ => {}
        }
    }

    out
}

fn to_grep_candidate(item: &ZoteroItem) -> GrepCandidate {
    GrepCandidate {
        key: item.key.clone(),
        title: item.title.clone(),
        item_type: item.item_type.clone(),
        tags: item.tags.clone(),
    }
}

fn apply_advanced_sort(
    items: &mut [ZoteroItem],
    sort_by: &ZoteroAdvancedSortBy,
    sort_direction: &ZoteroSortDirection,
    warnings: &mut Vec<String>,
) {
    match sort_by {
        ZoteroAdvancedSortBy::Title => {
            items.sort_by(|left, right| {
                left.title
                    .cmp(&right.title)
                    .then_with(|| left.key.cmp(&right.key))
            });
            if *sort_direction == ZoteroSortDirection::Desc {
                items.reverse();
            }
        }
        ZoteroAdvancedSortBy::Year => {
            items.sort_by(|left, right| {
                let left_year = left.year.as_deref().and_then(parse_year);
                let right_year = right.year.as_deref().and_then(parse_year);
                left_year
                    .cmp(&right_year)
                    .then_with(|| left.title.cmp(&right.title))
                    .then_with(|| left.key.cmp(&right.key))
            });
            if *sort_direction == ZoteroSortDirection::Desc {
                items.reverse();
            }
        }
        ZoteroAdvancedSortBy::Relevance => {
            if *sort_direction == ZoteroSortDirection::Desc {
                items.reverse();
            }
        }
        ZoteroAdvancedSortBy::DateAdded => {
            warnings.push(
                "sort_by=date_added is not available in summary payload; preserving candidate order"
                    .to_string(),
            );
        }
        ZoteroAdvancedSortBy::DateModified => {
            warnings.push(
                "sort_by=date_modified is not available in summary payload; preserving candidate order"
                    .to_string(),
            );
        }
    }
}

fn build_empty_advanced_hints(normalized: &NormalizedAdvancedParams) -> Vec<String> {
    let mut hints = Vec::new();

    if !normalized.scope_explicit {
        hints.push(
            "No explicit library scope provided. Verify with zotero_list_groups or zotero_get_collections."
                .to_string(),
        );
    }

    let has_long_term = normalized.conditions.iter().any(|condition| {
        condition
            .value
            .as_deref()
            .is_some_and(|value| value.split_whitespace().count() > 3)
    });
    if has_long_term {
        hints.push("Try fewer or broader search terms.".to_string());
    }

    hints.push("Try reducing conditions or switch join_mode to 'any'.".to_string());
    hints
}

fn format_field(field: ZoteroSearchConditionField) -> &'static str {
    match field {
        ZoteroSearchConditionField::Title => "title",
        ZoteroSearchConditionField::Creator => "creator",
        ZoteroSearchConditionField::Year => "year",
        ZoteroSearchConditionField::Tag => "tag",
        ZoteroSearchConditionField::ItemType => "item_type",
        ZoteroSearchConditionField::Publication => "publication",
        ZoteroSearchConditionField::Doi => "doi",
        ZoteroSearchConditionField::Note => "note",
        ZoteroSearchConditionField::Annotation => "annotation",
        ZoteroSearchConditionField::Fulltext => "fulltext",
    }
}

fn format_operation(operation: &ZoteroSearchConditionOperation) -> &'static str {
    match operation {
        ZoteroSearchConditionOperation::Contains => "contains",
        ZoteroSearchConditionOperation::NotContains => "not_contains",
        ZoteroSearchConditionOperation::Equals => "equals",
        ZoteroSearchConditionOperation::NotEquals => "not_equals",
        ZoteroSearchConditionOperation::Regex => "regex",
        ZoteroSearchConditionOperation::BeforeYear => "before_year",
        ZoteroSearchConditionOperation::AfterYear => "after_year",
        ZoteroSearchConditionOperation::IsEmpty => "is_empty",
        ZoteroSearchConditionOperation::IsNotEmpty => "is_not_empty",
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn build_item(key: &str, title: &str, authors: &str, year: Option<&str>) -> ZoteroItem {
        ZoteroItem {
            key: key.to_string(),
            title: title.to_string(),
            authors: authors.to_string(),
            year: year.map(ToString::to_string),
            item_type: "journalArticle".to_string(),
            doi: None,
            abstract_snippet: None,
            tags: Vec::new(),
            source_meta: None,
        }
    }

    fn normalized_condition(
        field: ZoteroSearchConditionField,
        operation: ZoteroSearchConditionOperation,
        value: Option<&str>,
        case_sensitive: Option<bool>,
    ) -> NormalizedCondition {
        normalize_advanced_condition(ZoteroSearchCondition {
            field,
            operation,
            value: value.map(ToString::to_string),
            case_sensitive,
        })
        .expect("condition normalization should succeed")
    }

    fn empty_collected() -> CollectedFieldText {
        CollectedFieldText {
            notes: Vec::new(),
            annotations: Vec::new(),
            fulltext: Vec::new(),
        }
    }

    #[test]
    fn condition_missing_field_semantics_follow_contract() {
        let item = build_item("I1", "Graph Models", "Alice Kim", Some("2024"));
        let collected = empty_collected();

        let contains = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::Contains,
            Some("neurips"),
            None,
        );
        assert!(
            !evaluate_condition(&contains, &item, None, None, &collected),
            "contains on missing field should be false"
        );

        let not_contains = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::NotContains,
            Some("neurips"),
            None,
        );
        assert!(
            evaluate_condition(&not_contains, &item, None, None, &collected),
            "not_contains on missing field should be true"
        );

        let equals = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::Equals,
            Some("neurips"),
            None,
        );
        assert!(
            !evaluate_condition(&equals, &item, None, None, &collected),
            "equals on missing field should be false"
        );

        let not_equals = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::NotEquals,
            Some("neurips"),
            None,
        );
        assert!(
            evaluate_condition(&not_equals, &item, None, None, &collected),
            "not_equals on missing field should be true"
        );

        let regex = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::Regex,
            Some("neur.*"),
            None,
        );
        assert!(
            !evaluate_condition(&regex, &item, None, None, &collected),
            "regex on missing field should be false"
        );

        let is_empty = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::IsEmpty,
            None,
            None,
        );
        assert!(
            evaluate_condition(&is_empty, &item, None, None, &collected),
            "is_empty on missing field should be true"
        );

        let is_not_empty = normalized_condition(
            ZoteroSearchConditionField::Publication,
            ZoteroSearchConditionOperation::IsNotEmpty,
            None,
            None,
        );
        assert!(
            !evaluate_condition(&is_not_empty, &item, None, None, &collected),
            "is_not_empty on missing field should be false"
        );
    }

    #[test]
    fn join_mode_all_and_any_behave_as_expected() {
        let item = build_item("I1", "Transformer Models", "Alice Kim", Some("2023"));
        let collected = empty_collected();
        let conditions = vec![
            normalized_condition(
                ZoteroSearchConditionField::Title,
                ZoteroSearchConditionOperation::Contains,
                Some("transformer"),
                Some(false),
            ),
            normalized_condition(
                ZoteroSearchConditionField::Creator,
                ZoteroSearchConditionOperation::Contains,
                Some("bob"),
                Some(false),
            ),
        ];

        assert!(
            !evaluate_joined_conditions(
                &conditions,
                &ZoteroAdvancedJoinMode::All,
                &item,
                None,
                None,
                &collected,
            ),
            "all join should fail when one condition does not match"
        );
        assert!(
            evaluate_joined_conditions(
                &conditions,
                &ZoteroAdvancedJoinMode::Any,
                &item,
                None,
                None,
                &collected,
            ),
            "any join should pass when at least one condition matches"
        );
    }

    #[test]
    fn year_and_regex_conditions_respect_case_sensitivity() {
        let item = build_item("I1", "Transformer Models", "Alice Kim", Some("2019-06-01"));
        let collected = empty_collected();

        let before_year = normalized_condition(
            ZoteroSearchConditionField::Year,
            ZoteroSearchConditionOperation::BeforeYear,
            Some("2020"),
            None,
        );
        assert!(evaluate_condition(
            &before_year,
            &item,
            None,
            None,
            &collected
        ));

        let after_year = normalized_condition(
            ZoteroSearchConditionField::Year,
            ZoteroSearchConditionOperation::AfterYear,
            Some("2020"),
            None,
        );
        assert!(!evaluate_condition(
            &after_year,
            &item,
            None,
            None,
            &collected
        ));

        let insensitive_regex = normalized_condition(
            ZoteroSearchConditionField::Title,
            ZoteroSearchConditionOperation::Regex,
            Some("transformer"),
            Some(false),
        );
        assert!(evaluate_condition(
            &insensitive_regex,
            &item,
            None,
            None,
            &collected
        ));

        let sensitive_regex = normalized_condition(
            ZoteroSearchConditionField::Title,
            ZoteroSearchConditionOperation::Regex,
            Some("transformer"),
            Some(true),
        );
        assert!(!evaluate_condition(
            &sensitive_regex,
            &item,
            None,
            None,
            &collected
        ));
    }

    #[test]
    fn apply_advanced_sort_orders_and_reverses_by_title() {
        let mut items = vec![
            build_item("3", "Zeta", "", None),
            build_item("1", "Alpha", "", None),
            build_item("2", "Beta", "", None),
        ];
        let mut warnings = Vec::new();
        apply_advanced_sort(
            &mut items,
            &ZoteroAdvancedSortBy::Title,
            &ZoteroSortDirection::Asc,
            &mut warnings,
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.key.clone())
                .collect::<Vec<_>>(),
            vec!["1".to_string(), "2".to_string(), "3".to_string()]
        );
        assert!(warnings.is_empty());

        apply_advanced_sort(
            &mut items,
            &ZoteroAdvancedSortBy::Title,
            &ZoteroSortDirection::Desc,
            &mut warnings,
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.key.clone())
                .collect::<Vec<_>>(),
            vec!["3".to_string(), "2".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn normalize_condition_rejects_non_year_before_after() {
        let condition = normalize_advanced_condition(ZoteroSearchCondition {
            field: ZoteroSearchConditionField::Title,
            operation: ZoteroSearchConditionOperation::BeforeYear,
            value: Some("2020".to_string()),
            case_sensitive: None,
        });
        assert!(condition.is_err());
    }
}
