use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::error::ResearchError;
use crate::error::Result;
use crate::types::LatexCompileParams;
use crate::types::LatexCompileResult;

const ENGINE: &str = "pdflatex";
const ENGINE_TIMEOUT: Duration = Duration::from_secs(60);
const TLMGR_TIMEOUT: Duration = Duration::from_secs(120);
const LOG_SNIPPET_MAX_CHARS: usize = 2000;
const MAX_INSTALL_RETRIES: usize = 3;
const AUX_EXTENSIONS: &[&str] = &[
    "aux", "log", "toc", "lof", "lot", "out", "nav", "snm", "vrb",
];

/// Validate that `filename` is a simple base name (no separators, no dots, non-empty).
fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty() {
        return Err(ResearchError::InvalidInput(
            "filename must not be empty".into(),
        ));
    }
    if filename.contains('/') || filename.contains('\\') || filename.contains('.') {
        return Err(ResearchError::InvalidInput(
            "filename must be a simple base name without path separators or dots".into(),
        ));
    }
    Ok(())
}

/// Check that `pdflatex` is available on `$PATH`.
async fn check_engine_installed() -> Result<()> {
    let status = Command::new("which")
        .arg(ENGINE)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| ResearchError::Internal(format!("failed to run `which {ENGINE}`: {e}")))?;

    if !status.success() {
        return Err(ResearchError::InvalidInput(format!(
            "{ENGINE} is not installed or not found on $PATH"
        )));
    }
    Ok(())
}

/// Run a single pdflatex pass. Returns `(success, stdout+stderr)`.
async fn run_engine_pass(tex_path: &Path, output_dir: &Path) -> Result<(bool, String)> {
    let output = tokio::time::timeout(
        ENGINE_TIMEOUT,
        Command::new(ENGINE)
            .args(["-interaction=nonstopmode", "-halt-on-error"])
            .arg(format!("-output-directory={}", output_dir.display()))
            .arg(tex_path)
            .output(),
    )
    .await
    .map_err(|_| {
        ResearchError::Internal(format!("{ENGINE} pass timed out after {ENGINE_TIMEOUT:?}"))
    })?
    .map_err(|e| ResearchError::Internal(format!("failed to run {ENGINE}: {e}")))?;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok((output.status.success(), combined))
}

/// Extract missing `.sty` / `.cls` file names from the log.
///
/// Matches patterns like:
/// - `! LaTeX Error: File `enumitem.sty' not found.`
/// - `! LaTeX Error: File `tcolorbox.sty' not found.`
fn extract_missing_packages(log: &str) -> Vec<String> {
    let mut packages = Vec::new();
    for line in log.lines() {
        // Pattern: File `<name>.sty' not found
        if let Some(start) = line.find("File `") {
            let after = &line[start + 6..];
            if let Some(end) = after.find("' not found") {
                let filename = &after[..end];
                // Strip .sty or .cls extension to get the package name.
                let pkg = filename
                    .strip_suffix(".sty")
                    .or_else(|| filename.strip_suffix(".cls"))
                    .unwrap_or(filename);
                if !pkg.is_empty() && !packages.contains(&pkg.to_string()) {
                    packages.push(pkg.to_string());
                }
            }
        }
    }
    packages
}

/// Check if `tlmgr` is available on `$PATH`.
async fn is_tlmgr_available() -> bool {
    Command::new("which")
        .arg("tlmgr")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Install LaTeX packages via `tlmgr install`. Returns (success, output).
async fn tlmgr_install(packages: &[String]) -> (bool, String) {
    if packages.is_empty() {
        return (true, String::new());
    }

    let result = tokio::time::timeout(
        TLMGR_TIMEOUT,
        Command::new("tlmgr").arg("install").args(packages).output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            (output.status.success(), combined)
        }
        Ok(Err(e)) => (false, format!("tlmgr failed to execute: {e}")),
        Err(_) => (false, format!("tlmgr timed out after {TLMGR_TIMEOUT:?}")),
    }
}

