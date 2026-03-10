//! Coordination relay server for cross-machine agent awareness.
//!
//! This module exposes [`run_server`] so the relay can be started from both
//! the standalone `codex-relay` binary and from `ata team relay`.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::IntoResponse;
use axum::response::sse::Event as SseEvent;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::patch;
use axum::routing::post;
use axum::routing::put;
use futures::stream::Stream;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the relay server.
pub struct RelayServerConfig {
    pub port: u16,
    pub secret: Option<String>,
    pub max_messages: usize,
}

/// Try to start the relay server as a background task.
///
/// Returns `true` if this process became the relay host (bound the port
/// successfully). Returns `false` if another process already holds the port
/// (the server is already running elsewhere).
pub async fn try_start_background(config: RelayServerConfig) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], config.port));

    // Try to bind — if it fails, another instance is already hosting.
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(_) => {
            tracing::info!(
                "relay port {} already in use — connecting as client",
                config.port
            );
            return false;
        }
    };

    tracing::info!("auto-started relay server on {addr}");

    let state = Arc::new(AppState {
        projects: RwLock::new(HashMap::new()),
        secret: config.secret,
        max_messages: config.max_messages,
    });

    tokio::spawn(prune_loop(Arc::clone(&state)));

    let api = build_api_router(Arc::clone(&state));
    let app = Router::new().route("/health", get(health)).merge(api);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::warn!("background relay server error: {e}");
        }
    });

    true
}

/// Start the coordination relay server. Blocks until Ctrl+C.
pub async fn run_server(config: RelayServerConfig) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        projects: RwLock::new(HashMap::new()),
        secret: config.secret,
        max_messages: config.max_messages,
    });

    tokio::spawn(prune_loop(Arc::clone(&state)));

    let api = build_api_router(Arc::clone(&state));
    let app = Router::new().route("/health", get(health)).merge(api);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("relay server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Build the API router with auth middleware.
fn build_api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/v1/projects/{project_id}/sessions/{session_id}",
            put(register_session).delete(deregister_session),
        )
        .route(
            "/v1/projects/{project_id}/sessions/{session_id}/heartbeat",
            post(heartbeat),
        )
        .route(
            "/v1/projects/{project_id}/sessions/{session_id}/description",
            patch(update_description),
        )
        .route("/v1/projects/{project_id}/sessions", get(list_sessions))
        .route(
            "/v1/projects/{project_id}/messages",
            post(post_message).get(list_messages),
        )
        .route("/v1/projects/{project_id}/events", get(events_stream))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth_layer,
        ))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct AppState {
    projects: RwLock<HashMap<String, ProjectState>>,
    secret: Option<String>,
    max_messages: usize,
}

struct ProjectState {
    sessions: HashMap<String, SessionData>,
    messages: VecDeque<MessageData>,
    next_msg_id: i64,
    msg_broadcast: broadcast::Sender<MessageData>,
}

/// Broadcast capacity for SSE event streaming.
const MSG_BROADCAST_CAPACITY: usize = 128;

impl ProjectState {
    fn new() -> Self {
        let (msg_broadcast, _) = broadcast::channel(MSG_BROADCAST_CAPACITY);
        Self {
            sessions: HashMap::new(),
            messages: VecDeque::new(),
            next_msg_id: 1,
            msg_broadcast,
        }
    }
}

#[derive(Clone, Serialize)]
struct SessionData {
    session_id: String,
    branch: Option<String>,
    description: Option<String>,
    started_at: i64,
    last_heartbeat: i64,
}

