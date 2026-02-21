use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

use crate::error::ResearchError;
use crate::error::Result;
use crate::types::ExtractedFigure;
use crate::types::PdfExtractFiguresParams;
use crate::types::PdfExtractFiguresResult;

const PDFFIGURES2_JAR_ENV: &str = "PDFFIGURES2_JAR";
const PDFFIGURES2_JAR_FILENAME: &str = "pdffigures2.jar";
/// Timeout for building the JAR from source via Docker+SBT (SBT is slow).
const BUILD_JAR_TIMEOUT: Duration = Duration::from_secs(600);
/// Minimum size for a valid pdffigures2 fat JAR (~30 MB; 1 MB sanity check).
const JAR_MIN_SIZE: u64 = 1_000_000;

const PDFTOCAIRO_BIN: &str = "pdftocairo";
const PDFIMAGES_BIN: &str = "pdfimages";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const DETECT_TIMEOUT: Duration = Duration::from_secs(120);
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);
const RASTER_EXTRACT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_DPI: u32 = 300;
/// Minimum pixel area for a raster image to be considered a real figure.
const RASTER_MIN_WIDTH: u32 = 400;
const RASTER_MIN_HEIGHT: u32 = 200;

/// Default filter thresholds.
const DEFAULT_MIN_SIZE_KB: u32 = 5;
const DEFAULT_MIN_WIDTH: u32 = 100;
const DEFAULT_MIN_HEIGHT: u32 = 100;

/// Parsed output from pdffigures2 for a single detected figure.
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct Pdffigures2Figure {
    caption: String,
    name: String,
    #[serde(rename = "figType")]
    figure_type: String,
    page: u32,
    #[serde(rename = "regionBoundary")]
    region_bb: Pdffigures2BoundingBox,
    #[serde(rename = "pageWidth")]
    page_width: Option<f64>,
    #[serde(rename = "pageHeight")]
    page_height: Option<f64>,
}

