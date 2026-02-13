use std::sync::Arc;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use rand::Rng;
use reqwest::Response;
use reqwest::header;
use serde::de::DeserializeOwned;

use crate::config::RetryConfig;
use crate::error::ResearchError;
use crate::error::Result;
use crate::error::is_retryable_http_error;
use crate::error::is_retryable_upstream_status;
use crate::rate_limiter::RateLimiter;
use crate::rate_limiter::ResearchApi;

#[derive(Debug)]
pub struct HttpClient {
    inner: reqwest::Client,
    rate_limiter: Arc<RateLimiter>,
    retry_config: RetryConfig,
    request_timeout: Duration,
    tool_timeout: Duration,
}

impl HttpClient {
    #[must_use]
    pub fn new(
        inner: reqwest::Client,
        rate_limiter: Arc<RateLimiter>,
        retry_config: RetryConfig,
        request_timeout: Duration,
        tool_timeout: Duration,
    ) -> Self {
        Self {
            inner,
            rate_limiter,
            retry_config,
            request_timeout,
            tool_timeout,
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

    pub async fn execute_bytes<F>(&self, api: ResearchApi, build_request: F) -> Result<bytes::Bytes>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let response = self.execute_response(api, build_request).await?;
        response.bytes().await.map_err(|err| ResearchError::Parse {
            api,
            message: err.to_string(),
        })
    }

    pub(crate) async fn execute_response<F>(
        &self,
        api: ResearchApi,
        build_request: F,
    ) -> Result<Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let started_at = tokio::time::Instant::now();

        for attempt in 0..=self.retry_config.max_retries {
            let Some(remaining) = self.remaining_tool_time(started_at) else {
                return Err(tool_timeout_error(api, self.tool_timeout));
            };
            let _permit =
                match tokio::time::timeout(remaining, self.rate_limiter.acquire(api)).await {
                    Ok(acquire_result) => acquire_result?,
                    Err(_) => return Err(tool_timeout_error(api, self.tool_timeout)),
                };

            let request = build_request();
            let Some(remaining) = self.remaining_tool_time(started_at) else {
                return Err(tool_timeout_error(api, self.tool_timeout));
            };
            let send_timeout = self.request_timeout.min(remaining);
            let response = match tokio::time::timeout(send_timeout, request.send()).await {
                Ok(response) => response,
                Err(_) => {
                    if send_timeout < remaining {
                        return Err(ResearchError::Timeout {
                            api,
                            timeout_ms: self.request_timeout_ms(),
                        });
                    }
                    return Err(tool_timeout_error(api, self.tool_timeout));
                }
            };

            match response {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return Ok(resp);
                    }

                    let status = resp.status();
                    if is_retryable_upstream_status(status)
                        && attempt < self.retry_config.max_retries
                    {
                        let retry_after = parse_retry_after(resp.headers());
                        let Some(remaining) = self.remaining_tool_time(started_at) else {
                            return Err(tool_timeout_error(api, self.tool_timeout));
                        };
                        tokio::time::sleep(self.retry_delay(attempt, retry_after).min(remaining))
                            .await;
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
                    if is_retryable_http_error(&err) && attempt < self.retry_config.max_retries {
                        let Some(remaining) = self.remaining_tool_time(started_at) else {
                            return Err(tool_timeout_error(api, self.tool_timeout));
                        };
                        tokio::time::sleep(self.retry_delay(attempt, None).min(remaining)).await;
                        continue;
                    }

                    if err.is_timeout() {
                        return Err(ResearchError::Timeout {
                            api,
                            timeout_ms: self.request_timeout_ms(),
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

    fn remaining_tool_time(&self, started_at: tokio::time::Instant) -> Option<Duration> {
        self.tool_timeout.checked_sub(started_at.elapsed())
    }

    fn request_timeout_ms(&self) -> u64 {
        u64::try_from(self.request_timeout.as_millis()).unwrap_or(u64::MAX)
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

fn parse_retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?;
    let raw = value.to_str().ok()?;
    parse_retry_after_value(raw)
}

const MIN_RETRY_AFTER_DELAY: Duration = Duration::from_millis(100);

fn parse_retry_after_value(raw: &str) -> Option<Duration> {
    if let Ok(seconds) = raw.parse::<u64>() {
        let delay = Duration::from_secs(seconds);
        return Some(delay.max(MIN_RETRY_AFTER_DELAY));
    }

    let retry_at = DateTime::parse_from_rfc2822(raw).ok()?.with_timezone(&Utc);
    let delay = retry_at.signed_duration_since(Utc::now());
    if delay <= chrono::Duration::zero() {
        Some(MIN_RETRY_AFTER_DELAY)
    } else {
        delay
            .to_std()
            .ok()
            .map(|value| value.max(MIN_RETRY_AFTER_DELAY))
    }
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

fn tool_timeout_error(api: ResearchApi, timeout: Duration) -> ResearchError {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    ResearchError::Timeout { api, timeout_ms }
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

    #[test]
    fn parse_retry_after_value_accepts_seconds() {
        assert_eq!(parse_retry_after_value("7"), Some(Duration::from_secs(7)));
    }

    #[test]
    fn parse_retry_after_value_enforces_floor_for_zero_seconds() {
        assert_eq!(
            parse_retry_after_value("0"),
            Some(Duration::from_millis(100))
        );
    }

    #[test]
    fn parse_retry_after_value_accepts_http_date() {
        let header_value = (Utc::now() + chrono::Duration::seconds(2))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let delay = parse_retry_after_value(&header_value).expect("valid retry-after date");
        assert!(delay <= Duration::from_secs(3));
    }

    #[test]
    fn parse_retry_after_value_enforces_floor_for_past_http_date() {
        let header_value = (Utc::now() - chrono::Duration::seconds(5))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert_eq!(
            parse_retry_after_value(&header_value),
            Some(Duration::from_millis(100))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn request_timeout_applies_even_without_reqwest_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .mount(&server)
            .await;

        let request_timeout = Duration::from_millis(25);
        let client = reqwest::Client::builder().build().expect("reqwest client");
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
            Duration::from_secs(2),
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
            Duration::from_secs(2),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_error_uses_tool_timeout_for_overall_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .mount(&server)
            .await;

        let request_timeout = Duration::from_secs(2);
        let tool_timeout = Duration::from_millis(30);
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
            tool_timeout,
        );

        let err = http
            .execute_text(ResearchApi::OpenAlex, || {
                http.client().get(format!("{}/slow", server.uri()))
            })
            .await
            .expect_err("request should hit tool timeout");

        match err {
            ResearchError::Timeout { api, timeout_ms } => {
                assert_eq!(api, ResearchApi::OpenAlex);
                assert_eq!(timeout_ms, 30);
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
