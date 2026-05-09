use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use image::GenericImageView;
use image::ImageFormat;
use pdfium_render::prelude::*;
use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::pdfium_downloader::ensure_pdfium_available;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::url_downloader::cache_entry_dir;
use crate::tools::url_validation::normalize_url_for_cache;

const TOOL_NAME: &str = "crop_figure";

// @agent-facing
pub(crate) static CROP_FIGURE_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "pdf_url".to_string(),
        JsonSchema::string(Some(
            "URL of the source PDF (must already be cached via attach_url_files).".to_string(),
        )),
    );
    properties.insert(
        "page".to_string(),
        JsonSchema::integer(Some(
            "1-based page number containing the figure.".to_string(),
        )),
    );
    properties.insert(
        "x".to_string(),
        JsonSchema::number(Some(
            "Left edge of crop region as fraction of page width [0.0, 1.0].".to_string(),
        )),
    );
    properties.insert(
        "y".to_string(),
        JsonSchema::number(Some(
            "Top edge of crop region as fraction of page height [0.0, 1.0].".to_string(),
        )),
    );
    properties.insert(
        "w".to_string(),
        JsonSchema::number(Some(
            "Width of crop region as fraction of page width (0.0, 1.0].".to_string(),
        )),
    );
    properties.insert(
        "h".to_string(),
        JsonSchema::number(Some(
            "Height of crop region as fraction of page height (0.0, 1.0].".to_string(),
        )),
    );
    properties.insert(
        "caption".to_string(),
        JsonSchema::string(Some(
            "Caption text (e.g., \"Figure 3: Architecture diagram\") for the figure.".to_string(),
        )),
    );
    properties.insert(
        "description".to_string(),
        JsonSchema::string(Some(
            "Brief description of what the figure depicts, for accessibility and search."
                .to_string(),
        )),
    );

    ToolSpec::Function(ResponsesApiTool {
        name: TOOL_NAME.to_string(),
        description: "Crop a figure from a cached PDF page and store it as an image asset that the reading view can reference. The PDF must already be cached via attach_url_files. Coordinates are fractional (0.0–1.0) relative to the page. Returns the asset path you can embed in reading-view markdown.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec![
                "pdf_url".to_string(),
                "page".to_string(),
                "x".to_string(),
                "y".to_string(),
                "w".to_string(),
                "h".to_string(),
                "caption".to_string(),
                "description".to_string(),
            ]),
            Some(false.into()),
        ),
        output_schema: None,
    })
});

pub struct CropFigureHandler;

#[derive(Deserialize)]
struct CropFigureArgs {
    pdf_url: String,
    page: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    caption: String,
    description: String,
}

/// Render a single PDF page at `page_index` (0-based) to an RGBA image buffer.
/// Searches for the pdfium library in `codex_home/lib/`, next to the executable,
/// and system library paths.
fn render_pdf_page(
    pdf_path: &std::path::Path,
    page_index: u32,
    codex_home: &std::path::Path,
) -> Result<image::DynamicImage, String> {
    // Search order: ~/.ata/lib/, executable dir, CWD, system paths.
    let lib_dir_str = codex_home.join("lib").to_string_lossy().into_owned();
    let exe_dir_str = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()));

    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(
        &lib_dir_str,
    ))
    .or_else(|_| match exe_dir_str.as_deref() {
        Some(dir) => Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir)),
        None => Pdfium::bind_to_system_library(), // will likely fail, caught below
    })
    .or_else(|_| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")))
    .or_else(|_| Pdfium::bind_to_system_library())
    .map_err(|e| {
        format!(
            "pdfium library not found. Searched: {lib_dir_str}, {exe_dir_str:?}, ./, system paths. \
             Download libpdfium and place it in {lib_dir_str}. Error: {e}",
        )
    })?;

    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|e| format!("failed to open PDF: {e}"))?;

    let page = document
        .pages()
        .get(page_index as u16)
        .map_err(|e| format!("failed to get page {page_index}: {e}"))?;

    // Render at 150 DPI for a good balance of quality and file size.
    let render_config = PdfRenderConfig::new()
        .set_target_width(2000)
        .set_maximum_height(4000);

    let bitmap = page
        .render_with_config(&render_config)
        .map_err(|e| format!("failed to render page: {e}"))?;

    let dyn_image = bitmap.as_image();

    Ok(dyn_image)
}

/// Find the cached PDF file for the given URL in the codex cache directory.
async fn find_cached_pdf(codex_home: &std::path::Path, pdf_url: &str) -> Result<PathBuf, String> {
    let parsed_url = url::Url::parse(pdf_url).map_err(|e| format!("invalid pdf_url: {e}"))?;
    let normalized_key = normalize_url_for_cache(&parsed_url);
    let cache_dir = cache_entry_dir(codex_home, &normalized_key);

    if !fs::try_exists(&cache_dir).await.unwrap_or(false) {
        return Err(format!(
            "PDF not found in cache. The PDF must first be attached via attach_url_files \
             before figures can be cropped. Cache dir: {}",
            cache_dir.display()
        ));
    }

    // Find the first .pdf file in the cache directory.
    let mut entries = fs::read_dir(&cache_dir)
        .await
        .map_err(|e| format!("failed to read cache dir: {e}"))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read dir entry: {e}"))?
    {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        {
            return Ok(path);
        }
        // Also accept files without extension that start with %PDF
        if path.is_file()
            && path.extension().is_none()
            && let Ok(header) = fs::read(&path).await
            && header.starts_with(b"%PDF")
        {
            return Ok(path);
        }
    }

    Err(format!(
        "no PDF file found in cache directory: {}",
        cache_dir.display()
    ))
}

