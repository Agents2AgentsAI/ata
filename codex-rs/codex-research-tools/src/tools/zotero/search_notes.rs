use crate::ResearchToolkit;
use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ZoteroGrepField;
use crate::types::ZoteroGrepParams;
use crate::types::ZoteroSearchNoteMatch;
use crate::types::ZoteroSearchNotesParams;
use crate::types::ZoteroSearchNotesResult;

use super::grep;

const DEFAULT_NOTES_LIMIT: u32 = 50;

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

    let mut fields = vec![ZoteroGrepField::Note];
    if params.include_annotations.unwrap_or(true) {
        fields.push(ZoteroGrepField::Annotation);
    }

    let limit = params.limit.unwrap_or(DEFAULT_NOTES_LIMIT).clamp(1, 200);
    let grep_result = grep::zotero_grep_text(
        toolkit,
        ZoteroGrepParams {
            pattern: query.clone(),
            match_mode: params.match_mode,
            case_sensitive: params.case_sensitive,
            library_type: params.library_type,
            library_id: params.library_id,
            parent_item_key: params.parent_item_key,
            query_hint: Some(query.clone()),
            item_type: None,
            fields: Some(fields),
            limit_items: None,
            limit_matches: Some(limit),
            max_matches_per_item: None,
            context_chars: None,
            max_chars_per_item: params.max_chars_per_item,
        },
    )
    .await?;

    let notes = grep_result
        .matches
        .into_iter()
        .map(|matched| ZoteroSearchNoteMatch {
            item_key: matched.item_key,
            parent_item: matched.parent_item_key,
            field: matched.field,
            snippet: matched.snippet,
            source_meta: matched.source_meta,
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
