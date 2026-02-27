use std::time::Duration;

use crate::ResearchToolkit;
use crate::error::ResearchError;
use crate::error::Result;

/// Attempt a single HTTP GET to `base_url` with a short timeout.
/// Any HTTP response (even 4xx/5xx) means Zotero is listening.
async fn ping_zotero(base_url: &str, client: &reqwest::Client) -> bool {
    client
        .get(base_url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok()
}

/// Launch the Zotero desktop application in the background.
fn launch_zotero() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-g", "-b", "org.zotero.zotero"])
            .spawn()
            .map_err(|e| ResearchError::Internal(format!("failed to launch Zotero: {e}")))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        // Try common Zotero binary names on Linux.
        let names = ["zotero", "zotero-bin"];
        for name in &names {
            if let Ok(_child) = std::process::Command::new(name)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                return Ok(());
            }
        }
        // Try flatpak as a fallback.
        if let Ok(_child) = std::process::Command::new("flatpak")
            .args(["run", "org.zotero.Zotero"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            return Ok(());
        }
        Err(ResearchError::Internal(
            "could not find Zotero on this system — install it or start it manually".to_string(),
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(ResearchError::Internal(
            "auto-starting Zotero is only supported on macOS and Linux — start it manually"
                .to_string(),
        ))
    }
}

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);

/// Ensure the local Zotero API is reachable, launching the application if
/// necessary. When talking to the remote (cloud) API this is a no-op.
pub(crate) async fn ensure_zotero_running_impl(toolkit: &ResearchToolkit) -> Result<()> {
    // Remote API — nothing to start.
    if !toolkit.config().uses_local_zotero_api() {
        return Ok(());
    }

    let base_url = &toolkit.config().zotero_base_url;
    let client = toolkit.http().client();

    // Already running — fast path.
    if ping_zotero(base_url, client).await {
        return Ok(());
    }

    tracing::info!("Zotero local API not responding — attempting to launch Zotero desktop");
    launch_zotero()?;

    // Poll until the API becomes reachable.
    let deadline = tokio::time::Instant::now() + POLL_TIMEOUT;
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if ping_zotero(base_url, client).await {
            tracing::info!("Zotero local API is now reachable");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ResearchError::Internal(
                "Zotero was launched but its local API did not become reachable within 8 seconds \
                 — is Zotero installed? Start it manually or check the connection."
                    .to_string(),
            ));
        }
    }
}
