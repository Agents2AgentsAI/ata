use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::error::ResearchError;
use crate::error::Result;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::PatentDetail;
use crate::types::PatentGetParams;
use crate::types::PatentSearchParams;
use crate::types::PatentSearchResult;

pub(crate) async fn patent_search(
    toolkit: &ResearchToolkit,
    params: PatentSearchParams,
) -> Result<PatentSearchResult> {
    let auth = toolkit.epo_auth().ok_or_else(|| ResearchError::NotConfigured {
        tool: "patent_search",
        reason: "EPO_CONSUMER_KEY and EPO_CONSUMER_SECRET are required. Register at https://developers.epo.org".to_string(),
    })?;

    let params = normalize_search_params(params);
    let params_hash = hash_cache_payload(&params)?;
    let key = CacheKey {
        tool_name: "patent_search",
        params_hash,
    };
    let ttl = toolkit.config().cache_ttls.patent_search;
    let base_url = toolkit.config().patents_base_url.clone();

    get_or_fetch_typed(toolkit, key, ttl, || async {
        crate::clients::patents::search(toolkit.http(), &base_url, auth, &params).await
    })
    .await
}

pub(crate) async fn patent_get(
    toolkit: &ResearchToolkit,
    params: PatentGetParams,
) -> Result<PatentDetail> {
    let auth = toolkit.epo_auth().ok_or_else(|| ResearchError::NotConfigured {
        tool: "patent_get",
        reason: "EPO_CONSUMER_KEY and EPO_CONSUMER_SECRET are required. Register at https://developers.epo.org".to_string(),
    })?;

    let params_hash = hash_cache_payload(&params)?;
    let key = CacheKey {
        tool_name: "patent_get",
        params_hash,
    };
    let ttl = toolkit.config().cache_ttls.patent_search;
    let base_url = toolkit.config().patents_base_url.clone();

    get_or_fetch_typed(toolkit, key, ttl, || async {
        crate::clients::patents::get_patent(toolkit.http(), &base_url, auth, &params).await
    })
    .await
}

fn normalize_search_params(mut params: PatentSearchParams) -> PatentSearchParams {
    params.size = Some(params.size.unwrap_or(25).clamp(10, 100));
    if params.sort_by.is_none() {
        params.sort_by = Some("relevance".to_string());
    }
    params
}
