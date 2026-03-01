use std::sync::Arc;

use tokio::sync::watch;

use super::TriggerEvent;
use super::TriggerSender;

/// Start a local HTTP server that listens for webhook POST requests.
/// Each registered path maps to a job ID.
pub async fn start_webhook_server(
    bindings: Vec<(String, String)>, // (path, job_id) pairs
    port: u16,
    tx: TriggerSender,
    mut shutdown: watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let server = tiny_http::Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("failed to start webhook server on {addr}: {e}"))?;

    tracing::info!("webhook server listening on {addr}");

    let server = Arc::new(server);
    let bindings: Arc<Vec<(String, String)>> = Arc::new(bindings);

    let server_clone = Arc::clone(&server);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        server_clone.unblock();
                        break;
                    }
                }
            }
        }
    });

    tokio::task::spawn_blocking(move || {
        for mut request in server.incoming_requests() {
            let url = request.url().to_string();

            // Find matching binding.
            let matched = bindings.iter().find(|(path, _)| *path == url);

            match matched {
                Some((_, job_id)) => {
                    // Read body.
                    let mut body = String::new();
                    let _ = request.as_reader().read_to_string(&mut body);

                    let data = serde_json::json!({
                        "trigger": "webhook",
                        "path": url,
                        "body": body,
                    });

                    let _ = tx.send(TriggerEvent {
                        job_id: job_id.clone(),
                        data: Some(data.to_string()),
                    });

                    let response = tiny_http::Response::from_string("OK\n").with_status_code(200);
                    let _ = request.respond(response);
                }
                None => {
                    let response =
                        tiny_http::Response::from_string("Not Found\n").with_status_code(404);
                    let _ = request.respond(response);
                }
            }
        }
    });

    Ok(())
}
