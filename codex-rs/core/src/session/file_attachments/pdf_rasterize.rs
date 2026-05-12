// Rasterizes PDF inputs into per-page PNG attachments for Copilot models
// that route through `/chat/completions`. Copilot's relay only accepts
// `text` + `image_url` content parts there, so inline PDF blocks get
// stripped before the wire. Rendering each page client-side as a PNG lets
// vision-capable Copilot models (Claude, Gemini, gpt-4.x) ingest the
// document content through the supported `image_url` channel.
//
// This is strictly a Copilot+chat-completions workaround. Copilot's
// `/responses` endpoint (gpt-5.x) forwards `input_file` natively and
// bypasses this path. All other providers (standalone OpenAI, Anthropic,
// Gemini) keep their native PDF ingestion unchanged.

use std::path::Path;
use std::path::PathBuf;

use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::user_input::UserInput;
use image::ImageEncoder;
use image::codecs::jpeg::JpegEncoder;
use pdfium_render::prelude::*;
use tokio::task;

use crate::client::copilot::requires_responses_api;

/// Maximum number of rasterized pages we'll attach per PDF. Capped at
/// Copilot's Gemini-3.x per-request limit (10 images), which is the
/// tightest reachable image budget on the chat-completions path we hit
/// in practice. Pages beyond this limit are dropped with a warning
/// surfaced via the returned `RasterizedPdf::skipped_pages` count.
const MAX_PAGES_PER_PDF: u16 = 10;

#[derive(Debug)]
pub(super) struct RasterizedPdf {
    pub(super) source_path: PathBuf,
    pub(super) page_paths: Vec<PathBuf>,
    pub(super) skipped_pages: u16,
}

/// Returns `true` when the active provider is Copilot routing through
/// `/chat/completions` for `model`. That's the only configuration where
/// inline PDFs would be silently dropped.
pub(super) fn should_rasterize_pdfs(provider: &ModelProviderInfo, model: &str) -> bool {
    matches!(provider.wire_api, WireApi::CopilotInline) && !requires_responses_api(model)
}

/// Scans `input` for `UserInput::LocalFile` entries with a `.pdf`
/// extension and rasterizes each to per-page PNG files. On success,
/// replaces the original LocalFile entries with `UserInput::LocalImage`
/// entries (one per rendered page).
///
/// Pages are written into the OS temp dir under a per-PDF subdirectory so
/// the rest of the file-attachment pipeline (size validation, inline
/// encoding) treats them as ordinary image attachments.
pub(super) async fn rasterize_pdfs_for_copilot_chat_completions(
    input: &mut Vec<UserInput>,
    codex_home: &Path,
) -> Result<Vec<RasterizedPdf>, String> {
    let pdf_indices: Vec<(usize, PathBuf)> = input
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| match item {
            UserInput::LocalFile { path } if path_is_pdf(path) => Some((idx, path.clone())),
            _ => None,
        })
        .collect();

    if pdf_indices.is_empty() {
        return Ok(Vec::new());
    }

    let mut rasterized: Vec<RasterizedPdf> = Vec::with_capacity(pdf_indices.len());
    for (_, path) in &pdf_indices {
        let pdf_path = path.clone();
        let home = codex_home.to_path_buf();
        let result = task::spawn_blocking(move || rasterize_one_pdf(&pdf_path, &home))
            .await
            .map_err(|e| format!("pdf rasterization task failed: {e}"))??;
        rasterized.push(result);
    }

    // Splice the rendered images into the input list, replacing the
    // original `LocalFile` entries. Walk in reverse so earlier indices
    // stay valid as we mutate.
    let mut sorted_indices: Vec<usize> = pdf_indices.iter().map(|(idx, _)| *idx).collect();
    sorted_indices.sort_unstable();
    for (drop_idx, raster) in sorted_indices.iter().rev().zip(rasterized.iter().rev()) {
        let replacements: Vec<UserInput> = raster
            .page_paths
            .iter()
            .map(|p| UserInput::LocalImage { path: p.clone() })
            .collect();
        input.splice(*drop_idx..=*drop_idx, replacements);
    }

    Ok(rasterized)
}

fn path_is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
}

