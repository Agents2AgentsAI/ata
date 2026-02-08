use std::collections::HashSet;

use serde::Serialize;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::clients::papers_with_code;
use crate::clients::papers_with_code::PapersWithCodeConfig;
use crate::clients::papers_with_code::PwcPaperIdentifier;
use crate::clients::papers_with_code::SotaSearchRequest;
use crate::clients::semantic_scholar;
use crate::clients::semantic_scholar::SemanticScholarConfig;
use crate::error::ResearchError;
use crate::error::Result;
use crate::paper_id::PaperId;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::CodeRepo;
use crate::types::Paper;
use crate::types::RepoListResult;
use crate::types::SotaResult;
use crate::types::SotaSearchParams;

#[derive(Debug, Clone, Serialize)]
struct NormalizedSotaSearchParams {
    task: String,
    dataset: Option<String>,
    limit: u32,
}

pub(crate) async fn paper_search_sota(
    toolkit: &ResearchToolkit,
    params: SotaSearchParams,
) -> Result<SotaResult> {
    let normalized = normalize_sota_search_params(params)?;
    let key = CacheKey {
        tool_name: "paper_search_sota",
        params_hash: hash_cache_payload(&normalized)?,
    };

    let entries = get_or_fetch_typed(toolkit, key, toolkit.config().cache_ttls.sota, || async {
        papers_with_code::search_sota(
            toolkit.http(),
            PapersWithCodeConfig {
                base_url: &toolkit.config().papers_with_code_base_url,
            },
            &SotaSearchRequest {
                task: &normalized.task,
                dataset: normalized.dataset.as_deref(),
                limit: normalized.limit,
            },
        )
        .await
    })
    .await?;

    Ok(SotaResult { entries })
}

pub(crate) async fn paper_find_repos(
    toolkit: &ResearchToolkit,
    paper_id: &str,
) -> Result<RepoListResult> {
    let parsed = paper_id
        .parse::<PaperId>()
        .map_err(ResearchError::InvalidInput)?;
    let key = CacheKey {
        tool_name: "paper_find_repos",
        params_hash: hash_cache_payload(&("paper_find_repos", &parsed.to_string()))?,
    };

    let repos = get_or_fetch_typed(toolkit, key, toolkit.config().cache_ttls.sota, || async {
        find_repos_uncached(toolkit, parsed).await
    })
    .await?;

    Ok(RepoListResult { repos })
}

async fn find_repos_uncached(toolkit: &ResearchToolkit, parsed: PaperId) -> Result<Vec<CodeRepo>> {
    let mut repos = match parsed {
        PaperId::ArxivId(arxiv_id) => {
            find_repos_for_identifier(
                toolkit,
                PwcPaperIdentifier::ArxivId(&arxiv_id),
                Some(format!("arxiv:{arxiv_id}")),
            )
            .await?
        }
        PaperId::Doi(doi) => {
            find_repos_for_identifier(
                toolkit,
                PwcPaperIdentifier::Doi(&doi),
                Some(format!("doi:{doi}")),
            )
            .await?
        }
        PaperId::S2Id(s2_id) => {
            let paper = semantic_scholar::get_paper(
                toolkit.http(),
                SemanticScholarConfig {
                    base_url: &toolkit.config().semantic_scholar_base_url,
                    api_key: toolkit.config().semantic_scholar_api_key.as_deref(),
                },
                &s2_id,
                false,
            )
            .await?;

            repos_for_paper_metadata(toolkit, &paper).await?
        }
        PaperId::OpenAlexId(openalex_id) => {
            return Err(ResearchError::InvalidInput(format!(
                "paper_find_repos does not support openalex id '{openalex_id}' directly; provide DOI or arXiv id"
            )));
        }
    };

    dedup_repos(&mut repos);
    Ok(repos)
}

async fn repos_for_paper_metadata(
    toolkit: &ResearchToolkit,
    paper: &Paper,
) -> Result<Vec<CodeRepo>> {
    if let Some(arxiv_id) = paper.arxiv_id.as_deref() {
        let repos = find_repos_for_identifier(
            toolkit,
            PwcPaperIdentifier::ArxivId(arxiv_id),
            Some(format!("arxiv:{arxiv_id}")),
        )
        .await?;
        if !repos.is_empty() {
            return Ok(repos);
        }
    }

    if let Some(doi) = paper.doi.as_deref() {
        return find_repos_for_identifier(
            toolkit,
            PwcPaperIdentifier::Doi(doi),
            Some(format!("doi:{doi}")),
        )
        .await;
    }

    Ok(Vec::new())
}