/// Parse errors (lines starting with `!` or containing `Error:`) from the log content.
fn parse_errors(log: &str) -> Vec<String> {
    log.lines()
        .filter(|line| line.starts_with('!') || line.contains("Error:"))
        .map(|line| line.trim().to_string())
        .collect()
}

/// Parse warnings (lines containing `Warning:`) from the log content.
fn parse_warnings(log: &str) -> Vec<String> {
    log.lines()
        .filter(|line| line.contains("Warning:"))
        .map(|line| line.trim().to_string())
        .collect()
}

/// Estimate page count from PDF bytes.
///
/// Modern pdflatex compresses page objects inside object streams, so a naive
/// byte scan for `/Type /Page` does not work. Instead we decompress every
/// flate-encoded stream and look for the Pages dictionary `/Count \d+`.
fn estimate_page_count(pdf_bytes: &[u8]) -> usize {
    // First try raw (uncompressed) bytes.
    if let Some(n) = extract_pages_count(pdf_bytes) {
        return n;
    }

    // Scan for flate-compressed stream bodies and decompress them.
    // We look for "\nstream\n" to avoid matching "endstream\n".
    let marker = b"\nstream\n";
    let end_marker = b"\nendstream";
    let mut offset = 0;
    while offset < pdf_bytes.len() {
        if let Some(pos) = find_subsequence(&pdf_bytes[offset..], marker) {
            let start = offset + pos + marker.len();
            if let Some(end_rel) = find_subsequence(&pdf_bytes[start..], end_marker) {
                let chunk = &pdf_bytes[start..start + end_rel];
                if let Some(decompressed) = try_flate_decompress(chunk)
                    && let Some(n) = extract_pages_count(&decompressed)
                {
                    return n;
                }
                offset = start + end_rel;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    0
}

/// Look for `/Type /Pages` followed by `/Count <N>` in `data`.
fn extract_pages_count(data: &[u8]) -> Option<usize> {
    let haystack = String::from_utf8_lossy(data);
    let pages_marker = "/Type /Pages";
    let idx = haystack.find(pages_marker)?;
    // Search for /Count within the next 200 bytes after /Type /Pages.
    let window_end = (idx + 200).min(haystack.len());
    let window = &haystack[idx..window_end];
    let count_prefix = "/Count ";
    let count_idx = window.find(count_prefix)?;
    let after = &window[count_idx + count_prefix.len()..];
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn try_flate_decompress(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Read the tail of the `.log` file, capped at [`LOG_SNIPPET_MAX_CHARS`].
async fn read_log_snippet(log_path: &Path) -> Option<String> {
    let content = tokio::fs::read_to_string(log_path).await.ok()?;
    if content.len() <= LOG_SNIPPET_MAX_CHARS {
        Some(content)
    } else {
        Some(content[content.len() - LOG_SNIPPET_MAX_CHARS..].to_string())
    }
}

/// Clean up auxiliary files produced by pdflatex.
async fn cleanup_aux_files(output_dir: &Path, base_name: &str) {
    for ext in AUX_EXTENSIONS {
        let path = output_dir.join(format!("{base_name}.{ext}"));
        let _ = tokio::fs::remove_file(&path).await;
    }
}

pub(crate) async fn latex_compile(params: LatexCompileParams) -> Result<LatexCompileResult> {
    let filename = params.filename.as_deref().unwrap_or("document");
    validate_filename(filename)?;
    check_engine_installed().await?;

    // Create output directory and write .tex file.
    let output_dir = PathBuf::from(&params.output_dir);
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| ResearchError::Internal(format!("failed to create output dir: {e}")))?;

    let tex_path = output_dir.join(format!("{filename}.tex"));
    tokio::fs::write(&tex_path, &params.content)
        .await
        .map_err(|e| ResearchError::Internal(format!("failed to write .tex file: {e}")))?;

    // Pass 1 — with auto-install retry for missing packages.
    let mut install_attempts = 0;
    let mut all_installed_packages: Vec<String> = Vec::new();
    let has_tlmgr = is_tlmgr_available().await;

    loop {
        let (ok, _) = run_engine_pass(&tex_path, &output_dir).await?;
        if ok {
            break;
        }

        // Check if failure is due to missing packages we can install.
        let log_path = output_dir.join(format!("{filename}.log"));
        let log_snippet = read_log_snippet(&log_path).await;
        let log_content = log_snippet.as_deref().unwrap_or("");
        let missing = extract_missing_packages(log_content);

        if missing.is_empty() || !has_tlmgr || install_attempts >= MAX_INSTALL_RETRIES {
            return Ok(LatexCompileResult {
                success: false,
                pdf_path: None,
                errors: parse_errors(log_content),
                warnings: parse_warnings(log_content),
                num_pages: None,
                compilation_log_snippet: log_snippet,
                packages_installed: all_installed_packages,
            });
        }

        // Try to install missing packages.
        tracing::info!(
            packages = ?missing,
            attempt = install_attempts + 1,
            "auto-installing missing LaTeX packages via tlmgr"
        );
        let (install_ok, install_output) = tlmgr_install(&missing).await;
        install_attempts += 1;

        if !install_ok {
            tracing::warn!(output = %install_output, "tlmgr install failed");
            return Ok(LatexCompileResult {
                success: false,
                pdf_path: None,
                errors: {
                    let mut errs = parse_errors(log_content);
                    errs.push(format!(
                        "attempted to install packages {missing:?} but tlmgr failed: {install_output}"
                    ));
                    errs
                },
                warnings: parse_warnings(log_content),
                num_pages: None,
                compilation_log_snippet: log_snippet,
                packages_installed: all_installed_packages,
            });
        }

        for pkg in &missing {
            if !all_installed_packages.contains(pkg) {
                all_installed_packages.push(pkg.clone());
            }
        }
        tracing::info!(packages = ?missing, "successfully installed packages, retrying compilation");
        // Clean aux files before retry so stale state doesn't interfere.
        cleanup_aux_files(&output_dir, filename).await;
    }

    // Pass 2 (resolve references)
    let _ = run_engine_pass(&tex_path, &output_dir).await?;

    // Pass 3 (cross-references)
    let (pass3_ok, _) = run_engine_pass(&tex_path, &output_dir).await?;

    let log_path = output_dir.join(format!("{filename}.log"));
    let log_snippet = read_log_snippet(&log_path).await;
    let log_content = log_snippet.as_deref().unwrap_or("");

    let pdf_path = output_dir.join(format!("{filename}.pdf"));
    let pdf_exists = pdf_path.exists();

    let num_pages = if pdf_exists {
        tokio::fs::read(&pdf_path)
            .await
            .ok()
            .map(|bytes| estimate_page_count(&bytes))
    } else {
        None
    };

    if pdf_exists {
        cleanup_aux_files(&output_dir, filename).await;
    }

    Ok(LatexCompileResult {
        success: pass3_ok && pdf_exists,
        pdf_path: if pdf_exists {
            Some(pdf_path.to_string_lossy().into_owned())
        } else {
            None
        },
        errors: parse_errors(log_content),
        warnings: parse_warnings(log_content),
        num_pages,
        compilation_log_snippet: log_snippet,
        packages_installed: all_installed_packages,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn validate_filename_rejects_empty() {
        assert!(validate_filename("").is_err());
    }

    #[test]
    fn validate_filename_rejects_path_separators() {
        assert!(validate_filename("foo/bar").is_err());
        assert!(validate_filename("foo\\bar").is_err());
    }

    #[test]
    fn validate_filename_rejects_dots() {
        assert!(validate_filename("document.tex").is_err());
    }

    #[test]
    fn validate_filename_accepts_simple_names() {
        assert!(validate_filename("document").is_ok());
        assert!(validate_filename("my-report").is_ok());
        assert!(validate_filename("paper_v2").is_ok());
    }

    #[test]
    fn parse_errors_extracts_error_lines() {
        let log =
            "This is fine\n! Undefined control sequence.\nMore text\nLaTeX Error: something\n";
        let errors = parse_errors(log);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "! Undefined control sequence.");
        assert_eq!(errors[1], "LaTeX Error: something");
    }

    #[test]
    fn parse_errors_extracts_error_colon_lines() {
        let log = "blah\nSome Error: bad thing\n! bang\n";
        let errors = parse_errors(log);
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn parse_warnings_extracts_warning_lines() {
        let log = "Normal line\nLaTeX Warning: Citation undefined\nAnother line\n";
        let warnings = parse_warnings(log);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Citation undefined"));
    }

    #[test]
    fn extract_missing_packages_finds_sty_files() {
        let log = r#"
! LaTeX Error: File `enumitem.sty' not found.
Some other line
! LaTeX Error: File `tcolorbox.sty' not found.
"#;
        let packages = extract_missing_packages(log);
        assert_eq!(packages, vec!["enumitem", "tcolorbox"]);
    }

    #[test]
    fn extract_missing_packages_finds_cls_files() {
        let log = "! LaTeX Error: File `memoir.cls' not found.\n";
        let packages = extract_missing_packages(log);
        assert_eq!(packages, vec!["memoir"]);
    }

    #[test]
    fn extract_missing_packages_deduplicates() {
        let log =
            "! LaTeX Error: File `foo.sty' not found.\n! LaTeX Error: File `foo.sty' not found.\n";
        let packages = extract_missing_packages(log);
        assert_eq!(packages, vec!["foo"]);
    }

    #[test]
    fn extract_missing_packages_empty_when_no_match() {
        let log = "! Undefined control sequence.\nLaTeX Error: something else\n";
        let packages = extract_missing_packages(log);
        assert!(packages.is_empty());
    }

    #[test]
    fn estimate_page_count_from_raw_bytes() {
        let pdf_content = b"some stuff /Type /Pages\n/Count 3\n/Kids [1 0 R]";
        assert_eq!(estimate_page_count(pdf_content), 3);
    }

    #[test]
    fn estimate_page_count_handles_no_pages() {
        assert_eq!(estimate_page_count(b"empty pdf"), 0);
    }

    #[tokio::test]
    async fn latex_compile_produces_pdf() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let content = r#"\documentclass{article}
\begin{document}
Hello, world!
\end{document}"#;

        let result = latex_compile(LatexCompileParams {
            content: content.into(),
            output_dir: tmp.path().to_string_lossy().into_owned(),
            filename: Some("hello".into()),
        })
        .await
        .expect("latex_compile should succeed");

        assert!(result.success, "compilation should succeed: {result:?}");
        assert!(
            result.errors.is_empty(),
            "no errors expected: {:?}",
            result.errors
        );

        let pdf_path = result.pdf_path.expect("pdf_path should be set");
        assert!(
            std::path::Path::new(&pdf_path).exists(),
            "PDF file should exist"
        );

        let num_pages = result.num_pages.expect("num_pages should be set");
        assert_eq!(num_pages, 1, "single-page document");

        // Aux files should be cleaned up.
        assert!(!tmp.path().join("hello.aux").exists());
        assert!(!tmp.path().join("hello.log").exists());

        // .tex and .pdf should remain.
        assert!(tmp.path().join("hello.tex").exists());
        assert!(tmp.path().join("hello.pdf").exists());
    }

    #[tokio::test]
    async fn latex_compile_returns_errors_for_bad_latex() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let content = r#"\documentclass{article}
\begin{document}
\badcommand
\end{document}"#;

        let result = latex_compile(LatexCompileParams {
            content: content.into(),
            output_dir: tmp.path().to_string_lossy().into_owned(),
            filename: None,
        })
        .await
        .expect("latex_compile should return Ok even on compile failure");

        assert!(!result.success);
        assert!(!result.errors.is_empty(), "should have errors");
        assert!(
            result.compilation_log_snippet.is_some(),
            "should have log snippet"
        );
    }

    #[tokio::test]
    async fn latex_compile_invalid_filename_returns_error() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let result = latex_compile(LatexCompileParams {
            content: "test".into(),
            output_dir: tmp.path().to_string_lossy().into_owned(),
            filename: Some("bad/name".into()),
        })
        .await;

        assert!(result.is_err());
        assert!(
            result
                .expect_err("should fail")
                .to_string()
                .contains("path separators")
        );
    }
}
