use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::url_downloader::DEFAULT_MAX_DOWNLOAD_CONCURRENCY;
use crate::tools::url_downloader::UrlDownloadOutcome;
use crate::tools::url_downloader::UrlDownloadRequest;
use crate::tools::url_downloader::download_url_files_to_cache;
use crate::tools::url_validation::ValidatedUrl;
use crate::tools::url_validation::redact_url_string_for_display;
use crate::tools::url_validation::validate_url_strict;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::LazyLock;

const TOOL_NAME: &str = "attach_url_files";
const MAX_URLS_PER_CALL: usize = 10;

// @agent-facing
pub(crate) static ATTACH_URL_FILES_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut file_props = BTreeMap::new();
    file_props.insert(
        "url".to_string(),
        JsonSchema::string(Some(
            "Direct HTTPS URL to a PDF file (e.g., https://example.com/paper.pdf).".to_string(),
        )),
    );
    file_props.insert(
        "filename".to_string(),
        JsonSchema::string(Some(
            "Optional display name for the attached file. If omitted, derived from URL."
                .to_string(),
        )),
    );

    let item_schema = JsonSchema::object(
        file_props,
        Some(vec!["url".to_string()]),
        Some(false.into()),
    );

    let files_schema = JsonSchema::array(
        item_schema,
        Some("List of PDF files to attach by URL.".to_string()),
    );

    let mut properties = BTreeMap::new();
    properties.insert("files".to_string(), files_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: TOOL_NAME.to_string(),
        description: "Download and cache PDF files from URLs so other tools (such as document_reader and crop_figure) can read them. Use this whenever you encounter a PDF URL that needs analysis. Each URL must point directly to a PDF file (e.g., https://arxiv.org/pdf/2512.04538.pdf). For arXiv, convert abstract URLs to PDF URLs: change /abs/<id> to /pdf/<id>.pdf. Supports up to 10 URLs per call.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["files".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

#[derive(Default)]
pub struct AttachUrlFilesHandler;

#[derive(Debug, Deserialize)]
struct AttachUrlFilesArgs {
    files: Vec<AttachUrlFileArg>,
}

#[derive(Debug, Deserialize)]
struct AttachUrlFileArg {
    url: String,
    #[serde(default)]
    filename: Option<String>,
}

#[derive(Debug, Clone)]
struct ValidatedAttachment {
    url: ValidatedUrl,
    filename: Option<String>,
}

#[derive(Debug, Clone)]
struct AttachmentFailure {
    redacted_url: String,
    reason: String,
}

#[derive(Debug, Clone)]
struct AttachmentSuccess {
    redacted_url: String,
    filename: String,
    cached_path: String,
}

impl ToolHandler for AttachUrlFilesHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(TOOL_NAME)
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { turn, payload, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "attach_url_files received unsupported payload".to_string(),
                ));
            }
        };

        let args: AttachUrlFilesArgs = parse_arguments(&arguments)?;
        if args.files.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "attach_url_files requires at least one file".to_string(),
            ));
        }
        if args.files.len() > MAX_URLS_PER_CALL {
            return Err(FunctionCallError::RespondToModel(format!(
                "attach_url_files accepts at most {MAX_URLS_PER_CALL} URLs per call"
            )));
        }

        let (validated_files, mut failures, warnings) = validate_and_dedup(args.files).await;
        if validated_files.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                render_failure_only_summary(&failures),
            ));
        }

        let requests: Vec<UrlDownloadRequest> = validated_files
            .iter()
            .map(|attachment| UrlDownloadRequest {
                url: attachment.url.clone(),
                filename_hint: attachment.filename.clone(),
            })
            .collect();
        let download_outcomes = download_url_files_to_cache(
            &turn.config.codex_home,
            requests,
            DEFAULT_MAX_DOWNLOAD_CONCURRENCY,
        )
        .await;

        let mut successes = Vec::new();
        for (outcome, attachment) in download_outcomes.into_iter().zip(&validated_files) {
            match outcome {
                UrlDownloadOutcome::Success(success) => {
                    let filename = attachment
                        .filename
                        .clone()
                        .unwrap_or_else(|| attachment.url.derive_pdf_filename(None));
                    successes.push(AttachmentSuccess {
                        redacted_url: attachment.url.redacted_for_display(),
                        filename,
                        cached_path: success.path.display().to_string(),
                    });
                }
                UrlDownloadOutcome::Failure(failure) => {
                    failures.push(AttachmentFailure {
                        redacted_url: failure.redacted_url,
                        reason: failure.reason,
                    });
                }
            }
        }

        if successes.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                render_failure_only_summary(&failures),
            ));
        }

        let summary = render_summary(&successes, &failures, &warnings);
        Ok(FunctionToolOutput::from_text(summary, Some(true)))
    }
}

