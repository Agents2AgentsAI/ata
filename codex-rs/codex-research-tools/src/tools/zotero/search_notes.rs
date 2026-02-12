use crate::ResearchToolkit;
use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ZoteroGrepField;
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
    let parent_item_key = super::normalize_optional_string(params.parent_item_key);
    let grep_params = ZoteroGrepParams {
        pattern: query.clone(),
        match_mode: params.match_mode,
        case_sensitive: params.case_sensitive,
        library_type: params.library_type,
        library_id: params.library_id,
        parent_item_key: parent_item_key.clone(),
        query_hint: if parent_item_key.is_some() {
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
        limit_items: None,
        limit_matches: Some(limit),
        max_matches_per_item: None,
        context_chars: None,
        max_chars_per_item: params.max_chars_per_item,
    };

    let grep_result = super::grep::zotero_grep_text(toolkit, grep_params).await?;
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
        total_available: if grep_result.truncated {
            None
        } else {
            Some(u64::try_from(notes.len()).unwrap_or(u64::MAX))
        },
        has_more: grep_result.truncated,
        warnings: grep_result.warnings,
        notes,
    })
}
