use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::error::Result;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::HnGetThreadParams;
use crate::types::HnSearchParams;
use crate::types::HnSearchResult;
use crate::types::HnThread;

pub(crate) async fn hn_search(
    toolkit: &ResearchToolkit,
    params: HnSearchParams,
) -> Result<HnSearchResult> {
    let params = normalize_search_params(params);
    let params_hash = hash_cache_payload(&params)?;
    let key = CacheKey {
        tool_name: "hn_search",
        params_hash,
    };
    let ttl = toolkit.config().cache_ttls.hn_search;
    let base_url = toolkit.config().hn_base_url.clone();

    get_or_fetch_typed(toolkit, key, ttl, || async {
        crate::clients::hackernews::search(toolkit.http(), &base_url, &params).await
    })
    .await
}

pub(crate) async fn hn_get_thread(
    toolkit: &ResearchToolkit,
    params: HnGetThreadParams,
) -> Result<HnThread> {
    let params = normalize_thread_params(params);
    let params_hash = hash_cache_payload(&params)?;
    let key = CacheKey {
        tool_name: "hn_get_thread",
        params_hash,
    };
    let ttl = toolkit.config().cache_ttls.hn_search;
    let base_url = toolkit.config().hn_base_url.clone();

    get_or_fetch_typed(toolkit, key, ttl, || async {
        crate::clients::hackernews::get_item(toolkit.http(), &base_url, &params).await
    })
    .await
}

fn normalize_search_params(mut params: HnSearchParams) -> HnSearchParams {
    params.limit = Some(params.limit.unwrap_or(20).clamp(1, 100));
    params.offset = Some(params.offset.unwrap_or(0));
    if params.sort_by.is_none() {
        params.sort_by = Some("relevance".to_string());
    }
    params
}

fn normalize_thread_params(mut params: HnGetThreadParams) -> HnGetThreadParams {
    params.max_depth = Some(params.max_depth.unwrap_or(5).clamp(1, 20));
    params.max_comments = Some(params.max_comments.unwrap_or(200).clamp(1, 500));
    params
}