async fn validate_and_dedup(
    files: Vec<AttachUrlFileArg>,
) -> (
    Vec<ValidatedAttachment>,
    Vec<AttachmentFailure>,
    Vec<String>,
) {
    use futures::stream::StreamExt;

    // Validate all URLs concurrently, preserving input order via index tracking.
    let validation_results: Vec<(usize, _)> = futures::stream::iter(files.into_iter().enumerate())
        .map(|(idx, file)| async move {
            let result = validate_url_strict(&file.url).await;
            (idx, file, result)
        })
        .buffer_unordered(MAX_URLS_PER_CALL)
        .map(|(idx, file, result)| (idx, file, result))
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|(idx, file, result)| (idx, (file, result)))
        .collect();

    // Sort by original index to preserve input order.
    let mut sorted_results = validation_results;
    sorted_results.sort_by_key(|(idx, _)| *idx);

    let mut seen = HashSet::new();
    let mut validated = Vec::new();
    let mut failures = Vec::new();
    let mut warnings = Vec::new();

    for (_, (file, result)) in sorted_results {
        match result {
            Ok(url) => {
                let normalized_key = url.normalized_cache_key().to_string();
                if !seen.insert(normalized_key) {
                    warnings.push(format!(
                        "Skipped duplicate URL: {}",
                        url.redacted_for_display()
                    ));
                    continue;
                }
                let filename = file
                    .filename
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                validated.push(ValidatedAttachment { url, filename });
            }
            Err(error) => failures.push(AttachmentFailure {
                redacted_url: redact_url_string_for_display(&file.url),
                reason: error.to_string(),
            }),
        }
    }

    (validated, failures, warnings)
}

fn render_summary(
    successes: &[AttachmentSuccess],
    failures: &[AttachmentFailure],
    warnings: &[String],
) -> String {
    let mut summary = format!(
        "Attached {} URL file(s) to the local cache. Other tools (document_reader, crop_figure) can now read them by URL or cached path.",
        successes.len()
    );
    if !successes.is_empty() {
        summary.push_str("\nFiles:");
        for success in successes {
            summary.push_str(&format!(
                "\n- {} ({}) -> {}",
                success.redacted_url, success.filename, success.cached_path
            ));
        }
    }
    if !warnings.is_empty() {
        summary.push_str("\nWarnings:");
        for warning in warnings {
            summary.push_str(&format!("\n- {warning}"));
        }
    }
    append_failures(&mut summary, failures);
    summary
}

fn render_failure_only_summary(failures: &[AttachmentFailure]) -> String {
    if failures.is_empty() {
        return "No valid URL files to attach.".to_string();
    }

    let mut summary = "No URL files were attached.".to_string();
    append_failures(&mut summary, failures);
    summary
}

