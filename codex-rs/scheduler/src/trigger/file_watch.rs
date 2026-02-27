use std::path::PathBuf;
use std::time::Duration;

use notify::Event;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use tokio::sync::watch;

use super::TriggerEvent;
use super::TriggerSender;

/// Start watching a filesystem path. Fires a trigger event when files matching
/// the optional glob pattern change, with debouncing.
pub async fn start_file_watch(
    job_id: String,
    path: PathBuf,
    pattern: Option<String>,
    debounce_seconds: u64,
    tx: TriggerSender,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            let _ = notify_tx.send(event);
        }
    })?;

    watcher.watch(&path, RecursiveMode::Recursive)?;

    let debounce = Duration::from_secs(debounce_seconds);

    tokio::spawn(async move {
        // Keep watcher alive.
        let _watcher = watcher;

        let mut pending = false;

        loop {
            tokio::select! {
                Some(event) = notify_rx.recv() => {
                    if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) {
                        // Check pattern match if specified.
                        let matches = match &pattern {
                            Some(pat) => event.paths.iter().any(|p| {
                                p.to_str().is_some_and(|s| s.contains(pat.trim_start_matches('*').trim_start_matches('.')))
                            }),
                            None => true,
                        };
                        if matches {
                            pending = true;
                        }
                    }
                }
                _ = tokio::time::sleep(debounce), if pending => {
                    pending = false;
                    let changed_data = serde_json::json!({
                        "trigger": "file_watch",
                        "path": path.display().to_string(),
                    });
                    let _ = tx.send(TriggerEvent {
                        job_id: job_id.clone(),
                        data: Some(changed_data.to_string()),
                    });
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    });

    Ok(())
}
