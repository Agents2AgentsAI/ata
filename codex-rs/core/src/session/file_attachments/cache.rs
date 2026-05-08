use super::*;

pub(super) fn dedup_local_files_from_cache(
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

pub(super) fn record_upload_paths(cache: &mut FileReferenceCache, inputs: &[UserInput]) {
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
