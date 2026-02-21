use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use codex_protocol::models::ContentItem;

use super::Session;
use crate::context_manager::DroppedUrlFileInfo;
use crate::tools::url_downloader::cache_entry_dir;
use crate::tools::url_validation::normalize_url_for_cache;

/// Drops URL file attachments from the last turn in history.
/// Returns metadata about each dropped item for potential cache-based recovery.
pub(crate) async fn drop_last_turn_url_file_attachments(
    sess: &Arc<Session>,
) -> Vec<DroppedUrlFileInfo> {
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
pub(crate) async fn read_cached_pdf_as_inline_content(
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
