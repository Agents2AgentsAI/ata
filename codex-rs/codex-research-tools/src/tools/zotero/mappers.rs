use crate::clients::zotero;
use crate::types::ZoteroAnnotation;

use super::match_engine;
use super::normalize_optional_string;

pub(super) fn map_zotero_annotation(annotation: zotero::ZoteroAnnotation) -> ZoteroAnnotation {
    let annotation_type = annotation
        .annotation_type
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|value| {
            matches!(
                value.as_str(),
                "highlight" | "note" | "image" | "underline" | "strikethrough" | "ink"
            )
        })
        .unwrap_or_else(|| "unknown".to_string());

    ZoteroAnnotation {
        key: annotation.key,
        parent_item: normalize_optional_string(annotation.parent_item),
        annotation_type,
        annotation_text: annotation
            .annotation_text
            .map(|text| match_engine::strip_html_to_text(text.as_str())),
        annotation_comment: annotation
            .annotation_comment
            .map(|comment| match_engine::strip_html_to_text(comment.as_str())),
        annotation_color: normalize_optional_string(annotation.annotation_color),
        annotation_page_label: normalize_optional_string(annotation.annotation_page_label),
        annotation_sort_index: normalize_optional_string(annotation.annotation_sort_index),
        parent_item_title: None,
        source_meta: annotation.source_meta,
    }
}
