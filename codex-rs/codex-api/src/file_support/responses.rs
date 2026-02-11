use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use serde_json::Value;

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

/// Rewrites internal `url_file` blocks into OpenAI wire-compatible
/// `input_file.file_url` blocks.
pub fn rewrite_openai_url_file_blocks_in_payload(payload: &mut Value) {
    match payload {
        Value::Array(items) => {
            for item in items {
                rewrite_openai_url_file_blocks_in_payload(item);
            }
        }
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("message")
                && let Some(Value::Array(content)) = map.get_mut("content")
            {
                rewrite_openai_url_file_blocks_in_content(content);
            }

            for value in map.values_mut() {
                rewrite_openai_url_file_blocks_in_payload(value);
            }
        }
        _ => {}
    }
}

pub fn rewrite_openai_url_file_blocks_in_content(content: &mut [Value]) {
    for block in content {
        let is_url_file = block
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|ty| ty == "url_file");
        if !is_url_file {
            continue;
        }

        let url = block
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty());
        let Some(url) = url else {
            continue;
        };

        let filename = block
            .get("filename")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|filename| !filename.is_empty())
            .map(ToString::to_string);

        let mut rewritten = serde_json::json!({
            "type": "input_file",
            "file_url": url,
        });
        if let Some(filename) = filename {
            rewritten["filename"] = serde_json::json!(filename);
        }
        *block = rewritten;
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

    #[test]
    fn rewrites_url_file_blocks_for_openai_payload() {
        let mut payload = serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "url_file",
                    "url": "https://example.com/report.pdf",
                    "filename": "report.pdf",
                    "mime_type": "application/pdf"
                }]
            }]
        });

        rewrite_openai_url_file_blocks_in_payload(&mut payload);

        assert_eq!(
            payload["input"][0]["content"][0],
            serde_json::json!({
                "type": "input_file",
                "file_url": "https://example.com/report.pdf",
                "filename": "report.pdf"
            })
        );
    }
}
