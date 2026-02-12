use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use crate::rate_limiter::ApiRateLimit;
use crate::rate_limiter::ResearchApi;

#[derive(Clone)]
pub struct ResearchConfig {
    pub semantic_scholar_api_key: Option<String>,
    pub zotero_api_key: Option<String>,
    pub zotero_user_id: Option<String>,
    pub openalex_email: Option<String>,
    pub github_token: Option<String>,
    pub zotero_library_type: Option<String>,
    pub zotero_group_id: Option<String>,
    pub semantic_scholar_base_url: String,
    pub arxiv_base_url: String,
    pub openalex_base_url: String,
    pub zotero_base_url: String,
    pub github_api_base_url: String,

    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub tool_timeout: Duration,

    pub cache_max_entries: usize,
    pub cache_ttls: CacheTtls,
    pub retry: RetryConfig,
    pub rate_limit_overrides: RateLimitOverrides,
}

#[derive(Debug, Clone, Copy)]
pub struct CacheTtls {
    pub paper_search: Duration,
    pub citations: Duration,
    pub zotero_items: Duration,
    pub repo_analysis: Duration,
    pub repo_health: Duration,
    pub negative: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct RateLimitOverrides {
    pub semantic_scholar: Option<ApiRateLimit>,
    pub arxiv: Option<ApiRateLimit>,
    pub openalex: Option<ApiRateLimit>,
    pub zotero: Option<ApiRateLimit>,
    pub github: Option<ApiRateLimit>,
}

impl Default for CacheTtls {
    fn default() -> Self {
        Self {
            paper_search: Duration::from_secs(5 * 60),
            citations: Duration::from_secs(10 * 60),
            zotero_items: Duration::from_secs(2 * 60),
            repo_analysis: Duration::from_secs(60 * 60),
            repo_health: Duration::from_secs(15 * 60),
            negative: Duration::from_secs(30),
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            semantic_scholar_api_key: None,
            zotero_api_key: None,
            zotero_user_id: None,
            openalex_email: None,
            github_token: None,
            zotero_library_type: None,
            zotero_group_id: None,
            semantic_scholar_base_url: "https://api.semanticscholar.org/graph/v1".to_string(),
            arxiv_base_url: "https://export.arxiv.org".to_string(),
            openalex_base_url: "https://api.openalex.org".to_string(),
            zotero_base_url: "https://api.zotero.org".to_string(),
            github_api_base_url: "https://api.github.com".to_string(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(60),
            cache_max_entries: 10_000,
            cache_ttls: CacheTtls::default(),
            retry: RetryConfig::default(),
            rate_limit_overrides: RateLimitOverrides::default(),
        }
    }
}

impl ResearchConfig {
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self {
            semantic_scholar_api_key: std::env::var("SEMANTIC_SCHOLAR_API_KEY").ok(),
            zotero_api_key: std::env::var("ZOTERO_API_KEY").ok(),
            zotero_user_id: std::env::var("ZOTERO_USER_ID").ok(),
            openalex_email: std::env::var("OPENALEX_EMAIL").ok(),
            github_token: std::env::var("GITHUB_TOKEN").ok(),
            zotero_library_type: std::env::var("ZOTERO_LIBRARY_TYPE").ok(),
            zotero_group_id: std::env::var("ZOTERO_GROUP_ID").ok(),
            semantic_scholar_base_url: std::env::var("SEMANTIC_SCHOLAR_BASE_URL")
                .unwrap_or_else(|_| "https://api.semanticscholar.org/graph/v1".to_string()),
            arxiv_base_url: std::env::var("ARXIV_BASE_URL")
                .unwrap_or_else(|_| "https://export.arxiv.org".to_string()),
            openalex_base_url: std::env::var("OPENALEX_BASE_URL")
                .unwrap_or_else(|_| "https://api.openalex.org".to_string()),
            zotero_base_url: std::env::var("ZOTERO_BASE_URL")
                .unwrap_or_else(|_| "https://api.zotero.org".to_string()),
            github_api_base_url: std::env::var("GITHUB_API_BASE_URL")
                .unwrap_or_else(|_| "https://api.github.com".to_string()),
            ..Self::default()
        };

        if let Ok(raw) = std::env::var("RESEARCH_CACHE_MAX_ENTRIES")
            && let Ok(parsed) = raw.parse::<usize>()
        {
            config.cache_max_entries = parsed.max(1);
        }

        config
    }

    #[must_use]
    pub fn rate_limits(&self) -> HashMap<ResearchApi, ApiRateLimit> {
        let mut limits = HashMap::from([
            (
                ResearchApi::SemanticScholar,
                if self.semantic_scholar_api_key.is_some() {
                    ApiRateLimit::new(10, Duration::from_secs(1), 3)
                } else {
                    ApiRateLimit::new(1, Duration::from_secs(1), 3)
                },
            ),
            (
                ResearchApi::Arxiv,
                ApiRateLimit::new(1, Duration::from_secs(3), 1),
            ),
            (
                ResearchApi::OpenAlex,
                ApiRateLimit::new(10, Duration::from_secs(1), 5),
            ),
            (
                ResearchApi::Zotero,
                ApiRateLimit::new(10, Duration::from_secs(1), 3),
            ),
            (
                ResearchApi::GitHub,
                if self.github_token.is_some() {
                    ApiRateLimit::new(5_000, Duration::from_secs(60 * 60), 3)
                } else {
                    ApiRateLimit::new(60, Duration::from_secs(60 * 60), 3)
                },
            ),
        ]);

        for (api, override_limit) in [
            (
                ResearchApi::SemanticScholar,
                self.rate_limit_overrides.semantic_scholar,
            ),
            (ResearchApi::Arxiv, self.rate_limit_overrides.arxiv),
            (ResearchApi::OpenAlex, self.rate_limit_overrides.openalex),
            (ResearchApi::Zotero, self.rate_limit_overrides.zotero),
            (ResearchApi::GitHub, self.rate_limit_overrides.github),
        ] {
            if let Some(rule) = override_limit {
                limits.insert(api, rule);
            }
        }

        limits
    }
}

impl fmt::Debug for ResearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResearchConfig")
            .field(
                "semantic_scholar_api_key",
                &redact(&self.semantic_scholar_api_key),
            )
            .field("zotero_api_key", &redact(&self.zotero_api_key))
            .field("zotero_user_id", &self.zotero_user_id)
            .field("openalex_email", &self.openalex_email)
            .field("github_token", &redact(&self.github_token))
            .field("zotero_library_type", &self.zotero_library_type)
            .field("zotero_group_id", &self.zotero_group_id)
            .field("semantic_scholar_base_url", &self.semantic_scholar_base_url)
            .field("arxiv_base_url", &self.arxiv_base_url)
            .field("openalex_base_url", &self.openalex_base_url)
            .field("zotero_base_url", &self.zotero_base_url)
            .field("github_api_base_url", &self.github_api_base_url)
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("tool_timeout", &self.tool_timeout)
            .field("cache_max_entries", &self.cache_max_entries)
            .field("cache_ttls", &self.cache_ttls)
            .field("retry", &self.retry)
            .field("rate_limit_overrides", &self.rate_limit_overrides)
            .finish()
    }
}

fn redact(value: &Option<String>) -> &'static str {
    if value.is_some() {
        "<redacted>"
    } else {
        "<unset>"
    }
}
