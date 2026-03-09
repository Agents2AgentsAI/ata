use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use codex_api::file_support::map_user_facing_file_error_from_message;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;

use super::Session;
use super::TurnContext;
use crate::context_manager::DroppedUrlFileInfo;
use crate::protocol::EventMsg;
use crate::protocol::WarningEvent;
use crate::tools::url_downloader::cache_entry_dir;
use crate::tools::url_validation::normalize_url_for_cache;

#[derive(Default)]
pub(crate) struct UrlFileRecoveryState {
    context_window_url_file_recovery_attempted: bool,
    file_rejection_url_file_recovery_attempted: bool,
    file_error_soft_recovery_attempted: bool,
}

impl UrlFileRecoveryState {
    pub(crate) async fn maybe_recover_context_window_exceeded(
        &mut self,
        sess: &Arc<Session>,
        turn_context: &Arc<TurnContext>,
    ) -> bool {
        if self.context_window_url_file_recovery_attempted {
            return false;
        }

        let dropped = drop_last_turn_url_file_attachments(sess).await;
        if dropped.is_empty() {
            return false;
        }

        self.context_window_url_file_recovery_attempted = true;
        sess.send_event(
            turn_context,
            EventMsg::Warning(WarningEvent {
                message: format!(
                    "Dropped {} attachment(s) because the context window was exceeded.",
                    dropped.len()
                ),
            }),
        )
        .await;
        true
    }

    pub(crate) async fn maybe_recover_file_related_invalid_request(
        &mut self,
        sess: &Arc<Session>,
        turn_context: &Arc<TurnContext>,
        message: &str,
    ) -> bool {
        let mapped = map_user_facing_file_error_from_message(message);

        if !self.file_rejection_url_file_recovery_attempted
            && let Some(mapped) = mapped.as_ref()
        {
            let user_message = &mapped.user_message;
            tracing::debug!(
                raw_message = %message,
                "file-related InvalidRequest mapped for recovery"
            );
            let dropped = drop_last_turn_url_file_attachments(sess).await;
            if !dropped.is_empty() {
                self.file_rejection_url_file_recovery_attempted = true;

                let codex_home = &turn_context.config.codex_home;
                let mut reinjected = Vec::new();
                for info in &dropped {
                    if let Some(url) = &info.url
                        && let Some(item) = read_cached_pdf_as_inline_content(
                            codex_home,
                            url,
                            info.filename.clone(),
                        )
                        .await
                    {
                        reinjected.push(item);
                    }
                }

                let message = if reinjected.is_empty() {
                    format!(
                        "Dropped {} file attachment(s) that the provider rejected: {user_message}",
                        dropped.len()
                    )
                } else {
                    let _ = sess
                        .inject_response_items(vec![ResponseInputItem::Message {
                            role: "user".to_string(),
                            content: reinjected,
                        }])
                        .await;
                    format!(
                        "Re-attached {} file(s) using cached bytes after provider rejected URL: {user_message}",
                        dropped.len()
                    )
                };

                sess.send_event(turn_context, EventMsg::Warning(WarningEvent { message }))
                    .await;
                return true;
            }

            tracing::info!("Turn error (file-related): {user_message}");
        } else if mapped.is_none() {
            tracing::info!("Turn error: {message:#}");
        }

        if !self.file_error_soft_recovery_attempted
            && let Some(mapped) = mapped.as_ref()
        {
            self.file_error_soft_recovery_attempted = true;
            let user_message = &mapped.user_message;
            tracing::info!("File error soft recovery: {user_message}");

            drop_last_turn_url_file_attachments(sess).await;

            sess.send_event(
                turn_context,
                EventMsg::Warning(WarningEvent {
                    message: format!("File error (continuing): {user_message}"),
                }),
            )
            .await;

            let _ = sess
                .inject_response_items(vec![ResponseInputItem::Message {
                    role: "user".to_string(),
                    content: vec![ContentItem::InputText {
                        text: format!(
                            "[System: A file attachment was rejected by the provider: \
                             \"{user_message}\". The file has been removed from context. \
                             Continue your task without the rejected file.]"
                        ),
                    }],
                }])
                .await;
            return true;
        }

        false
    }
}

/// Drops URL file attachments from the last turn in history.
/// Returns metadata about each dropped item for potential cache-based recovery.
async fn drop_last_turn_url_file_attachments(sess: &Arc<Session>) -> Vec<DroppedUrlFileInfo> {
    let url_attachments_in_turn = {
        let mut active = sess.active_turn.lock().await;
        match active.as_mut() {
            Some(active_turn) => {
                let turn_state = active_turn.turn_state.lock().await;
                turn_state.url_attachments_injected()
            }
            None => 0,
        }
    };
    let mut state = sess.state.lock().await;
    state
        .history
        .drop_last_turn_url_files(url_attachments_in_turn)
}

/// Reads a cached PDF from disk and returns it as an inline base64 ContentItem.
/// Returns `None` if the URL can't be parsed, the cache dir doesn't exist,
/// or no valid PDF is found.
async fn read_cached_pdf_as_inline_content(
    codex_home: &Path,
    url_str: &str,
    filename: Option<String>,
) -> Option<ContentItem> {
    let parsed_url = url::Url::parse(url_str).ok()?;
    let normalized_key = normalize_url_for_cache(&parsed_url);
    let cache_dir = cache_entry_dir(codex_home, &normalized_key);
    let mut entries = tokio::fs::read_dir(&cache_dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "pdf") {
            let bytes = tokio::fs::read(&path).await.ok()?;
            if bytes.starts_with(b"%PDF") {
                let base64_data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                return Some(ContentItem::inline_file(
                    base64_data,
                    "application/pdf".to_string(),
                    filename,
                ));
            }
        }
    }
    None
}
