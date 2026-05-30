use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use crate::rate_limiter::ApiRateLimit;
use crate::rate_limiter::RateLimitBucket;
use crate::rate_limiter::ResearchApi;

pub const DEFAULT_REMOTE_ZOTERO_BASE_URL: &str = "https://api.zotero.org";
pub const DEFAULT_LOCAL_ZOTERO_BASE_URL: &str = "http://localhost:23119/api";

fn read_optional_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

#[derive(Clone)]
pub struct ResearchConfig {
    pub semantic_scholar_api_key: Option<String>,
    pub zotero_api_key: Option<String>,
    pub zotero_user_id: Option<String>,
    pub openalex_email: Option<String>,
    pub github_token: Option<String>,
    pub epo_consumer_key: Option<String>,
    pub epo_consumer_secret: Option<String>,
    pub zotero_library_type: Option<String>,
    pub zotero_group_id: Option<String>,
    pub zotero_storage_dir: Option<String>,
    pub semantic_scholar_base_url: String,
    pub arxiv_base_url: String,
    pub openalex_base_url: String,
    pub zotero_base_url: String,
    pub github_api_base_url: String,
    pub hn_base_url: String,
    pub patents_base_url: String,

    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub tool_timeout: Duration,
    /// Maximum time to wait for a single source in multi-source searches.
    /// If a source exceeds this, partial results from other sources are returned.
    pub per_source_timeout: Duration,

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
    pub hn_search: Duration,
    pub patent_search: Duration,
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
    pub hackernews: Option<ApiRateLimit>,
    pub patents: Option<ApiRateLimit>,
}

impl Default for CacheTtls {
    fn default() -> Self {
        Self {
            paper_search: Duration::from_secs(5 * 60),
            citations: Duration::from_secs(10 * 60),
            zotero_items: Duration::from_secs(2 * 60),
            hn_search: Duration::from_secs(5 * 60),
            patent_search: Duration::from_secs(10 * 60),
            repo_analysis: Duration::from_secs(30 * 60),
            repo_health: Duration::from_secs(15 * 60),
            negative: Duration::from_secs(30),
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
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
            epo_consumer_key: None,
            epo_consumer_secret: None,
            zotero_library_type: None,
            zotero_group_id: None,
            zotero_storage_dir: None,
            semantic_scholar_base_url: "https://api.semanticscholar.org/graph/v1".to_string(),
            arxiv_base_url: "https://export.arxiv.org".to_string(),
            openalex_base_url: "https://api.openalex.org".to_string(),
            zotero_base_url: DEFAULT_REMOTE_ZOTERO_BASE_URL.to_string(),
            github_api_base_url: "https://api.github.com".to_string(),
            hn_base_url: "https://hn.algolia.com/api/v1".to_string(),
            patents_base_url: "https://ops.epo.org/3.2".to_string(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(15),
            tool_timeout: Duration::from_secs(30),
            per_source_timeout: Duration::from_secs(12),
            cache_max_entries: 10_000,
            cache_ttls: CacheTtls::default(),
            retry: RetryConfig::default(),
            rate_limit_overrides: RateLimitOverrides::default(),
        }
    }
}

impl ResearchConfig {
    #[must_use]
    pub fn has_zotero_api_key(&self) -> bool {
        self.zotero_api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
    }

    #[must_use]
    pub fn uses_local_zotero_api(&self) -> bool {
        if self.has_zotero_api_key() {
            return false;
        }

        let base = self
            .zotero_base_url
            .trim_end_matches('/')
            .to_ascii_lowercase();
        base == DEFAULT_LOCAL_ZOTERO_BASE_URL
            || base.starts_with("http://localhost:")
            || base.starts_with("http://127.0.0.1:")
            || base.starts_with("http://[::1]:")
    }

