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

fn rewrite_openai_url_file_blocks_in_content(content: &mut [Value]) {
    for block in content {
        let block_type = block.get("type").and_then(Value::as_str);

        match block_type {
            Some("url_file") => {
                rewrite_url_file_block(block);
            }
            Some("input_file") => {
                normalize_input_file_block_for_openai(block);
            }
            _ => {}
        }
    }
}

/// Rewrites a `url_file` block into an OpenAI-compatible `input_file` block with `file_url`.
fn rewrite_url_file_block(block: &mut Value) {
    let url = block
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty());
    let Some(url) = url else {
        return;
    };

    // OpenAI treats `filename` as mutually exclusive with `file_url` (and `file_id`),
    // so we only emit `type` + `file_url` here.
    *block = serde_json::json!({
        "type": "input_file",
        "file_url": url,
    });
}

/// Wraps raw base64 `file_data` into a `data:<mime>;base64,...` URI and strips `mime_type`
/// so the block conforms to the OpenAI Responses API wire shape.
fn normalize_input_file_block_for_openai(block: &mut Value) {
    if let Some(file_data) = block.get("file_data").and_then(Value::as_str)
        && !file_data.starts_with("data:")
    {
        let mime = block
            .get("mime_type")
            .and_then(Value::as_str)
            .unwrap_or("application/pdf");
        let data_uri = build_data_url(mime, file_data);
        block["file_data"] = Value::String(data_uri);
    }
    if let Some(obj) = block.as_object_mut() {
        // OpenAI Responses API does not accept `mime_type` on `input_file` blocks.
        obj.remove("mime_type");
        // `filename` is only valid alongside `file_data` (inline); strip it when `file_id` is present.
        if obj.contains_key("file_id") {
            obj.remove("filename");
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
                "file_url": "https://example.com/report.pdf"
            })
        );
    }

    #[test]
    fn strips_filename_when_file_id_present() {
        let mut payload = serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_file",
                    "file_id": "file-abc123",
                    "filename": "report.pdf"
                }]
            }]
        });

        rewrite_openai_url_file_blocks_in_payload(&mut payload);

        assert_eq!(
            payload["input"][0]["content"][0],
            serde_json::json!({
                "type": "input_file",
                "file_id": "file-abc123"
            })
        );
    }

    #[test]
    fn normalizes_input_file_blocks_for_openai_payload() {
        let mut payload = serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_file",
                    "file_data": "JVBERi0xLjQ=",
                    "mime_type": "application/pdf",
                    "filename": "report.pdf"
                }]
            }]
        });

        rewrite_openai_url_file_blocks_in_payload(&mut payload);

        assert_eq!(
            payload["input"][0]["content"][0],
            serde_json::json!({
                "type": "input_file",
                "file_data": "data:application/pdf;base64,JVBERi0xLjQ=",
                "filename": "report.pdf"
            })
        );
    }

    #[test]
    fn normalizes_input_file_already_data_uri() {
        let mut payload = serde_json::json!({
            "input": [{
                "type": "message",
                "role": "user",
                "content": [{
                    "type": "input_file",
                    "file_data": "data:application/pdf;base64,JVBERi0xLjQ=",
                    "mime_type": "application/pdf",
                    "filename": "report.pdf"
                }]
            }]
        });

        rewrite_openai_url_file_blocks_in_payload(&mut payload);

        assert_eq!(
            payload["input"][0]["content"][0],
            serde_json::json!({
                "type": "input_file",
                "file_data": "data:application/pdf;base64,JVBERi0xLjQ=",
                "filename": "report.pdf"
            })
        );
    }
}
