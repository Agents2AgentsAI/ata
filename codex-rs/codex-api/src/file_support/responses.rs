use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

use crate::file_support::build_data_url;

/// Wrap raw base64 `input_file.file_data` payloads in a `data:<mime>;base64,` URI and strip the
/// `mime_type` field so it is not serialized in the outgoing request.
///
/// The OpenAI Responses API does not accept `mime_type` on `input_file` blocks — the MIME type
/// must be embedded exclusively in the `file_data` data-URI. Provider adapters for Anthropic and
/// Gemini read `mime_type` from the serialized JSON before this function runs, so they are
/// unaffected.
///
/// This mutates `input` in-place. Callers should generally operate on a cloned request payload,
/// not persisted conversation history, since this transformation is permanent.
pub fn wrap_responses_input_file_data_uris(input: &mut [ResponseItem]) {
    for item in input {
        if let ResponseItem::Message { content, .. } = item {
            for content_item in content {
                if let ContentItem::InputFile {
                    file_data,
                    mime_type,
                    ..
                } = content_item
                {
                    if let Some(data) = file_data
                        && !data.starts_with("data:")
                    {
                        let mime = mime_type.as_deref().unwrap_or("application/pdf");
                        *data = build_data_url(mime, data);
                    }
                    // Strip mime_type so it is not serialized for the OpenAI Responses API.
                    *mime_type = None;
                }
            }
        }
    }
}

/// Convenience helper that clones `input` before applying [`wrap_responses_input_file_data_uris`].
pub fn wrapped_responses_input_file_data_uris(input: &[ResponseItem]) -> Vec<ResponseItem> {
    let mut normalized = input.to_vec();
    wrap_responses_input_file_data_uris(&mut normalized);
    normalized
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
                mime_type: Some("application/pdf".to_string()),
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
                    mime_type: None,
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
                mime_type: Some("application/pdf".to_string()),
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
                    mime_type: None,
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
                mime_type: Some("application/pdf".to_string()),
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

    #[test]
    fn wrapper_does_not_mutate_original_input() {
        let input = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputFile {
                file_data: Some("JVBERi0xLjQ=".to_string()),
                file_id: None,
                mime_type: Some("application/pdf".to_string()),
                filename: Some("report.pdf".to_string()),
            }],
            end_turn: None,
            phase: None,
        }];

        let wrapped = wrapped_responses_input_file_data_uris(&input);

        assert_eq!(
            input[0],
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputFile {
                    file_data: Some("JVBERi0xLjQ=".to_string()),
                    file_id: None,
                    mime_type: Some("application/pdf".to_string()),
                    filename: Some("report.pdf".to_string()),
                }],
                end_turn: None,
                phase: None,
            }
        );
        assert_eq!(
            wrapped[0],
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputFile {
                    file_data: Some("data:application/pdf;base64,JVBERi0xLjQ=".to_string()),
                    file_id: None,
                    mime_type: None,
                    filename: Some("report.pdf".to_string()),
                }],
                end_turn: None,
                phase: None,
            }
        );
    }
}