    #[must_use]
    pub fn from_env() -> Self {
        let zotero_api_key = read_optional_env("ZOTERO_API_KEY");
        let zotero_base_url = read_optional_env("ZOTERO_BASE_URL").unwrap_or_else(|| {
            if zotero_api_key.is_some() {
                DEFAULT_REMOTE_ZOTERO_BASE_URL.to_string()
            } else {
                DEFAULT_LOCAL_ZOTERO_BASE_URL.to_string()
            }
        });

        let mut config = Self {
            semantic_scholar_api_key: read_optional_env("SEMANTIC_SCHOLAR_API_KEY"),
            zotero_api_key,
            zotero_user_id: read_optional_env("ZOTERO_USER_ID"),
            openalex_email: read_optional_env("OPENALEX_EMAIL"),
            github_token: read_optional_env("GITHUB_TOKEN"),
            epo_consumer_key: read_optional_env("EPO_CONSUMER_KEY"),
            epo_consumer_secret: read_optional_env("EPO_CONSUMER_SECRET"),
            zotero_library_type: read_optional_env("ZOTERO_LIBRARY_TYPE"),
            zotero_group_id: read_optional_env("ZOTERO_GROUP_ID"),
            zotero_storage_dir: read_optional_env("ZOTERO_STORAGE_DIR"),
            semantic_scholar_base_url: std::env::var("SEMANTIC_SCHOLAR_BASE_URL")
                .unwrap_or_else(|_| "https://api.semanticscholar.org/graph/v1".to_string()),
            arxiv_base_url: std::env::var("ARXIV_BASE_URL")
                .unwrap_or_else(|_| "https://export.arxiv.org".to_string()),
            openalex_base_url: std::env::var("OPENALEX_BASE_URL")
                .unwrap_or_else(|_| "https://api.openalex.org".to_string()),
            zotero_base_url,
            github_api_base_url: std::env::var("GITHUB_API_BASE_URL")
                .unwrap_or_else(|_| "https://api.github.com".to_string()),
            hn_base_url: std::env::var("HN_BASE_URL")
                .unwrap_or_else(|_| "https://hn.algolia.com/api/v1".to_string()),
            patents_base_url: std::env::var("EPO_BASE_URL")
                .unwrap_or_else(|_| "https://ops.epo.org/3.2".to_string()),
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
    pub fn rate_limits(&self) -> HashMap<RateLimitBucket, ApiRateLimit> {
        let mut limits = HashMap::from([
            // Ata keeps Semantic Scholar search at 1 RPS and single concurrency
            // because search is the most rate-sensitive path.
            (
                RateLimitBucket::SemanticScholarSearch,
                ApiRateLimit::new(1, Duration::from_secs(1), 1),
            ),
            (
                RateLimitBucket::SemanticScholarGraph,
                if self.semantic_scholar_api_key.is_some() {
                    ApiRateLimit::new(10, Duration::from_secs(1), 3)
                } else {
                    ApiRateLimit::new(1, Duration::from_secs(1), 1)
                },
            ),
            // arXiv: official guideline is 1 request per 3 seconds, single connection.
            (
                RateLimitBucket::Arxiv,
                ApiRateLimit::new(1, Duration::from_secs(3), 1),
            ),
            // OpenAlex: 100 req/sec hard cap, credit-based system (search costs 100
            // credits, free tier has 100k credits/day ≈ 1000 searches/day). 10 req/sec
            // with 5 concurrent is well within limits.
            (
                RateLimitBucket::OpenAlex,
                ApiRateLimit::new(10, Duration::from_secs(1), 5),
            ),
            (
                RateLimitBucket::Zotero,
                ApiRateLimit::new(10, Duration::from_secs(1), 3),
            ),
            (
                RateLimitBucket::GitHub,
                if self.github_token.is_some() {
                    ApiRateLimit::new(5_000, Duration::from_secs(60 * 60), 3)
                } else {
                    ApiRateLimit::new(60, Duration::from_secs(60 * 60), 3)
                },
            ),
            (
                RateLimitBucket::HackerNews,
                ApiRateLimit::new(10, Duration::from_secs(1), 3),
            ),
            (
                RateLimitBucket::Patents,
                ApiRateLimit::new(25, Duration::from_secs(60), 2),
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
            (
                ResearchApi::HackerNews,
                self.rate_limit_overrides.hackernews,
            ),
            (ResearchApi::Patents, self.rate_limit_overrides.patents),
        ] {
            if let Some(rule) = override_limit {
                match api {
                    ResearchApi::SemanticScholar => {
                        limits.insert(RateLimitBucket::SemanticScholarSearch, rule);
                        limits.insert(RateLimitBucket::SemanticScholarGraph, rule);
                    }
                    other => {
                        limits.insert(RateLimitBucket::from(other), rule);
                    }
                }
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
            .field("epo_consumer_key", &redact(&self.epo_consumer_key))
            .field("epo_consumer_secret", &redact(&self.epo_consumer_secret))
            .field("zotero_library_type", &self.zotero_library_type)
            .field("zotero_group_id", &self.zotero_group_id)
            .field("zotero_storage_dir", &self.zotero_storage_dir)
            .field("semantic_scholar_base_url", &self.semantic_scholar_base_url)
            .field("arxiv_base_url", &self.arxiv_base_url)
            .field("openalex_base_url", &self.openalex_base_url)
            .field("zotero_base_url", &self.zotero_base_url)
            .field("github_api_base_url", &self.github_api_base_url)
            .field("hn_base_url", &self.hn_base_url)
            .field("patents_base_url", &self.patents_base_url)
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("tool_timeout", &self.tool_timeout)
            .field("per_source_timeout", &self.per_source_timeout)
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::DEFAULT_LOCAL_ZOTERO_BASE_URL;
    use super::RateLimitOverrides;
    use super::ResearchConfig;
    use crate::rate_limiter::ApiRateLimit;
    use crate::rate_limiter::RateLimitBucket;
    use pretty_assertions::assert_eq;

    #[test]
    fn has_zotero_api_key_ignores_blank_strings() {
        let config = ResearchConfig {
            zotero_api_key: Some("   ".to_string()),
            ..ResearchConfig::default()
        };

        assert_eq!(config.has_zotero_api_key(), false);
    }

    #[test]
    fn uses_local_zotero_api_treats_blank_api_key_as_unset() {
        let config = ResearchConfig {
            zotero_api_key: Some(String::new()),
            zotero_base_url: DEFAULT_LOCAL_ZOTERO_BASE_URL.to_string(),
            ..ResearchConfig::default()
        };

        assert_eq!(config.uses_local_zotero_api(), true);
    }

    #[test]
    fn semantic_scholar_rate_limits_split_search_and_graph_buckets() {
        let limits = ResearchConfig::default().rate_limits();

        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarSearch),
            Some(&ApiRateLimit::new(1, Duration::from_secs(1), 1))
        );
        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarGraph),
            Some(&ApiRateLimit::new(1, Duration::from_secs(1), 1))
        );
    }

    #[test]
    fn semantic_scholar_graph_bucket_relaxes_when_api_key_is_configured() {
        let config = ResearchConfig {
            semantic_scholar_api_key: Some("test-key".to_string()),
            ..ResearchConfig::default()
        };
        let limits = config.rate_limits();

        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarSearch),
            Some(&ApiRateLimit::new(1, Duration::from_secs(1), 1))
        );
        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarGraph),
            Some(&ApiRateLimit::new(10, Duration::from_secs(1), 3))
        );
    }

    #[test]
    fn semantic_scholar_override_applies_to_both_buckets() {
        let override_limit = ApiRateLimit::new(7, Duration::from_secs(2), 4);
        let config = ResearchConfig {
            rate_limit_overrides: RateLimitOverrides {
                semantic_scholar: Some(override_limit),
                ..RateLimitOverrides::default()
            },
            ..ResearchConfig::default()
        };
        let limits = config.rate_limits();

        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarSearch),
            Some(&override_limit)
        );
        assert_eq!(
            limits.get(&RateLimitBucket::SemanticScholarGraph),
            Some(&override_limit)
        );
    }
}
