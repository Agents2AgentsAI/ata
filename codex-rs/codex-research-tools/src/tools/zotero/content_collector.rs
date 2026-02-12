use std::collections::HashMap;

use crate::ResearchToolkit;
use crate::clients::zotero;
use crate::clients::zotero::ZoteroChildrenRequest;
use crate::text_utils::truncate_chars;
use crate::types::SourceMeta;
use crate::types::ZoteroItemParams;
use crate::types::ZoteroNote;

use super::DEFAULT_CHILDREN_LIMIT;
use super::NormalizedScope;
use super::grep::GrepCandidate;
use super::grep::LibraryAnnotationPrefetch;
use super::grep::NormalizedGrepParams;
use super::grep::field_enabled;
use super::match_engine::strip_html_to_text;
use super::to_scope;
use super::zotero_config;

#[derive(Debug, Clone)]
pub(super) struct GrepTextSegment {
    pub(super) item_key: String,
    pub(super) field: &'static str,
    pub(super) text: String,
    pub(super) parent_item_key: Option<String>,
    pub(super) source_meta: Option<SourceMeta>,
}

#[derive(Debug, Default)]
pub(super) struct SegmentCollection {
    pub(super) segments: Vec<GrepTextSegment>,
    pub(super) warnings: Vec<String>,
}

pub(super) async fn collect_grep_segments(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
    candidate: &GrepCandidate,
    parent_scoped_notes: &HashMap<String, ZoteroNote>,
    parent_scoped_annotations: &HashMap<String, zotero::ZoteroAnnotation>,
    library_annotation_prefetch: &LibraryAnnotationPrefetch,
) -> SegmentCollection {
    let mut collection = SegmentCollection::default();
    let max_chars = normalized.max_chars_per_item.map(|value| value as usize);

    if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Title) {
        push_segment(
            &mut collection.segments,
            candidate.key.clone(),
            "title",
            candidate.title.clone(),
            candidate.parent_item_key.clone(),
            candidate.source_meta.clone(),
            max_chars,
        );
    }

    if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Tag) {
        for tag in &candidate.tags {
            push_segment(
                &mut collection.segments,
                candidate.key.clone(),
                "tag",
                tag.clone(),
                candidate.parent_item_key.clone(),
                candidate.source_meta.clone(),
                max_chars,
            );
        }
    }

    if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Abstract)
        || field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Extra)
    {
        let detail = toolkit
            .zotero_get_item(build_item_params(
                &normalized.scope,
                candidate.key.as_str(),
                normalized.max_chars_per_item,
            ))
            .await;
        match detail {
            Ok(item) => {
                if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Abstract)
                    && let Some(abstract_text) = item.abstract_text
                {
                    push_segment(
                        &mut collection.segments,
                        candidate.key.clone(),
                        "abstract",
                        abstract_text,
                        candidate.parent_item_key.clone(),
                        item.source_meta.clone(),
                        max_chars,
                    );
                }
                if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Extra)
                    && let Some(extra) = item.extra
                {
                    push_segment(
                        &mut collection.segments,
                        candidate.key.clone(),
                        "extra",
                        extra,
                        candidate.parent_item_key.clone(),
                        item.source_meta,
                        max_chars,
                    );
                }
            }
            Err(err) => {
                collection.warnings.push(format!(
                    "failed to load item detail for {}: {err}",
                    candidate.key
                ));
            }
        }
    }

    if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Note) {
        let note_segments = collect_note_segments(
            toolkit,
            normalized,
            candidate,
            parent_scoped_notes,
            max_chars,
        )
        .await;
        collection.warnings.extend(note_segments.warnings);
        collection.segments.extend(note_segments.segments);
    }

    if field_enabled(
        &normalized.fields,
        crate::types::ZoteroGrepField::Annotation,
    ) {
        let annotation_segments = collect_annotation_segments(
            toolkit,
            normalized,
            candidate,
            parent_scoped_annotations,
            library_annotation_prefetch,
            max_chars,
        )
        .await;
        collection.warnings.extend(annotation_segments.warnings);
        collection.segments.extend(annotation_segments.segments);
    }

    if field_enabled(&normalized.fields, crate::types::ZoteroGrepField::Fulltext) {
        let fulltext = toolkit
            .zotero_get_fulltext(build_item_params(
                &normalized.scope,
                candidate.key.as_str(),
                normalized.max_chars_per_item,
            ))
            .await;
        match fulltext {
            Ok(fulltext) => {
                push_segment(
                    &mut collection.segments,
                    candidate.key.clone(),
                    "fulltext",
                    fulltext.content,
                    candidate.parent_item_key.clone(),
                    fulltext.source_meta,
                    max_chars,
                );
            }
            Err(err) => {
                collection.warnings.push(format!(
                    "failed to load fulltext for {}: {err}",
                    candidate.key
                ));
            }
        }
    }

    collection
}

