//! Integration test for the embedded WebSocket server with token auth.
//!
//! Starts the embedded server on a random port, then connects with
//! tokio-tungstenite to verify:
//! 1. Connections without a valid token are rejected
//! 2. Connections with a valid token succeed
//! 3. The initialize handshake works

use codex_app_server::EmbeddedWebSocketConfig;
use codex_app_server::run_embedded_websocket;
use codex_arg0::Arg0DispatchPaths;
use codex_core::AuthManager;
use codex_core::ThreadManager;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::config::ConfigBuilder;
use codex_core::models_manager::collaboration_mode_presets::CollaborationModesConfig;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use futures::SinkExt;
use futures::StreamExt;
use serde_json::json;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

/// Find a free port by binding to port 0.
async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind to free port");
    listener.local_addr().expect("local addr").port()
}

/// Start the embedded WebSocket server and return (port, shutdown_token).
async fn start_server(token: &str) -> (u16, CancellationToken) {
    let port = free_port().await;
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let shutdown = CancellationToken::new();

    // Build a minimal Config using the default builder.
    let config = ConfigBuilder::default()
        .build()
        .await
        .expect("build config");

    let auth_manager = Arc::new(AuthManager::new(
        config.codex_home.clone(),
        false,
        AuthCredentialsStoreMode::Ephemeral,
    ));

    let thread_manager = Arc::new(ThreadManager::new(
        config.codex_home.clone(),
        auth_manager,
        SessionSource::Exec,
        None,
        CollaborationModesConfig::default(),
    ));

    let ws_config = EmbeddedWebSocketConfig {
        bind_address: bind_addr,
        thread_manager,
        config: Arc::new(config),
        shutdown: shutdown.clone(),
        arg0_paths: Arg0DispatchPaths {
            codex_linux_sandbox_exe: None,
            main_execve_wrapper_exe: None,
        },
        cli_overrides: vec![],
        loader_overrides: codex_core::config_loader::LoaderOverrides::default(),
        cloud_requirements: codex_core::config_loader::CloudRequirementsLoader::default(),
        feedback: CodexFeedback::new(),
        auth_token: Some(token.to_string()),
    };

    tokio::spawn(async move {
        let _ = run_embedded_websocket(ws_config).await;
    });

    // Wait for the server to start listening.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(bind_addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    (port, shutdown)
}

#[tokio::test]
async fn test_valid_token_connects_and_initializes() {
    let token = "test-secret-token-12345";
    let (port, shutdown) = start_server(token).await;

    // Connect with valid token.
    let url = format!("ws://127.0.0.1:{port}/?token={token}");
    let result = timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await;

    match result {
        Ok(Ok((mut ws, _response))) => {
            // Send initialize request.
            let init_msg = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "clientInfo": {
                        "name": "test-client",
                        "version": "1.0.0"
                    },
                    "capabilities": null
                }
            });
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                init_msg.to_string().into(),
            ))
            .await
            .expect("send initialize");

            // Read response.
            let response = timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("timeout reading response")
                .expect("stream not ended")
                .expect("valid message");

            let text = response.to_text().expect("text message");
            let json: serde_json::Value = serde_json::from_str(text).expect("valid JSON response");

            // Verify it's a valid JSON-RPC response with id=1.
            assert_eq!(json["id"], 1, "response id should match request");
            assert!(
                json.get("result").is_some(),
                "response should have a result field: {json}"
            );

            println!("Initialize response: {json}");
            println!("TEST PASSED: Valid token connects and initializes successfully!");
        }
        Ok(Err(e)) => panic!("WebSocket connection failed: {e}"),
        Err(_) => panic!("Connection timed out"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_invalid_token_rejected() {
    let token = "correct-token";
    let (port, shutdown) = start_server(token).await;

    // Connect with wrong token.
    let url = format!("ws://127.0.0.1:{port}/?token=wrong-token");
    let result = timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await;

    match result {
        Ok(Ok(_)) => panic!("Should have been rejected with wrong token!"),
        Ok(Err(e)) => {
            let err_str = format!("{e}");
            println!("Connection correctly rejected: {err_str}");
            assert!(
                err_str.contains("401")
                    || err_str.contains("Unauthorized")
                    || err_str.contains("HTTP error"),
                "Expected 401 error, got: {err_str}"
            );
            println!("TEST PASSED: Invalid token correctly rejected!");
        }
        Err(_) => panic!("Connection timed out (expected rejection, not timeout)"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_no_token_rejected() {
    let token = "my-secret";
    let (port, shutdown) = start_server(token).await;

    // Connect without any token.
    let url = format!("ws://127.0.0.1:{port}/");
    let result = timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&url),
    )
    .await;

    match result {
        Ok(Ok(_)) => panic!("Should have been rejected without token!"),
        Ok(Err(e)) => {
            println!("Connection correctly rejected (no token): {e}");
            println!("TEST PASSED: No token correctly rejected!");
        }
        Err(_) => panic!("Connection timed out"),
    }

    shutdown.cancel();
}
