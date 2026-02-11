use crate::ModelProviderInfo;
use crate::codex::FileInputPreparationError;
use crate::codex::Session;
use crate::codex::file_capabilities_for_provider;
use crate::codex::resolve_and_prepare_file_inputs;
use crate::config::Config;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::user_input::UserInput;
use std::path::PathBuf;

pub(crate) struct FileInjectionContext<'a> {
    pub(crate) session: &'a Session,
    pub(crate) provider: &'a ModelProviderInfo,
    pub(crate) config: &'a Config,
    pub(crate) http_client: &'a reqwest::Client,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedLocalFilesForInjection {
    pub(crate) content_items: Vec<ContentItem>,
    pub(crate) warnings: Vec<String>,
    pub(crate) attachment_count: usize,
}

pub(crate) async fn resolve_and_prepare_local_files_for_injection(
    context: FileInjectionContext<'_>,
    paths: Vec<PathBuf>,
) -> Result<PreparedLocalFilesForInjection, FileInputPreparationError> {
    if paths.is_empty() {
        return Ok(PreparedLocalFilesForInjection {
            content_items: Vec::new(),
            warnings: Vec::new(),
            attachment_count: 0,
        });
    }

    let mut inputs: Vec<UserInput> = paths
        .into_iter()
        .map(|path| UserInput::LocalFile { path })
        .collect();

    {
        let (provider_id, _capabilities) =
            file_capabilities_for_provider(context.provider, context.config.model.as_deref());
        context
            .session
            .dedup_local_files_for_provider(&mut inputs, &provider_id)
            .await;
    }

    let outcome = resolve_and_prepare_file_inputs(
        &mut inputs,
        context.provider,
        context.config,
        context.http_client,
    )
    .await?;
    context
        .session
        .record_uploaded_files_and_paths(outcome.uploaded_files, &inputs)
        .await;

    let attachment_count = inputs
        .iter()
        .filter(|input| {
            matches!(
                input,
                UserInput::LocalFile { .. } | UserInput::UploadedFile { .. }
            )
        })
        .count();

    let content_items = match ResponseInputItem::from(inputs) {
        ResponseInputItem::Message { content, .. } => content,
        _ => Vec::new(),
    };

    Ok(PreparedLocalFilesForInjection {
        content_items,
        warnings: outcome.warnings,
        attachment_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex::make_session_and_context;
    use pretty_assertions::assert_eq;

    #[tokio::test(flavor = "multi_thread")]
    async fn prepares_local_file_content_items_for_injection() {
        let (session, turn_context) = make_session_and_context().await;
        let session = std::sync::Arc::new(session);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"%PDF-1.4\nhello").expect("write pdf");

        let prepared = resolve_and_prepare_local_files_for_injection(
            FileInjectionContext {
                session: session.as_ref(),
                provider: &turn_context.provider,
                config: turn_context.config.as_ref(),
                http_client: session.file_upload_http_client(),
            },
            vec![path],
        )
        .await
        .expect("prepare local file");

        assert_eq!(prepared.attachment_count, 1);
        assert_eq!(prepared.warnings, Vec::<String>::new());
        assert!(
            prepared
                .content_items
                .iter()
                .any(|item| matches!(item, ContentItem::InputFile { .. }))
        );
    }
}
