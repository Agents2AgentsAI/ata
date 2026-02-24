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
    #[cfg(not(target_os = "macos"))]
    {
        Err(ResearchError::Internal(
            "auto-starting Zotero is only supported on macOS".to_string(),
        ))
    }
}

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const POLL_TIMEOUT: Duration = Duration::from_secs(20);

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
                "Zotero was launched but its local API did not become reachable within 20 seconds"
                    .to_string(),
            ));
        }
    }
}
