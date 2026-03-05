use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use codex_api::file_support::FileCapabilityConfig;
use codex_api::file_support::FileReferenceCache;
use codex_api::file_support::FileRoutingError;
use codex_api::file_support::FileUploadError;
use codex_api::file_support::file_capabilities_for;
use codex_api::file_support::maybe_upload_file;
use codex_api::file_support::upload::AnthropicFileUpload;
use codex_api::file_support::upload::FileUploadService;
use codex_api::file_support::upload::GeminiFileUpload;
use codex_api::file_support::upload::OpenAiFileUpload;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use codex_utils_file::FileProcessingError;
use codex_utils_file::analyze_file;
use codex_utils_file::bytes_to_megabytes;
use codex_utils_file::encode_inline_cached;
use codex_utils_file::file_name_or_default;
use futures::stream::StreamExt;
use tracing::warn;

use super::Session;
use super::TurnContext;
use crate::ModelProviderInfo;
use crate::config::Config;
use crate::model_provider_info::WireApi;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UrlAttachmentInjectionError {
    NoActiveTurn,
    PerTurnLimitExceeded {
        attempted: usize,
        current: usize,
        limit: usize,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FileInputPreparationError {
    #[error(transparent)]
    Processing(#[from] FileProcessingError),
    #[error(transparent)]
    Routing(#[from] FileRoutingError),
    #[error("failed to prepare local file attachments: {0}")]
    Task(String),
    #[error("provider `{provider}` does not support PDF attachments")]
    UnsupportedProvider { provider: String },
    #[error(
        "file `{path}` is {size_mb:.1} MB and exceeds the inline limit of {max_mb:.1} MB for provider `{provider}`"
    )]
    InlineFileTooLarge {
        provider: String,
        path: String,
        size_mb: f64,
        max_mb: f64,
    },
    #[error(
        "total local file payload is {total_mb:.1} MB and exceeds the inline budget of {max_mb:.1} MB for provider `{provider}`"
    )]
    InlinePayloadTooLarge {
        provider: String,
        total_mb: f64,
        max_mb: f64,
    },
}

pub(crate) fn file_capabilities_for_provider(
    provider: &ModelProviderInfo,
    model: Option<&str>,
) -> (String, FileCapabilityConfig) {
    let provider_id = match provider.wire_api {
        WireApi::Responses => "openai",
        WireApi::AnthropicMessages => "anthropic",
        WireApi::GeminiGenerate => "gemini",
    };
    (
        provider_id.to_string(),
        file_capabilities_for(provider_id, model),
    )
}

/// Rewrite `LocalFile` inputs to `UploadedFile` when the cache has a valid
/// entry for the same canonical path, provider, and mtime.
fn dedup_local_files_from_cache(
    inputs: &mut [UserInput],
    cache: &FileReferenceCache,
    provider_id: &str,
    now: SystemTime,
) {
    for input in inputs.iter_mut() {
        let UserInput::LocalFile { path } = input else {
            continue;
        };
        let Ok(canonical) = std::fs::canonicalize(&*path) else {
            tracing::debug!(path = %path.display(), "file dedup: canonicalize failed");
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&canonical) else {
            tracing::debug!(path = %canonical.display(), "file dedup: metadata failed");
            continue;
        };
        let Ok(mtime) = metadata.modified() else {
            tracing::debug!(path = %canonical.display(), "file dedup: mtime failed");
            continue;
        };

        if let Some(hit) = cache.lookup_by_path(&canonical, mtime, provider_id, now) {
            tracing::debug!(
                path = %path.display(),
                file_id = %hit.file_id,
                "reusing previously uploaded file"
            );
            *input = UserInput::UploadedFile {
                file_id: hit.file_id,
                mime_type: hit.mime_type,
                filename: hit.filename,
                source_path: std::mem::take(path),
            };
        }
    }
}

fn extract_local_pdf_paths_from_text_inputs(
    inputs: &[UserInput],
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) -> Vec<PathBuf> {
    let mut discovered = Vec::new();
    let mut seen = HashSet::new();
    for input in inputs {
        let UserInput::Text { text, .. } = input else {
            continue;
        };
        for token in split_text_path_tokens(text) {
            let candidate = strip_wrapping_quote_pair(token);
            if !is_pdf_path_token(candidate) || looks_like_url(candidate) {
                continue;
            }
            let resolved = resolve_candidate_path(candidate, cwd);
            let Ok(canonical) = std::fs::canonicalize(&resolved) else {
                continue;
            };
            if !canonical.is_file()
                || !is_canonical_path_within_allowed_roots(&canonical, cwd, sandbox_policy)
            {
                continue;
            }
            if seen.insert(canonical.clone()) {
                discovered.push(canonical);
            }
        }
    }
    discovered
}

