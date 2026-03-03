//! Integration tests using a fake LSP server (Python script).
//!
//! These tests verify the full client lifecycle:
//! spawn → initialize → document sync → diagnostics → queries → shutdown.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use codex_lsp_client::LspClient;
use codex_lsp_client::ServerRegistry;
use codex_lsp_client::server_config::LspServerConfig;
use tempfile::TempDir;

/// Inline Python script implementing a minimal LSP server.
///
/// Supports: initialize, initialized, textDocument/didOpen (publishes diagnostics),
/// textDocument/hover, workspace/didChangeWatchedFiles, shutdown, exit,
/// and server-initiated requests (window/workDoneProgress/create, etc.).
const FAKE_LSP_PY: &str = r#"
import sys, json

def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        decoded = line.decode('ascii')
        if decoded == '\r\n':
            break
        if ': ' in decoded:
            key, value = decoded.strip().split(': ', 1)
            headers[key] = value
    if 'Content-Length' not in headers:
        return None
    length = int(headers['Content-Length'])
    body = sys.stdin.buffer.read(length)
    return json.loads(body)

def send_message(msg):
    body = json.dumps(msg).encode('utf-8')
    header = f'Content-Length: {len(body)}\r\n\r\n'.encode('ascii')
    sys.stdout.buffer.write(header)
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

def handle_request(msg):
    method = msg.get('method', '')
    req_id = msg.get('id')

    if method == 'initialize':
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': {
                'capabilities': {
                    'textDocumentSync': 1,
                    'hoverProvider': True,
                    'definitionProvider': True,
                }
            }
        })
    elif method == 'shutdown':
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': None
        })
    elif method == 'textDocument/hover':
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': {
                'contents': {
                    'kind': 'markdown',
                    'value': '```rust\nfn hello()\n```\nHello function docs.'
                }
            }
        })
    elif method == 'textDocument/definition':
        params = msg.get('params', {})
        uri = params.get('textDocumentPosition', params.get('textDocument', {})).get('uri', '')
        if not uri:
            uri = params.get('textDocument', {}).get('uri', '')
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': {
                'uri': uri,
                'range': {
                    'start': {'line': 0, 'character': 0},
                    'end': {'line': 0, 'character': 5}
                }
            }
        })
    elif method == 'workspace/symbol':
        params = msg.get('params', {})
        query = params.get('query', '')
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': [
                {
                    'name': f'{query}_symbol',
                    'kind': 12,
                    'location': {
                        'uri': 'file:///test.rs',
                        'range': {
                            'start': {'line': 0, 'character': 0},
                            'end': {'line': 0, 'character': 1}
                        }
                    }
                }
            ]
        })
    else:
        # Unknown request — send empty result.
        send_message({
            'jsonrpc': '2.0',
            'id': req_id,
            'result': None
        })

def handle_notification(msg):
    method = msg.get('method', '')

    if method == 'textDocument/didOpen':
        uri = msg['params']['textDocument']['uri']
        # Publish diagnostics with one error.
        send_message({
            'jsonrpc': '2.0',
            'method': 'textDocument/publishDiagnostics',
            'params': {
                'uri': uri,
                'diagnostics': [
                    {
                        'range': {
                            'start': {'line': 0, 'character': 0},
                            'end': {'line': 0, 'character': 5}
                        },
                        'severity': 1,
                        'message': 'fake error: type mismatch'
                    },
                    {
                        'range': {
                            'start': {'line': 2, 'character': 4},
                            'end': {'line': 2, 'character': 10}
                        },
                        'severity': 2,
                        'message': 'fake warning: unused variable'
                    }
                ]
            }
        })
    elif method == 'exit':
        sys.exit(0)
    # All other notifications (initialized, didChange, etc.) are silently accepted.

while True:
    msg = read_message()
    if msg is None:
        break

    if 'id' in msg:
        handle_request(msg)
    else:
        handle_notification(msg)
"#;

/// Write the fake LSP server script to a temp file and return its path.
fn write_fake_server(dir: &TempDir) -> PathBuf {
    let script_path = dir.path().join("fake_lsp.py");
    let mut f = std::fs::File::create(&script_path).expect("create script");
    f.write_all(FAKE_LSP_PY.as_bytes()).expect("write script");
    script_path
}

/// Create an LspServerConfig pointing to the fake server.
fn fake_config(script: &PathBuf) -> LspServerConfig {
    LspServerConfig {
        extensions: vec![".rs".into()],
        command: vec!["python3".into(), script.to_string_lossy().to_string()],
        command_candidates: Vec::new(),
        env: HashMap::new(),
        root_markers: vec!["Cargo.toml".into()],
        initialization_options: None,
        disabled: false,
        install: None,
        root_strategy: Default::default(),
        post_root_hook: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawn_and_initialize() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    // Write a Cargo.toml marker so root discovery works.
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

    let client = LspClient::create("fake", &config, root).await;
    assert!(client.is_ok(), "should initialize: {:?}", client.err());

    let client = client.unwrap();
    client.shutdown().await;
}

#[tokio::test]
async fn diagnostics_published_after_did_open() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("src/main.rs");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(&file_path, "fn main() {}\n").unwrap();

    let client = LspClient::create("fake", &config, root)
        .await
        .expect("spawn");

    // Sync file and wait for diagnostics.
    client.notify_open(&file_path).await.expect("notify_open");
    let diags = client.wait_for_diagnostics(&file_path).await;

    // The fake server publishes 2 diagnostics (1 error + 1 warning).
    assert_eq!(
        diags.len(),
        2,
        "expected 2 diagnostics, got {}",
        diags.len()
    );
    assert_eq!(diags[0].message, "fake error: type mismatch");
    assert_eq!(
        diags[0].severity,
        Some(codex_lsp_client::lsp_types::DiagnosticSeverity::ERROR)
    );
    assert_eq!(diags[1].message, "fake warning: unused variable");
    assert_eq!(
        diags[1].severity,
        Some(codex_lsp_client::lsp_types::DiagnosticSeverity::WARNING)
    );

    client.shutdown().await;
}

