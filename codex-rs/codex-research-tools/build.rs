#[cfg(feature = "pdf_images")]
use std::path::PathBuf;

fn main() {
    #[cfg(feature = "pdf_images")]
    build_pdffigures2_jar();

    // Suppress unused warning when feature is off.
    #[cfg(not(feature = "pdf_images"))]
    let _ = ();
}

#[cfg(feature = "pdf_images")]
fn build_pdffigures2_jar() {
    // Re-run if these env vars change.
    println!("cargo:rerun-if-env-changed=PDFFIGURES2_JAR");
    println!("cargo:rerun-if-env-changed=JAVA_HOME");

    // If the user explicitly provides a JAR path, use it directly.
    if let Ok(p) = std::env::var("PDFFIGURES2_JAR") {
        let path = PathBuf::from(&p);
        if path.exists() {
            println!("cargo:rustc-env=PDFFIGURES2_JAR_BUILDTIME={p}");
            return;
        }
    }

    // Check if JAR is already cached.
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("codex-research-tools");
    let cached_jar = cache_dir.join("pdffigures2.jar");

    if cached_jar.exists()
        && let Ok(meta) = std::fs::metadata(&cached_jar)
        && meta.len() >= 1_000_000
    {
        println!(
            "cargo:rustc-env=PDFFIGURES2_JAR_BUILDTIME={}",
            cached_jar.display()
        );
        return;
    }

    // Try to find a compatible JDK (11 or 8) for building pdffigures2.
    let jdk_home = find_build_jdk();
    let has_sbt = has_command("sbt");

    let jdk_home = match (jdk_home, has_sbt) {
        (Some(jdk), true) => jdk,
        (jdk, sbt) => {
            if jdk.is_none() {
                println!(
                    "cargo:warning=pdffigures2: JDK 11 not found. \
                     Install with: brew install openjdk@11 (macOS) or \
                     sudo apt install openjdk-11-jdk-headless (Linux)"
                );
            }
            if !sbt {
                println!(
                    "cargo:warning=pdffigures2: sbt not found. \
                     Install with: brew install sbt (macOS) or see \
                     https://www.scala-sbt.org/download/"
                );
            }
            println!(
                "cargo:warning=pdffigures2: Skipping compile-time JAR build. \
                 Will fall back to runtime build on first use."
            );
            return;
        }
    };

    // Create cache directory.
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        println!("cargo:warning=pdffigures2: failed to create cache dir: {e}");
        return;
    }

    // Clone pdffigures2 into a temp directory.
    let tmp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            println!("cargo:warning=pdffigures2: failed to create temp dir: {e}");
            return;
        }
    };
    let clone_dir = tmp_dir.path().join("pdffigures2");

    eprintln!("  [build.rs] Cloning pdffigures2 repository...");
    let clone = std::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "https://github.com/allenai/pdffigures2.git",
        ])
        .arg(&clone_dir)
        .output();

    match clone {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!("cargo:warning=pdffigures2: git clone failed: {stderr}");
            return;
        }
        Err(e) => {
            println!("cargo:warning=pdffigures2: failed to run git clone: {e}");
            return;
        }
    }

    // Build with sbt assembly.
    eprintln!(
        "  [build.rs] Running sbt assembly with JAVA_HOME={} (this may take several minutes)...",
        jdk_home.display()
    );
    let build = std::process::Command::new("sbt")
        .arg("assembly")
        .env("JAVA_HOME", &jdk_home)
        .current_dir(&clone_dir)
        .output();

    match build {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!("cargo:warning=pdffigures2: sbt assembly failed: {stderr}");
            return;
        }
        Err(e) => {
            println!("cargo:warning=pdffigures2: failed to run sbt assembly: {e}");
            return;
        }
    }

    // The fat JAR lands in the project root (not target/).
    let built_jar = clone_dir.join("pdffigures2.jar");
    if !built_jar.exists() {
        println!("cargo:warning=pdffigures2: sbt assembly completed but pdffigures2.jar not found");
        return;
    }

    // Validate size.
    if let Ok(meta) = std::fs::metadata(&built_jar)
        && meta.len() < 1_000_000
    {
        println!(
            "cargo:warning=pdffigures2: built JAR too small ({} bytes), skipping",
            meta.len()
        );
        return;
    }

    // Atomic copy to cache.
    let tmp_dest = cached_jar.with_extension("jar.tmp");
    if let Err(e) = std::fs::copy(&built_jar, &tmp_dest) {
        println!("cargo:warning=pdffigures2: failed to copy JAR to cache: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_dest, &cached_jar) {
        println!("cargo:warning=pdffigures2: failed to rename cached JAR: {e}");
        let _ = std::fs::remove_file(&tmp_dest);
        return;
    }

    eprintln!(
        "  [build.rs] pdffigures2.jar cached at {}",
        cached_jar.display()
    );
    println!(
        "cargo:rustc-env=PDFFIGURES2_JAR_BUILDTIME={}",
        cached_jar.display()
    );
}

/// Well-known JAVA_HOME locations for JDK 11 or 8 (compatible with pdffigures2).
#[cfg(feature = "pdf_images")]
const BUILD_JDK_HOMES: &[&str] = &[
    // Homebrew JDK 11 (Apple Silicon / Intel)
    "/opt/homebrew/opt/openjdk@11",
    "/usr/local/opt/openjdk@11",
    // Homebrew JDK 8 (Intel only)
    "/opt/homebrew/opt/openjdk@8",
    "/usr/local/opt/openjdk@8",
    // Linux apt JDK 11
    "/usr/lib/jvm/java-11-openjdk-amd64",
    "/usr/lib/jvm/java-11-openjdk-arm64",
    // Linux apt JDK 8
    "/usr/lib/jvm/java-8-openjdk-amd64",
    "/usr/lib/jvm/java-8-openjdk-arm64",
];

/// Find a compatible JDK (11 or 8) for building pdffigures2.
#[cfg(feature = "pdf_images")]
fn find_build_jdk() -> Option<PathBuf> {
    // Check JAVA_HOME env first.
    if let Ok(home) = std::env::var("JAVA_HOME") {
        let p = PathBuf::from(&home);
        if p.join("bin/java").exists() {
            return Some(p);
        }
    }

    // Check well-known paths.
    for home in BUILD_JDK_HOMES {
        let p = PathBuf::from(home);
        if p.join("bin/java").exists() {
            return Some(p);
        }
    }

    // macOS: ask /usr/libexec/java_home for JDK 11.
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/libexec/java_home")
            .args(["-v", "11"])
            .output()
            .ok();
        if let Some(out) = output
            && out.status.success()
        {
            let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = PathBuf::from(&home);
            if p.join("bin/java").exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Check whether a command is available on PATH.
#[cfg(feature = "pdf_images")]
fn has_command(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
