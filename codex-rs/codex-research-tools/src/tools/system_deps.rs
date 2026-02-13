use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;

const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) enum PackageManager {
    Brew,
    AptGet,
}

/// Detect the system package manager (brew on macOS, apt-get on Linux).
pub(crate) async fn detect_package_manager() -> Option<PackageManager> {
    if is_on_path("brew").await {
        return Some(PackageManager::Brew);
    }
    if is_on_path("apt-get").await {
        return Some(PackageManager::AptGet);
    }
    None
}

/// Install a package via `brew install`.
pub(crate) async fn brew_install(package: &str, extra_args: &[&str]) -> Result<(), String> {
    tracing::info!("auto-installing {package} via brew …");
    let output = tokio::time::timeout(
        INSTALL_TIMEOUT,
        Command::new("brew")
            .arg("install")
            .args(extra_args)
            .arg(package)
            .output(),
    )
    .await
    .map_err(|_| format!("brew install {package} timed out after {INSTALL_TIMEOUT:?}"))?
    .map_err(|e| format!("failed to run brew install {package}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("brew install {package} failed: {stderr}"));
    }
    tracing::info!("successfully installed {package} via brew");
    Ok(())
}

/// Install packages via `sudo apt-get install -y`.
pub(crate) async fn apt_install(packages: &[&str]) -> Result<(), String> {
    let pkg_list = packages.join(", ");
    tracing::info!("auto-installing {pkg_list} via apt-get …");
    let output = tokio::time::timeout(
        INSTALL_TIMEOUT,
        Command::new("sudo")
            .args(["apt-get", "install", "-y"])
            .args(packages)
            .output(),
    )
    .await
    .map_err(|_| format!("apt-get install timed out after {INSTALL_TIMEOUT:?}"))?
    .map_err(|e| format!("failed to run apt-get install: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("apt-get install [{pkg_list}] failed: {stderr}"));
    }
    tracing::info!("successfully installed {pkg_list} via apt-get");
    Ok(())
}

/// Check whether `name` is on `$PATH` via `which`.
pub(crate) async fn is_on_path(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// Return the first existing path from `candidates`.
pub(crate) fn find_in_well_known_paths(candidates: &[&str]) -> Option<PathBuf> {
    for path in candidates {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn is_on_path_finds_common_binary() {
        // `which` itself must be on PATH for any of this to work.
        assert!(is_on_path("which").await);
    }

    #[tokio::test]
    async fn is_on_path_returns_false_for_nonexistent() {
        assert!(!is_on_path("nonexistent_binary_xyz_123").await);
    }

    #[tokio::test]
    async fn detect_package_manager_returns_some_on_this_machine() {
        // On any CI or dev machine, at least one of brew/apt-get should exist.
        // If neither exists this test will still pass — it just won't assert.
        let pm = detect_package_manager().await;
        if cfg!(target_os = "macos") {
            // On macOS dev machines, brew is virtually always present.
            assert!(pm.is_some(), "expected brew to be detected on macOS");
        }
    }

    #[test]
    fn find_in_well_known_paths_returns_none_for_empty_list() {
        assert!(find_in_well_known_paths(&[]).is_none());
    }

    #[test]
    fn find_in_well_known_paths_returns_none_when_nothing_exists() {
        let result = find_in_well_known_paths(&["/nonexistent/path/aaa", "/nonexistent/path/bbb"]);
        assert!(result.is_none());
    }

    #[test]
    fn find_in_well_known_paths_finds_existing_path() {
        // /bin/sh exists on essentially every Unix system.
        let result = find_in_well_known_paths(&["/nonexistent/path/xyz", "/bin/sh"]);
        assert_eq!(result, Some(PathBuf::from("/bin/sh")));
    }

    #[test]
    fn find_in_well_known_paths_returns_first_match() {
        let result = find_in_well_known_paths(&["/bin/sh", "/usr/bin/env"]);
        // Should return the first one, not the second.
        assert_eq!(result, Some(PathBuf::from("/bin/sh")));
    }
}