fn append_failures(summary: &mut String, failures: &[AttachmentFailure]) {
    if !failures.is_empty() {
        summary.push_str("\nFailures:");
        for failure in failures {
            summary.push_str(&format!(
                "\n- {} ({})",
                failure.redacted_url, failure.reason
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::make_session_and_context;
    use crate::tools::url_downloader::prepopulate_pdf_cache;
    use crate::tools::url_validation::validated_url_for_test;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    const VALID_PDF_BYTES: &[u8] = b"%PDF-1.4\ntest content";

    #[tokio::test]
    async fn attaches_cached_pdf_url() {
        let (session, turn_context) = make_session_and_context().await;
        let session = Arc::new(session);

        let url = validated_url_for_test("https://example.com/doc.pdf");
        prepopulate_pdf_cache(&turn_context.config.codex_home, &url, VALID_PDF_BYTES).await;

        let handler = AttachUrlFilesHandler;
        let output = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-1".to_string(),
                tool_name: TOOL_NAME.into(),
                cancellation_token: tokio_util::sync::CancellationToken::new(),
                source: crate::tools::context::ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/doc.pdf"}]}"#.to_string(),
                },
            })
            .await
            .expect("tool call should succeed");

        let text = output.into_text();
        assert!(text.contains("Attached 1 URL file(s)"));
        assert!(text.contains("https://example.com/doc.pdf"));
    }

    #[tokio::test]
    async fn rejects_more_than_per_call_limit() {
        let (session, turn_context) = make_session_and_context().await;
        let session = Arc::new(session);

        let files = (0..(MAX_URLS_PER_CALL + 1))
            .map(|idx| format!(r#"{{"url":"https://example.com/{idx}.pdf"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let args = format!(r#"{{"files":[{files}]}}"#);

        let handler = AttachUrlFilesHandler;
        let result = handler
            .handle(ToolInvocation {
                session,
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-2".to_string(),
                tool_name: TOOL_NAME.into(),
                cancellation_token: tokio_util::sync::CancellationToken::new(),
                source: crate::tools::context::ToolCallSource::Direct,
                payload: ToolPayload::Function { arguments: args },
            })
            .await;

        assert!(matches!(result, Err(FunctionCallError::RespondToModel(_))));
    }

    #[tokio::test]
    async fn duplicate_urls_emit_warning_and_attach_once() {
        let (session, turn_context) = make_session_and_context().await;
        let session = Arc::new(session);

        let url = validated_url_for_test("https://example.com/doc.pdf");
        prepopulate_pdf_cache(&turn_context.config.codex_home, &url, VALID_PDF_BYTES).await;

        let handler = AttachUrlFilesHandler;
        let output = handler
            .handle(ToolInvocation {
                session,
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-3".to_string(),
                tool_name: TOOL_NAME.into(),
                cancellation_token: tokio_util::sync::CancellationToken::new(),
                source: crate::tools::context::ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/doc.pdf"},{"url":"https://example.com/doc.pdf"}]}"#.to_string(),
                },
            })
            .await
            .expect("tool call should succeed");

        let text = output.into_text();
        assert!(text.contains("Attached 1 URL file(s)"));
        assert!(text.contains("Skipped duplicate URL: https://example.com/doc.pdf"));
    }

    #[tokio::test]
    async fn all_invalid_urls_produce_failure_only_summary() {
        let (session, turn_context) = make_session_and_context().await;
        let session = Arc::new(session);

        let handler = AttachUrlFilesHandler;
        let result = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-5".to_string(),
                tool_name: TOOL_NAME.into(),
                cancellation_token: tokio_util::sync::CancellationToken::new(),
                source: crate::tools::context::ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"ftp://example.com/a.pdf"},{"url":"http://example.com/b.pdf"}]}"#.to_string(),
                },
            })
            .await;

        let Err(FunctionCallError::RespondToModel(message)) = result else {
            panic!("expected failure-only summary");
        };
        assert!(
            message.contains("No URL files were attached."),
            "expected failure-only header, got: {message}"
        );
        assert!(
            message.contains("Failures:"),
            "expected failures section, got: {message}"
        );
    }

    #[tokio::test]
    async fn rejects_non_pdf_downloads() {
        let (session, turn_context) = make_session_and_context().await;
        let session = Arc::new(session);

        // Pre-populate cache with non-PDF content (HTML).
        let url = validated_url_for_test("https://example.com/blocked.pdf");
        prepopulate_pdf_cache(
            &turn_context.config.codex_home,
            &url,
            b"<html><body>Bot protection</body></html>",
        )
        .await;

        let handler = AttachUrlFilesHandler;
        let result = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-7".to_string(),
                tool_name: TOOL_NAME.into(),
                cancellation_token: tokio_util::sync::CancellationToken::new(),
                source: crate::tools::context::ToolCallSource::Direct,
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/blocked.pdf"}]}"#
                        .to_string(),
                },
            })
            .await;

        // The non-PDF should be rejected, resulting in a failure-only summary.
        let Err(FunctionCallError::RespondToModel(message)) = result else {
            panic!("expected failure for non-PDF content");
        };
        assert!(
            message.contains("No URL files were attached."),
            "expected failure-only header, got: {message}"
        );
        assert!(
            message.contains("https://example.com/blocked.pdf"),
            "expected specific URL in failure, got: {message}"
        );
    }
}
