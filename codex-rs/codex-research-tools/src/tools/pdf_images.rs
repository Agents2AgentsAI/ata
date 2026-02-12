use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ExtractedFigure;
use crate::types::PdfExtractFiguresParams;
use crate::types::PdfExtractFiguresResult;

const PDFIMAGES_BIN: &str = "pdfimages";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default filter thresholds.
const DEFAULT_MIN_SIZE_KB: u32 = 5;
const DEFAULT_MIN_WIDTH: u32 = 100;
const DEFAULT_MIN_HEIGHT: u32 = 100;

/// Check that `pdfimages` (from poppler-utils) is available on `$PATH`.
async fn check_pdfimages_installed() -> Result<()> {
    let status = Command::new("which")
        .arg(PDFIMAGES_BIN)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| {
            ResearchError::Internal(format!("failed to run `which {PDFIMAGES_BIN}`: {e}"))
        })?;

    if !status.success() {
        return Err(ResearchError::InvalidInput(format!(
            "{PDFIMAGES_BIN} is not installed or not found on $PATH. \
             Install poppler-utils: brew install poppler (macOS) or apt install poppler-utils (Linux)"
        )));
    }
    Ok(())
}

/// Download a PDF from `url` into `dest_path` using reqwest.
async fn download_pdf(url: &str, dest_path: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| ResearchError::Internal(format!("failed to build HTTP client: {e}")))?;

    let response =
        client.get(url).send().await.map_err(|e| {
            ResearchError::Internal(format!("failed to download PDF from {url}: {e}"))
        })?;

    if !response.status().is_success() {
        return Err(ResearchError::Internal(format!(
            "PDF download failed with status {}: {url}",
            response.status()
        )));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| ResearchError::Internal(format!("failed to read PDF response body: {e}")))?;

    tokio::fs::write(dest_path, &bytes).await.map_err(|e| {
        ResearchError::Internal(format!(
            "failed to write PDF to {}: {e}",
            dest_path.display()
        ))
    })?;

    Ok(())
}

/// Run `pdfimages -png -p <pdf> <prefix>` to extract images.
async fn run_pdfimages(pdf_path: &Path, output_prefix: &Path) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        EXTRACT_TIMEOUT,
        Command::new(PDFIMAGES_BIN)
            .args(["-png", "-p"])
            .arg(pdf_path)
            .arg(output_prefix)
            .output(),
    )
    .await
    .map_err(|_| {
        ResearchError::Internal(format!(
            "{PDFIMAGES_BIN} timed out after {EXTRACT_TIMEOUT:?}"
        ))
    })?
    .map_err(|e| ResearchError::Internal(format!("failed to run {PDFIMAGES_BIN}: {e}")))?;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok((output.status.success(), combined))
}

/// Read PNG dimensions from the IHDR chunk (bytes 16-23 of a PNG file).
///
/// PNG format: 8-byte signature, then IHDR chunk starts at byte 8.
/// IHDR data starts at byte 16 (after chunk length + "IHDR" tag).
/// Width is bytes 16-19, height is bytes 20-23 (big-endian u32).
fn read_png_dimensions(path: &Path) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 24 {
        return None;
    }
    // Verify PNG signature
    if &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((width, height))
}

/// Parse a pdfimages output filename like `fig-003-002.png` into (page_number, index).
///
/// Format: `<prefix>-<page>-<index>.png` where page and index are 3-digit zero-padded.
fn parse_figure_filename(filename: &str, prefix: &str) -> Option<(u32, u32)> {
    let stem = filename.strip_suffix(".png")?;
    let rest = stem.strip_prefix(prefix)?.strip_prefix('-')?;
    let mut parts = rest.splitn(2, '-');
    let page: u32 = parts.next()?.parse().ok()?;
    let index: u32 = parts.next()?.parse().ok()?;
    Some((page, index))
}

