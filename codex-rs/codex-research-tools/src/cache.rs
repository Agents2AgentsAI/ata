use std::future::Future;
use std::hash::Hash;
use std::num::NonZeroUsize;
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
    in_flight: DashMap<CacheKey, ArcNotify>,
}

type ArcNotify = std::sync::Arc<Notify>;

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
        let mut cache = self.inner.lock().await;
        if let Some(entry) = cache.get(key)
            && entry.inserted_at.elapsed() < entry.ttl
        {
            return Some(entry.data.clone());
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
        if let Some(cached) = self.get(&key).await {
            return Ok(FetchOutput::positive(cached));
        }

        enum Role {
            Leader(ArcNotify),
            Follower(ArcNotify),
        }

        let role = match self.in_flight.entry(key.clone()) {
            Entry::Vacant(entry) => {
                let notify = std::sync::Arc::new(Notify::new());
                entry.insert(notify.clone());
                Role::Leader(notify)
            }
            Entry::Occupied(entry) => Role::Follower(entry.get().clone()),
        };

        match role {
            Role::Leader(notify) => {
                let fetch_result = fetch().await;
                if let Ok(output) = &fetch_result {
                    self.insert(key.clone(), output.data.clone(), ttl, output.is_negative)
                        .await;
                }

                self.in_flight.remove(&key);
                notify.notify_waiters();
                fetch_result
            }
            Role::Follower(notify) => {
                notify.notified().await;
                if let Some(cached) = self.get(&key).await {
                    return Ok(FetchOutput::positive(cached));
                }

                Err(ResearchError::Internal(format!(
                    "singleflight fetch completed without cache entry for {}:{}",
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
        cache.get(key).map(|entry| entry.is_negative)
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
    use crate::error::Result;

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
}
