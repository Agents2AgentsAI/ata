use std::future::Future;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use lru::LruCache;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::Notify;

use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub tool_name: &'static str,
    pub params_hash: u64,
}

#[derive(Clone, Debug)]
struct CachedEntry {
    data: Value,
    inserted_at: Instant,
    ttl: Duration,
    is_negative: bool,
}

#[derive(Clone, Debug)]
pub struct FetchOutput {
    pub data: Value,
    pub is_negative: bool,
}

impl FetchOutput {
    #[must_use]
    pub fn positive(data: Value) -> Self {
        Self {
            data,
            is_negative: false,
        }
    }

    #[must_use]
    pub fn negative(data: Value) -> Self {
        Self {
            data,
            is_negative: true,
        }
    }
}

#[derive(Debug)]
pub struct ResponseCache {
    inner: Mutex<LruCache<CacheKey, CachedEntry>>,
    in_flight: DashMap<CacheKey, ArcInFlight>,
}

type ArcInFlight = Arc<InFlightRequest>;

#[derive(Debug)]
struct InFlightRequest {
    notify: Notify,
    outcome: Mutex<Option<std::result::Result<FetchOutput, SharedError>>>,
}

#[derive(Debug, Clone)]
enum SharedError {
    NotConfigured {
        tool: &'static str,
        reason: String,
    },
    InvalidInput(String),
    RateLimiterClosed {
        api: ResearchApi,
    },
    Timeout {
        api: ResearchApi,
        timeout_ms: u64,
    },
    Http {
        api: ResearchApi,
        message: String,
    },
    Upstream {
        api: ResearchApi,
        status: reqwest::StatusCode,
        message: String,
    },
    Parse {
        api: ResearchApi,
        message: String,
    },
    InternalPanic,
    Internal(String),
}

impl SharedError {
    fn from_research_error(error: &ResearchError) -> Self {
        match error {
            ResearchError::NotConfigured { tool, reason } => Self::NotConfigured {
                tool,
                reason: reason.clone(),
            },
            ResearchError::InvalidInput(message) => Self::InvalidInput(message.clone()),
            ResearchError::RateLimiterClosed { api } => Self::RateLimiterClosed { api: *api },
            ResearchError::Timeout { api, timeout_ms } => Self::Timeout {
                api: *api,
                timeout_ms: *timeout_ms,
            },
            ResearchError::Http { api, source } => Self::Http {
                api: *api,
                message: source.to_string(),
            },
            ResearchError::HttpMessage { api, message } => Self::Http {
                api: *api,
                message: message.clone(),
            },
            ResearchError::Upstream {
                api,
                status,
                message,
            } => Self::Upstream {
                api: *api,
                status: *status,
                message: message.clone(),
            },
            ResearchError::Parse { api, message } => Self::Parse {
                api: *api,
                message: message.clone(),
            },
            ResearchError::InternalPanic => Self::InternalPanic,
            ResearchError::Internal(message) => Self::Internal(message.clone()),
        }
    }

    fn into_research_error(self) -> ResearchError {
        match self {
            Self::NotConfigured { tool, reason } => ResearchError::NotConfigured { tool, reason },
            Self::InvalidInput(message) => ResearchError::InvalidInput(message),
            Self::RateLimiterClosed { api } => ResearchError::RateLimiterClosed { api },
            Self::Timeout { api, timeout_ms } => ResearchError::Timeout { api, timeout_ms },
            Self::Http { api, message } => ResearchError::HttpMessage { api, message },
            Self::Upstream {
                api,
                status,
                message,
            } => ResearchError::Upstream {
                api,
                status,
                message,
            },
            Self::Parse { api, message } => ResearchError::Parse { api, message },
            Self::InternalPanic => ResearchError::InternalPanic,
            Self::Internal(message) => ResearchError::Internal(message),
        }
    }
}

