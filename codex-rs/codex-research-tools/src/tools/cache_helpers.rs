use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::error::ResearchError;
use crate::error::Result;

pub(crate) async fn get_or_fetch_typed<T, F, Fut>(
    toolkit: &ResearchToolkit,
    key: CacheKey,
    ttl: std::time::Duration,
    fetch: F,
) -> Result<T>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let value = toolkit
        .cache()
        .get_or_fetch(key, ttl, || async {
            let output = fetch().await?;
            serde_json::to_value(output).map_err(|err| {
                ResearchError::Internal(format!("failed to serialize cached value: {err}"))
            })
        })
        .await?;

    serde_json::from_value(value).map_err(|err| {
        ResearchError::Internal(format!("failed to deserialize cached value: {err}"))
    })
}

pub(crate) fn hash_cache_payload<T: Serialize>(payload: &T) -> Result<u64> {
    let serialized = serde_json::to_string(payload)
        .map_err(|err| ResearchError::Internal(format!("failed to serialize cache key: {err}")))?;
    let mut hasher = DefaultHasher::new();
    serialized.hash(&mut hasher);
    Ok(hasher.finish())
}