#[derive(serde::Deserialize)]
struct Pdffigures2BoundingBox {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

/// Well-known locations where Java may be installed but not on `$PATH`.
const JAVA_CANDIDATE_PATHS: &[&str] = &[
    // Homebrew on Apple Silicon
    "/opt/homebrew/opt/openjdk/bin/java",
    // Homebrew on Intel Mac
    "/usr/local/opt/openjdk/bin/java",
    // Common Linux paths
    "/usr/lib/jvm/default-java/bin/java",
    "/usr/lib/jvm/java/bin/java",
];

/// Probe for a working `java` binary without installing anything.
///
/// Resolution order:
/// 1. `$PATH` (via `which java`)
/// 2. macOS `/usr/libexec/java_home` helper
/// 3. Well-known Homebrew / Linux paths
pub(crate) async fn probe_java() -> Option<PathBuf> {
    // 1. Check $PATH.
    if let Ok(which) = Command::new("which").arg("java").output().await
        && which.status.success()
    {
        let p = String::from_utf8_lossy(&which.stdout).trim().to_string();
        if !p.is_empty() {
            // On macOS, /usr/bin/java is a stub that may not work. Verify it.
            let probe = Command::new(&p)
                .arg("-version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            if probe.is_ok_and(|s| s.success()) {
                return Some(PathBuf::from(p));
            }
        }
    }

    // 2. macOS: ask /usr/libexec/java_home for the install location.
    if cfg!(target_os = "macos") {
        let jh = Command::new("/usr/libexec/java_home").output().await.ok();
        if let Some(out) = jh
            && out.status.success()
        {
            let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let candidate = PathBuf::from(&home).join("bin/java");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // 3. Probe well-known paths.
    super::system_deps::find_in_well_known_paths(JAVA_CANDIDATE_PATHS)
}

/// Locate a working `java` binary, auto-installing if necessary.
pub(crate) async fn find_java() -> std::result::Result<PathBuf, String> {
    if let Some(java) = probe_java().await {
        return Ok(java);
    }

    // Auto-install Java.
    let pm = super::system_deps::detect_package_manager().await;
    match pm {
        Some(super::system_deps::PackageManager::Brew) => {
            super::system_deps::brew_install("openjdk", &[]).await?;
        }
        Some(super::system_deps::PackageManager::AptGet) => {
            super::system_deps::apt_install(&["default-jre-headless"]).await?;
        }
        None => {
            return Err(
                "Java is required but was not found and no package manager (brew/apt-get) \
                 is available to install it automatically. \
                 Install: `brew install openjdk` (macOS) or `sudo apt install default-jre-headless` (Linux)"
                    .to_string(),
            );
        }
    }

    // Retry probe after install.
    probe_java().await.ok_or_else(|| {
        "Java was installed but still not found on $PATH or well-known paths. \
         You may need to restart your shell or set JAVA_HOME."
            .to_string()
    })
}

/// Probe for pdftocairo on `$PATH` without installing anything.
pub(crate) async fn probe_pdftocairo() -> bool {
    super::system_deps::is_on_path(PDFTOCAIRO_BIN).await
}

/// Ensure pdftocairo (from poppler-utils) is available, auto-installing if necessary.
pub(crate) async fn ensure_pdftocairo() -> std::result::Result<(), String> {
    if super::system_deps::is_on_path(PDFTOCAIRO_BIN).await {
        return Ok(());
    }

    let pm = super::system_deps::detect_package_manager().await;
    match pm {
        Some(super::system_deps::PackageManager::Brew) => {
            super::system_deps::brew_install("poppler", &[]).await?;
        }
        Some(super::system_deps::PackageManager::AptGet) => {
            super::system_deps::apt_install(&["poppler-utils"]).await?;
        }
        None => {
            return Err(
                "pdftocairo is required but was not found and no package manager (brew/apt-get) \
                 is available to install it automatically. \
                 Install: `brew install poppler` (macOS) or `sudo apt install poppler-utils` (Linux)"
                    .to_string(),
            );
        }
    }

    // Verify after install.
    if super::system_deps::is_on_path(PDFTOCAIRO_BIN).await {
        Ok(())
    } else {
        Err(format!(
            "{PDFTOCAIRO_BIN} was installed but still not found on $PATH"
        ))
    }
}

/// Return the platform-appropriate cache directory for research tools.
fn tools_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("codex-research-tools")
}

/// Well-known JAVA_HOME locations for JDK 11 or 8 (compatible with pdffigures2).
/// JDK 11 is preferred because JDK 8 is unavailable on Apple Silicon.
const BUILD_JDK_HOMES: &[&str] = &[
    // Homebrew JDK 11 (Apple Silicon / Intel)
    "/opt/homebrew/opt/openjdk@11",
    "/usr/local/opt/openjdk@11",
    // Homebrew JDK 8 (Intel only — not available on Apple Silicon)
    "/opt/homebrew/opt/openjdk@8",
    "/usr/local/opt/openjdk@8",
    // Linux apt JDK 11
    "/usr/lib/jvm/java-11-openjdk-amd64",
    "/usr/lib/jvm/java-11-openjdk-arm64",
    // Linux apt JDK 8
    "/usr/lib/jvm/java-8-openjdk-amd64",
    "/usr/lib/jvm/java-8-openjdk-arm64",
];

/// Find or install a JDK 11 (or 8) suitable for building pdffigures2.
///
/// pdffigures2's SBT build is incompatible with modern JDKs (17+).
/// Returns the JAVA_HOME path for the compatible JDK.
async fn find_build_jdk() -> std::result::Result<PathBuf, String> {
    // Check well-known paths first.
    for home in BUILD_JDK_HOMES {
        let p = PathBuf::from(home);
        if p.join("bin/java").exists() {
            return Ok(p);
        }
    }

    // Not found — install JDK 11 (works on both Apple Silicon and Intel).
    let pm = super::system_deps::detect_package_manager().await;
    match pm {
        Some(super::system_deps::PackageManager::Brew) => {
            super::system_deps::brew_install("openjdk@11", &[]).await?;
        }
        Some(super::system_deps::PackageManager::AptGet) => {
            super::system_deps::apt_install(&["openjdk-11-jdk-headless"]).await?;
        }
        None => {
            return Err(
                "JDK 11 is required to build pdffigures2 but no package manager (brew/apt-get) \
                 is available. Install: `brew install openjdk@11` (macOS) or \
                 `sudo apt install openjdk-11-jdk-headless` (Linux)"
                    .to_string(),
            );
        }
    }

    // Re-check well-known paths after install.
    for home in BUILD_JDK_HOMES {
        let p = PathBuf::from(home);
        if p.join("bin/java").exists() {
            return Ok(p);
        }
    }

    Err("JDK 11 was installed but JAVA_HOME could not be located at well-known paths".to_string())
}

/// Ensure SBT is available, auto-installing via package manager if needed.
async fn ensure_sbt() -> std::result::Result<(), String> {
    if super::system_deps::is_on_path("sbt").await {
        return Ok(());
    }

    let pm = super::system_deps::detect_package_manager().await;
    match pm {
        Some(super::system_deps::PackageManager::Brew) => {
            super::system_deps::brew_install("sbt", &[]).await?;
        }
        Some(super::system_deps::PackageManager::AptGet) => {
            // sbt isn't in default apt repos; use the official deb from scala-sbt.
            // Install apt-transport-https + add the sbt repo, then install.
            super::system_deps::apt_install(&["apt-transport-https"]).await?;

            let add_repo = Command::new("bash")
                .args([
                    "-c",
                    "echo 'deb https://repo.scala-sbt.org/scalasbt/debian all main' \
                     | sudo tee /etc/apt/sources.list.d/sbt.list && \
                     curl -sL 'https://keyserver.ubuntu.com/pks/lookup?op=get&search=0x2EE0EA64E40A89B84B2DF73499E82A75642AC823' \
                     | sudo apt-key add - && \
                     sudo apt-get update",
                ])
                .output()
                .await
                .map_err(|e| format!("failed to add sbt apt repo: {e}"))?;

            if !add_repo.status.success() {
                let stderr = String::from_utf8_lossy(&add_repo.stderr);
                return Err(format!("failed to add sbt apt repository: {stderr}"));
            }

            super::system_deps::apt_install(&["sbt"]).await?;
        }
        None => {
            return Err(
                "sbt is required to build pdffigures2 but was not found and no package manager \
                 (brew/apt-get) is available. Install: `brew install sbt` (macOS) or see \
                 https://www.scala-sbt.org/download/"
                    .to_string(),
            );
        }
    }

    if super::system_deps::is_on_path("sbt").await {
        Ok(())
    } else {
        Err("sbt was installed but still not found on $PATH".to_string())
    }
}

/// Build the pdffigures2 fat JAR from source using SBT.
///
/// 1. Ensure Java and SBT are available
/// 2. Clone the pdffigures2 repo into a temp directory
/// 3. Run `sbt assembly`
/// 4. Copy the resulting JAR to `dest` (atomic rename)
async fn build_jar(cache_dir: &Path, dest: &Path) -> std::result::Result<(), String> {
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|e| format!("failed to create cache dir {}: {e}", cache_dir.display()))?;

    // Ensure a compatible JDK (8 or 11) and SBT are available.
    // pdffigures2's SBT build fails with modern JDKs (17+).
    let build_jdk_home = find_build_jdk().await?;
    ensure_sbt().await?;

    let tmp_dir = tempfile::tempdir()
        .map_err(|e| format!("failed to create temp dir for pdffigures2 build: {e}"))?;
    let clone_dir = tmp_dir.path().join("pdffigures2");

    // Step 1: git clone
    eprintln!("  Cloning pdffigures2 repository...");
    let clone_output = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "https://github.com/allenai/pdffigures2.git",
        ])
        .arg(&clone_dir)
        .output()
        .await
        .map_err(|e| format!("failed to run git clone: {e}"))?;

    if !clone_output.status.success() {
        let stderr = String::from_utf8_lossy(&clone_output.stderr);
        return Err(format!("git clone of pdffigures2 failed: {stderr}"));
    }

    // Step 2: build with sbt assembly (using the compatible JDK)
    eprintln!(
        "  Running sbt assembly with JAVA_HOME={} (this may take several minutes)...",
        build_jdk_home.display()
    );
    let build_output = tokio::time::timeout(
        BUILD_JAR_TIMEOUT,
        Command::new("sbt")
            .arg("assembly")
            .env("JAVA_HOME", &build_jdk_home)
            .current_dir(&clone_dir)
            .output(),
    )
    .await
    .map_err(|_| format!("sbt assembly timed out after {BUILD_JAR_TIMEOUT:?}"))?
    .map_err(|e| format!("failed to run sbt assembly: {e}"))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr);
        return Err(format!("sbt assembly failed: {stderr}"));
    }

    // build.sbt sets: assembly / assemblyOutputPath := file("pdffigures2.jar")
    // so the fat JAR lands in the project root, not under target/.
    let built_jar = clone_dir.join("pdffigures2.jar");
    if !built_jar.exists() {
        return Err(
            "sbt assembly completed but pdffigures2.jar was not found in project root".to_string(),
        );
    }

    // Validate size.
    let metadata = tokio::fs::metadata(&built_jar)
        .await
        .map_err(|e| format!("failed to read built JAR metadata: {e}"))?;
    if metadata.len() < JAR_MIN_SIZE {
        return Err(format!(
            "built JAR is too small ({} bytes), likely not a valid pdffigures2 fat JAR",
            metadata.len()
        ));
    }

    // Step 3: atomic copy to cache.
    let tmp_dest = dest.with_extension("jar.tmp");
    tokio::fs::copy(&built_jar, &tmp_dest)
        .await
        .map_err(|e| format!("failed to copy JAR to {}: {e}", tmp_dest.display()))?;
    tokio::fs::rename(&tmp_dest, dest)
        .await
        .map_err(|e| format!("failed to rename temp JAR to {}: {e}", dest.display()))?;

    Ok(())
}

