use crate::ResearchToolkit;
use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepMatchMode;
use crate::types::ZoteroGrepParams;
use crate::types::ZoteroSearchNotesMatch;
use crate::types::ZoteroSearchNotesParams;
use crate::types::ZoteroSearchNotesResult;

pub(super) async fn zotero_search_notes(
    toolkit: &ResearchToolkit,
    params: ZoteroSearchNotesParams,
) -> Result<ZoteroSearchNotesResult> {
    let query = params.query.trim().to_string();
    if query.is_empty() {
        return Err(ResearchError::InvalidInput(
            "zotero_search_notes query must not be empty".to_string(),
        ));
    }
    let include_annotations = params.include_annotations.unwrap_or(true);
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let match_mode = params.match_mode.unwrap_or(ZoteroGrepMatchMode::Literal);
    let parent_item_key = super::normalize_optional_string(params.parent_item_key);
    let grep_params = ZoteroGrepParams {
        pattern: query.clone(),
        match_mode: Some(match_mode.clone()),
        case_sensitive: params.case_sensitive,
        library_type: params.library_type,
        library_id: params.library_id,
        parent_item_key: parent_item_key.clone(),
        query_hint: if parent_item_key.is_some() || match_mode == ZoteroGrepMatchMode::Regex {
            None
        } else {
            Some(query.clone())
        },
        item_type: None,
        fields: if include_annotations {
            Some(vec![ZoteroGrepField::Note, ZoteroGrepField::Annotation])
        } else {
            Some(vec![ZoteroGrepField::Note])
        },
        limit_items: Some(limit.max(50)),
        limit_matches: Some(limit),
        max_matches_per_item: None,
        context_chars: None,
        max_chars_per_item: params.max_chars_per_item,
    };

    let grep_result = super::grep::zotero_grep_text(toolkit, grep_params).await?;
    let candidate_strategy = grep_result.candidate_strategy;
    let scanned_items = grep_result.scanned_items;
    let total_available = if grep_result.truncated {
        None
    } else {
        Some(grep_result.returned_matches as u64)
    };
    let has_more = grep_result.truncated;
    let warnings = grep_result.warnings;
    let hints = grep_result.hints;
    let notes = grep_result
        .matches
        .into_iter()
        .map(|entry| ZoteroSearchNotesMatch {
            item_key: entry.item_key,
            parent_item: entry.parent_item_key,
            field: entry.field,
            snippet: entry.snippet,
            source_meta: entry.source_meta,
        })
        .collect::<Vec<_>>();

    Ok(ZoteroSearchNotesResult {
        query,
        candidate_strategy,
        scanned_items,
        total_available,
        has_more,
        warnings,
        hints,
        notes,
    })
}
