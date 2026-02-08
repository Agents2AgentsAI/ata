use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use reqwest::Response;
use reqwest::header;
use serde::de::DeserializeOwned;

use crate::config::RetryConfig;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::RateLimiter;
use crate::rate_limiter::ResearchApi;

#[derive(Debug)]
pub struct HttpClient {
    inner: reqwest::Client,
    rate_limiter: Arc<RateLimiter>,
    retry_config: RetryConfig,
    request_timeout: Duration,
}

impl HttpClient {
    #[must_use]
    pub fn new(
        inner: reqwest::Client,
        rate_limiter: Arc<RateLimiter>,
        retry_config: RetryConfig,
        request_timeout: Duration,
    ) -> Self {
        Self {
            inner,
            rate_limiter,
            retry_config,
            request_timeout,
        }
    }

    #[must_use]
    pub fn client(&self) -> &reqwest::Client {
        &self.inner
    }

    pub async fn execute_json<T, F>(&self, api: ResearchApi, build_request: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::RequestBuilder,
    {
        let response = self.execute_response(api, build_request).await?;
        response.json().await.map_err(|err| ResearchError::Parse {
            api,
            message: err.to_string(),
        })
    }

    pub async fn execute_text<F>(&self, api: ResearchApi, build_request: F) -> Result<String>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let response = self.execute_response(api, build_request).await?;
        response.text().await.map_err(|err| ResearchError::Parse {
            api,
            message: err.to_string(),
        })
    }

    async fn execute_response<F>(&self, api: ResearchApi, build_request: F) -> Result<Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        for attempt in 0..=self.retry_config.max_retries {
            let _permit = self.rate_limiter.acquire(api).await?;
            let request = build_request();
            let response = request.send().await;

            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(resp);
                    }

                    let status = resp.status();
                    if should_retry_status(status) && attempt < self.retry_config.max_retries {
                        let retry_after = parse_retry_after(resp.headers());
                        tokio::time::sleep(self.retry_delay(attempt, retry_after)).await;
                        continue;
                    }

                    let message = match resp.text().await {
                        Ok(text) => text,
                        Err(err) => err.to_string(),
                    };
                    return Err(ResearchError::Upstream {
                        api,
                        status,
                        message: truncate_error_body(&message),
                    });
                }
                Err(err) => {
                    if should_retry_error(&err) && attempt < self.retry_config.max_retries {
                        tokio::time::sleep(self.retry_delay(attempt, None)).await;
                        continue;
                    }

                    if err.is_timeout() {
                        return Err(ResearchError::Timeout {
                            api,
                            timeout_ms: self.request_timeout.as_millis() as u64,
                        });
                    }

                    return Err(ResearchError::Http { api, source: err });
                }
            }
        }

        Err(ResearchError::Internal(format!(
            "retry loop exhausted unexpectedly for {api}"
        )))
    }

    fn retry_delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(delay) = retry_after {
            return delay.min(self.retry_config.max_delay);
        }

        let exp = 2_u32.saturating_pow(attempt);
        let base = self
            .retry_config
            .base_delay
            .checked_mul(exp)
            .unwrap_or(self.retry_config.max_delay)
            .min(self.retry_config.max_delay);

        let max_jitter_ms = (base.as_millis() as u64 / 2).max(1);
        let mut rng = rand::rng();
        let jitter_ms = rng.random_range(0..=max_jitter_ms);
        base + Duration::from_millis(jitter_ms)
    }
}

fn should_retry_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn should_retry_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn parse_retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?;
    let raw = value.to_str().ok()?;
    let seconds = raw.parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}

fn truncate_error_body(body: &str) -> String {
    const MAX: usize = 512;
    if body.len() <= MAX {
        return body.to_string();
    }

    let mut end = MAX;
    while !body.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }

    format!("{}...", &body[..end])
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    use super::*;
    use crate::rate_limiter::ResearchApi;

    #[test]
    fn truncate_error_body_respects_utf8_boundaries() {
        let body = format!("{}étrès long", "a".repeat(511));
        let truncated = truncate_error_body(&body);
        assert!(truncated.ends_with("..."));
        assert_eq!(truncated.as_bytes()[510], b'a');
        assert_eq!(truncated.as_bytes()[511], b'.');
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_error_uses_request_timeout_not_retry_delay_cap() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .mount(&server)
            .await;

        let request_timeout = Duration::from_millis(25);
        let client = reqwest::Client::builder()
            .timeout(request_timeout)
            .build()
            .expect("reqwest client");
        let rate_limiter = Arc::new(RateLimiter::new(HashMap::new()));
        let http = HttpClient::new(
            client,
            rate_limiter,
            RetryConfig {
                max_retries: 0,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_secs(30),
            },
            request_timeout,
        );

        let err = http
            .execute_text(ResearchApi::OpenAlex, || {
                http.client().get(format!("{}/slow", server.uri()))
            })
            .await
            .expect_err("request should time out");

        match err {
            ResearchError::Timeout { api, timeout_ms } => {
                assert_eq!(api, ResearchApi::OpenAlex);
                assert_eq!(timeout_ms, 25);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