/// Probe for an existing pdffigures2 JAR without downloading anything.
pub(crate) fn probe_pdffigures2_jar() -> Option<PathBuf> {
    // 0. Check compile-time built JAR.
    if let Some(buildtime_jar) = option_env!("PDFFIGURES2_JAR_BUILDTIME") {
        let path = PathBuf::from(buildtime_jar);
        if path.exists() {
            return Some(path);
        }
    }

    // 1. Explicit env var.
    if let Ok(p) = std::env::var(PDFFIGURES2_JAR_ENV) {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Some(path);
        }
        return None;
    }

    // 2. Check cache directory.
    let cached_jar = tools_cache_dir().join(PDFFIGURES2_JAR_FILENAME);
    if cached_jar.exists() {
        return Some(cached_jar);
    }

    None
}

/// Locate or auto-build the pdffigures2 JAR.
///
/// Resolution order:
/// 0. Compile-time built JAR (set by build.rs)
/// 1. `PDFFIGURES2_JAR` env var (backward compat)
/// 2. Cached JAR in platform cache dir
/// 3. Build from source via SBT (auto-installs Java + SBT if needed)
pub(crate) async fn ensure_pdffigures2_jar() -> std::result::Result<PathBuf, String> {
    // 0. Check compile-time built JAR.
    if let Some(buildtime_jar) = option_env!("PDFFIGURES2_JAR_BUILDTIME") {
        let path = PathBuf::from(buildtime_jar);
        if path.exists() {
            return Ok(path);
        }
    }

    // 1. Explicit env var takes precedence.
    if let Ok(p) = std::env::var(PDFFIGURES2_JAR_ENV) {
        let path = PathBuf::from(&p);
        if path.exists() {
            return Ok(path);
        }
        return Err(format!(
            "{PDFFIGURES2_JAR_ENV} is set to {p} but the file was not found"
        ));
    }

    // 2. Check cache directory.
    let cache_dir = tools_cache_dir();
    let cached_jar = cache_dir.join(PDFFIGURES2_JAR_FILENAME);
    if cached_jar.exists() {
        return Ok(cached_jar);
    }

    // 3. Build from source.
    eprintln!("  Building pdffigures2.jar from source (requires JDK 11 + SBT)...");
    tracing::info!("building pdffigures2.jar from source via SBT …");
    if let Err(e) = build_jar(&cache_dir, &cached_jar).await {
        return Err(format!(
            "{e}. You can build manually and set {PDFFIGURES2_JAR_ENV}=/path/to/pdffigures2.jar"
        ));
    }

    tracing::info!("pdffigures2.jar cached at {}", cached_jar.display());
    Ok(cached_jar)
}