async fn find_repos_for_identifier(
    toolkit: &ResearchToolkit,
    identifier: PwcPaperIdentifier<'_>,
    canonical_id: Option<String>,
) -> Result<Vec<CodeRepo>> {
    papers_with_code::find_repos_by_identifier(
        toolkit.http(),
        PapersWithCodeConfig {
            base_url: &toolkit.config().papers_with_code_base_url,
        },
        &identifier,
        canonical_id,
    )
    .await
}

fn normalize_sota_search_params(params: SotaSearchParams) -> Result<NormalizedSotaSearchParams> {
    let task = params.task.trim().to_string();
    if task.is_empty() {
        return Err(ResearchError::InvalidInput(
            "paper_search_sota task must not be empty".to_string(),
        ));
    }

    Ok(NormalizedSotaSearchParams {
        task,
        dataset: params.dataset,
        limit: params.limit.unwrap_or(20).clamp(1, 50),
    })
}

fn dedup_repos(repos: &mut Vec<CodeRepo>) {
    let mut seen = HashSet::new();
    repos.retain(|repo| seen.insert(repo.url.to_ascii_lowercase()));
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    use crate::ResearchToolkit;
    use crate::config::ResearchConfig;
    use crate::tools::test_helpers::build_test_toolkit_with_config;
    use crate::types::SotaSearchParams;

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_search_sota_filters_dataset_and_returns_entries() {
        let pwc_server = MockServer::start().await;
        let s2_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/tasks/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "task-1", "name": "Object Detection"}
                ]
            })))
            .mount(&pwc_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/evaluations/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "task": {"name": "Object Detection"},
                        "dataset": {"name": "COCO"},
                        "metric": "mAP",
                        "best_method": "Method A",
                        "best_score": 64.2,
                        "paper_title": "Paper A",
                        "paper_url": "https://paper-a",
                        "code_url": "https://repo-a"
                    },
                    {
                        "task": {"name": "Object Detection"},
                        "dataset": {"name": "ImageNet"},
                        "metric": "top1",
                        "best_method": "Method B",
                        "best_score": 82.0,
                        "paper_title": "Paper B",
                        "paper_url": "https://paper-b",
                        "code_url": "https://repo-b"
                    }
                ]
            })))
            .mount(&pwc_server)
            .await;

        let toolkit = build_test_toolkit(s2_server.uri(), pwc_server.uri());
        let result = toolkit
            .paper_search_sota(SotaSearchParams {
                task: "Object Detection".to_string(),
                dataset: Some("COCO".to_string()),
                limit: Some(10),
            })
            .await
            .expect("paper_search_sota should succeed");

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].dataset, "COCO");
        assert_eq!(result.entries[0].best_method, "Method A");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paper_find_repos_supports_s2_fallback_to_arxiv() {
        let pwc_server = MockServer::start().await;
        let s2_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/graph/v1/paper/s2-id-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "paperId": "s2-id-1",
                "title": "S2 Paper",
                "year": 2023,
                "externalIds": {"ArXiv": "2301.12345", "DOI": "10.1000/x"},
                "authors": [{"name": "Alice"}]
            })))
            .mount(&s2_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/papers/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {"id": "pwc-paper-1"}
                ]
            })))
            .mount(&pwc_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/papers/pwc-paper-1/repositories/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "url": "https://github.com/org/repo",
                        "framework": "pytorch",
                        "stars": 123,
                        "is_official": true,
                        "description": "Official impl"
                    },
                    {
                        "repository_url": "https://github.com/org/repo"
                    }
                ]
            })))
            .mount(&pwc_server)
            .await;

        let toolkit = build_test_toolkit(format!("{}/graph/v1", s2_server.uri()), pwc_server.uri());

        let result = toolkit
            .paper_find_repos("s2:s2-id-1")
            .await
            .expect("paper_find_repos should succeed for s2 fallback");

        assert_eq!(result.repos.len(), 1);
        assert_eq!(result.repos[0].url, "https://github.com/org/repo");
        assert_eq!(result.repos[0].framework, Some("pytorch".to_string()));
    }

    fn build_test_toolkit(
        semantic_scholar_base_url: String,
        papers_with_code_base_url: String,
    ) -> ResearchToolkit {
        build_test_toolkit_with_config(ResearchConfig {
            semantic_scholar_base_url,
            papers_with_code_base_url,
            ..ResearchConfig::default()
        })
    }
}
