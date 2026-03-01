use std::collections::HashMap;
use std::time::Duration;

use tokio::sync::watch;

use super::TriggerEvent;
use super::TriggerSender;

/// Poll an HTTP endpoint at a fixed interval. Fires a trigger event when the
/// response body changes (or when a JSON path value changes).
pub async fn start_http_poll(
    job_id: String,
    url: String,
    interval_seconds: u64,
    headers: HashMap<String, String>,
    change_path: Option<String>,
    tx: TriggerSender,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let interval_dur = Duration::from_secs(interval_seconds);

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut last_value: Option<String> = None;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval_dur) => {
                    let result = fetch_value(&client, &url, &headers, &change_path).await;
                    match result {
                        Ok(current) => {
                            let changed = last_value.as_ref().is_some_and(|prev| prev != &current);
                            if changed || last_value.is_none() {
                                if last_value.is_some() {
                                    // Only fire on actual change, not first poll.
                                    let data = serde_json::json!({
                                        "trigger": "http_poll",
                                        "url": url,
                                        "value": current,
                                    });
                                    let _ = tx.send(TriggerEvent {
                                        job_id: job_id.clone(),
                                        data: Some(data.to_string()),
                                    });
                                }
                                last_value = Some(current);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("http_poll for '{job_id}' failed: {e:#}");
                        }
                    }
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

async fn fetch_value(
    client: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
    change_path: &Option<String>,
) -> anyhow::Result<String> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await?;
    let body = resp.text().await?;

    match change_path {
        Some(path) => {
            // Simple dot-path extraction from JSON.
            let json: serde_json::Value = serde_json::from_str(&body)?;
            let value = extract_json_path(&json, path);
            Ok(value.to_string())
        }
        None => Ok(body),
    }
}

/// Simple dot-path JSON value extraction (e.g. "tag_name" or "data.version").
fn extract_json_path<'a>(json: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    let mut current = json;
    for key in path.split('.') {
        current = current.get(key).unwrap_or(&serde_json::Value::Null);
    }
    current
}