fn split_text_path_tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();

    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            let mut token_end = text.len();
            let mut found_closing = false;
            for (idx, next_ch) in chars.by_ref() {
                if next_ch == quote {
                    token_end = idx + next_ch.len_utf8();
                    found_closing = true;
                    break;
                }
            }

            if found_closing
                && chars
                    .peek()
                    .is_none_or(|(_, trailing)| trailing.is_whitespace())
            {
                tokens.push(&text[start..token_end]);
                continue;
            }

            while let Some((idx, next_ch)) = chars.peek().copied() {
                if next_ch.is_whitespace() {
                    token_end = idx;
                    break;
                }
                token_end = idx + next_ch.len_utf8();
                let _ = chars.next();
            }
            tokens.push(&text[start..token_end]);
            continue;
        }

        let mut token_end = text.len();
        while let Some((idx, next_ch)) = chars.peek().copied() {
            if next_ch.is_whitespace() {
                token_end = idx;
                break;
            }
            token_end = idx + next_ch.len_utf8();
            let _ = chars.next();
        }
        tokens.push(&text[start..token_end]);
    }

    tokens
}

fn strip_wrapping_quote_pair(token: &str) -> &str {
    if token.len() >= 2
        && ((token.starts_with('"') && token.ends_with('"'))
            || (token.starts_with('\'') && token.ends_with('\'')))
    {
        return &token[1..token.len() - 1];
    }
    token
}

fn is_pdf_path_token(token: &str) -> bool {
    !token.is_empty() && token.to_ascii_lowercase().ends_with(".pdf")
}

fn looks_like_url(token: &str) -> bool {
    token.contains("://")
}