async fn collect_note_segments(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
    candidate: &GrepCandidate,
    parent_scoped_notes: &HashMap<String, ZoteroNote>,
    max_chars: Option<usize>,
) -> SegmentCollection {
    let mut collection = SegmentCollection::default();

    if normalized.candidate_strategy == crate::types::ZoteroGrepCandidateStrategy::ParentScoped
        && let Some(note) = parent_scoped_notes.get(&candidate.key)
    {
        if let Some(note_text) = note.note.as_deref() {
            push_segment(
                &mut collection.segments,
                note.key.clone(),
                "note",
                strip_html_to_text(note_text),
                note.parent_item.clone(),
                note.source_meta.clone(),
                max_chars,
            );
        }
        return collection;
    }

    if candidate.item_type == "note" {
        let scope = to_scope(&normalized.scope);
        let note = zotero::get_note_item(
            toolkit.http(),
            zotero_config(toolkit),
            &scope,
            candidate.key.as_str(),
        )
        .await;
        match note {
            Ok(Some(note)) => {
                if let Some(note_text) = note.note.as_deref() {
                    push_segment(
                        &mut collection.segments,
                        note.key.clone(),
                        "note",
                        strip_html_to_text(note_text),
                        note.parent_item.clone(),
                        note.source_meta,
                        max_chars,
                    );
                }
            }
            Ok(None) => {}
            Err(err) => collection
                .warnings
                .push(format!("failed to load note item {}: {err}", candidate.key)),
        }
        return collection;
    }

    let notes = toolkit
        .zotero_get_notes(build_item_params(
            &normalized.scope,
            candidate.key.as_str(),
            normalized.max_chars_per_item,
        ))
        .await;
    match notes {
        Ok(notes) => {
            for note in notes.notes {
                if let Some(note_text) = note.note.as_deref() {
                    push_segment(
                        &mut collection.segments,
                        note.key.clone(),
                        "note",
                        strip_html_to_text(note_text),
                        note.parent_item.clone(),
                        note.source_meta,
                        max_chars,
                    );
                }
            }
        }
        Err(err) => collection
            .warnings
            .push(format!("failed to load notes for {}: {err}", candidate.key)),
    }

    collection
}

async fn collect_annotation_segments(
    toolkit: &ResearchToolkit,
    normalized: &NormalizedGrepParams,
    candidate: &GrepCandidate,
    parent_scoped_annotations: &HashMap<String, zotero::ZoteroAnnotation>,
    library_annotation_prefetch: &LibraryAnnotationPrefetch,
    max_chars: Option<usize>,
) -> SegmentCollection {
    let mut collection = SegmentCollection::default();

    if normalized.candidate_strategy == crate::types::ZoteroGrepCandidateStrategy::ParentScoped
        && let Some(annotation) = parent_scoped_annotations.get(&candidate.key)
    {
        push_annotation_text_segments(&mut collection.segments, annotation, max_chars);
        return collection;
    }

    if candidate.item_type == "annotation" {
        let scope = to_scope(&normalized.scope);
        let annotation = zotero::get_annotation_item(
            toolkit.http(),
            zotero_config(toolkit),
            &scope,
            candidate.key.as_str(),
        )
        .await;
        match annotation {
            Ok(Some(annotation)) => {
                push_annotation_text_segments(&mut collection.segments, &annotation, max_chars);
            }
            Ok(None) => {}
            Err(err) => collection.warnings.push(format!(
                "failed to load annotation item {}: {err}",
                candidate.key
            )),
        }
        return collection;
    }

    if let Some(annotations) = library_annotation_prefetch.by_parent.get(&candidate.key) {
        for annotation in annotations {
            push_annotation_text_segments(&mut collection.segments, annotation, max_chars);
        }
        return collection;
    }

    if library_annotation_prefetch.complete {
        return collection;
    }

    let scope = to_scope(&normalized.scope);
    let annotations = zotero::get_annotations(
        toolkit.http(),
        zotero_config(toolkit),
        &scope,
        &ZoteroChildrenRequest {
            item_key: candidate.key.as_str(),
            offset: 0,
            limit: DEFAULT_CHILDREN_LIMIT,
        },
    )
    .await;
    match annotations {
        Ok(annotations) => {
            for annotation in annotations.annotations {
                push_annotation_text_segments(&mut collection.segments, &annotation, max_chars);
            }
        }
        Err(err) => collection.warnings.push(format!(
            "failed to load annotations for {}: {err}",
            candidate.key
        )),
    }

    collection
}

fn push_annotation_text_segments(
    out: &mut Vec<GrepTextSegment>,
    annotation: &zotero::ZoteroAnnotation,
    max_chars: Option<usize>,
) {
    if let Some(annotation_text) = annotation.annotation_text.as_deref() {
        push_segment(
            out,
            annotation.key.clone(),
            "annotation_text",
            strip_html_to_text(annotation_text),
            annotation.parent_item.clone(),
            annotation.source_meta.clone(),
            max_chars,
        );
    }
    if let Some(annotation_comment) = annotation.annotation_comment.as_deref() {
        push_segment(
            out,
            annotation.key.clone(),
            "annotation_comment",
            strip_html_to_text(annotation_comment),
            annotation.parent_item.clone(),
            annotation.source_meta.clone(),
            max_chars,
        );
    }
}

fn push_segment(
    out: &mut Vec<GrepTextSegment>,
    item_key: String,
    field: &'static str,
    text: String,
    parent_item_key: Option<String>,
    source_meta: Option<SourceMeta>,
    max_chars: Option<usize>,
) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    let segment_text = if let Some(max_chars) = max_chars {
        truncate_chars(trimmed, max_chars)
    } else {
        trimmed.to_string()
    };
    if segment_text.is_empty() {
        return;
    }

    out.push(GrepTextSegment {
        item_key,
        field,
        text: segment_text,
        parent_item_key,
        source_meta,
    });
}

fn build_item_params(
    scope: &NormalizedScope,
    item_key: &str,
    max_chars_per_item: Option<u32>,
) -> ZoteroItemParams {
    ZoteroItemParams {
        item_key: item_key.to_string(),
        library_type: Some(scope.library_type.clone()),
        library_id: Some(scope.library_id.clone()),
        max_chars_per_item,
    }
}