#[derive(Clone, Serialize)]
struct MessageData {
    id: i64,
    session_id: String,
    message: String,
    message_type: String,
    created_at: i64,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RegisterBody {
    branch: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct DescriptionBody {
    description: String,
}

#[derive(Deserialize)]
struct PostMessageBody {
    session_id: String,
    message: String,
    message_type: Option<String>,
}

#[derive(Deserialize)]
struct MessageQuery {
    since: Option<i64>,
}

#[derive(Deserialize)]
struct EventsQuery {
    since: Option<i64>,
}

// ---------------------------------------------------------------------------
// Auth middleware
// ---------------------------------------------------------------------------

async fn auth_layer(
    State(state): State<Arc<AppState>>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> impl IntoResponse {
    if let Some(ref expected) = state.secret {
        let provided = req
            .headers()
            .get("X-Relay-Secret")
            .and_then(|v| v.to_str().ok());
        if provided != Some(expected.as_str()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    next.run(req).await.into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

async fn register_session(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<RegisterBody>,
) -> StatusCode {
    let now = chrono::Utc::now().timestamp();
    let mut projects = state.projects.write().await;
    let project = projects.entry(project_id).or_insert_with(ProjectState::new);
    project.sessions.insert(
        session_id.clone(),
        SessionData {
            session_id,
            branch: body.branch,
            description: body.description,
            started_at: now,
            last_heartbeat: now,
        },
    );
    StatusCode::OK
}

async fn deregister_session(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id)): Path<(String, String)>,
) -> StatusCode {
    let mut projects = state.projects.write().await;
    if let Some(project) = projects.get_mut(&project_id) {
        project.sessions.remove(&session_id);
        project.messages.retain(|m| m.session_id != session_id);
        if project.sessions.is_empty() && project.messages.is_empty() {
            projects.remove(&project_id);
        }
    }
    StatusCode::OK
}

async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id)): Path<(String, String)>,
) -> StatusCode {
    let now = chrono::Utc::now().timestamp();
    let mut projects = state.projects.write().await;
    if let Some(project) = projects.get_mut(&project_id) {
        if let Some(session) = project.sessions.get_mut(&session_id) {
            session.last_heartbeat = now;
            return StatusCode::OK;
        }
    }
    StatusCode::NOT_FOUND
}

async fn update_description(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<DescriptionBody>,
) -> StatusCode {
    let mut projects = state.projects.write().await;
    if let Some(project) = projects.get_mut(&project_id) {
        if let Some(session) = project.sessions.get_mut(&session_id) {
            session.description = Some(body.description);
            return StatusCode::OK;
        }
    }
    StatusCode::NOT_FOUND
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> axum::Json<Vec<SessionData>> {
    let now = chrono::Utc::now().timestamp();
    let projects = state.projects.read().await;
    let sessions = projects
        .get(&project_id)
        .map(|p| {
            p.sessions
                .values()
                .filter(|s| now - s.last_heartbeat < 300)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    axum::Json(sessions)
}

async fn post_message(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    axum::Json(body): axum::Json<PostMessageBody>,
) -> StatusCode {
    let now = chrono::Utc::now().timestamp();
    let max = state.max_messages;
    let mut projects = state.projects.write().await;
    let project = projects.entry(project_id).or_insert_with(ProjectState::new);
    let id = project.next_msg_id;
    project.next_msg_id += 1;
    let msg = MessageData {
        id,
        session_id: body.session_id,
        message: body.message,
        message_type: body.message_type.unwrap_or_else(|| "info".to_string()),
        created_at: now,
    };
    // Broadcast to SSE subscribers (best-effort, ignore if no receivers).
    let _ = project.msg_broadcast.send(msg.clone());
    project.messages.push_back(msg);
    while project.messages.len() > max {
        project.messages.pop_front();
    }
    StatusCode::CREATED
}

async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(query): Query<MessageQuery>,
) -> axum::Json<Vec<MessageData>> {
    let projects = state.projects.read().await;
    let messages = projects
        .get(&project_id)
        .map(|p| {
            let since = query.since.unwrap_or(0);
            p.messages
                .iter()
                .filter(|m| m.id > since)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    axum::Json(messages)
}

// ---------------------------------------------------------------------------
// SSE event stream
// ---------------------------------------------------------------------------

async fn events_stream(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let since = query.since.unwrap_or(0);

    // Subscribe to the broadcast *before* reading the backfill so we don't
    // miss messages that arrive between backfill read and subscribe.
    let (backfill, mut rx) = {
        let mut projects = state.projects.write().await;
        let project = projects.entry(project_id).or_insert_with(ProjectState::new);
        let rx = project.msg_broadcast.subscribe();
        let backfill: Vec<MessageData> = project
            .messages
            .iter()
            .filter(|m| m.id > since)
            .cloned()
            .collect();
        (backfill, rx)
    };

    let stream = async_stream::stream! {
        // Phase 1: emit backfill messages.
        let mut last_id = since;
        for msg in backfill {
            if msg.id > last_id {
                last_id = msg.id;
            }
            if let Ok(json) = serde_json::to_string(&msg) {
                yield Ok(SseEvent::default().data(json).event("message"));
            }
        }

        // Phase 2: stream live messages from the broadcast channel.
        // Also send keepalive pings every 15 seconds.
        let mut keepalive = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(msg) => {
                            // Skip messages already covered by backfill.
                            if msg.id <= last_id {
                                continue;
                            }
                            last_id = msg.id;
                            if let Ok(json) = serde_json::to_string(&msg) {
                                yield Ok(SseEvent::default().data(json).event("message"));
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("SSE subscriber lagged by {n} messages");
                            // Continue — we'll just miss some messages.
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = keepalive.tick() => {
                    yield Ok(SseEvent::default().comment("keepalive"));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

// ---------------------------------------------------------------------------
// Pruning background task
// ---------------------------------------------------------------------------

async fn prune_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let now = chrono::Utc::now().timestamp();
        let stale_threshold = now - 300; // 5 minutes
        let msg_max_age = now - 86_400; // 24 hours

        let mut projects = state.projects.write().await;
        let mut empty_projects = Vec::new();

        for (pid, project) in projects.iter_mut() {
            // Remove stale sessions.
            project
                .sessions
                .retain(|_, s| s.last_heartbeat > stale_threshold);

            // Remove old messages.
            project.messages.retain(|m| m.created_at > msg_max_age);

            if project.sessions.is_empty() && project.messages.is_empty() {
                empty_projects.push(pid.clone());
            }
        }

        for pid in empty_projects {
            projects.remove(&pid);
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("shutting down relay server");
}