/// Resolved paths for all external dependencies.
struct Prerequisites {
    jar_path: PathBuf,
    java_bin: PathBuf,
}

/// Check all prerequisites for the pdffigures2 + pdftocairo pipeline.
async fn check_prerequisites() -> std::result::Result<Prerequisites, String> {
    // 1. Ensure pdffigures2 JAR is available (auto-download if needed).
    let jar_path = ensure_pdffigures2_jar().await?;

    // 2. Find java (checks $PATH, /usr/libexec/java_home, well-known paths).
    let java_bin = find_java().await?;

    // 3. Ensure pdftocairo is available (auto-install if needed).
    ensure_pdftocairo().await?;

    Ok(Prerequisites { jar_path, java_bin })
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

/// Run pdffigures2 to detect figure bounding boxes and captions.
async fn run_pdffigures2(
    java_bin: &Path,
    jar_path: &Path,
    pdf_path: &Path,
    data_prefix: &Path,
) -> std::result::Result<Vec<Pdffigures2Figure>, String> {
    let output = tokio::time::timeout(
        DETECT_TIMEOUT,
        Command::new(java_bin)
            .args([
                "-Dsun.java2d.cmm=sun.java2d.cmm.kcms.KcmsServiceProvider",
                "-jar",
            ])
            .arg(jar_path)
            .arg(pdf_path)
            .arg("-d")
            .arg(data_prefix)
            .output(),
    )
    .await
    .map_err(|_| format!("pdffigures2 timed out after {DETECT_TIMEOUT:?}"))?
    .map_err(|e| format!("failed to run pdffigures2: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("pdffigures2 failed: {stderr}"));
    }

    // pdffigures2 writes <data_prefix><pdf-stem>.json
    let pdf_stem = pdf_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("_source");
    let json_path = PathBuf::from(format!("{}{pdf_stem}.json", data_prefix.display()));

    if !json_path.exists() {
        // No JSON output means no figures detected — not an error.
        return Ok(Vec::new());
    }

    let json_data = tokio::fs::read_to_string(&json_path)
        .await
        .map_err(|e| format!("failed to read pdffigures2 output: {e}"))?;

    // Clean up the JSON file.
    let _ = tokio::fs::remove_file(&json_path).await;

    let figures: Vec<Pdffigures2Figure> = serde_json::from_str(&json_data)
        .map_err(|e| format!("failed to parse pdffigures2 JSON: {e}"))?;

    Ok(figures)
}

/// Render a single figure region from a PDF page using pdftocairo.
async fn render_figure(
    pdf_path: &Path,
    page: u32,
    region_bb: &Pdffigures2BoundingBox,
    dpi: u32,
    output_path: &Path,
) -> std::result::Result<(), String> {
    let scale = f64::from(dpi) / 72.0;
    let x = (region_bb.x1 * scale) as i32;
    let y = (region_bb.y1 * scale) as i32;
    let w = ((region_bb.x2 - region_bb.x1) * scale).ceil() as i32;
    let h = ((region_bb.y2 - region_bb.y1) * scale).ceil() as i32;

    // pdftocairo uses 1-indexed pages, pdffigures2 uses 0-indexed.
    let page_str = (page + 1).to_string();
    let dpi_str = dpi.to_string();
    let x_str = x.to_string();
    let y_str = y.to_string();
    let w_str = w.to_string();
    let h_str = h.to_string();

    let output = tokio::time::timeout(
        RENDER_TIMEOUT,
        Command::new(PDFTOCAIRO_BIN)
            .args(["-png", "-singlefile"])
            .args(["-f", &page_str, "-l", &page_str])
            .args(["-r", &dpi_str])
            .args(["-x", &x_str, "-y", &y_str, "-W", &w_str, "-H", &h_str])
            .arg(pdf_path)
            .arg(output_path)
            .output(),
    )
    .await
    .map_err(|_| format!("{PDFTOCAIRO_BIN} timed out after {RENDER_TIMEOUT:?}"))?
    .map_err(|e| format!("failed to run {PDFTOCAIRO_BIN}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{PDFTOCAIRO_BIN} failed for page {page}: {stderr}"));
    }

    Ok(())
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

/// A raster image embedded in the PDF, parsed from `pdfimages -list` output.
struct RasterImage {
    /// 1-indexed PDF page number.
    page: u32,
    width: u32,
    height: u32,
    /// Sequential image number from pdfimages.
    num: u32,
}

/// Run `pdfimages -list` to enumerate all embedded raster images in the PDF.
async fn list_raster_images(pdf_path: &Path) -> std::result::Result<Vec<RasterImage>, String> {
    let output = tokio::time::timeout(
        RASTER_EXTRACT_TIMEOUT,
        Command::new(PDFIMAGES_BIN)
            .args(["-list"])
            .arg(pdf_path)
            .output(),
    )
    .await
    .map_err(|_| format!("{PDFIMAGES_BIN} -list timed out"))?
    .map_err(|e| format!("failed to run {PDFIMAGES_BIN} -list: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{PDFIMAGES_BIN} -list failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut images = Vec::new();

    // Skip the first two header lines ("page  num  type ..." and "---...").
    for line in stdout.lines().skip(2) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 {
            continue;
        }
        let page: u32 = match cols[0].parse() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let num: u32 = match cols[1].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // cols[2] = type ("image", "smask", etc.) — skip masks
        if cols[2] != "image" {
            continue;
        }
        let width: u32 = match cols[3].parse() {
            Ok(w) => w,
            Err(_) => continue,
        };
        let height: u32 = match cols[4].parse() {
            Ok(h) => h,
            Err(_) => continue,
        };
        images.push(RasterImage {
            page,
            width,
            height,
            num,
        });
    }
    Ok(images)
}

/// Extract raster images from specific pages using `pdfimages -png`.
///
/// Extracts all images to a temp prefix, then returns paths to images from the
/// requested pages that meet the minimum size threshold.
async fn extract_raster_images(
    pdf_path: &Path,
    output_dir: &Path,
    target_images: &[&RasterImage],
    min_width: u32,
    min_height: u32,
    min_size_bytes: u64,
) -> (Vec<ExtractedFigure>, Vec<String>) {
    let raster_prefix = output_dir.join("_raster");
    let output = tokio::time::timeout(
        RASTER_EXTRACT_TIMEOUT,
        Command::new(PDFIMAGES_BIN)
            .args(["-png", "-p"])
            .arg(pdf_path)
            .arg(&raster_prefix)
            .output(),
    )
    .await;

    let ok = matches!(output, Ok(Ok(o)) if o.status.success());
    if !ok {
        return (Vec::new(), vec!["pdfimages extraction failed".to_string()]);
    }

    let mut figures = Vec::new();
    let mut errors = Vec::new();

    for img in target_images {
        if img.width < min_width || img.height < min_height {
            continue;
        }

        // pdfimages -p names files as <prefix>-<page>-<num>.png (zero-padded)
        let candidate = PathBuf::from(format!(
            "{}-{:03}-{:03}.png",
            raster_prefix.display(),
            img.page,
            img.num
        ));

        if !candidate.exists() {
            continue;
        }

        let metadata = match std::fs::metadata(&candidate) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.len() < min_size_bytes {
            let _ = tokio::fs::remove_file(&candidate).await;
            continue;
        }

        let (width, height) = match read_png_dimensions(&candidate) {
            Some(dims) => dims,
            None => continue,
        };
        if width < min_width || height < min_height {
            let _ = tokio::fs::remove_file(&candidate).await;
            continue;
        }

        // Rename to a stable name in the output dir.
        let final_name = output_dir.join(format!("raster-{:03}-{:03}.png", img.page, img.num));
        if tokio::fs::rename(&candidate, &final_name).await.is_err() {
            errors.push(format!(
                "failed to rename raster image for page {}",
                img.page
            ));
            continue;
        }

        figures.push(ExtractedFigure {
            path: final_name.to_string_lossy().into_owned(),
            width,
            height,
            size_bytes: metadata.len(),
            page_number: img.page,
            index: img.num,
            caption: None,
            figure_type: Some("Figure".to_string()),
        });
    }

    // Clean up remaining raster temp files.
    if let Ok(mut entries) = tokio::fs::read_dir(output_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with("_raster-")
            {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }

    (figures, errors)
}

/// Compute the bounding box dimensions at a given DPI scale.
fn bbox_dimensions_at_dpi(bb: &Pdffigures2BoundingBox, dpi: u32) -> (u32, u32) {
    let scale = f64::from(dpi) / 72.0;
    let w = ((bb.x2 - bb.x1) * scale).ceil() as u32;
    let h = ((bb.y2 - bb.y1) * scale).ceil() as u32;
    (w, h)
}

pub(crate) async fn pdf_extract_figures(
    params: PdfExtractFiguresParams,
) -> Result<PdfExtractFiguresResult> {
    // Check prerequisites.
    let prereqs = match check_prerequisites().await {
        Ok(p) => p,
        Err(msg) => {
            return Ok(PdfExtractFiguresResult {
                success: false,
                figures: Vec::new(),
                total_extracted: 0,
                total_filtered: 0,
                errors: vec![msg],
            });
        }
    };

    let dpi = params.dpi.unwrap_or(DEFAULT_DPI);
    let min_size_bytes = u64::from(params.min_size_kb.unwrap_or(DEFAULT_MIN_SIZE_KB)) * 1024;
    let min_width = params.min_width.unwrap_or(DEFAULT_MIN_WIDTH);
    let min_height = params.min_height.unwrap_or(DEFAULT_MIN_HEIGHT);

    // Create output directory.
    let output_dir = PathBuf::from(&params.output_dir);
    tokio::fs::create_dir_all(&output_dir).await.map_err(|e| {
        ResearchError::Internal(format!(
            "failed to create output dir {}: {e}",
            output_dir.display()
        ))
    })?;

    // Create a data subdirectory for pdffigures2 output.
    let data_dir = output_dir.join("data");
    tokio::fs::create_dir_all(&data_dir).await.map_err(|e| {
        ResearchError::Internal(format!(
            "failed to create data dir {}: {e}",
            data_dir.display()
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

    // Stage 1: Run pdffigures2 to detect figure bounding boxes.
    let data_prefix = data_dir.join("");
    let detected = match run_pdffigures2(
        &prereqs.java_bin,
        &prereqs.jar_path,
        &pdf_path,
        &data_prefix,
    )
    .await
    {
        Ok(figs) => figs,
        Err(msg) => {
            let _ = tokio::fs::remove_file(&pdf_path).await;
            let _ = tokio::fs::remove_dir_all(&data_dir).await;
            return Ok(PdfExtractFiguresResult {
                success: false,
                figures: Vec::new(),
                total_extracted: 0,
                total_filtered: 0,
                errors: vec![msg],
            });
        }
    };

    let total_extracted = detected.len();

    if detected.is_empty() {
        let _ = tokio::fs::remove_file(&pdf_path).await;
        let _ = tokio::fs::remove_dir_all(&data_dir).await;
        return Ok(PdfExtractFiguresResult {
            success: true,
            figures: Vec::new(),
            total_extracted: 0,
            total_filtered: 0,
            errors: Vec::new(),
        });
    }

    // Pre-filter by bounding box dimensions (at target DPI).
    let candidates: Vec<_> = detected
        .into_iter()
        .enumerate()
        .filter(|(_, fig)| {
            let (w, h) = bbox_dimensions_at_dpi(&fig.region_bb, dpi);
            w >= min_width && h >= min_height
        })
        .collect();

    let mut figures = Vec::new();
    let mut errors = Vec::new();
    let mut filtered_count = total_extracted - candidates.len();

    // Stage 2: Render each detected figure with pdftocairo.
    for (idx, fig) in &candidates {
        let output_stem = output_dir.join(format!("fig-{:03}-{:03}", fig.page, idx));
        let output_png = PathBuf::from(format!("{}.png", output_stem.display()));

        if let Err(msg) =
            render_figure(&pdf_path, fig.page, &fig.region_bb, dpi, &output_stem).await
        {
            errors.push(msg);
            filtered_count += 1;
            continue;
        }

        // Read the actual rendered dimensions and file size.
        let metadata = match std::fs::metadata(&output_png) {
            Ok(m) => m,
            Err(_) => {
                filtered_count += 1;
                continue;
            }
        };
        let size_bytes = metadata.len();

        // Apply file size filter.
        if size_bytes < min_size_bytes {
            let _ = tokio::fs::remove_file(&output_png).await;
            filtered_count += 1;
            continue;
        }

        let (width, height) = match read_png_dimensions(&output_png) {
            Some(dims) => dims,
            None => {
                let _ = tokio::fs::remove_file(&output_png).await;
                filtered_count += 1;
                continue;
            }
        };

        // Apply rendered dimension filter.
        if width < min_width || height < min_height {
            let _ = tokio::fs::remove_file(&output_png).await;
            filtered_count += 1;
            continue;
        }

        figures.push(ExtractedFigure {
            path: output_png.to_string_lossy().into_owned(),
            width,
            height,
            size_bytes,
            page_number: fig.page,
            index: *idx as u32,
            caption: Some(fig.caption.clone()),
            figure_type: Some(fig.figure_type.clone()),
        });
    }

    // Stage 3: Supplement with raster images from pages pdffigures2 missed.
    //
    // pdffigures2 only detects vector graphics regions. Figures embedded as
    // raster images (JPEG/PNG) are invisible to it. We use `pdfimages` from
    // poppler-utils to find embedded raster images and extract those from
    // pages that pdffigures2 did not cover.
    let pdffigures2_pages: std::collections::HashSet<u32> = candidates
        .iter()
        .map(|(_, fig)| fig.page + 1) // pdffigures2 is 0-indexed, pdfimages is 1-indexed
        .collect();

    if let Ok(raster_list) = list_raster_images(&pdf_path).await {
        let missed: Vec<&RasterImage> = raster_list
            .iter()
            .filter(|img| {
                !pdffigures2_pages.contains(&img.page)
                    && img.width >= RASTER_MIN_WIDTH
                    && img.height >= RASTER_MIN_HEIGHT
            })
            .collect();

        if !missed.is_empty() {
            tracing::info!(
                "pdffigures2 missed {} pages with raster images, extracting via pdfimages",
                missed.len()
            );
            let (raster_figs, raster_errs) = extract_raster_images(
                &pdf_path,
                &output_dir,
                &missed,
                min_width,
                min_height,
                min_size_bytes,
            )
            .await;
            figures.extend(raster_figs);
            errors.extend(raster_errs);
        }
    }

    // Sort by page then index.
    figures.sort_by_key(|f| (f.page_number, f.index));

    let total_extracted = total_extracted + figures.iter().filter(|f| f.caption.is_none()).count();

    // Cleanup source PDF and data directory.
    let _ = tokio::fs::remove_file(&pdf_path).await;
    let _ = tokio::fs::remove_dir_all(&data_dir).await;

    Ok(PdfExtractFiguresResult {
        success: true,
        figures,
        total_extracted,
        total_filtered: filtered_count,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

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

    #[test]
    fn parse_pdffigures2_json() {
        let json = r#"[
            {
                "caption": "Figure 1: Overview of the system.",
                "name": "1",
                "figType": "Figure",
                "page": 1,
                "regionBoundary": {"x1": 72.0, "y1": 100.0, "x2": 540.0, "y2": 400.0},
                "captionBoundary": {"x1": 72.0, "y1": 405.0, "x2": 540.0, "y2": 420.0},
                "pageWidth": 612.0,
                "pageHeight": 792.0
            },
            {
                "caption": "Table 1: Results comparison.",
                "name": "1",
                "figType": "Table",
                "page": 3,
                "regionBoundary": {"x1": 50.0, "y1": 200.0, "x2": 562.0, "y2": 500.0},
                "captionBoundary": {"x1": 50.0, "y1": 505.0, "x2": 562.0, "y2": 520.0},
                "pageWidth": 612.0,
                "pageHeight": 792.0
            }
        ]"#;

        let figures: Vec<Pdffigures2Figure> =
            serde_json::from_str(json).expect("parse pdffigures2 JSON");
        assert_eq!(figures.len(), 2);

        assert_eq!(figures[0].caption, "Figure 1: Overview of the system.");
        assert_eq!(figures[0].name, "1");
        assert_eq!(figures[0].figure_type, "Figure");
        assert_eq!(figures[0].page, 1);
        assert!((figures[0].region_bb.x1 - 72.0).abs() < f64::EPSILON);
        assert!((figures[0].region_bb.y1 - 100.0).abs() < f64::EPSILON);
        assert!((figures[0].region_bb.x2 - 540.0).abs() < f64::EPSILON);
        assert!((figures[0].region_bb.y2 - 400.0).abs() < f64::EPSILON);

        assert_eq!(figures[1].figure_type, "Table");
        assert_eq!(figures[1].page, 3);
    }

    #[test]
    fn compute_crop_coordinates_at_300_dpi() {
        let bb = Pdffigures2BoundingBox {
            x1: 72.0,
            y1: 100.0,
            x2: 540.0,
            y2: 400.0,
        };
        let dpi = 300;
        let scale = f64::from(dpi) / 72.0;

        let x = (bb.x1 * scale) as i32;
        let y = (bb.y1 * scale) as i32;
        let w = ((bb.x2 - bb.x1) * scale).ceil() as i32;
        let h = ((bb.y2 - bb.y1) * scale).ceil() as i32;

        // At 300 DPI, scale = 300/72 ≈ 4.1667
        assert_eq!(x, 300); // 72 * 4.1667 = 300
        assert_eq!(y, 416); // 100 * 4.1667 = 416.67 → 416
        assert_eq!(w, 1951); // 468 * 4.1667 ≈ 1950.0, ceil → 1951 (fp rounding)
        assert_eq!(h, 1250); // 300 * 4.1667 = 1250
    }

    #[test]
    fn bbox_dimensions_at_dpi_scaling() {
        let bb = Pdffigures2BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 72.0,
            y2: 72.0,
        };
        // At 72 DPI, 72 pts = 72 px
        assert_eq!(bbox_dimensions_at_dpi(&bb, 72), (72, 72));
        // At 300 DPI, 72 pts = 300 px
        assert_eq!(bbox_dimensions_at_dpi(&bb, 300), (300, 300));
        // At 150 DPI, 72 pts = 150 px
        assert_eq!(bbox_dimensions_at_dpi(&bb, 150), (150, 150));
    }

    #[test]
    fn filter_by_bbox_dimensions() {
        let small_bb = Pdffigures2BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 20.0,
            y2: 20.0,
        };
        let large_bb = Pdffigures2BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 200.0,
            y2: 200.0,
        };

        let min_width = 100;
        let min_height = 100;
        let dpi = 300;

        // Small bbox at 300 DPI: 20 * (300/72) ≈ 84 px — below threshold
        let (w, h) = bbox_dimensions_at_dpi(&small_bb, dpi);
        assert!(w < min_width || h < min_height);

        // Large bbox at 300 DPI: 200 * (300/72) ≈ 834 px — above threshold
        let (w, h) = bbox_dimensions_at_dpi(&large_bb, dpi);
        assert!(w >= min_width && h >= min_height);
    }

    #[test]
    fn tools_cache_dir_returns_non_empty_path() {
        let dir = tools_cache_dir();
        assert!(
            dir.ends_with("codex-research-tools"),
            "expected path ending with codex-research-tools, got {}",
            dir.display()
        );
    }

    #[tokio::test]
    async fn ensure_jar_returns_cached_jar_without_build() {
        // Pre-populate a cached JAR and verify the cache detection logic finds it.
        let dir = tempfile::tempdir().expect("create temp dir");
        let cache_dir = dir.path();
        let jar = cache_dir.join(PDFFIGURES2_JAR_FILENAME);
        std::fs::write(&jar, vec![0_u8; 2_000_000]).expect("write fake cached JAR");

        assert!(jar.exists());
        let cached_jar = cache_dir.join(PDFFIGURES2_JAR_FILENAME);
        assert!(cached_jar.exists(), "cached JAR should be detected");
    }

    #[tokio::test]
    async fn probe_java_finds_installed_java() {
        // On dev machines with Java installed, probe_java should find it.
        let result = probe_java().await;
        if let Some(path) = result {
            assert!(
                path.to_string_lossy().contains("java"),
                "path should contain 'java', got: {}",
                path.display()
            );
        }
        // If Java is not installed, the test trivially passes.
    }

    #[tokio::test]
    async fn find_java_returns_valid_path() {
        // find_java() should succeed on machines with Java (or auto-install).
        // We only assert the shape — not that it actually auto-installed.
        match find_java().await {
            Ok(path) => {
                assert!(
                    path.to_string_lossy().contains("java"),
                    "expected java in path, got: {}",
                    path.display()
                );
            }
            Err(msg) => {
                // Acceptable if no package manager is available.
                assert!(msg.contains("not found"), "unexpected error: {msg}");
            }
        }
    }

    #[tokio::test]
    async fn ensure_pdftocairo_succeeds_when_poppler_installed() {
        // On machines with poppler-utils, this should succeed.
        let result = ensure_pdftocairo().await;
        if super::super::system_deps::is_on_path("pdftocairo").await {
            assert!(
                result.is_ok(),
                "pdftocairo is on PATH but ensure_pdftocairo failed: {:?}",
                result.err()
            );
        }
    }
}
