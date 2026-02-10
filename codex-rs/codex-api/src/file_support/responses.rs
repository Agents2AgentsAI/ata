use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

use crate::file_support::build_data_url;

pub fn wrap_responses_input_file_data_uris(input: &mut [ResponseItem]) {
    for item in input {
        if let ResponseItem::Message { content, .. } = item {
            for content_item in content {
                if let ContentItem::InputFile {
                    file_data: Some(file_data),
                    mime_type,
                    ..
                } = content_item
                    && !file_data.starts_with("data:")
                {
                    let wrapped = build_data_url(mime_type, file_data);
                    *file_data = wrapped;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn wraps_inline_file_data_to_data_uri() {
        let mut input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputFile {
                file_data: Some("JVBERi0xLjQ=".to_string()),
                file_id: None,
                mime_type: "application/pdf".to_string(),
                filename: Some("report.pdf".to_string()),
            }],
            end_turn: None,
            phase: None,
        }];

        wrap_responses_input_file_data_uris(&mut input);

        assert_eq!(
            input[0],
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputFile {
                    file_data: Some("data:application/pdf;base64,JVBERi0xLjQ=".to_string()),
                    file_id: None,
                    mime_type: "application/pdf".to_string(),
                    filename: Some("report.pdf".to_string()),
                }],
                end_turn: None,
                phase: None,
            }
        );
    }

    #[test]
    fn leaves_file_references_unchanged() {
        let mut input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputFile {
                file_data: None,
                file_id: Some("file_123".to_string()),
                mime_type: "application/pdf".to_string(),
                filename: Some("report.pdf".to_string()),
            }],
            end_turn: None,
            phase: None,
        }];

        wrap_responses_input_file_data_uris(&mut input);

        assert_eq!(
            input[0],
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputFile {
                    file_data: None,
                    file_id: Some("file_123".to_string()),
                    mime_type: "application/pdf".to_string(),
                    filename: Some("report.pdf".to_string()),
                }],
                end_turn: None,
                phase: None,
            }
        );
    }

    #[test]
    fn wrapping_is_idempotent() {
        let mut input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputFile {
                file_data: Some("JVBERi0xLjQ=".to_string()),
                file_id: None,
                mime_type: "application/pdf".to_string(),
                filename: Some("report.pdf".to_string()),
            }],
            end_turn: None,
            phase: None,
        }];

        wrap_responses_input_file_data_uris(&mut input);
        let once = input.clone();
        wrap_responses_input_file_data_uris(&mut input);

        assert_eq!(input, once);
    }
}