fn rasterize_one_pdf(pdf_path: &Path, codex_home: &Path) -> Result<RasterizedPdf, String> {
    let bindings = bind_pdfium(codex_home)?;
    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| format!("failed to open PDF {}: {e}", pdf_path.display()))?;

    let pages = document.pages();
    let total = pages.len();
    let render_count = total.min(MAX_PAGES_PER_PDF);
    let skipped = total.saturating_sub(render_count);

    let out_dir = tempfile::Builder::new()
        .prefix("ata-copilot-pdf-")
        .tempdir()
        .map_err(|e| format!("failed to create temp dir for rasterized pages: {e}"))?;
    // Keep the directory alive past this function by leaking it; the OS
    // will reclaim the files on next reboot. Long-lived sessions don't
    // re-rasterize the same PDF because the upstream dedup cache keys
    // off the source path.
    let out_dir = out_dir.keep();

    // Lower-resolution than `crop_figure.rs` (which targets a single
    // cropped figure) because we're inlining N pages into a single chat
    // request and Copilot's `/chat/completions` enforces a per-request
    // payload cap that 11+ pages at 2000x4000 will exceed (HTTP 413).
    // 1024x2048 keeps each page small (~200-400 KB PNG) while staying
    // legible for vision models (~120 DPI on US-letter).
    let render_config = PdfRenderConfig::new()
        .set_target_width(1024)
        .set_maximum_height(2048);

    let mut page_paths: Vec<PathBuf> = Vec::with_capacity(render_count as usize);
    for page_index in 0..render_count {
        let page = pages
            .get(page_index)
            .map_err(|e| format!("failed to read page {page_index}: {e}"))?;
        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|e| format!("failed to render page {page_index}: {e}"))?;
        let dyn_image = bitmap.as_image();
        let stem = pdf_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document");
        let jpeg_path = out_dir.join(format!("{stem}-p{:03}.jpg", page_index + 1));
        write_jpeg(&dyn_image, &jpeg_path)?;
        page_paths.push(jpeg_path);
    }

    Ok(RasterizedPdf {
        source_path: pdf_path.to_path_buf(),
        page_paths,
        skipped_pages: skipped,
    })
}

/// Encodes `image` as a JPEG with quality 80 — good middle ground for
/// document pages: ~5x smaller than equivalent PNG, still legible by
/// vision models, no perceptible ringing on text at this quality.
fn write_jpeg(image: &image::DynamicImage, path: &Path) -> Result<(), String> {
    let rgb = image.to_rgb8();
    let file = std::fs::File::create(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    let writer = std::io::BufWriter::new(file);
    let encoder = JpegEncoder::new_with_quality(writer, 80);
    encoder
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("failed to encode JPEG {}: {e}", path.display()))?;
    Ok(())
}

fn bind_pdfium(codex_home: &Path) -> Result<Box<dyn PdfiumLibraryBindings>, String> {
    let lib_dir_str = codex_home.join("lib").to_string_lossy().into_owned();
    let exe_dir_str = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()));

    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib_dir_str))
        .or_else(|_| match exe_dir_str.as_deref() {
            Some(dir) => Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir)),
            None => Pdfium::bind_to_system_library(),
        })
        .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")))
        .or_else(|_| Pdfium::bind_to_system_library())
        .map_err(|e| {
            format!(
                "pdfium library not found. Searched: {lib_dir_str}, {exe_dir_str:?}, ./, system paths. \
                 Download libpdfium and place it in {lib_dir_str}. Error: {e}",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_model_provider_info::ModelProviderInfo;

    #[test]
    fn should_rasterize_only_for_copilot_chat_completions() {
        let copilot = ModelProviderInfo::create_copilot_provider();
        assert!(should_rasterize_pdfs(&copilot, "claude-opus-4.7"));
        assert!(should_rasterize_pdfs(&copilot, "gemini-3.1-pro-preview"));
        assert!(should_rasterize_pdfs(&copilot, "gpt-4.1"));
        // gpt-5.x routes through /responses (native PDF) — no rasterize.
        assert!(!should_rasterize_pdfs(&copilot, "gpt-5.5"));
        assert!(!should_rasterize_pdfs(&copilot, "gpt-5.4"));
    }

    #[test]
    fn should_not_rasterize_for_non_copilot_providers() {
        let anthropic = ModelProviderInfo::create_anthropic_provider();
        let gemini = ModelProviderInfo::create_gemini_provider();
        assert!(!should_rasterize_pdfs(&anthropic, "claude-opus-4.7"));
        assert!(!should_rasterize_pdfs(&gemini, "gemini-3.1-pro-preview"));
    }
}