fn clone_result_for_followers(
    result: &Result<FetchOutput>,
) -> std::result::Result<FetchOutput, SharedError> {
    match result {
        Ok(output) => Ok(output.clone()),
        Err(error) => Err(SharedError::from_research_error(error)),
    }
}

impl ResponseCache {
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
            in_flight: DashMap::new(),
        }
    }

    pub async fn get(&self, key: &CacheKey) -> Option<Value> {
        self.get_with_meta(key).await.map(|output| output.data)
    }

    pub async fn get_with_meta(&self, key: &CacheKey) -> Option<FetchOutput> {
        let mut cache = self.inner.lock().await;
        if let Some(entry) = cache.get(key)
            && entry.inserted_at.elapsed() < entry.ttl
        {
            return Some(FetchOutput {
                data: entry.data.clone(),
                is_negative: entry.is_negative,
            });
        }

        cache.pop(key);
        None
    }

    pub async fn insert(&self, key: CacheKey, data: Value, ttl: Duration, is_negative: bool) {
        let mut cache = self.inner.lock().await;
        cache.put(
            key,
            CachedEntry {
                data,
                inserted_at: Instant::now(),
                ttl,
                is_negative,
            },
        );
    }

    pub async fn get_or_fetch<F, Fut>(
        &self,
        key: CacheKey,
        ttl: Duration,
        fetch: F,
    ) -> Result<Value>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value>>,
    {
        self.get_or_fetch_with_meta(key, ttl, || async move {
            fetch().await.map(FetchOutput::positive)
        })
        .await
        .map(|output| output.data)
    }

    pub async fn get_or_fetch_with_meta<F, Fut>(
        &self,
        key: CacheKey,
        ttl: Duration,
        fetch: F,
    ) -> Result<FetchOutput>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<FetchOutput>>,
    {
        self.get_or_fetch_with_meta_ttls(key, ttl, ttl, fetch).await
    }

    pub async fn get_or_fetch_with_meta_ttls<F, Fut>(
        &self,
        key: CacheKey,
        ttl: Duration,
        negative_ttl: Duration,
        fetch: F,
    ) -> Result<FetchOutput>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<FetchOutput>>,
    {
        if let Some(cached) = self.get_with_meta(&key).await {
            return Ok(cached);
        }

        enum Role {
            Leader(ArcInFlight),
            Follower(ArcInFlight),
        }

        let role = match self.in_flight.entry(key.clone()) {
            Entry::Vacant(entry) => {
                let request = Arc::new(InFlightRequest {
                    notify: Notify::new(),
                    outcome: Mutex::new(None),
                });
                entry.insert(request.clone());
                Role::Leader(request)
            }
            Entry::Occupied(entry) => Role::Follower(entry.get().clone()),
        };

        match role {
            Role::Leader(request) => {
                let fetch_result = fetch().await;
                if let Ok(output) = &fetch_result {
                    let entry_ttl = if output.is_negative {
                        negative_ttl
                    } else {
                        ttl
                    };
                    self.insert(
                        key.clone(),
                        output.data.clone(),
                        entry_ttl,
                        output.is_negative,
                    )
                    .await;
                }
                {
                    let mut outcome = request.outcome.lock().await;
                    *outcome = Some(clone_result_for_followers(&fetch_result));
                }

                self.in_flight.remove(&key);
                request.notify.notify_waiters();
                fetch_result
            }
            Role::Follower(request) => {
                // Register the waiter first, then check state to avoid missing a notification
                // that arrives between deciding "Follower" and awaiting.
                let notified = request.notify.notified();
                tokio::pin!(notified);

                if let Some(outcome) = request.outcome.lock().await.clone() {
                    return outcome.map_err(SharedError::into_research_error);
                }

                if let Some(cached) = self.get_with_meta(&key).await {
                    return Ok(cached);
                }

                notified.await;

                if let Some(outcome) = request.outcome.lock().await.clone() {
                    return outcome.map_err(SharedError::into_research_error);
                }

                if let Some(cached) = self.get_with_meta(&key).await {
                    return Ok(cached);
                }

                Err(ResearchError::Internal(format!(
                    "singleflight fetch completed without outcome or cache entry for {}:{}",
                    key.tool_name, key.params_hash
                )))
            }
        }
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    pub async fn is_negative(&self, key: &CacheKey) -> Option<bool> {
        let mut cache = self.inner.lock().await;
        if let Some(entry) = cache.get(key)
            && entry.inserted_at.elapsed() < entry.ttl
        {
            return Some(entry.is_negative);
        }

        cache.pop(key);
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::cache::CacheKey;
    use crate::cache::FetchOutput;
    use crate::cache::ResponseCache;
    use crate::error::ResearchError;
    use crate::error::Result;
    use crate::rate_limiter::ResearchApi;

    #[tokio::test(flavor = "multi_thread")]
    async fn expires_entries_after_ttl() -> Result<()> {
        let cache = ResponseCache::new(16);
        let key = CacheKey {
            tool_name: "paper_search",
            params_hash: 7,
        };

        cache
            .insert(
                key.clone(),
                json!({"ok": true}),
                Duration::from_millis(20),
                false,
            )
            .await;

        assert_eq!(cache.get(&key).await, Some(json!({"ok": true})));
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert_eq!(cache.get(&key).await, None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn evicts_lru_entries_when_capacity_exceeded() -> Result<()> {
        let cache = ResponseCache::new(2);
        let ttl = Duration::from_secs(60);

        let key_a = CacheKey {
            tool_name: "paper_search",
            params_hash: 1,
        };
        let key_b = CacheKey {
            tool_name: "paper_search",
            params_hash: 2,
        };
        let key_c = CacheKey {
            tool_name: "paper_search",
            params_hash: 3,
        };

        cache.insert(key_a.clone(), json!("a"), ttl, false).await;
        cache.insert(key_b.clone(), json!("b"), ttl, false).await;
        assert_eq!(cache.get(&key_a).await, Some(json!("a")));

        cache.insert(key_c.clone(), json!("c"), ttl, false).await;

        assert_eq!(cache.get(&key_b).await, None);
        assert_eq!(cache.get(&key_a).await, Some(json!("a")));
        assert_eq!(cache.get(&key_c).await, Some(json!("c")));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singleflight_coalesces_concurrent_fetches() -> Result<()> {
        let cache = Arc::new(ResponseCache::new(32));
        let key = CacheKey {
            tool_name: "paper_get",
            params_hash: 42,
        };
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let handles = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let fetch_count = Arc::clone(&fetch_count);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch_with_meta(key, Duration::from_secs(60), || async move {
                            fetch_count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(25)).await;
                            Ok::<FetchOutput, crate::error::ResearchError>(FetchOutput::positive(
                                json!({"paper": "x"}),
                            ))
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let output = handle.await.map_err(|err| {
                crate::error::ResearchError::Internal(format!("join error: {err}"))
            })??;
            assert_eq!(output.data, json!({"paper": "x"}));
        }

        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singleflight_propagates_leader_error_to_followers() -> Result<()> {
        let cache = Arc::new(ResponseCache::new(8));
        let key = CacheKey {
            tool_name: "paper_search",
            params_hash: 404,
        };
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let handles = (0..6)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let fetch_count = Arc::clone(&fetch_count);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch_with_meta(key, Duration::from_secs(60), || async move {
                            fetch_count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Err::<FetchOutput, ResearchError>(ResearchError::Upstream {
                                api: ResearchApi::SemanticScholar,
                                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                                message: "rate limited".to_string(),
                            })
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            match handle.await.map_err(|err| {
                crate::error::ResearchError::Internal(format!("join error: {err}"))
            })? {
                Ok(output) => panic!("expected error, got output: {:?}", output.data),
                Err(ResearchError::Upstream {
                    api,
                    status,
                    message,
                }) => {
                    assert_eq!(api, ResearchApi::SemanticScholar);
                    assert_eq!(status, reqwest::StatusCode::TOO_MANY_REQUESTS);
                    assert!(message.contains("rate limited"));
                }
                Err(other) => panic!("unexpected error variant: {other}"),
            }
        }

        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singleflight_followers_keep_negative_metadata() -> Result<()> {
        let cache = Arc::new(ResponseCache::new(8));
        let key = CacheKey {
            tool_name: "paper_get",
            params_hash: 111,
        };
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let handles = (0..6)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let fetch_count = Arc::clone(&fetch_count);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch_with_meta(key, Duration::from_secs(60), || async move {
                            fetch_count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Ok::<FetchOutput, ResearchError>(FetchOutput::negative(
                                json!({"error": "not_found"}),
                            ))
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let output = handle.await.map_err(|err| {
                crate::error::ResearchError::Internal(format!("join error: {err}"))
            })??;
            assert_eq!(output.data, json!({"error": "not_found"}));
            assert!(output.is_negative);
        }

        assert_eq!(cache.is_negative(&key).await, Some(true));
        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn singleflight_followers_preserve_http_error_retryability() -> Result<()> {
        let cache = Arc::new(ResponseCache::new(8));
        let key = CacheKey {
            tool_name: "paper_search",
            params_hash: 212,
        };
        let fetch_count = Arc::new(AtomicUsize::new(0));

        let handles = (0..6)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let fetch_count = Arc::clone(&fetch_count);
                tokio::spawn(async move {
                    cache
                        .get_or_fetch_with_meta(key, Duration::from_secs(60), || async move {
                            fetch_count.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;

                            let source = reqwest::Client::new()
                                .get("http://127.0.0.1:9/singleflight-http-error")
                                .send()
                                .await
                                .expect_err("request should fail");

                            Err::<FetchOutput, ResearchError>(ResearchError::Http {
                                api: ResearchApi::OpenAlex,
                                source,
                            })
                        })
                        .await
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            match handle.await.map_err(|err| {
                crate::error::ResearchError::Internal(format!("join error: {err}"))
            })? {
                Ok(output) => panic!("expected error, got output: {:?}", output.data),
                Err(ResearchError::Http { api, .. })
                | Err(ResearchError::HttpMessage { api, .. }) => {
                    assert_eq!(api, ResearchApi::OpenAlex);
                }
                Err(other) => panic!("unexpected error variant: {other}"),
            }
        }

        assert_eq!(fetch_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn negative_entries_are_marked() -> Result<()> {
        let cache = ResponseCache::new(8);
        let key = CacheKey {
            tool_name: "paper_get",
            params_hash: 99,
        };

        cache
            .insert(
                key.clone(),
                json!({"error": "not_found"}),
                Duration::from_secs(60),
                true,
            )
            .await;

        assert_eq!(cache.is_negative(&key).await, Some(true));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expired_negative_entries_are_not_reported_as_negative() -> Result<()> {
        let cache = ResponseCache::new(8);
        let key = CacheKey {
            tool_name: "paper_get",
            params_hash: 100,
        };

        cache
            .insert(
                key.clone(),
                json!({"error": "not_found"}),
                Duration::from_millis(20),
                true,
            )
            .await;

        assert_eq!(cache.is_negative(&key).await, Some(true));
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert_eq!(cache.is_negative(&key).await, None);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dual_ttls_expire_negative_entries_earlier_than_positive_entries() -> Result<()> {
        let cache = ResponseCache::new(8);
        let key = CacheKey {
            tool_name: "repo_get_health",
            params_hash: 501,
        };

        let output = cache
            .get_or_fetch_with_meta_ttls(
                key.clone(),
                Duration::from_secs(60),
                Duration::from_millis(20),
                || async move {
                    Ok::<FetchOutput, ResearchError>(FetchOutput::negative(
                        json!({"error": "missing"}),
                    ))
                },
            )
            .await?;
        assert!(output.is_negative);

        tokio::time::sleep(Duration::from_millis(35)).await;
        assert_eq!(cache.get(&key).await, None);
        Ok(())
    }
}
