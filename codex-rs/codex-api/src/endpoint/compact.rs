use crate::auth::SharedAuthProvider;
use crate::common::CompactionInput;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::file_support::rewrite_openai_url_file_blocks_in_payload;
use crate::provider::Provider;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::models::ResponseItem;
use http::HeaderMap;
use http::Method;
use serde::Deserialize;
use serde_json::to_value;
use std::sync::Arc;

pub struct CompactClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

impl<T: HttpTransport> CompactClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
        }
    }

    fn path() -> &'static str {
        "responses/compact"
    }

    pub async fn compact(
        &self,
        body: serde_json::Value,
        extra_headers: HeaderMap,
    ) -> Result<Vec<ResponseItem>, ApiError> {
        let resp = self
            .session
            .execute(Method::POST, Self::path(), extra_headers, Some(body))
            .await?;
        let parsed: CompactHistoryResponse =
            serde_json::from_slice(&resp.body).map_err(|e| ApiError::Stream(e.to_string()))?;
        Ok(parsed.output)
    }

    pub async fn compact_input(
        &self,
        input: &CompactionInput<'_>,
        extra_headers: HeaderMap,
    ) -> Result<Vec<ResponseItem>, ApiError> {
        let mut body = to_value(input)
            .map_err(|e| ApiError::Stream(format!("failed to encode compaction input: {e}")))?;
        rewrite_openai_url_file_blocks_in_payload(&mut body);
        self.compact(body, extra_headers).await
    }
}

#[derive(Debug, Deserialize)]
struct CompactHistoryResponse {
    output: Vec<ResponseItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthProvider;
    use crate::provider::RetryConfig;
    use async_trait::async_trait;
    use codex_client::Request;
    use codex_client::RequestBody;
    use codex_client::Response;
    use codex_client::StreamResponse;
    use codex_client::TransportError;
    use codex_protocol::models::ContentItem;
    use std::time::Duration;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct DummyTransport;

    #[async_trait]
    impl HttpTransport for DummyTransport {
        async fn execute(&self, _req: Request) -> Result<Response, TransportError> {
            Err(TransportError::Build("execute should not run".to_string()))
        }

        async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
            Err(TransportError::Build("stream should not run".to_string()))
        }
    }

    #[test]
    fn path_is_responses_compact() {
        assert_eq!(CompactClient::<DummyTransport>::path(), "responses/compact");
    }

    #[derive(Clone)]
    struct CapturingTransport {
        captured: Arc<Mutex<Option<serde_json::Value>>>,
    }

    #[async_trait]
    impl HttpTransport for CapturingTransport {
        async fn execute(&self, req: Request) -> Result<Response, TransportError> {
            if let Some(body) = req.body.and_then(|body| match body {
                RequestBody::Json(value) => Some(value),
                RequestBody::Raw(_) => None,
            }) {
                *self.captured.lock().await = Some(body);
            }
            let resp_body = serde_json::to_vec(&serde_json::json!({ "output": [] })).unwrap();
            Ok(Response {
                status: http::StatusCode::OK,
                headers: HeaderMap::new(),
                body: resp_body.into(),
            })
        }

        async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
            Err(TransportError::Build("stream should not run".to_string()))
        }
    }

    #[derive(Clone, Default)]
    struct DummyAuth;

    impl AuthProvider for DummyAuth {
        fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
    }

    fn test_provider() -> Provider {
        Provider {
            name: "test".to_string(),
            base_url: "http://localhost".to_string(),
            query_params: None,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: false,
                retry_transport: false,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn compact_input_rewrites_url_file_blocks() {
        let captured = Arc::new(Mutex::new(None));
        let transport = CapturingTransport {
            captured: Arc::clone(&captured),
        };
        let client = CompactClient::new(transport, test_provider(), Arc::new(DummyAuth));

        let items = vec![ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::UrlFile {
                url: "https://example.com/report.pdf".to_string(),
                mime_type: Some("application/pdf".to_string()),
                filename: Some("report.pdf".to_string()),
            }],
            phase: None,
        }];
        let input = CompactionInput {
            model: "gpt-4o",
            input: &items,
            instructions: "summarize",
            tools: Vec::new(),
            parallel_tool_calls: false,
            reasoning: None,
            service_tier: None,
            prompt_cache_key: None,
            text: None,
        };

        client
            .compact_input(&input, HeaderMap::new())
            .await
            .expect("compact request should succeed");

        let body = captured
            .lock()
            .await
            .take()
            .expect("request body should be captured");
        assert_eq!(
            body["input"][0]["content"][0],
            serde_json::json!({
                "type": "input_file",
                "file_url": "https://example.com/report.pdf"
            })
        );
    }
}
