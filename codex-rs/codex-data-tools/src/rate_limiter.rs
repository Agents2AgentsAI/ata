use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::Semaphore;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataApi {
    HuggingFace,
    Kaggle,
    IsaacSim,
    Cosmos,
}

#[derive(Debug, Clone, Copy)]
pub struct ApiRateLimit {
    pub requests_per_window: u32,
    pub window_duration: Duration,
    pub max_concurrent: u32,
}

impl ApiRateLimit {
    #[must_use]
    pub fn new(requests_per_window: u32, window_duration: Duration, max_concurrent: u32) -> Self {
        Self {
            requests_per_window,
            window_duration,
            max_concurrent,
        }
    }
}

#[derive(Debug)]
struct ApiState {
    semaphore: Arc<Semaphore>,
    request_times: tokio::sync::Mutex<Vec<Instant>>,
    window_duration: Duration,
    requests_per_window: u32,
}

#[derive(Debug)]
pub struct RateLimiter {
    states: DashMap<DataApi, ApiState>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(limits: HashMap<DataApi, ApiRateLimit>) -> Self {
        let states = DashMap::new();

        for (api, limit) in limits {
            states.insert(
                api,
                ApiState {
                    semaphore: Arc::new(Semaphore::new(limit.max_concurrent as usize)),
                    request_times: tokio::sync::Mutex::new(Vec::new()),
                    window_duration: limit.window_duration,
                    requests_per_window: limit.requests_per_window,
                },
            );
        }

        Self { states }
    }

    pub async fn acquire(&self, api: DataApi) -> crate::error::Result<RateLimitPermit> {
        let state = self.states.get(&api).ok_or_else(|| {
            crate::error::DataError::Internal(format!("no rate limit configured for {api}"))
        })?;

        // Acquire concurrency permit
        let permit = state
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| crate::error::DataError::Internal("semaphore closed".to_string()))?;

        // Check sliding window rate limit
        let mut request_times = state.request_times.lock().await;
        let now = Instant::now();
        let window_start = now - state.window_duration;

        // Remove old requests outside the window
        request_times.retain(|&time| time > window_start);

        // If at capacity, wait until oldest request expires
        if request_times.len() >= state.requests_per_window as usize
            && let Some(&oldest) = request_times.first()
        {
            let wait_time = state.window_duration - (now - oldest);
            drop(request_times);
            tokio::time::sleep(wait_time).await;
            request_times = state.request_times.lock().await;
            let new_now = Instant::now();
            let new_window_start = new_now - state.window_duration;
            request_times.retain(|&time| time > new_window_start);
        }

        request_times.push(Instant::now());

        Ok(RateLimitPermit { _permit: permit })
    }
}

pub struct RateLimitPermit {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limiter_allows_requests_within_limit() {
        let limits = HashMap::from([(
            DataApi::HuggingFace,
            ApiRateLimit::new(5, Duration::from_secs(1), 2),
        )]);
        let limiter = RateLimiter::new(limits);

        // Should be able to acquire 2 permits (max concurrent)
        let _p1 = limiter.acquire(DataApi::HuggingFace).await.unwrap();
        let _p2 = limiter.acquire(DataApi::HuggingFace).await.unwrap();
    }

    #[tokio::test]
    async fn rate_limiter_returns_error_for_unconfigured_api() {
        let limiter = RateLimiter::new(HashMap::new());
        let result = limiter.acquire(DataApi::HuggingFace).await;
        assert!(result.is_err());
    }
}
