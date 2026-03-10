//! Embedded WebSocket server for remote-control mode.
//!
//! This module provides [`run_embedded_websocket`], which starts a WebSocket
//! endpoint that shares a pre-existing [`ThreadManager`] with the host process
//! (e.g. the TUI). Remote clients (such as the ATA-Swift mobile app) connect
//! over this WebSocket and interact with the same threads via the standard
//! app-server JSON-RPC protocol.
//!
//! All new code lives here to minimise the conflict surface with upstream.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Result as IoResult;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicBool;

use codex_app_server_protocol::JSONRPCMessage;
use codex_arg0::Arg0DispatchPaths;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_core::config_loader::CloudRequirementsLoader;
use codex_core::config_loader::LoaderOverrides;
use codex_core::features::Feature;
use codex_feedback::CodexFeedback;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use toml::Value as TomlValue;
use tracing::info;
use tracing::warn;

use crate::OutboundControlEvent;
use crate::message_processor::MessageProcessor;
use crate::message_processor::MessageProcessorArgs;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingEnvelope;
use crate::outgoing_message::OutgoingMessageSender;
use crate::transport::AppServerTransport;
use crate::transport::CHANNEL_CAPACITY;
use crate::transport::ConnectionState;
use crate::transport::OutboundConnectionState;
use crate::transport::TransportEvent;
use crate::transport::route_outgoing_envelope;
use crate::transport::start_websocket_acceptor_with_auth;

/// Configuration for running an embedded WebSocket server that shares a
/// pre-existing [`ThreadManager`] with the host process.
pub struct EmbeddedWebSocketConfig {
    pub bind_address: SocketAddr,
    pub thread_manager: Arc<ThreadManager>,
    pub config: Arc<Config>,
    pub shutdown: CancellationToken,

    pub arg0_paths: Arg0DispatchPaths,
    pub cli_overrides: Vec<(String, TomlValue)>,
    pub loader_overrides: LoaderOverrides,
    pub cloud_requirements: CloudRequirementsLoader,
    pub feedback: CodexFeedback,

    /// Optional token that must be supplied by WebSocket clients as a
    /// `?token=<value>` query-string parameter on the upgrade request.
    /// When `Some`, connections without a valid token are rejected with 401.
    pub auth_token: Option<String>,
}

/// Extend `MessageProcessor` with an alternative constructor that accepts an
/// external `ThreadManager` rather than creating its own.
///
/// This `impl` block lives in a separate file so we do not modify
/// `message_processor.rs` (which upstream changes frequently).
impl MessageProcessor {
    pub(crate) fn new_with_thread_manager(
        args: MessageProcessorArgs,
        thread_manager: Arc<ThreadManager>,
    ) -> Self {
        // Build a standard processor (creates its own ThreadManager), then
        // swap in the shared one. Reusing `new()` means we pick up any new
        // initialisation steps that upstream adds automatically.
        let mut processor = Self::new(args);
        processor.set_thread_manager(thread_manager);
        processor
    }
}