fn resolve_candidate_path(token: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(token);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn canonical_or_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_canonical_path_within_allowed_roots(
    canonical_path: &Path,
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) -> bool {
    let canonical_cwd = canonical_or_path(cwd);
    if canonical_path.starts_with(&canonical_cwd) {
        return true;
    }

    sandbox_policy
        .get_writable_roots_with_cwd(cwd)
        .iter()
        .any(|root| {
            if root.is_path_writable(canonical_path) {
                return true;
            }

            let canonical_root = canonical_or_path(root.root.as_path());
            if !canonical_path.starts_with(&canonical_root) {
                return false;
            }

            for read_only_subpath in &root.read_only_subpaths {
                let canonical_subpath = canonical_or_path(read_only_subpath.as_path());
                if canonical_path.starts_with(&canonical_subpath) {
                    return false;
                }
            }
            true
        })
}

pub(crate) fn inject_local_pdf_paths_from_text_inputs(
    inputs: &mut Vec<UserInput>,
    cwd: &Path,
    sandbox_policy: &SandboxPolicy,
) {
    let mut seen_paths = HashSet::new();
    for input in inputs.iter() {
        match input {
            UserInput::LocalFile { path } => {
                seen_paths.insert(canonical_or_path(path));
            }
            UserInput::UploadedFile { source_path, .. } => {
                seen_paths.insert(canonical_or_path(source_path));
            }
            _ => {}
        }
    }

    let discovered = extract_local_pdf_paths_from_text_inputs(inputs, cwd, sandbox_policy);
    for path in discovered {
        if seen_paths.insert(path.clone()) {
            inputs.push(UserInput::LocalFile { path });
        }
    }
}

/// After a successful upload round, record path→file_id mappings for future dedup.
fn record_upload_paths(cache: &mut FileReferenceCache, inputs: &[UserInput]) {
    for input in inputs {
        let UserInput::UploadedFile {
            file_id,
            mime_type,
            filename,
            source_path,
        } = input
        else {
            continue;
        };
        let Ok(canonical) = std::fs::canonicalize(source_path) else {
            tracing::debug!(
                path = %source_path.display(),
                "record_upload_paths: canonicalize failed"
            );
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&canonical) else {
            tracing::debug!(
                path = %canonical.display(),
                "record_upload_paths: metadata failed"
            );
            continue;
        };
        let Ok(mtime) = metadata.modified() else {
            tracing::debug!(path = %canonical.display(), "record_upload_paths: mtime failed");
            continue;
        };
        let Some(uploaded) = cache.get(file_id) else {
            tracing::warn!(
                file_id,
                "skipping path record: file_id not found in cache entries"
            );
            continue;
        };
        let (expires_at, provider) = (uploaded.expires_at, uploaded.provider.clone());
        cache.record_path(
            canonical.clone(),
            file_id,
            &provider,
            mime_type.clone(),
            filename.clone(),
            mtime,
            expires_at,
        );
        tracing::debug!(
            path = %canonical.display(),
            file_id,
            provider,
            "recorded file upload path in cache"
        );
    }
}

/// Convert a base64 payload budget into raw-byte budget using the 4:3 base64 expansion ratio.
fn max_raw_inline_bytes(max_inline_payload_bytes: u64) -> u64 {
    max_inline_payload_bytes.saturating_mul(3).saturating_div(4)
}

fn upload_base_url_for_provider(provider_id: &str, provider: &ModelProviderInfo) -> String {
    let fallback = match provider_id {
        "openai" => "https://api.openai.com",
        "anthropic" => "https://api.anthropic.com",
        "gemini" => "https://generativelanguage.googleapis.com",
        _ => "",
    };
    let base = provider
        .base_url
        .as_deref()
        .unwrap_or(fallback)
        .trim_end_matches('/');

    match provider_id {
        "openai" | "anthropic" => base.strip_suffix("/v1").unwrap_or(base).to_string(),
        "gemini" => base.strip_suffix("/v1beta").unwrap_or(base).to_string(),
        _ => base.to_string(),
    }
}

fn upload_service_for_provider(provider_id: &str) -> Option<Box<dyn FileUploadService>> {
    match provider_id {
        "openai" => Some(Box::new(OpenAiFileUpload)),
        "anthropic" => Some(Box::new(AnthropicFileUpload)),
        "gemini" => Some(Box::new(GeminiFileUpload)),
        _ => None,
    }
}

async fn delete_uploaded_files_best_effort(
    uploaded_files: &[codex_api::file_support::UploadedFile],
    upload_service: &dyn FileUploadService,
    http_client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
) {
    let mut seen = HashSet::new();
    for uploaded in uploaded_files {
        if !seen.insert(uploaded.file_id.as_str()) {
            continue;
        }

        if let Err(error) = upload_service
            .delete_file(http_client, &uploaded.file_id, api_key, base_url)
            .await
        {
            warn!(
                file_id = %uploaded.file_id,
                provider = %uploaded.provider,
                %error,
                "failed to delete orphaned uploaded file"
            );
        }
    }
}

#[derive(Debug)]
pub(crate) struct FileUploadOutcome {
    pub(crate) uploaded_files: Vec<codex_api::file_support::UploadedFile>,
    pub(crate) warnings: Vec<String>,
}

/// Maximum number of concurrent file uploads.
const MAX_CONCURRENT_FILE_UPLOADS: usize = 4;

async fn resolve_file_inputs_for_uploads(
    inputs: &mut [UserInput],
    provider: &ModelProviderInfo,
    config: &Config,
    http_client: &reqwest::Client,
) -> std::result::Result<FileUploadOutcome, FileInputPreparationError> {
    let empty = Ok(FileUploadOutcome {
        uploaded_files: Vec::new(),
        warnings: Vec::new(),
    });

    let (provider_id, capabilities) =
        file_capabilities_for_provider(provider, config.model.as_deref());
    if !capabilities.supports_pdf {
        tracing::debug!("skipping file uploads: provider does not support PDF");
        return empty;
    }

    let Some(api_key) = provider
        .api_key_with_auth(&config.codex_home, config.cli_auth_credentials_store_mode)
        .ok()
        .flatten()
    else {
        tracing::debug!("skipping file uploads: no API key available for file upload");
        return empty;
    };
    let Some(upload_service) = upload_service_for_provider(&provider_id) else {
        tracing::debug!(
            provider_id,
            "skipping file uploads: no upload service for provider"
        );
        return empty;
    };
    let base_url = upload_base_url_for_provider(&provider_id, provider);

    // Collect (index, path) for all LocalFile entries.
    let file_entries: Vec<(usize, PathBuf)> = inputs
        .iter()
        .enumerate()
        .filter_map(|(i, input)| match input {
            UserInput::LocalFile { path } => Some((i, path.clone())),
            _ => None,
        })
        .collect();

    if file_entries.is_empty() {
        return empty;
    }

    type UploadResult = (
        usize,
        PathBuf,
        Result<codex_api::file_support::MaybeUploadResult, FileRoutingError>,
    );

    let mut results: Vec<UploadResult> = futures::stream::iter(file_entries)
        .map(|(idx, path)| {
            let capabilities = &capabilities;
            let upload_service = upload_service.as_ref();
            let api_key = &api_key;
            let base_url = &base_url;
            async move {
                let result = maybe_upload_file(
                    &path,
                    capabilities,
                    Some(upload_service),
                    http_client,
                    api_key,
                    base_url,
                )
                .await;
                (idx, path, result)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_FILE_UPLOADS)
        .collect()
        .await;

    // Sort by original index so outputs are deterministic regardless of completion order.
    results.sort_by_key(|(idx, _, _)| *idx);

    // Process results: collect uploads, warnings, and check for errors.
    let mut uploaded_files = Vec::new();
    let mut warnings = Vec::new();
    let mut first_error: Option<FileRoutingError> = None;

    for (idx, path, result) in results {
        match result {
            Ok(upload_result) => {
                tracing::debug!(
                    path = %path.display(),
                    uploaded = upload_result.uploaded.is_some(),
                    has_warning = upload_result.warning.is_some(),
                    "file upload result"
                );
                if let Some(warning) = upload_result.warning {
                    warnings.push(warning);
                }
                if let Some((uploaded, metadata)) = upload_result.uploaded {
                    let file_id = uploaded.file_id.clone();
                    uploaded_files.push(uploaded);
                    inputs[idx] = UserInput::UploadedFile {
                        file_id,
                        mime_type: metadata.mime_type,
                        filename: metadata.filename,
                        source_path: path,
                    };
                }
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if let Some(error) = first_error {
        delete_uploaded_files_best_effort(
            &uploaded_files,
            upload_service.as_ref(),
            http_client,
            &api_key,
            &base_url,
        )
        .await;
        return Err(error.into());
    }

    Ok(FileUploadOutcome {
        uploaded_files,
        warnings,
    })
}

fn prepare_file_inputs(
    inputs: &[UserInput],
    provider: &ModelProviderInfo,
    model: Option<&str>,
) -> std::result::Result<(), FileInputPreparationError> {
    let (provider_id, capabilities) = file_capabilities_for_provider(provider, model);
    let has_file_input = inputs.iter().any(|input| {
        matches!(
            input,
            UserInput::LocalFile { .. } | UserInput::UploadedFile { .. }
        )
    });
    if has_file_input && !capabilities.supports_pdf {
        return Err(FileInputPreparationError::UnsupportedProvider {
            provider: provider_id,
        });
    }

    let max_total_raw_bytes = max_raw_inline_bytes(capabilities.max_inline_payload_bytes);
    let mut total_raw_bytes = 0_u64;

    // UploadedFile entries already reference provider-side file IDs and do not consume inline
    // request payload budget.
    for input in inputs {
        if let UserInput::LocalFile { path } = input {
            let metadata = analyze_file(path)?;
            if metadata.size_bytes > capabilities.max_inline_file_size {
                return Err(FileInputPreparationError::InlineFileTooLarge {
                    provider: provider_id,
                    path: path.display().to_string(),
                    size_mb: bytes_to_megabytes(metadata.size_bytes),
                    max_mb: bytes_to_megabytes(capabilities.max_inline_file_size),
                });
            }

            total_raw_bytes = total_raw_bytes.saturating_add(metadata.size_bytes);
            if total_raw_bytes > max_total_raw_bytes {
                return Err(FileInputPreparationError::InlinePayloadTooLarge {
                    provider: provider_id,
                    total_mb: bytes_to_megabytes(total_raw_bytes),
                    max_mb: bytes_to_megabytes(max_total_raw_bytes),
                });
            }

            encode_inline_cached(path)?;
        }
    }

    Ok(())
}

async fn prepare_file_inputs_async(
    inputs: &[UserInput],
    provider: &ModelProviderInfo,
    model: Option<&str>,
) -> std::result::Result<(), FileInputPreparationError> {
    let inputs = inputs.to_vec();
    let provider = provider.clone();
    let model = model.map(str::to_string);
    tokio::task::spawn_blocking(move || prepare_file_inputs(&inputs, &provider, model.as_deref()))
        .await
        .map_err(|error| FileInputPreparationError::Task(error.to_string()))?
}

pub(crate) async fn resolve_and_prepare_file_inputs(
    inputs: &mut [UserInput],
    provider: &ModelProviderInfo,
    config: &Config,
    http_client: &reqwest::Client,
) -> std::result::Result<FileUploadOutcome, FileInputPreparationError> {
    // Upload-step errors are handled inside `resolve_file_inputs_for_uploads`, which already
    // performs best-effort cleanup of any earlier uploads.
    let outcome = resolve_file_inputs_for_uploads(inputs, provider, config, http_client).await?;

    if let Err(error) = prepare_file_inputs_async(inputs, provider, config.model.as_deref()).await {
        if !outcome.uploaded_files.is_empty() {
            let (provider_id, _capabilities) =
                file_capabilities_for_provider(provider, config.model.as_deref());
            if let Some(api_key) = provider
                .api_key_with_auth(&config.codex_home, config.cli_auth_credentials_store_mode)
                .ok()
                .flatten()
                && let Some(upload_service) = upload_service_for_provider(&provider_id)
            {
                let base_url = upload_base_url_for_provider(&provider_id, provider);
                delete_uploaded_files_best_effort(
                    &outcome.uploaded_files,
                    upload_service.as_ref(),
                    http_client,
                    &api_key,
                    &base_url,
                )
                .await;
            }
        }

        return Err(error);
    }

    Ok(outcome)
}

#[derive(Debug, thiserror::Error)]
pub(super) enum FileReferenceRefreshError {
    #[error(
        "cannot refresh uploaded file attachments without an API key for provider `{provider}`"
    )]
    MissingApiKey { provider: String },
    #[error("provider `{provider}` does not support file uploads")]
    MissingUploadService { provider: String },
    #[error("failed to refresh file attachment `{filename}`: {error}")]
    RefreshFailed {
        filename: String,
        #[source]
        error: FileUploadError,
    },
}

fn collect_referenced_upload_file_ids(
    items: &[ResponseItem],
) -> (Vec<String>, HashMap<String, Option<String>>) {
    let mut referenced_file_ids = Vec::new();
    let mut referenced_mime_types = HashMap::new();

    for item in items {
        let ResponseItem::Message { content, .. } = item else {
            continue;
        };

        for content_item in content {
            let ContentItem::InputFile {
                file_id: Some(file_id),
                mime_type,
                ..
            } = content_item
            else {
                continue;
            };

            referenced_file_ids.push(file_id.clone());
            referenced_mime_types
                .entry(file_id.clone())
                .or_insert_with(|| mime_type.clone());
        }
    }

    (referenced_file_ids, referenced_mime_types)
}

fn rewrite_uploaded_file_ids(items: &mut [ResponseItem], replacements: &HashMap<String, String>) {
    for item in items {
        let ResponseItem::Message { content, .. } = item else {
            continue;
        };

        for content_item in content {
            let ContentItem::InputFile {
                file_id: Some(file_id),
                ..
            } = content_item
            else {
                continue;
            };

            if let Some(new_file_id) = replacements.get(file_id) {
                *file_id = new_file_id.clone();
            }
        }
    }
}

pub(super) async fn refresh_uploaded_file_references(
    sess: &Session,
    turn_context: &TurnContext,
) -> Result<(), FileReferenceRefreshError> {
    let (provider_id, _capabilities) = file_capabilities_for_provider(
        &turn_context.provider,
        turn_context.config.model.as_deref(),
    );

    let (referenced_file_ids, referenced_mime_types) = {
        let state = sess.state.lock().await;
        collect_referenced_upload_file_ids(state.history.raw_items())
    };
    if referenced_file_ids.is_empty() {
        return Ok(());
    }

    let now = SystemTime::now();
    let refresh_candidates = {
        let cache = sess.services.file_reference_cache.lock().await;
        cache.refresh_candidates(
            referenced_file_ids.iter().map(String::as_str),
            &provider_id,
            now,
        )
    };
    if refresh_candidates.is_empty() {
        return Ok(());
    }

    let requires_provider_switch_refresh = refresh_candidates
        .iter()
        .any(|entry| entry.provider != provider_id);

    let Some(api_key) = turn_context
        .provider
        .api_key_with_auth(
            &turn_context.config.codex_home,
            turn_context.config.cli_auth_credentials_store_mode,
        )
        .ok()
        .flatten()
    else {
        if requires_provider_switch_refresh {
            return Err(FileReferenceRefreshError::MissingApiKey {
                provider: provider_id,
            });
        }
        warn!(
            provider_id = %provider_id,
            "skipping uploaded file refresh because no API key is configured"
        );
        return Ok(());
    };

    let Some(upload_service) = upload_service_for_provider(&provider_id) else {
        if requires_provider_switch_refresh {
            return Err(FileReferenceRefreshError::MissingUploadService {
                provider: provider_id,
            });
        }
        warn!(
            provider_id = %provider_id,
            "skipping uploaded file refresh because upload service is unavailable"
        );
        return Ok(());
    };

    let base_url = upload_base_url_for_provider(&provider_id, &turn_context.provider);
    let http_client = &sess.services.file_upload_http_client;

    let mut updated_file_ids = HashMap::new();
    let mut refreshed_uploads = Vec::new();
    let mut stale_file_ids = Vec::new();

    for candidate in refresh_candidates {
        let is_provider_switch = candidate.provider != provider_id;
        let mime_type = referenced_mime_types
            .get(&candidate.file_id)
            .and_then(|m| m.as_deref())
            .unwrap_or("application/pdf");

        match upload_service
            .upload_file(
                http_client,
                &candidate.source_path,
                mime_type,
                &api_key,
                &base_url,
            )
            .await
        {
            Ok(uploaded) => {
                updated_file_ids.insert(candidate.file_id.clone(), uploaded.file_id.clone());
                stale_file_ids.push(candidate.file_id);
                refreshed_uploads.push(uploaded);
            }
            Err(error) if is_provider_switch => {
                return Err(FileReferenceRefreshError::RefreshFailed {
                    filename: file_name_or_default(&candidate.source_path, "file"),
                    error,
                });
            }
            Err(error) => {
                warn!(
                    file_id = %candidate.file_id,
                    path = %candidate.source_path.display(),
                    %error,
                    "failed to refresh near-expiry uploaded file; continuing with existing reference"
                );
            }
        }
    }

    if updated_file_ids.is_empty() {
        return Ok(());
    }

    {
        let mut state = sess.state.lock().await;
        let mut items = state.history.raw_items().to_vec();
        rewrite_uploaded_file_ids(&mut items, &updated_file_ids);
        state.replace_history(items, None);
    }

    {
        let mut cache = sess.services.file_reference_cache.lock().await;
        for file_id in &stale_file_ids {
            cache.remove(file_id);
        }
        cache.record_all(refreshed_uploads);
    }

    Ok(())
}

impl Session {
    /// Injects response items while atomically enforcing a per-turn URL attachment cap.
    pub(crate) async fn inject_response_items_with_url_attachment_budget(
        &self,
        input: Vec<ResponseInputItem>,
        url_attachments_to_add: usize,
        per_turn_limit: usize,
    ) -> Result<(), UrlAttachmentInjectionError> {
        let mut active = self.active_turn.lock().await;
        let Some(at) = active.as_mut() else {
            return Err(UrlAttachmentInjectionError::NoActiveTurn);
        };

        let mut turn_state = at.turn_state.lock().await;
        if let Err(current) =
            turn_state.reserve_url_attachments(url_attachments_to_add, per_turn_limit)
        {
            return Err(UrlAttachmentInjectionError::PerTurnLimitExceeded {
                attempted: url_attachments_to_add,
                current,
                limit: per_turn_limit,
            });
        }

        for item in input {
            turn_state.push_pending_input(item);
        }
        Ok(())
    }

    pub(crate) fn file_upload_http_client(&self) -> &reqwest::Client {
        &self.services.file_upload_http_client
    }

    pub(crate) async fn dedup_local_files_for_provider(
        &self,
        input: &mut [UserInput],
        provider_id: &str,
    ) {
        let cache = self.services.file_reference_cache.lock().await;
        dedup_local_files_from_cache(input, &cache, provider_id, SystemTime::now());
    }

    pub(crate) async fn record_uploaded_files_and_paths(
        &self,
        uploaded_files: Vec<codex_api::file_support::UploadedFile>,
        input: &[UserInput],
    ) {
        if uploaded_files.is_empty() {
            return;
        }
        let mut cache = self.services.file_reference_cache.lock().await;
        cache.record_all(uploaded_files);
        record_upload_paths(&mut cache, input);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::AuthCredentialsStoreMode;
    use crate::auth::PROVIDER_OPENAI;
    use crate::auth::login_with_provider_api_key;
    use crate::codex::SteerInputError;
    use crate::config::ConfigBuilder;
    use codex_protocol::protocol::ReadOnlyAccess;
    use codex_protocol::protocol::SandboxPolicy;

    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    fn workspace_policy_restricted_to_cwd() -> SandboxPolicy {
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            read_only_access: ReadOnlyAccess::FullAccess,
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    }

    fn local_file_count(inputs: &[UserInput]) -> usize {
        inputs
            .iter()
            .filter(|input| matches!(input, UserInput::LocalFile { .. }))
            .count()
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_attaches_relative_pdf_in_cwd() {
        let cwd = tempfile::tempdir().expect("cwd");
        let file = cwd.path().join("report.pdf");
        std::fs::write(&file, b"%PDF-1.4\n").expect("write pdf");

        let mut inputs = vec![UserInput::Text {
            text: "summarize report.pdf".to_string(),
            text_elements: Vec::new(),
        }];
        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 1);
        assert_eq!(
            inputs,
            vec![
                UserInput::Text {
                    text: "summarize report.pdf".to_string(),
                    text_elements: Vec::new(),
                },
                UserInput::LocalFile {
                    path: std::fs::canonicalize(file).expect("canonical report path"),
                },
            ]
        );
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_attaches_quoted_pdf_path_with_spaces() {
        let cwd = tempfile::tempdir().expect("cwd");
        let file = cwd.path().join("meeting notes.pdf");
        std::fs::write(&file, b"%PDF-1.4\n").expect("write pdf");

        let mut inputs = vec![UserInput::Text {
            text: "summarize \"meeting notes.pdf\"".to_string(),
            text_elements: Vec::new(),
        }];
        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 1);
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_ignores_missing_paths() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut inputs = vec![UserInput::Text {
            text: "summarize missing.pdf".to_string(),
            text_elements: Vec::new(),
        }];

        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 0);
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_ignores_paths_outside_allowed_roots() {
        let cwd = tempfile::tempdir().expect("cwd");
        let outside_dir = tempfile::tempdir().expect("outside");
        let outside_pdf = outside_dir.path().join("outside.pdf");
        std::fs::write(&outside_pdf, b"%PDF-1.4\n").expect("write outside pdf");
        let outside_pdf_text = outside_pdf.display().to_string();

        let mut inputs = vec![UserInput::Text {
            text: format!("summarize {outside_pdf_text}"),
            text_elements: Vec::new(),
        }];
        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 0);
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_ignores_urls() {
        let cwd = tempfile::tempdir().expect("cwd");
        let mut inputs = vec![UserInput::Text {
            text: "summarize https://example.com/report.pdf".to_string(),
            text_elements: Vec::new(),
        }];

        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 0);
    }

    #[test]
    fn inject_local_pdf_paths_from_text_inputs_dedups_repeated_tokens_and_existing_entries() {
        let cwd = tempfile::tempdir().expect("cwd");
        let file = cwd.path().join("report.pdf");
        std::fs::write(&file, b"%PDF-1.4\n").expect("write pdf");

        let mut inputs = vec![
            UserInput::Text {
                text: "report.pdf report.pdf".to_string(),
                text_elements: Vec::new(),
            },
            UserInput::LocalFile { path: file },
        ];
        inject_local_pdf_paths_from_text_inputs(
            &mut inputs,
            cwd.path(),
            &workspace_policy_restricted_to_cwd(),
        );

        assert_eq!(local_file_count(&inputs), 1);
    }

    #[tokio::test]
    async fn steer_input_surfaces_file_prepare_errors() {
        let (sess, _tc, _rx) = crate::codex::make_session_and_context_with_rx().await;
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("missing.pdf");

        let err = sess
            .steer_input(vec![UserInput::LocalFile { path: missing }], None)
            .await
            .expect_err("missing local file should fail");

        assert!(matches!(err, SteerInputError::InvalidFileInput(_)));
    }

    #[test]
    fn prepare_file_inputs_rejects_file_over_provider_inline_limit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("large.pdf");
        std::fs::write(&path, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open pdf")
            .set_len(21 * 1024 * 1024)
            .expect("grow file");

        let provider = ModelProviderInfo::create_gemini_provider();
        let err = prepare_file_inputs(&[UserInput::LocalFile { path }], &provider, None)
            .expect_err("gemini inline limit should reject 21MB file");
        assert!(matches!(
            err,
            FileInputPreparationError::InlineFileTooLarge { .. }
        ));
    }

    #[test]
    fn prepare_file_inputs_rejects_total_inline_payload_budget_overflow() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("first.pdf");
        let second = dir.path().join("second.pdf");

        for path in [&first, &second] {
            std::fs::write(path, b"%PDF-1.4\n").expect("write pdf header");
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("open pdf")
                .set_len(8 * 1024 * 1024)
                .expect("grow file");
        }

        let provider = ModelProviderInfo::create_gemini_provider();
        let err = prepare_file_inputs(
            &[
                UserInput::LocalFile { path: first },
                UserInput::LocalFile { path: second },
            ],
            &provider,
            None,
        )
        .expect_err("gemini inline payload budget should reject 16MB raw payload");
        assert!(matches!(
            err,
            FileInputPreparationError::InlinePayloadTooLarge { .. }
        ));
    }

    #[tokio::test]
    async fn resolve_file_inputs_for_uploads_rewrites_to_uploaded_file() {
        let codex_home = tempfile::tempdir().expect("codex home");
        login_with_provider_api_key(
            codex_home.path(),
            PROVIDER_OPENAI,
            "sk-test-key",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "filename": "mid.pdf",
                "purpose": "user_data",
                "bytes": 10
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut provider = ModelProviderInfo::create_openai_provider();
        provider.base_url = Some(format!("{}/v1", server.uri()));

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("mid.pdf");
        std::fs::write(&path, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open pdf")
            .set_len(3 * 1024 * 1024)
            .expect("grow file");

        let mut inputs = vec![UserInput::LocalFile { path: path.clone() }];
        let http_client = reqwest::Client::new();
        let outcome =
            resolve_file_inputs_for_uploads(&mut inputs, &provider, &config, &http_client)
                .await
                .expect("resolve files");

        assert_eq!(
            inputs,
            vec![UserInput::UploadedFile {
                file_id: "file-123".to_string(),
                mime_type: "application/pdf".to_string(),
                filename: "mid.pdf".to_string(),
                source_path: path.clone(),
            }]
        );
        assert_eq!(
            outcome.uploaded_files,
            vec![codex_api::file_support::UploadedFile {
                file_id: "file-123".to_string(),
                provider: "openai".to_string(),
                expires_at: None,
                source_path: path,
            }]
        );
        assert!(outcome.warnings.is_empty());
    }

    #[tokio::test]
    async fn resolve_file_inputs_for_uploads_routes_multiple_files() {
        let codex_home = tempfile::tempdir().expect("codex home");
        login_with_provider_api_key(
            codex_home.path(),
            PROVIDER_OPENAI,
            "sk-test-key",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "filename": "uploaded.pdf",
                "purpose": "user_data",
                "bytes": 10
            })))
            .expect(2)
            .mount(&server)
            .await;

        let mut provider = ModelProviderInfo::create_openai_provider();
        provider.base_url = Some(format!("{}/v1", server.uri()));

        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("first.pdf");
        let second = dir.path().join("second.pdf");
        for path in [&first, &second] {
            std::fs::write(path, b"%PDF-1.4\n").expect("write pdf header");
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("open pdf")
                .set_len(3 * 1024 * 1024)
                .expect("grow file");
        }

        let mut inputs = vec![
            UserInput::LocalFile {
                path: first.clone(),
            },
            UserInput::LocalFile {
                path: second.clone(),
            },
        ];
        let http_client = reqwest::Client::new();
        let outcome =
            resolve_file_inputs_for_uploads(&mut inputs, &provider, &config, &http_client)
                .await
                .expect("resolve files");

        assert_eq!(
            inputs,
            vec![
                UserInput::UploadedFile {
                    file_id: "file-123".to_string(),
                    mime_type: "application/pdf".to_string(),
                    filename: "first.pdf".to_string(),
                    source_path: first.clone(),
                },
                UserInput::UploadedFile {
                    file_id: "file-123".to_string(),
                    mime_type: "application/pdf".to_string(),
                    filename: "second.pdf".to_string(),
                    source_path: second.clone(),
                },
            ]
        );
        assert_eq!(
            outcome.uploaded_files,
            vec![
                codex_api::file_support::UploadedFile {
                    file_id: "file-123".to_string(),
                    provider: "openai".to_string(),
                    expires_at: None,
                    source_path: first,
                },
                codex_api::file_support::UploadedFile {
                    file_id: "file-123".to_string(),
                    provider: "openai".to_string(),
                    expires_at: None,
                    source_path: second,
                },
            ]
        );
    }

    #[tokio::test]
    async fn resolve_file_inputs_for_uploads_cleans_up_orphaned_uploads_on_error() {
        let codex_home = tempfile::tempdir().expect("codex home");
        login_with_provider_api_key(
            codex_home.path(),
            PROVIDER_OPENAI,
            "sk-test-key",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "filename": "uploaded.pdf",
                "purpose": "user_data",
                "bytes": 10
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/v1/files/file-123"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "deleted": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut provider = ModelProviderInfo::create_openai_provider();
        provider.base_url = Some(format!("{}/v1", server.uri()));

        let dir = tempfile::tempdir().expect("temp dir");
        let uploaded_path = dir.path().join("uploaded.pdf");
        std::fs::write(&uploaded_path, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&uploaded_path)
            .expect("open pdf")
            .set_len(3 * 1024 * 1024)
            .expect("grow file");

        let too_large_path = dir.path().join("too_large.pdf");
        std::fs::write(&too_large_path, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&too_large_path)
            .expect("open pdf")
            .set_len(codex_utils_file::MAX_FILE_SIZE + 1)
            .expect("grow file");

        let mut inputs = vec![
            UserInput::LocalFile {
                path: uploaded_path,
            },
            UserInput::LocalFile {
                path: too_large_path,
            },
        ];
        let http_client = reqwest::Client::new();
        let err = resolve_file_inputs_for_uploads(&mut inputs, &provider, &config, &http_client)
            .await
            .expect_err("oversized file should fail routing");
        assert!(matches!(
            err,
            FileInputPreparationError::Routing(FileRoutingError::Processing(
                FileProcessingError::TooLarge { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn resolve_file_inputs_for_uploads_dedupes_orphan_cleanup_across_three_files() {
        let codex_home = tempfile::tempdir().expect("codex home");
        login_with_provider_api_key(
            codex_home.path(),
            PROVIDER_OPENAI,
            "sk-test-key",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("config");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "filename": "uploaded.pdf",
                "purpose": "user_data",
                "bytes": 10
            })))
            .expect(2)
            .mount(&server)
            .await;
        // `delete_uploaded_files_best_effort` should only delete each file id once even if the
        // upstream upload endpoint returned duplicates.
        Mock::given(method("DELETE"))
            .and(path("/v1/files/file-123"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-123",
                "object": "file",
                "deleted": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut provider = ModelProviderInfo::create_openai_provider();
        provider.base_url = Some(format!("{}/v1", server.uri()));

        let dir = tempfile::tempdir().expect("temp dir");
        let first = dir.path().join("first.pdf");
        let second = dir.path().join("second.pdf");
        for path in [&first, &second] {
            std::fs::write(path, b"%PDF-1.4\n").expect("write pdf header");
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("open pdf")
                .set_len(3 * 1024 * 1024)
                .expect("grow file");
        }

        let too_large = dir.path().join("too_large.pdf");
        std::fs::write(&too_large, b"%PDF-1.4\n").expect("write pdf header");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&too_large)
            .expect("open pdf")
            .set_len(codex_utils_file::MAX_FILE_SIZE + 1)
            .expect("grow file");

        let mut inputs = vec![
            UserInput::LocalFile { path: first },
            UserInput::LocalFile { path: second },
            UserInput::LocalFile { path: too_large },
        ];

        let http_client = reqwest::Client::new();
        let err = resolve_file_inputs_for_uploads(&mut inputs, &provider, &config, &http_client)
            .await
            .expect_err("oversized file should fail routing");
        assert!(matches!(
            err,
            FileInputPreparationError::Routing(FileRoutingError::Processing(
                FileProcessingError::TooLarge { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn refresh_uploaded_file_references_reuploads_near_expiry_and_rewrites_history() {
        let (sess, mut turn_context) = crate::codex::make_session_and_context().await;
        login_with_provider_api_key(
            turn_context.config.codex_home.as_path(),
            PROVIDER_OPENAI,
            "sk-test-key",
            AuthCredentialsStoreMode::File,
        )
        .expect("store api key");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .and(header("authorization", "Bearer sk-test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "file-999",
                "object": "file",
                "filename": "report.pdf",
                "purpose": "user_data",
                "bytes": 10
            })))
            .expect(1)
            .mount(&server)
            .await;

        turn_context.provider = ModelProviderInfo::create_openai_provider();
        turn_context.provider.base_url = Some(format!("{}/v1", server.uri()));

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"%PDF-1.4\npayload").expect("write pdf");

        let old_file_id = "file-old";
        {
            let mut state = sess.state.lock().await;
            state.replace_history(
                vec![ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputFile {
                        file_data: None,
                        file_id: Some(old_file_id.to_string()),
                        mime_type: Some("application/pdf".to_string()),
                        filename: Some("report.pdf".to_string()),
                    }],
                    end_turn: None,
                    phase: None,
                }],
                None,
            );
        }
        {
            let mut cache = sess.services.file_reference_cache.lock().await;
            cache.record(codex_api::file_support::UploadedFile {
                file_id: old_file_id.to_string(),
                provider: "openai".to_string(),
                expires_at: Some(std::time::SystemTime::now() + std::time::Duration::from_secs(30)),
                source_path: path,
            });
        }

        refresh_uploaded_file_references(&sess, &turn_context)
            .await
            .expect("refresh should succeed");

        {
            let state = sess.state.lock().await;
            assert_eq!(
                state.history.raw_items(),
                &[ResponseItem::Message {
                    id: None,
                    role: "user".to_string(),
                    content: vec![ContentItem::InputFile {
                        file_data: None,
                        file_id: Some("file-999".to_string()),
                        mime_type: Some("application/pdf".to_string()),
                        filename: Some("report.pdf".to_string()),
                    }],
                    end_turn: None,
                    phase: None,
                }]
            );
        }
        {
            let cache = sess.services.file_reference_cache.lock().await;
            assert!(!cache.contains(old_file_id));
            assert!(cache.contains("file-999"));
        }
    }
}