/// Compute a short hash of the PDF URL for use as a directory name.
fn pdf_url_hash(pdf_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pdf_url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    hash[..16].to_string()
}

/// Count existing figure files in the assets directory to determine the next
/// figure number.
async fn next_figure_number(assets_dir: &std::path::Path) -> u32 {
    let mut count = 0u32;
    if let Ok(mut entries) = fs::read_dir(assets_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("fig-") && name_str.ends_with(".png") {
                count += 1;
            }
        }
    }
    count + 1
}

impl ToolHandler for CropFigureHandler {
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
                    "crop_figure handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CropFigureArgs = parse_arguments(&arguments)?;

        // Validate fractional coordinates are in [0.0, 1.0].
        for (name, val) in [("x", args.x), ("y", args.y), ("w", args.w), ("h", args.h)] {
            if !(0.0..=1.0).contains(&val) {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{name} must be between 0.0 and 1.0, got {val}"
                )));
            }
        }

        if args.w <= 0.0 || args.h <= 0.0 {
            return Err(FunctionCallError::RespondToModel(
                "w and h must be positive".to_string(),
            ));
        }
        if args.page == 0 {
            return Err(FunctionCallError::RespondToModel(
                "page must be 1 or greater".to_string(),
            ));
        }

        // 1. Find the cached PDF.
        let codex_home = turn.config.codex_home.clone();

        // Ensure pdfium library is available (downloads on first use).
        ensure_pdfium_available(&codex_home)
            .await
            .map_err(FunctionCallError::RespondToModel)?;

        let pdf_path = find_cached_pdf(&codex_home, &args.pdf_url)
            .await
            .map_err(FunctionCallError::RespondToModel)?;

        // 2. Render the page and crop the region (blocking work on spawn_blocking).
        let page_0indexed = args.page - 1;
        let (crop_x, crop_y, crop_w, crop_h) = (args.x, args.y, args.w, args.h);
        let pdf_path_clone = pdf_path.clone();
        let codex_home_clone = codex_home.clone();

        let cropped_png = spawn_blocking(move || -> Result<Vec<u8>, String> {
            let full_image = render_pdf_page(&pdf_path_clone, page_0indexed, &codex_home_clone)?;
            let (img_w, img_h) = full_image.dimensions();

            // Add 2% padding around the crop to compensate for LLM spatial
            // estimation imprecision, then convert to pixel coordinates.
            let pad_x = 0.02;
            let pad_y = 0.02;
            let padded_x = (crop_x - pad_x).max(0.0);
            let padded_y = (crop_y - pad_y).max(0.0);
            let padded_w = (crop_w + 2.0 * pad_x).min(1.0 - padded_x);
            let padded_h = (crop_h + 2.0 * pad_y).min(1.0 - padded_y);

            let px_x = (padded_x * img_w as f64).round() as u32;
            let px_y = (padded_y * img_h as f64).round() as u32;
            let px_w = (padded_w * img_w as f64).round().max(1.0) as u32;
            let px_h = (padded_h * img_h as f64).round().max(1.0) as u32;

            // Clamp to image bounds.
            let px_x = px_x.min(img_w.saturating_sub(1));
            let px_y = px_y.min(img_h.saturating_sub(1));
            let px_w = px_w.min(img_w.saturating_sub(px_x));
            let px_h = px_h.min(img_h.saturating_sub(px_y));

            let cropped = full_image.crop_imm(px_x, px_y, px_w, px_h);

            let mut buf = std::io::Cursor::new(Vec::new());
            cropped
                .write_to(&mut buf, ImageFormat::Png)
                .map_err(|e| format!("failed to encode PNG: {e}"))?;

            Ok(buf.into_inner())
        })
        .await
        .map_err(|e| FunctionCallError::RespondToModel(format!("render task failed: {e}")))?
        .map_err(FunctionCallError::RespondToModel)?;

        // 3. Save to ~/.codex/knowledge-base/assets/<paper-hash>/fig-<N>.png
        let paper_hash = pdf_url_hash(&args.pdf_url);
        let assets_dir = codex_home
            .join("knowledge-base")
            .join("assets")
            .join(&paper_hash);

        fs::create_dir_all(&assets_dir).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to create assets dir: {e}"))
        })?;

        let fig_num = next_figure_number(&assets_dir).await;
        let fig_filename = format!("fig-{fig_num}.png");
        let fig_path = assets_dir.join(&fig_filename);

        fs::write(&fig_path, &cropped_png).await.map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to write figure: {e}"))
        })?;

        // 4. Save JSON sidecar with metadata.
        let sidecar_path = assets_dir.join(format!("fig-{fig_num}.json"));
        let sidecar = serde_json::json!({
            "pdf_url": args.pdf_url,
            "page": args.page,
            "bbox": { "x": args.x, "y": args.y, "w": args.w, "h": args.h },
            "caption": args.caption,
            "description": args.description,
            "asset_path": format!("/assets/{paper_hash}/{fig_filename}"),
        });
        fs::write(
            &sidecar_path,
            serde_json::to_string_pretty(&sidecar).unwrap_or_default(),
        )
        .await
        .map_err(|e| FunctionCallError::RespondToModel(format!("failed to write sidecar: {e}")))?;

        // 5. Return the asset path for use in reading view markdown.
        let asset_path = format!("/assets/{paper_hash}/{fig_filename}");
        let result = serde_json::json!({
            "asset_path": asset_path,
            "absolute_path": fig_path.display().to_string(),
            "caption": args.caption,
        });

        Ok(FunctionToolOutput::from_text(
            result.to_string(),
            Some(true),
        ))
    }
}