/// Run a WebSocket server that shares a [`ThreadManager`] owned by the caller.
///
/// This is intended for the "embedded remote-control" use-case where the TUI
/// creates a `ThreadManager` and then starts a WebSocket endpoint so remote
/// clients can interact with the same set of threads.
///
/// Returns when `shutdown` is cancelled or all channels close.
pub async fn run_embedded_websocket(ws_config: EmbeddedWebSocketConfig) -> IoResult<()> {
    let EmbeddedWebSocketConfig {
        bind_address,
        thread_manager,
        config,
        shutdown,
        arg0_paths,
        cli_overrides,
        loader_overrides,
        cloud_requirements,
        feedback,
        auth_token,
    } = ws_config;

    let (transport_event_tx, mut transport_event_rx) =
        mpsc::channel::<TransportEvent>(CHANNEL_CAPACITY);
    let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<OutgoingEnvelope>(CHANNEL_CAPACITY);
    let (outbound_control_tx, mut outbound_control_rx) =
        mpsc::channel::<OutboundControlEvent>(CHANNEL_CAPACITY);

    let accept_shutdown = shutdown.clone();
    let accept_handle = start_websocket_acceptor_with_auth(
        bind_address,
        transport_event_tx.clone(),
        accept_shutdown.clone(),
        auth_token,
    )
    .await?;

    info!("embedded remote-control websocket listening on ws://{bind_address}");

    // --- outbound router (routes messages to per-connection writers) ---
    let outbound_handle = tokio::spawn(async move {
        let mut outbound_connections = HashMap::<ConnectionId, OutboundConnectionState>::new();
        loop {
            tokio::select! {
                biased;
                event = outbound_control_rx.recv() => {
                    let Some(event) = event else { break };
                    match event {
                        OutboundControlEvent::Opened {
                            connection_id, writer, disconnect_sender,
                            initialized, experimental_api_enabled,
                            opted_out_notification_methods,
                        } => {
                            outbound_connections.insert(
                                connection_id,
                                OutboundConnectionState::new(
                                    writer, initialized, experimental_api_enabled,
                                    opted_out_notification_methods, disconnect_sender,
                                ),
                            );
                        }
                        OutboundControlEvent::Closed { connection_id } => {
                            outbound_connections.remove(&connection_id);
                        }
                        OutboundControlEvent::DisconnectAll => {
                            for state in outbound_connections.values() {
                                state.request_disconnect();
                            }
                            outbound_connections.clear();
                        }
                    }
                }
                envelope = outgoing_rx.recv() => {
                    let Some(envelope) = envelope else { break };
                    route_outgoing_envelope(&mut outbound_connections, envelope).await;
                }
            }
        }
    });

    // --- state DB for the processor ---
    let log_db = if config.features.enabled(Feature::Sqlite) {
        codex_state::StateRuntime::init(
            config.sqlite_home.clone(),
            config.model_provider_id.clone(),
            None,
        )
        .await
        .ok()
        .map(codex_state::log_db::start)
    } else {
        None
    };

    // --- processor loop (shared ThreadManager) ---
    let processor_handle = tokio::spawn({
        let outgoing_message_sender = Arc::new(OutgoingMessageSender::new(outgoing_tx));
        let mut processor = MessageProcessor::new_with_thread_manager(
            MessageProcessorArgs {
                outgoing: outgoing_message_sender,
                arg0_paths,
                config,
                cli_overrides,
                loader_overrides,
                cloud_requirements,
                feedback,
                log_db,
                config_warnings: Vec::new(),
            },
            thread_manager,
        );
        let mut thread_created_rx = processor.thread_created_receiver();
        let shutdown_token = shutdown.clone();
        async move {
            let mut listen_for_threads = true;
            let mut connections = HashMap::<ConnectionId, ConnectionState>::new();
            loop {
                tokio::select! {
                    _ = shutdown_token.cancelled() => {
                        let _ = outbound_control_tx
                            .send(OutboundControlEvent::DisconnectAll)
                            .await;
                        break;
                    }
                    event = transport_event_rx.recv() => {
                        let Some(event) = event else { break };
                        match event {
                            TransportEvent::ConnectionOpened {
                                connection_id, writer, disconnect_sender,
                            } => {
                                let outbound_initialized = Arc::new(AtomicBool::new(false));
                                let outbound_experimental_api_enabled =
                                    Arc::new(AtomicBool::new(false));
                                let outbound_opted_out_notification_methods =
                                    Arc::new(RwLock::new(HashSet::new()));
                                if outbound_control_tx
                                    .send(OutboundControlEvent::Opened {
                                        connection_id,
                                        writer,
                                        disconnect_sender,
                                        initialized: Arc::clone(&outbound_initialized),
                                        experimental_api_enabled: Arc::clone(
                                            &outbound_experimental_api_enabled,
                                        ),
                                        opted_out_notification_methods: Arc::clone(
                                            &outbound_opted_out_notification_methods,
                                        ),
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                connections.insert(
                                    connection_id,
                                    ConnectionState::new(
                                        outbound_initialized,
                                        outbound_experimental_api_enabled,
                                        outbound_opted_out_notification_methods,
                                    ),
                                );
                            }
                            TransportEvent::ConnectionClosed { connection_id } => {
                                if connections.remove(&connection_id).is_none() {
                                    continue;
                                }
                                if outbound_control_tx
                                    .send(OutboundControlEvent::Closed { connection_id })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                processor.connection_closed(connection_id).await;
                            }
                            TransportEvent::IncomingMessage { connection_id, message } => {
                                match message {
                                    JSONRPCMessage::Request(request) => {
                                        let Some(connection_state) =
                                            connections.get_mut(&connection_id)
                                        else {
                                            warn!(
                                                "dropping request from unknown connection: \
                                                 {connection_id:?}"
                                            );
                                            continue;
                                        };
                                        let was_initialized =
                                            connection_state.session.initialized;
                                        processor
                                            .process_request(
                                                connection_id,
                                                request,
                                                AppServerTransport::WebSocket {
                                                    bind_address,
                                                },
                                                &mut connection_state.session,
                                                &connection_state.outbound_initialized,
                                            )
                                            .await;
                                        if let Ok(mut opted_out) = connection_state
                                            .outbound_opted_out_notification_methods
                                            .write()
                                        {
                                            *opted_out = connection_state
                                                .session
                                                .opted_out_notification_methods
                                                .clone();
                                        }
                                        connection_state
                                            .outbound_experimental_api_enabled
                                            .store(
                                                connection_state
                                                    .session
                                                    .experimental_api_enabled,
                                                std::sync::atomic::Ordering::Release,
                                            );
                                        if !was_initialized
                                            && connection_state.session.initialized
                                        {
                                            processor.send_initialize_notifications().await;
                                        }
                                    }
                                    JSONRPCMessage::Response(response) => {
                                        if !connections.contains_key(&connection_id) {
                                            continue;
                                        }
                                        processor.process_response(response).await;
                                    }
                                    JSONRPCMessage::Notification(notification) => {
                                        if !connections.contains_key(&connection_id) {
                                            continue;
                                        }
                                        processor.process_notification(notification).await;
                                    }
                                    JSONRPCMessage::Error(err) => {
                                        if !connections.contains_key(&connection_id) {
                                            continue;
                                        }
                                        processor.process_error(err).await;
                                    }
                                }
                            }
                        }
                    }
                    created = thread_created_rx.recv(), if listen_for_threads => {
                        match created {
                            Ok(thread_id) => {
                                let initialized_ids: Vec<ConnectionId> = connections
                                    .iter()
                                    .filter_map(|(id, state)| {
                                        state.session.initialized.then_some(*id)
                                    })
                                    .collect();
                                processor
                                    .try_attach_thread_listener(thread_id, initialized_ids)
                                    .await;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                warn!("thread_created receiver lagged");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                listen_for_threads = false;
                            }
                        }
                    }
                }
            }
            info!("embedded processor task exited");
        }
    });

    drop(transport_event_tx);

    let _ = processor_handle.await;
    let _ = outbound_handle.await;

    accept_shutdown.cancel();
    let _ = accept_handle.await;

    Ok(())
}