#[tokio::test]
async fn hover_returns_result() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("main.rs");
    std::fs::write(&file_path, "fn hello() {}\n").unwrap();

    let client = LspClient::create("fake", &config, root)
        .await
        .expect("spawn");

    client.notify_open(&file_path).await.expect("notify_open");
    let hover = client.hover(&file_path, 0, 3).await;

    assert!(hover.is_some(), "expected hover result");
    let hover = hover.unwrap();
    match hover.contents {
        codex_lsp_client::lsp_types::HoverContents::Markup(markup) => {
            assert!(
                markup.value.contains("fn hello()"),
                "hover should contain function signature: {}",
                markup.value
            );
        }
        other => panic!("expected Markup hover contents, got: {:?}", other),
    }

    client.shutdown().await;
}

#[tokio::test]
async fn server_registry_touch_file_collects_diagnostics() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("lib.rs");
    std::fs::write(&file_path, "pub fn lib_fn() {}\n").unwrap();

    let mut servers = HashMap::new();
    servers.insert("fake".to_string(), config);
    let registry = Arc::new(ServerRegistry::new(servers, root.to_path_buf(), None));

    // touch_file with wait=true should return diagnostics.
    let diags = registry.touch_file(&file_path, true).await;
    assert!(!diags.is_empty(), "expected diagnostics from touch_file");
    let server_diags = diags.get("fake").expect("expected 'fake' server entry");
    assert_eq!(server_diags.len(), 2);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn server_registry_ignores_non_matching_extensions() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script); // .rs only
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("readme.txt");
    std::fs::write(&file_path, "hello").unwrap();

    let mut servers = HashMap::new();
    servers.insert("fake".to_string(), config);
    let registry = Arc::new(ServerRegistry::new(servers, root.to_path_buf(), None));

    // .txt doesn't match .rs — no clients spawned, no diagnostics.
    let diags = registry.touch_file(&file_path, true).await;
    assert!(diags.is_empty(), "no diagnostics expected for .txt file");

    registry.shutdown_all().await;
}

#[tokio::test]
async fn server_registry_disabled_server_skipped() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let mut config = fake_config(&script);
    config.disabled = true;
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("main.rs");
    std::fs::write(&file_path, "fn main() {}").unwrap();

    let mut servers = HashMap::new();
    servers.insert("fake".to_string(), config);
    let registry = Arc::new(ServerRegistry::new(servers, root.to_path_buf(), None));

    let diags = registry.touch_file(&file_path, true).await;
    assert!(
        diags.is_empty(),
        "disabled server should not produce diagnostics"
    );

    registry.shutdown_all().await;
}

#[tokio::test]
async fn did_change_on_second_sync() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
    let file_path = root.join("main.rs");
    std::fs::write(&file_path, "fn main() {}\n").unwrap();

    let client = LspClient::create("fake", &config, root)
        .await
        .expect("spawn");

    // First sync — didOpen.
    client.notify_open(&file_path).await.expect("first open");
    let diags1 = client.wait_for_diagnostics(&file_path).await;
    assert_eq!(diags1.len(), 2);

    // Modify the file and sync again — didChange.
    std::fs::write(&file_path, "fn main() { println!(\"hi\"); }\n").unwrap();
    client.notify_open(&file_path).await.expect("second open");

    // The fake server only publishes diagnostics on didOpen, not didChange,
    // so we still see the same diagnostics from the first sync.
    let diags2 = client.diagnostics_for(&file_path).await;
    assert_eq!(
        diags2.len(),
        2,
        "diagnostics should persist after didChange"
    );

    client.shutdown().await;
}

#[tokio::test]
async fn workspace_symbol_autospawns_clients() {
    let dir = TempDir::new().unwrap();
    let script = write_fake_server(&dir);
    let config = fake_config(&script);
    let root = dir.path();

    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

    let mut servers = HashMap::new();
    servers.insert("fake".to_string(), config);
    let registry = Arc::new(ServerRegistry::new(servers, root.to_path_buf(), None));

    let symbols = registry.workspace_symbol("hello").await;
    assert!(
        symbols.iter().any(|s| s.name == "hello_symbol"),
        "expected 'hello_symbol' from workspace/symbol response, got: {symbols:?}",
        symbols = symbols
    );

    registry.shutdown_all().await;
}
