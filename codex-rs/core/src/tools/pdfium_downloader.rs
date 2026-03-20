use std::path::Path;
use std::path::PathBuf;

/// Returns the platform-specific pdfium shared library filename.
fn pdfium_lib_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

/// Maps the current OS + architecture to the GitHub release slug used by
/// <https://github.com/bblanchon/pdfium-binaries>.
fn pdfium_platform_slug() -> Result<&'static str, String> {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        Ok("mac-arm64")
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        Ok("mac-x64")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        Ok("linux-x64")
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
        Ok("linux-arm64")
    } else {
        Err(format!(
            "unsupported platform: os={}, arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    }
}

/// Ensure the pdfium native library is present under `codex_home/lib/`.
///
/// On the first call this downloads the library from the pdfium-binaries
/// GitHub release and extracts it. Subsequent calls return immediately once
/// the file exists on disk.
pub(crate) async fn ensure_pdfium_available(codex_home: &Path) -> Result<(), String> {
    let lib_filename = pdfium_lib_filename();
    let lib_dir = codex_home.join("lib");
    let dest_path = lib_dir.join(lib_filename);

    // Fast path: already present.
    if tokio::fs::try_exists(&dest_path).await.unwrap_or(false) {
        return Ok(());
    }

    let slug = pdfium_platform_slug()?;
    let url = format!(
        "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-{slug}.tgz"
    );

    tracing::info!("downloading pdfium library from {url}");

    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("failed to download pdfium: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "pdfium download returned HTTP {}",
            response.status()
        ));
    }

    let tgz_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("failed to read pdfium download body: {e}"))?;

    // Create lib directory if it does not exist.
    tokio::fs::create_dir_all(&lib_dir)
        .await
        .map_err(|e| format!("failed to create lib dir: {e}"))?;

    // Extract in a blocking task (tar + flate2 are synchronous).
    let lib_filename_owned = lib_filename.to_string();
    let dest_path_clone = dest_path.clone();
    tokio::task::spawn_blocking(move || {
        extract_pdfium_lib_from_tgz(&tgz_bytes, &lib_filename_owned, &dest_path_clone)
    })
    .await
    .map_err(|e| format!("pdfium extraction task failed: {e}"))??;

    tracing::info!("pdfium library installed to {}", dest_path.display());

    Ok(())
}

/// Synchronous helper: extract the pdfium shared library from a `.tgz` archive.
///
/// Writes to a `.part` temp file first, then atomically renames to the final
/// destination so that concurrent readers never observe a partial file.
fn extract_pdfium_lib_from_tgz(
    tgz_bytes: &[u8],
    lib_filename: &str,
    dest_path: &Path,
) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(tgz_bytes);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive
        .entries()
        .map_err(|e| format!("failed to read tar entries: {e}"))?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("failed to read tar entry: {e}"))?;

        let path = entry
            .path()
            .map_err(|e| format!("failed to read entry path: {e}"))?;

        let matches = path
            .file_name()
            .is_some_and(|name| name.to_string_lossy() == lib_filename);

        if matches {
            let temp_path = temp_part_path(dest_path);

            // Write to .part file first.
            {
                let mut out_file = std::fs::File::create(&temp_path)
                    .map_err(|e| format!("failed to create temp file: {e}"))?;
                std::io::copy(&mut entry, &mut out_file)
                    .map_err(|e| format!("failed to write pdfium library: {e}"))?;
            }

            // Atomic rename.
            std::fs::rename(&temp_path, dest_path)
                .map_err(|e| format!("failed to rename pdfium library into place: {e}"))?;

            return Ok(());
        }
    }

    Err(format!(
        "pdfium library {lib_filename} not found in downloaded archive"
    ))
}

/// Build a `.part` sibling path for atomic writes.
fn temp_part_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "libpdfium".to_string());
    path.with_file_name(format!("{file_name}.part"))
}
