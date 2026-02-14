use anyhow::Result;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::sse_failed;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use serde_json::Value;

fn request_contains_url_file(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .any(|span| match span.get("type").and_then(Value::as_str) {
            Some("url_file") => true,
            Some("input_file") => span.get("file_url").and_then(Value::as_str).is_some(),
            _ => false,
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_related_invalid_request_drops_url_attachments_and_retries() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let call_id = "attach-1";
    let args = serde_json::json!({
        "files": [
            {"url": "https://example.com/doc.pdf"}
        ],
    });
    let arguments = serde_json::to_string(&args)?;

    mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "attach_url_files", &arguments),
            ev_completed("resp-1"),
        ]),
    )
    .await;

    let failed_mock = mount_sse_once(
        &server,
        sse_failed(
            "resp-2",
            "invalid_request_error",
            "File reference is no longer valid. Please re-attach the file.",
        ),
    )
    .await;

    let retry_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-1", "continuing without file"),
            ev_completed("resp-3"),
        ]),
    )
    .await;

    let fixture = test_codex().with_model("gpt-5.1").build(&server).await?;
    fixture
        .submit_turn("attach a pdf and then continue if it fails")
        .await?;

    let failed_body = failed_mock.single_request().body_json();
    assert!(
        request_contains_url_file(&failed_body),
        "expected failed request to include a url_file block: {failed_body:?}"
    );

    let retry_body = retry_mock.single_request().body_json();
    assert!(
        !request_contains_url_file(&retry_body),
        "expected retry request to omit url_file blocks after recovery: {retry_body:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_file_invalid_request_does_not_retry_or_drop_url_attachments() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let call_id = "attach-2";
    let args = serde_json::json!({
        "files": [
            {"url": "https://example.com/doc.pdf"}
        ],
    });
    let arguments = serde_json::to_string(&args)?;

    mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call(call_id, "attach_url_files", &arguments),
            ev_completed("resp-1"),
        ]),
    )
    .await;

    mount_sse_once(
        &server,
        sse_failed(
            "resp-2",
            "invalid_request_error",
            "Invalid request: missing required field.",
        ),
    )
    .await;

    let retry_mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-3"),
            ev_assistant_message("msg-1", "should not reach"),
            ev_completed("resp-3"),
        ]),
    )
    .await;

    let fixture = test_codex().with_model("gpt-5.1").build(&server).await?;
    fixture
        .submit_turn("attach a pdf and then error for non-file reasons")
        .await?;

    assert!(
        retry_mock.last_request().is_none(),
        "expected no retry request for non-file InvalidRequest"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn context_window_exceeded_drops_url_attachments_only_once_per_turn() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let args = serde_json::json!({
        "files": [
            {"url": "https://example.com/doc.pdf"}
        ],
    });
    let arguments = serde_json::to_string(&args)?;

    mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-1"),
            ev_function_call("attach-ctx-1", "attach_url_files", &arguments),
            ev_completed("resp-1"),
        ]),
    )
    .await;

    let first_failed = mount_sse_once(
        &server,
        sse_failed(
            "resp-2",
            "context_length_exceeded",
            "Your input exceeds the context window of this model. Please adjust your input and try again.",
        ),
    )
    .await;

    let retry_tool = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-3"),
            ev_function_call("attach-ctx-2", "attach_url_files", &arguments),
            ev_completed("resp-3"),
        ]),
    )
    .await;

    let second_failed = mount_sse_once(
        &server,
        sse_failed(
            "resp-4",
            "context_length_exceeded",
            "Your input exceeds the context window of this model. Please adjust your input and try again.",
        ),
    )
    .await;

    let should_not_retry = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-5"),
            ev_assistant_message("msg-1", "should not reach"),
            ev_completed("resp-5"),
        ]),
    )
    .await;

    let fixture = test_codex().with_model("gpt-5.1").build(&server).await?;
    fixture
        .submit_turn("attach a pdf repeatedly, even if the context window is exceeded")
        .await?;

    let first_failed_body = first_failed.single_request().body_json();
    assert!(
        request_contains_url_file(&first_failed_body),
        "expected first failed request to include a url_file block: {first_failed_body:?}"
    );

    let retry_tool_body = retry_tool.single_request().body_json();
    assert!(
        !request_contains_url_file(&retry_tool_body),
        "expected recovery retry to omit url_file blocks after drop: {retry_tool_body:?}"
    );

    let second_failed_body = second_failed.single_request().body_json();
    assert!(
        request_contains_url_file(&second_failed_body),
        "expected second failed request to include a url_file block: {second_failed_body:?}"
    );

    assert!(
        should_not_retry.last_request().is_none(),
        "expected no additional retry after second ContextWindowExceeded"
    );

    Ok(())
}
