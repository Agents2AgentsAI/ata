use crate::text_utils::truncate_chars;
use crate::types::ZoteroAnnotation;
use crate::types::ZoteroAttachment;
use crate::types::ZoteroItem;
use crate::types::ZoteroNote;

pub(super) fn apply_items_budget(items: &mut [ZoteroItem], max_chars_per_item: Option<u32>) {
    let max_chars = max_chars_per_item.map(|value| value as usize);

    for item in items {
        if let Some(max) = max_chars {
            truncate_optional_string(&mut item.abstract_snippet, max);
            item.title = truncate_chars(&item.title, max);
            item.authors = truncate_chars(&item.authors, max);
            item.tags = item
                .tags
                .iter()
                .map(|tag| truncate_chars(tag, max))
                .collect();
        }
    }
}

pub(super) fn apply_notes_budget(notes: &mut [ZoteroNote], max_chars: usize) {
    for note in notes {
        truncate_optional_string(&mut note.title, max_chars);
        truncate_optional_string(&mut note.note, max_chars);
    }
}

pub(super) fn apply_annotations_budget(annotations: &mut [ZoteroAnnotation], max_chars: usize) {
    for annotation in annotations {
        truncate_optional_string(&mut annotation.annotation_text, max_chars);
        truncate_optional_string(&mut annotation.annotation_comment, max_chars);
    }
}

pub(super) fn apply_attachments_budget(attachments: &mut [ZoteroAttachment], max_chars: usize) {
    for attachment in attachments {
        truncate_optional_string(&mut attachment.title, max_chars);
        truncate_optional_string(&mut attachment.filename, max_chars);
        truncate_optional_string(&mut attachment.content_type, max_chars);
        truncate_optional_string(&mut attachment.link_mode, max_chars);
        truncate_optional_string(&mut attachment.url, max_chars);
        truncate_optional_string(&mut attachment.path, max_chars);
    }
}

pub(super) fn truncate_optional_string(value: &mut Option<String>, max_chars: usize) {
    if let Some(value_ref) = value.as_mut() {
        *value_ref = truncate_chars(value_ref, max_chars);
    }
}
