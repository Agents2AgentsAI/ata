//! A lightweight local HTTP + WebSocket server that serves the Living
//! Reading View HTML template and streams document events to connected
//! browser clients.
//!
//! The WebSocket is bidirectional: the server sends document events
//! (section updates, karaoke highlights, etc.) to the browser, and the
//! browser can send messages back (follow-up questions, read-aloud
//! requests, etc.) which are forwarded to the app via an optional
//! callback channel.
//!
//! Usage:
//! ```ignore
//! let server = ReadingViewServer::start(None, None).await?;
//! println!("Open {}", server.url());
//! server.send_event(r#"{"type":"present_document","title":"Hello","sections":[]}"#);
//! ```

use axum::Router;
use axum::extract::ws::Message;
use axum::extract::ws::WebSocket;
use axum::extract::ws::WebSocketUpgrade;
use axum::response::Html;
use axum::routing::get;
use futures::stream::StreamExt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tower_http::services::ServeDir;

/// Capacity for the broadcast channel. Should be generous enough that a
/// burst of section updates does not cause dropped messages for slow
/// WebSocket clients.
const BROADCAST_CAPACITY: usize = 256;

/// Embedded HTML template — the same file used by the Swift WKWebView.
/// The template already contains a WebSocket client that connects to
/// `ws://<host>/ws` when not running inside a WKWebView.
const HTML_TEMPLATE: &str = include_str!("assets/LivingReadingView.html");

/// A local HTTP server that serves the Living Reading View page and
/// broadcasts document events to all connected WebSocket clients.
///
/// Messages are buffered so that late-connecting WebSocket clients
/// (e.g. the browser opening after `presentDocument` was sent) receive
/// the full event history on connect.
///
/// `ReadingViewServer` is cheaply cloneable — all clones share the same
/// broadcast channel and event buffer, so events sent through any clone
/// are received by all connected WebSocket clients and recorded in the
/// shared replay buffer.
#[derive(Clone)]
pub struct ReadingViewServer {
    port: u16,
    tx: broadcast::Sender<String>,
    /// All events sent so far, replayed to each new WebSocket client.
    event_buffer: Arc<Mutex<Vec<String>>>,
    /// Optional root directory for serving static assets (e.g. figure images).
    #[allow(dead_code)]
    assets_root: Option<PathBuf>,
}

impl std::fmt::Debug for ReadingViewServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadingViewServer")
            .field("port", &self.port)
            .finish()
    }
}

impl ReadingViewServer {
    /// Starts the server on `127.0.0.1` with a random available port.
    ///
    /// If `assets_root` is `Some(path)`, the server will serve static files
    /// from that directory under the `/assets` URL prefix (e.g. for figure
    /// images referenced by the reading view).
    ///
    /// If `incoming_tx` is `Some`, messages received from browser WebSocket
    /// clients (e.g. follow-up questions, read-aloud requests) will be
    /// forwarded to this channel as raw JSON strings.
    ///
    /// Returns immediately once the listener is bound; the server runs in
    /// a background tokio task.
    pub async fn start(
        assets_root: Option<PathBuf>,
        incoming_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        let event_buffer: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let incoming_tx: Arc<Option<mpsc::UnboundedSender<String>>> = Arc::new(incoming_tx);

        let app = {
            let tx_for_ws = tx.clone();
            let buffer_for_ws = Arc::clone(&event_buffer);
            let incoming_for_ws = Arc::clone(&incoming_tx);
            let router = Router::new()
                .route("/", get(|| async { Html(HTML_TEMPLATE) }))
                .route(
                    "/ws",
                    get(move |ws: WebSocketUpgrade| {
                        let rx = tx_for_ws.subscribe();
                        let buf = Arc::clone(&buffer_for_ws);
                        let inc = Arc::clone(&incoming_for_ws);
                        async { ws.on_upgrade(|socket| handle_ws(socket, rx, buf, inc)) }
                    }),
                );
            if let Some(ref root) = assets_root {
                router.nest_service("/assets", ServeDir::new(root))
            } else {
                router
            }
        };

        // Use a fixed port so browser tabs from previous sessions can reconnect
        // via WebSocket without needing a new tab. Fall back to random if busy.
        let listener = {
            let fixed = SocketAddr::from(([127, 0, 0, 1], 14_523));
            match tokio::net::TcpListener::bind(fixed).await {
                Ok(l) => l,
                Err(_) => {
                    // Fixed port busy (another ATA session?) — use random.
                    let random = SocketAddr::from(([127, 0, 0, 1], 0));
                    tokio::net::TcpListener::bind(random).await?
                }
            }
        };
        let port = listener.local_addr()?.port();

        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("reading-view-server error: {e}");
            }
        });

        tracing::info!("Reading view server listening on http://127.0.0.1:{port}");

        Ok(Self {
            port,
            tx,
            event_buffer,
            assets_root,
        })
    }

    /// Broadcasts a JSON event string to all connected WebSocket clients.
    ///
    /// The event is also appended to an internal buffer so that
    /// late-connecting clients receive the full history on connect.
    ///
    /// Returns the number of receivers that received the message. Returns
    /// `0` if no clients are connected (this is not an error).
    pub fn send_event(&self, json: &str) -> usize {
        if let Ok(mut buf) = self.event_buffer.lock() {
            buf.push(json.to_string());
        }
        self.tx.send(json.to_string()).unwrap_or(0)
    }

    /// The URL where the reading view page is served.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Replays buffered events to a newly connected WebSocket client, then
/// runs a bidirectional loop: forwarding live broadcast events to the
/// client and routing incoming client messages to the app.
async fn handle_ws(
    socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    event_buffer: Arc<Mutex<Vec<String>>>,
    incoming_tx: Arc<Option<mpsc::UnboundedSender<String>>>,
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Replay all buffered events so late-connecting clients get the full state.
    {
        use futures::SinkExt;
        let buffered = event_buffer
            .lock()
            .map(|buf| buf.clone())
            .unwrap_or_default();
        for msg in buffered {
            if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                return; // client disconnected during replay
            }
        }
    }

    // Run send and receive concurrently.
    // The send side forwards broadcast events to the browser.
    // The receive side routes browser messages to the app.
    let send_task = async {
        use futures::SinkExt;
        while let Ok(msg) = rx.recv().await {
            if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                break; // client disconnected
            }
        }
    };

    let recv_task = async {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Text(text) => {
                    if let Some(ref tx) = *incoming_tx {
                        let _ = tx.send(text.to_string());
                    }
                }
                Message::Close(_) => break,
                _ => {} // ignore binary, ping, pong
            }
        }
    };

    // When either side finishes (client disconnect, broadcast channel closed),
    // we're done with this connection.
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_starts_and_serves_html() {
        let server = ReadingViewServer::start(None, None)
            .await
            .expect("server should start");

        // The URL should be a valid localhost address.
        let url = server.url();
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(server.port() > 0);

        // send_event with no clients should return 0, not panic.
        assert_eq!(server.send_event(r#"{"type":"test"}"#), 0);
    }
}
