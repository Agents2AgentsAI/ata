use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use tokio::sync::Mutex;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;

use crate::error::ResearchError;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchApi {
    SemanticScholar,
    Arxiv,
    OpenAlex,
    Zotero,
    GitHub,
}

impl fmt::Display for ResearchApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SemanticScholar => write!(f, "semantic_scholar"),
            Self::Arxiv => write!(f, "arxiv"),
            Self::OpenAlex => write!(f, "openalex"),
            Self::Zotero => write!(f, "zotero"),
            Self::GitHub => write!(f, "github"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiRateLimit {
    pub max_requests: u32,
    pub per: Duration,
    pub max_concurrent: usize,
}

impl ApiRateLimit {
    #[must_use]
    pub fn new(max_requests: u32, per: Duration, max_concurrent: usize) -> Self {
        Self {
            max_requests: max_requests.max(1),
            per,
            max_concurrent: max_concurrent.max(1),
        }
    }
}

#[derive(Debug)]
struct ApiLimiter {
    rule: ApiRateLimit,
    semaphore: Arc<Semaphore>,
    window: Mutex<VecDeque<Instant>>,
}

#[derive(Debug)]
pub enum RateLimitPermit {
    Limited(OwnedSemaphorePermit),
    Unlimited,
}

#[derive(Debug)]
pub struct RateLimiter {
    limiters: HashMap<ResearchApi, Arc<ApiLimiter>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(rules: HashMap<ResearchApi, ApiRateLimit>) -> Self {
        let limiters = rules
            .into_iter()
            .map(|(api, rule)| {
                (
                    api,
                    Arc::new(ApiLimiter {
                        rule,
                        semaphore: Arc::new(Semaphore::new(rule.max_concurrent)),
                        window: Mutex::new(VecDeque::with_capacity(rule.max_requests as usize)),
                    }),
                )
            })
            .collect();

        Self { limiters }
    }

    pub async fn acquire(&self, api: ResearchApi) -> Result<RateLimitPermit> {
        let Some(limiter) = self.limiters.get(&api) else {
            return Ok(RateLimitPermit::Unlimited);
        };

        loop {
            let permit = limiter
                .semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| ResearchError::RateLimiterClosed { api })?;

            let mut window = limiter.window.lock().await;
            let now = Instant::now();
            while let Some(oldest) = window.front() {
                if now.duration_since(*oldest) >= limiter.rule.per {
                    window.pop_front();
                } else {
                    break;
                }
            }

            if window.len() < limiter.rule.max_requests as usize {
                window.push_back(now);
                return Ok(RateLimitPermit::Limited(permit));
            }

            let wait_for = if let Some(oldest) = window.front().copied() {
                limiter.rule.per.saturating_sub(now.duration_since(oldest))
            } else {
                Duration::from_millis(1)
            };

            drop(window);
            drop(permit);
            tokio::time::sleep(wait_for).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    use std::time::Instant;

    use pretty_assertions::assert_eq;

    use crate::error::Result;
    use crate::rate_limiter::ApiRateLimit;
    use crate::rate_limiter::RateLimiter;
    use crate::rate_limiter::ResearchApi;

    #[tokio::test(flavor = "multi_thread")]
    async fn sequential_limit_enforces_spacing() -> Result<()> {
        let mut rules = HashMap::new();
        rules.insert(
            ResearchApi::Arxiv,
            ApiRateLimit::new(1, Duration::from_millis(50), 1),
        );

        let limiter = RateLimiter::new(rules);
        let start = Instant::now();

        let _permit_1 = limiter.acquire(ResearchApi::Arxiv).await?;
        drop(_permit_1);
        let _permit_2 = limiter.acquire(ResearchApi::Arxiv).await?;
        drop(_permit_2);
        let _permit_3 = limiter.acquire(ResearchApi::Arxiv).await?;
        drop(_permit_3);

        assert!(start.elapsed() >= Duration::from_millis(90));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_acquire_completes_for_all_waiters() -> Result<()> {
        let mut rules = HashMap::new();
        rules.insert(
            ResearchApi::SemanticScholar,
            ApiRateLimit::new(2, Duration::from_millis(20), 2),
        );

        let limiter = Arc::new(RateLimiter::new(rules));
        let completed = Arc::new(AtomicUsize::new(0));

        let handles = (0..6)
            .map(|_| {
                let limiter = Arc::clone(&limiter);
                let completed = Arc::clone(&completed);
                tokio::spawn(async move {
                    let permit = limiter.acquire(ResearchApi::SemanticScholar).await?;
                    drop(permit);
                    completed.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), crate::error::ResearchError>(())
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.await.map_err(|err| {
                crate::error::ResearchError::Internal(format!("join error: {err}"))
            })??;
        }

        assert_eq!(completed.load(Ordering::SeqCst), 6);
        Ok(())
    }
}