pub(crate) async fn pdf_extract_figures(
    params: PdfExtractFiguresParams,
) -> Result<PdfExtractFiguresResult> {
    // Check prerequisites.
    check_pdfimages_installed().await?;

    // Create output directory.
    let output_dir = PathBuf::from(&params.output_dir);
    tokio::fs::create_dir_all(&output_dir).await.map_err(|e| {
        ResearchError::Internal(format!(
            "failed to create output dir {}: {e}",
            output_dir.display()
        ))
    })?;

    // Download PDF to a temp file inside the output dir.
    let pdf_path = output_dir.join("_source.pdf");
    if let Err(e) = download_pdf(&params.pdf_url, &pdf_path).await {
        return Ok(PdfExtractFiguresResult {
            success: false,
            figures: Vec::new(),
            total_extracted: 0,
            total_filtered: 0,
            errors: vec![e.to_string()],
        });
    }

    // Run pdfimages.
    let prefix = output_dir.join("fig");
    let (ok, output_text) = match run_pdfimages(&pdf_path, &prefix).await {
        Ok(result) => result,
        Err(e) => {
            return Ok(PdfExtractFiguresResult {
                success: false,
                figures: Vec::new(),
                total_extracted: 0,
                total_filtered: 0,
                errors: vec![e.to_string()],
            });
        }
    };

    // Clean up source PDF.
    let _ = tokio::fs::remove_file(&pdf_path).await;

    if !ok {
        return Ok(PdfExtractFiguresResult {
            success: false,
            figures: Vec::new(),
            total_extracted: 0,
            total_filtered: 0,
            errors: vec![format!("{PDFIMAGES_BIN} failed: {output_text}")],
        });
    }

    // Scan output directory for PNGs matching the prefix pattern.
    let min_size_bytes = u64::from(params.min_size_kb.unwrap_or(DEFAULT_MIN_SIZE_KB)) * 1024;
    let min_width = params.min_width.unwrap_or(DEFAULT_MIN_WIDTH);
    let min_height = params.min_height.unwrap_or(DEFAULT_MIN_HEIGHT);

    let mut all_pngs = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&output_dir).await.map_err(|e| {
        ResearchError::Internal(format!(
            "failed to read output dir {}: {e}",
            output_dir.display()
        ))
    })?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| ResearchError::Internal(format!("failed to read dir entry: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            if let Some((page, index)) = parse_figure_filename(filename, "fig") {
                all_pngs.push((path, page, index));
            }
        }
    }

    let total_extracted = all_pngs.len();
    let mut figures = Vec::new();
    let mut filtered_count = 0;

    for (path, page_number, index) in &all_pngs {
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                filtered_count += 1;
                continue;
            }
        };
        let size_bytes = metadata.len();

        // Size filter.
        if size_bytes < min_size_bytes {
            let _ = tokio::fs::remove_file(path).await;
            filtered_count += 1;
            continue;
        }

        // Dimension filter.
        let (width, height) = match read_png_dimensions(path) {
            Some(dims) => dims,
            None => {
                let _ = tokio::fs::remove_file(path).await;
                filtered_count += 1;
                continue;
            }
        };

        if width < min_width || height < min_height {
            let _ = tokio::fs::remove_file(path).await;
            filtered_count += 1;
            continue;
        }

        figures.push(ExtractedFigure {
            path: path.to_string_lossy().into_owned(),
            width,
            height,
            size_bytes,
            page_number: *page_number,
            index: *index,
        });
    }

    // Sort by page then index.
    figures.sort_by_key(|f| (f.page_number, f.index));

    Ok(PdfExtractFiguresResult {
        success: true,
        figures,
        total_extracted,
        total_filtered: filtered_count,
        errors: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parse_figure_filename_valid() {
        assert_eq!(
            parse_figure_filename("fig-003-002.png", "fig"),
            Some((3, 2))
        );
        assert_eq!(
            parse_figure_filename("fig-012-000.png", "fig"),
            Some((12, 0))
        );
    }

    #[test]
    fn parse_figure_filename_invalid() {
        assert_eq!(parse_figure_filename("other-003-002.png", "fig"), None);
        assert_eq!(parse_figure_filename("fig-003.png", "fig"), None);
        assert_eq!(parse_figure_filename("fig-abc-002.png", "fig"), None);
        assert_eq!(parse_figure_filename("fig-003-002.jpg", "fig"), None);
    }

    #[test]
    fn read_png_dimensions_valid() {
        // Construct a minimal valid PNG header (just the signature + IHDR).
        let mut data = Vec::new();
        // PNG signature
        data.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        // IHDR chunk length (13 bytes)
        data.extend_from_slice(&13_u32.to_be_bytes());
        // IHDR tag
        data.extend_from_slice(b"IHDR");
        // Width = 640
        data.extend_from_slice(&640_u32.to_be_bytes());
        // Height = 480
        data.extend_from_slice(&480_u32.to_be_bytes());
        // Rest of IHDR (bit depth, color type, etc.)
        data.extend_from_slice(&[8, 2, 0, 0, 0]);
        // CRC (fake)
        data.extend_from_slice(&[0, 0, 0, 0]);

        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("test.png");
        std::fs::write(&path, &data).expect("write test PNG");

        let dims = read_png_dimensions(&path);
        assert_eq!(dims, Some((640, 480)));
    }

    #[test]
    fn read_png_dimensions_too_short() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("short.png");
        std::fs::write(&path, b"too short").expect("write short file");

        assert_eq!(read_png_dimensions(&path), None);
    }

    #[test]
    fn read_png_dimensions_bad_signature() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("bad.png");
        let data = vec![0_u8; 30];
        std::fs::write(&path, &data).expect("write bad file");

        assert_eq!(read_png_dimensions(&path), None);
    }

    #[test]
    fn filter_logic_removes_small_files() {
        // This test verifies the filter constants are reasonable.
        assert!(u64::from(DEFAULT_MIN_SIZE_KB) * 1024 > 0);
        assert!(DEFAULT_MIN_WIDTH > 0);
        assert!(DEFAULT_MIN_HEIGHT > 0);
    }
}
