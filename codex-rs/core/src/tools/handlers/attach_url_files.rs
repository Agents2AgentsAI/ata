use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::function_tool::FunctionCallError;
use crate::provider_transport_capabilities::provider_transport_capabilities;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::file_injection::FileInjectionContext;
use crate::tools::file_injection::resolve_and_prepare_local_files_for_injection;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::registry::ToolRegistryBuilder;
use crate::tools::spec::JsonSchema;
use crate::tools::spec::ToolsConfig;
use crate::tools::url_downloader::DEFAULT_MAX_DOWNLOAD_CONCURRENCY;
use crate::tools::url_downloader::UrlDownloadOutcome;
use crate::tools::url_downloader::UrlDownloadRequest;
use crate::tools::url_downloader::download_url_files_to_cache;
use crate::tools::url_validation::ValidatedUrl;
use crate::tools::url_validation::redact_url_string_for_display;
use crate::tools::url_validation::validate_url_strict;
use async_trait::async_trait;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::ResponseInputItem;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::LazyLock;

const TOOL_NAME: &str = "attach_url_files";
const MAX_URLS_PER_CALL: usize = 10;
const MAX_URLS_PER_TURN: usize = 20;

pub(crate) static ATTACH_URL_FILES_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut file_props = BTreeMap::new();
    file_props.insert(
        "url".to_string(),
        JsonSchema::String {
            description: Some(
                "Direct HTTPS URL to a PDF file (e.g., https://example.com/paper.pdf).".to_string(),
            ),
        },
    );
    file_props.insert(
        "filename".to_string(),
        JsonSchema::String {
            description: Some(
                "Optional display name for the attached file. If omitted, derived from URL."
                    .to_string(),
            ),
        },
    );

    let files_schema = JsonSchema::Array {
        description: Some("List of PDF files to attach by URL.".to_string()),
        items: Box::new(JsonSchema::Object {
            properties: file_props,
            required: Some(vec!["url".to_string()]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut properties = BTreeMap::new();
    properties.insert("files".to_string(), files_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: TOOL_NAME.to_string(),
        description: "Attach PDF files from URLs so you can read and analyze their contents. Use this tool whenever you need to read a PDF — always prefer this over downloading PDFs via shell commands. Each URL must point directly to a PDF file (e.g., https://arxiv.org/pdf/2512.04538.pdf). For arXiv, convert abstract URLs to PDF URLs: change /abs/<id> to /pdf/<id>.pdf. Supports up to 10 URLs per call.".to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["files".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
});

#[derive(Default)]
pub(crate) struct AttachUrlFilesHandler;

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

pub(crate) fn register_attach_url_files(builder: &mut ToolRegistryBuilder, config: &ToolsConfig) {
    if config.web_search_mode == Some(WebSearchMode::Disabled) {
        return;
    }

    builder.push_spec(ATTACH_URL_FILES_TOOL.clone());
    builder.register_handler(TOOL_NAME, Arc::new(AttachUrlFilesHandler));
}

#[async_trait]
impl ToolHandler for AttachUrlFilesHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;

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

        let (mut validated_files, mut failures, mut warnings) =
            validate_and_dedup(args.files).await;
        if validated_files.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                render_failure_only_summary(&failures),
            ));
        }

        let supports_url_ingestion =
            provider_transport_capabilities(&turn.provider).supports_file_url_ingestion;
        let mut success_count = 0usize;

        if supports_url_ingestion {
            // Download + validate each URL before passing to the provider.
            // This catches non-PDF responses (e.g. HTML bot-protection pages)
            // before they cause the provider to reject the entire request.
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

            let mut content = Vec::new();
            for (outcome, attachment) in download_outcomes.into_iter().zip(&validated_files) {
                match outcome {
                    UrlDownloadOutcome::Success(_) => {
                        let filename = attachment
                            .filename
                            .clone()
                            .or_else(|| Some(attachment.url.derive_pdf_filename(None)));
                        content.push(ContentItem::url_file(
                            attachment.url.as_str().to_string(),
                            Some("application/pdf".to_string()),
                            filename,
                        ));
                        success_count += 1;
                    }
                    UrlDownloadOutcome::Failure(failure) => {
                        failures.push(AttachmentFailure {
                            redacted_url: failure.redacted_url,
                            reason: failure.reason,
                        });
                    }
                }
            }

            if success_count > 0 {
                session
                    .inject_response_items_with_url_attachment_budget(
                        vec![user_message_response_input(content)],
                        success_count,
                        MAX_URLS_PER_TURN,
                    )
                    .await
                    .map_err(map_budget_injection_error)?;
            }
        } else {
            let remaining_budget = {
                let mut active = session.active_turn.lock().await;
                let Some(active_turn) = active.as_mut() else {
                    return Err(map_budget_injection_error(
                        crate::codex::UrlAttachmentInjectionError::NoActiveTurn,
                    ));
                };
                let turn_state = active_turn.turn_state.lock().await;
                let already_used = turn_state.url_attachments_injected();
                MAX_URLS_PER_TURN.saturating_sub(already_used)
            };

            if remaining_budget == 0 {
                return Err(map_budget_injection_error(
                    crate::codex::UrlAttachmentInjectionError::PerTurnLimitExceeded {
                        attempted: validated_files.len(),
                        current: MAX_URLS_PER_TURN,
                        limit: MAX_URLS_PER_TURN,
                    },
                ));
            }

            if validated_files.len() > remaining_budget {
                for attachment in validated_files.drain(remaining_budget..) {
                    warnings.push(format!(
                        "Skipped {} (per-turn attachment budget exceeded)",
                        attachment.url.redacted_for_display()
                    ));
                }
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

            let mut prepared_content = Vec::new();
            let mut downloaded_successes = Vec::new();
            for outcome in download_outcomes {
                match outcome {
                    UrlDownloadOutcome::Success(success) => downloaded_successes.push(success),
                    UrlDownloadOutcome::Failure(failure) => {
                        failures.push(AttachmentFailure {
                            redacted_url: failure.redacted_url,
                            reason: failure.reason,
                        });
                    }
                }
            }

            if !downloaded_successes.is_empty() {
                let downloaded_paths = downloaded_successes
                    .iter()
                    .map(|success| success.path.clone())
                    .collect();
                let batch_prepared = resolve_and_prepare_local_files_for_injection(
                    FileInjectionContext {
                        session: session.as_ref(),
                        provider: &turn.provider,
                        config: turn.config.as_ref(),
                        http_client: session.file_upload_http_client(),
                    },
                    downloaded_paths,
                )
                .await;

                match batch_prepared {
                    Ok(prepared) => {
                        success_count = success_count.saturating_add(prepared.attachment_count);
                        prepared_content.extend(prepared.content_items);
                        warnings.extend(prepared.warnings);
                    }
                    Err(_) => {
                        for success in downloaded_successes {
                            match resolve_and_prepare_local_files_for_injection(
                                FileInjectionContext {
                                    session: session.as_ref(),
                                    provider: &turn.provider,
                                    config: turn.config.as_ref(),
                                    http_client: session.file_upload_http_client(),
                                },
                                vec![success.path],
                            )
                            .await
                            {
                                Ok(prepared) => {
                                    success_count =
                                        success_count.saturating_add(prepared.attachment_count);
                                    prepared_content.extend(prepared.content_items);
                                    warnings.extend(prepared.warnings);
                                }
                                Err(error) => failures.push(AttachmentFailure {
                                    redacted_url: success.url.redacted_for_display(),
                                    reason: error.to_string(),
                                }),
                            }
                        }
                    }
                }
            }

            if success_count > 0 {
                session
                    .inject_response_items_with_url_attachment_budget(
                        vec![user_message_response_input(prepared_content)],
                        success_count,
                        MAX_URLS_PER_TURN,
                    )
                    .await
                    .map_err(map_budget_injection_error)?;
            }
        }

        if success_count == 0 {
            return Err(FunctionCallError::RespondToModel(
                render_failure_only_summary(&failures),
            ));
        }

        let summary = render_summary(success_count, &failures, &warnings);
        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(summary),
            success: Some(success_count > 0),
        })
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
    success_count: usize,
    failures: &[AttachmentFailure],
    warnings: &[String],
) -> String {
    let mut summary = format!(
        "Attached {success_count} URL file(s). The file contents have been added to the conversation and are available for you to read and reference directly."
    );
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

fn user_message_response_input(content: Vec<ContentItem>) -> ResponseInputItem {
    ResponseInputItem::Message {
        role: "user".to_string(),
        content,
    }
}

fn map_budget_injection_error(
    error: crate::codex::UrlAttachmentInjectionError,
) -> FunctionCallError {
    match error {
        crate::codex::UrlAttachmentInjectionError::NoActiveTurn => {
            FunctionCallError::RespondToModel(
                "unable to attach URL files (no active turn)".to_string(),
            )
        }
        crate::codex::UrlAttachmentInjectionError::PerTurnLimitExceeded {
            attempted,
            current,
            limit,
        } => FunctionCallError::RespondToModel(format!(
            "attach_url_files would exceed the per-turn attachment limit ({current} + {attempted} > {limit})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::make_session_and_context;
    use crate::state::ActiveTurn;
    use crate::tools::url_downloader::prepopulate_pdf_cache;
    use crate::tools::url_validation::validated_url_for_test;
    use pretty_assertions::assert_eq;
    use tokio::sync::Mutex;

    const VALID_PDF_BYTES: &[u8] = b"%PDF-1.4\ntest content";

    #[tokio::test]
    async fn provider_path_injects_url_files() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_openai_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());

        let url = validated_url_for_test("https://example.com/doc.pdf");
        prepopulate_pdf_cache(&turn_context.config.codex_home, &url, VALID_PDF_BYTES).await;

        let handler = AttachUrlFilesHandler;
        let output = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-1".to_string(),
                tool_name: TOOL_NAME.to_string(),
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/doc.pdf"}]}"#.to_string(),
                },
            })
            .await
            .expect("tool call should succeed");

        let ToolOutput::Function { body, .. } = output else {
            panic!("expected function output");
        };
        assert!(
            body.to_text()
                .is_some_and(|text| text.contains("Attached 1 URL file(s)."))
        );

        let pending = session.get_pending_input().await;
        assert_eq!(pending.len(), 1);
        let ResponseInputItem::Message { content, .. } = &pending[0] else {
            panic!("expected message input");
        };
        assert!(matches!(content[0], ContentItem::UrlFile { .. }));
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
                tool_name: TOOL_NAME.to_string(),
                payload: ToolPayload::Function { arguments: args },
            })
            .await;

        assert!(matches!(result, Err(FunctionCallError::RespondToModel(_))));
    }

    #[tokio::test]
    async fn duplicate_urls_emit_warning_and_attach_once() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_openai_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());

        let url = validated_url_for_test("https://example.com/doc.pdf");
        prepopulate_pdf_cache(&turn_context.config.codex_home, &url, VALID_PDF_BYTES).await;

        let handler = AttachUrlFilesHandler;
        let output = handler
            .handle(ToolInvocation {
                session,
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-3".to_string(),
                tool_name: TOOL_NAME.to_string(),
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/doc.pdf"},{"url":"https://example.com/doc.pdf"}]}"#.to_string(),
                },
            })
            .await
            .expect("tool call should succeed");

        let ToolOutput::Function { body, success } = output else {
            panic!("expected function output");
        };
        let text = body.to_text().expect("text body");
        assert!(text.contains("Attached 1 URL file(s)."));
        assert!(text.contains("Skipped duplicate URL: https://example.com/doc.pdf"));
        assert_eq!(success, Some(true));
    }

    #[tokio::test]
    async fn download_path_fails_fast_when_per_turn_budget_is_exhausted() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_gemini_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());
        {
            let mut active = session.active_turn.lock().await;
            let active_turn = active.as_mut().expect("active turn");
            let mut turn_state = active_turn.turn_state.lock().await;
            turn_state
                .reserve_url_attachments(MAX_URLS_PER_TURN, MAX_URLS_PER_TURN)
                .expect("reserve URL attachment budget");
        }

        let handler = AttachUrlFilesHandler;
        let result = handler
            .handle(ToolInvocation {
                session,
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-4".to_string(),
                tool_name: TOOL_NAME.to_string(),
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/doc.pdf"}]}"#.to_string(),
                },
            })
            .await;

        let Err(FunctionCallError::RespondToModel(message)) = result else {
            panic!("expected per-turn budget error");
        };
        assert!(message.contains("per-turn attachment limit"));
    }

    #[tokio::test]
    async fn all_invalid_urls_produce_failure_only_summary() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_openai_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());

        let handler = AttachUrlFilesHandler;
        let result = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-5".to_string(),
                tool_name: TOOL_NAME.to_string(),
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

        let pending = session.get_pending_input().await;
        assert!(pending.is_empty(), "no pending input for all-invalid URLs");
    }

    #[tokio::test]
    async fn mixed_valid_and_invalid_urls_partial_success() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_openai_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());

        let url = validated_url_for_test("https://example.com/valid.pdf");
        prepopulate_pdf_cache(&turn_context.config.codex_home, &url, VALID_PDF_BYTES).await;

        let handler = AttachUrlFilesHandler;
        let output = handler
            .handle(ToolInvocation {
                session: Arc::clone(&session),
                turn: Arc::new(turn_context),
                tracker: Arc::new(Mutex::new(crate::turn_diff_tracker::TurnDiffTracker::new())),
                call_id: "call-6".to_string(),
                tool_name: TOOL_NAME.to_string(),
                payload: ToolPayload::Function {
                    arguments: r#"{"files":[{"url":"https://example.com/valid.pdf"},{"url":"ftp://bad.com/invalid.pdf"}]}"#.to_string(),
                },
            })
            .await
            .expect("mixed URLs should succeed overall");

        let ToolOutput::Function { body, success } = output else {
            panic!("expected function output");
        };
        let text = body.to_text().expect("text body");
        assert!(
            text.contains("Attached 1 URL file(s)."),
            "expected 1 attachment, got: {text}"
        );
        assert!(
            text.contains("Failures:"),
            "expected failures section, got: {text}"
        );
        assert_eq!(success, Some(true));

        let pending = session.get_pending_input().await;
        assert_eq!(pending.len(), 1);
        let ResponseInputItem::Message { content, .. } = &pending[0] else {
            panic!("expected message input");
        };
        assert_eq!(content.len(), 1);
        assert!(matches!(content[0], ContentItem::UrlFile { .. }));
    }

    #[test]
    fn tool_gating_by_web_search_mode() {
        let modes_that_register = [
            (Some(WebSearchMode::Cached), "Cached"),
            (Some(WebSearchMode::Live), "Live"),
            (None, "None"),
        ];
        for (mode, label) in modes_that_register {
            let mut builder = ToolRegistryBuilder::new();
            let config = ToolsConfig {
                web_search_mode: mode,
                ..minimal_tools_config()
            };
            register_attach_url_files(&mut builder, &config);
            let (specs, _) = builder.build();
            assert!(
                specs.iter().any(|s| s.spec.name() == TOOL_NAME),
                "expected {TOOL_NAME} registered for web_search_mode={label}"
            );
        }

        let mut builder = ToolRegistryBuilder::new();
        let config = ToolsConfig {
            web_search_mode: Some(WebSearchMode::Disabled),
            ..minimal_tools_config()
        };
        register_attach_url_files(&mut builder, &config);
        let (specs, _) = builder.build();
        assert!(
            !specs.iter().any(|s| s.spec.name() == TOOL_NAME),
            "expected {TOOL_NAME} NOT registered for web_search_mode=Disabled"
        );
    }

    #[tokio::test]
    async fn url_ingestion_path_rejects_non_pdf_downloads() {
        let (session, mut turn_context) = make_session_and_context().await;
        turn_context.provider = crate::ModelProviderInfo::create_openai_provider();
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());

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
                tool_name: TOOL_NAME.to_string(),
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

        let pending = session.get_pending_input().await;
        assert!(pending.is_empty(), "no pending input for rejected PDF");
    }

    fn minimal_tools_config() -> ToolsConfig {
        ToolsConfig {
            shell_type: codex_protocol::openai_models::ConfigShellToolType::Disabled,
            apply_patch_tool_type: None,
            web_search_mode: None,
            search_tool: false,
            collab_tools: false,
            collaboration_modes_tools: false,
            js_repl_enabled: false,
            js_repl_tools_only: false,
            experimental_supported_tools: Vec::new(),
        }
    }
}
