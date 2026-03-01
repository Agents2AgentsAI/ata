//! JSON-RPC codec implementing Content-Length framing for LSP communication.

use bytes::{BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::codec::{Decoder, Encoder};

use crate::error::LspError;

// ---------------------------------------------------------------------------
// JSON-RPC message types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: i64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A parsed JSON-RPC message from the LSP server.
#[derive(Debug, Clone)]
pub enum JsonRpcMessage {
    Response(JsonRpcResponse),
    Request(JsonRpcRequest),
    Notification(JsonRpcNotification),
}

// ---------------------------------------------------------------------------
// Content-Length framed codec
// ---------------------------------------------------------------------------

/// Codec that encodes/decodes LSP messages using `Content-Length` framing.
#[derive(Debug, Default)]
pub struct LspCodec {
    /// Expected body length parsed from the header, if available.
    content_length: Option<usize>,
}

impl LspCodec {
    pub fn new() -> Self {
        Self::default()
    }
}

const HEADER_SEPARATOR: &[u8] = b"\r\n\r\n";

impl Decoder for LspCodec {
    type Item = JsonRpcMessage;
    type Error = LspError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            if let Some(len) = self.content_length {
                // We already parsed the header — wait for the full body.
                if src.len() < len {
                    return Ok(None);
                }
                let body = src.split_to(len);
                self.content_length = None;

                let raw: Value = serde_json::from_slice(&body)?;
                let msg = classify_message(raw)?;
                return Ok(Some(msg));
            }

            // Look for the header/body separator.
            let sep_pos = src
                .windows(HEADER_SEPARATOR.len())
                .position(|w| w == HEADER_SEPARATOR);

            let Some(sep_pos) = sep_pos else {
                return Ok(None);
            };

            let header_bytes = &src[..sep_pos];
            let header_str =
                std::str::from_utf8(header_bytes).map_err(|_| LspError::InvalidHeader)?;

            let mut content_length = None;
            for line in header_str.split("\r\n") {
                if let Some(val) = line.strip_prefix("Content-Length: ") {
                    content_length =
                        Some(val.trim().parse::<usize>().map_err(|_| LspError::InvalidHeader)?);
                }
            }

            let len = content_length.ok_or(LspError::InvalidHeader)?;

            // Consume the header + separator.
            let _ = src.split_to(sep_pos + HEADER_SEPARATOR.len());
            self.content_length = Some(len);
            // Loop back to try reading the body.
        }
    }
}

impl Encoder<Vec<u8>> for LspCodec {
    type Error = LspError;

    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let header = format!("Content-Length: {}\r\n\r\n", item.len());
        dst.reserve(header.len() + item.len());
        dst.put_slice(header.as_bytes());
        dst.put_slice(&item);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn classify_message(raw: Value) -> Result<JsonRpcMessage, LspError> {
    // A response has an `id` and either `result` or `error`, but no `method`.
    if raw.get("id").is_some() && raw.get("method").is_none() {
        let resp: JsonRpcResponse = serde_json::from_value(raw)?;
        return Ok(JsonRpcMessage::Response(resp));
    }
    // A request has both `id` and `method`.
    if raw.get("id").is_some() && raw.get("method").is_some() {
        let req: JsonRpcRequest = serde_json::from_value(raw)?;
        return Ok(JsonRpcMessage::Request(req));
    }
    // A notification has `method` but no `id`.
    if raw.get("method").is_some() {
        let notif: JsonRpcNotification = serde_json::from_value(raw)?;
        return Ok(JsonRpcMessage::Notification(notif));
    }
    // Fallback: treat as response (server may send weird things).
    let resp: JsonRpcResponse = serde_json::from_value(raw)?;
    Ok(JsonRpcMessage::Response(resp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let mut codec = LspCodec::new();
        let body = br#"{"jsonrpc":"2.0","id":1,"result":null}"#.to_vec();

        let mut buf = BytesMut::new();
        codec.encode(body, &mut buf).expect("encode");

        let msg = codec.decode(&mut buf).expect("decode").expect("Some");
        match msg {
            JsonRpcMessage::Response(resp) => {
                assert_eq!(resp.id, Some(1));
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[test]
    fn decode_partial_header() {
        let mut codec = LspCodec::new();
        let mut buf = BytesMut::from("Content-Length: 2\r\n");
        assert!(codec.decode(&mut buf).expect("ok").is_none());
    }

    #[test]
    fn decode_partial_body() {
        let mut codec = LspCodec::new();
        let mut buf = BytesMut::from("Content-Length: 100\r\n\r\n{\"jsonrpc\"");
        assert!(codec.decode(&mut buf).expect("ok").is_none());
    }

    #[test]
    fn classify_request() {
        let raw: Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":5,"method":"window/workDoneProgress/create","params":{}}"#)
                .expect("parse");
        let msg = classify_message(raw).expect("classify");
        assert!(matches!(msg, JsonRpcMessage::Request(_)));
    }

    #[test]
    fn classify_notification() {
        let raw: Value =
            serde_json::from_str(r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{}}"#)
                .expect("parse");
        let msg = classify_message(raw).expect("classify");
        assert!(matches!(msg, JsonRpcMessage::Notification(_)));
    }
}
