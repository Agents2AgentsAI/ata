use std::hash::Hash;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use lru::LruCache;
use std::num::NonZeroUsize;

/// A cache entry with TTL
#[derive(Debug, Clone)]
struct CacheEntry<V> {
    value: V,
    expires_at: Instant,
}

impl<V> CacheEntry<V> {
    fn new(value: V, ttl: Duration) -> Self {
        Self {
            value,
            expires_at: Instant::now() + ttl,
        }
    }

    fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    #[cfg(test)]
    fn force_expired(&mut self) {
        self.expires_at = Instant::now() - Duration::from_secs(1);
    }
}

/// Thread-safe LRU cache with TTL support
#[derive(Debug)]
pub struct ResponseCache<K = String, V = String>
where
    K: Eq + Hash,
{
    cache: Mutex<LruCache<K, CacheEntry<V>>>,
}

impl<K, V> ResponseCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    #[must_use]
    pub fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            cache: Mutex::new(LruCache::new(capacity)),
        }
    }

    #[cfg(test)]
    pub fn expire_entry_for_test(&self, key: &K) {
        if let Ok(mut cache) = self.cache.lock()
            && let Some(entry) = cache.get_mut(key)
        {
            entry.force_expired();
        }
    }

    /// Get a value from the cache if it exists and hasn't expired
    pub fn get(&self, key: &K) -> Option<V> {
        let mut cache = self.cache.lock().ok()?;

        if let Some(entry) = cache.get(key) {
            if !entry.is_expired() {
                return Some(entry.value.clone());
            }
            // Entry expired, remove it
            cache.pop(key);
        }
        None
    }

    /// Insert a value into the cache with a TTL
    pub fn insert(&self, key: K, value: V, ttl: Duration) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(key, CacheEntry::new(value, ttl));
        }
    }

    /// Remove a value from the cache
    pub fn remove(&self, key: &K) -> Option<V> {
        self.cache
            .lock()
            .ok()
            .and_then(|mut cache| cache.pop(key).map(|e| e.value))
    }

    /// Clear all entries from the cache
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
    }

    /// Get the number of entries in the cache (including expired ones)
    pub fn len(&self) -> usize {
        self.cache.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove all expired entries
    pub fn evict_expired(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            let keys_to_remove: Vec<K> = cache
                .iter()
                .filter(|(_, entry)| entry.is_expired())
                .map(|(k, _)| k.clone())
                .collect();

            for key in keys_to_remove {
                cache.pop(&key);
            }
        }
    }
}

/// Cache key builder for structured cache keys
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub tool: String,
    pub params_hash: String,
}

impl CacheKey {
    pub fn new(tool: &str, params: &impl serde::Serialize) -> Self {
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let hash = simple_hash(&params_json);
        Self {
            tool: tool.to_string(),
            params_hash: hash,
        }
    }

    pub fn as_string(&self) -> String {
        format!("{}:{}", self.tool, self.params_hash)
    }
}

fn simple_hash(s: &str) -> String {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn cache_stores_and_retrieves_values() {
        let cache: ResponseCache<String, String> = ResponseCache::new(10);
        cache.insert(
            "key1".to_string(),
            "value1".to_string(),
            Duration::from_secs(60),
        );

        assert_eq!(cache.get(&"key1".to_string()), Some("value1".to_string()));
        assert_eq!(cache.get(&"key2".to_string()), None);
    }

    #[test]
    fn cache_expires_entries() {
        let cache: ResponseCache<String, String> = ResponseCache::new(10);
        let key = "key1".to_string();
        cache.insert(key.clone(), "value1".to_string(), Duration::from_secs(60));

        // Should exist immediately
        assert!(cache.get(&key).is_some());

        // Move expiry time into the past so we can deterministically exercise eviction.
        cache.expire_entry_for_test(&key);

        // Should be gone
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn cache_respects_capacity() {
        let cache: ResponseCache<String, String> = ResponseCache::new(2);
        cache.insert(
            "key1".to_string(),
            "value1".to_string(),
            Duration::from_secs(60),
        );
        cache.insert(
            "key2".to_string(),
            "value2".to_string(),
            Duration::from_secs(60),
        );
        cache.insert(
            "key3".to_string(),
            "value3".to_string(),
            Duration::from_secs(60),
        );

        // key1 should have been evicted (LRU)
        assert!(cache.get(&"key1".to_string()).is_none());
        assert!(cache.get(&"key2".to_string()).is_some());
        assert!(cache.get(&"key3".to_string()).is_some());
    }

    #[test]
    fn cache_key_generates_consistent_hash() {
        #[derive(serde::Serialize)]
        struct Params {
            query: String,
            limit: u32,
        }

        let params = Params {
            query: "test".to_string(),
            limit: 10,
        };

        let key1 = CacheKey::new("dataset_search", &params);
        let key2 = CacheKey::new("dataset_search", &params);

        assert_eq!(key1, key2);
        assert_eq!(key1.as_string(), key2.as_string());
    }
}
