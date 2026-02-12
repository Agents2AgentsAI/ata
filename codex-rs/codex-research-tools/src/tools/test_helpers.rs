use std::time::Duration;

use crate::ResearchToolkit;
use crate::config::RateLimitOverrides;
use crate::config::ResearchConfig;
use crate::rate_limiter::ApiRateLimit;

pub(crate) fn build_test_toolkit_with_config(mut config: ResearchConfig) -> ResearchToolkit {
    config.rate_limit_overrides = permissive_rate_limit_overrides();

    let http_client = reqwest::Client::builder()
        .connect_timeout(config.connect_timeout)
        .timeout(config.request_timeout)
        .build()
        .expect("test http client should build");

    ResearchToolkit::new(http_client, config)
}

fn permissive_rate_limit_overrides() -> RateLimitOverrides {
    RateLimitOverrides {
        semantic_scholar: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
        arxiv: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
        openalex: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
        zotero: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
        github: Some(ApiRateLimit::new(100, Duration::from_millis(1), 20)),
    }
}
