use codex_api::ResponseEvent;
use codex_api::sse::anthropic::AnthropicStreamState;
use codex_api::sse::anthropic::parse_anthropic_event;
use codex_api::sse::gemini::GeminiStreamState;
use codex_api::sse::gemini::finalize_gemini_stream;
use codex_api::sse::gemini::parse_gemini_chunk;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;

#[test]
fn providers_emit_message_item_before_completion_for_resume_persistence() {
    let gemini_first_chunk = r#"{
        "candidates": [{
            "content": {
                "parts": [{"text": "Gemini reply"}],
                "role": "model"
            },
            "index": 0
        }]
    }"#;
    let mut gemini_state = GeminiStreamState::new();
    let gemini_events = parse_gemini_chunk(gemini_first_chunk, &mut gemini_state)
        .expect("gemini chunk should parse");
    assert!(
        gemini_events
            .iter()
            .any(|event| matches!(event, ResponseEvent::OutputTextDelta(_)))
    );
    let gemini_final = finalize_gemini_stream(&mut gemini_state, None);
    assert_eq!(gemini_final.len(), 2);
    match &gemini_final[0] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. }) => {
            assert_eq!(
                content,
                &vec![ContentItem::OutputText {
                    text: "Gemini reply".to_string(),
                }]
            );
        }
        other => panic!("expected Gemini message item before completion, got {other:?}"),
    }
    assert!(matches!(
        gemini_final[1],
        ResponseEvent::Completed {
            token_usage: None,
            ..
        }
    ));

    let anthropic_start = r#"{
        "type": "message_start",
        "message": {
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 0
            }
        }
    }"#;
    let anthropic_delta = r#"{
        "type": "content_block_delta",
        "index": 0,
        "delta": {
            "type": "text_delta",
            "text": "Anthropic reply"
        }
    }"#;
    let anthropic_stop = r#"{"type": "message_stop"}"#;
    let mut anthropic_state = AnthropicStreamState::new();

    let _start_events =
        parse_anthropic_event("message_start", anthropic_start, &mut anthropic_state)
            .expect("anthropic message_start should parse");
    let delta_events =
        parse_anthropic_event("content_block_delta", anthropic_delta, &mut anthropic_state)
            .expect("anthropic content_block_delta should parse");
    assert!(
        delta_events
            .iter()
            .any(|event| matches!(event, ResponseEvent::OutputTextDelta(_)))
    );

    let anthropic_final =
        parse_anthropic_event("message_stop", anthropic_stop, &mut anthropic_state)
            .expect("anthropic message_stop should parse");
    assert_eq!(anthropic_final.len(), 2);
    match &anthropic_final[0] {
        ResponseEvent::OutputItemDone(ResponseItem::Message { content, .. }) => {
            assert_eq!(
                content,
                &vec![ContentItem::OutputText {
                    text: "Anthropic reply".to_string(),
                }]
            );
        }
        other => panic!("expected Anthropic message item before completion, got {other:?}"),
    }
    assert!(matches!(
        anthropic_final[1],
        ResponseEvent::Completed {
            token_usage: Some(_),
            ..
        }
    ));
}
