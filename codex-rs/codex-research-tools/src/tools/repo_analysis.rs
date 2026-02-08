use serde::Deserialize;
use serde::Serialize;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::cache::FetchOutput;
use crate::clients::github;
use crate::clients::github::GitHubConfig;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::RepoHealth;

#[derive(Debug, Clone, Serialize)]
struct RepoHealthCacheKeyPayload {
    repo_id: String,
    auth_tier: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RepoHealthCacheEntry {
    Hit { health: RepoHealth },
    Miss { status: u16, message: String },
}

pub(crate) async fn repo_get_health(
    toolkit: &ResearchToolkit,
    repo_url: &str,
) -> Result<RepoHealth> {
    let repo_ref = github::normalize_repo_url(repo_url)?;
    let cache_key = repo_health_cache_key(toolkit, &repo_ref.normalized_id())?;

    let cached = toolkit
        .cache()
        .get_or_fetch_with_meta_ttls(
            cache_key,
            toolkit.config().cache_ttls.repo_health,
            toolkit.config().cache_ttls.negative,
            || async move {
                match github::get_repo_health(
                    toolkit.http(),
                    GitHubConfig {
                        api_base_url: &toolkit.config().github_api_base_url,
                        token: toolkit.config().github_token.as_deref(),
                    },
                    &repo_ref,
                )
                .await
                {
                    Ok(health) => {
                        serialize_cache_entry(RepoHealthCacheEntry::Hit { health }, false)
                    }
                    Err(error) if should_negative_cache(&error) => serialize_negative_error(error),
                    Err(error) => Err(error),
                }
            },
        )
        .await?;

    deserialize_cache_entry(cached)
}

fn repo_health_cache_key(toolkit: &ResearchToolkit, repo_id: &str) -> Result<CacheKey> {
    let auth_tier = if toolkit.config().github_token.is_some() {
        "authenticated"
    } else {
        "unauthenticated"
    };
    let payload = RepoHealthCacheKeyPayload {
        repo_id: repo_id.to_string(),
        auth_tier,
    };

    Ok(CacheKey {
        tool_name: "repo_get_health",
        params_hash: hash_cache_payload(&payload)?,
    })
}

fn should_negative_cache(error: &ResearchError) -> bool {
    matches!(
        error,
        ResearchError::Upstream {
            api: ResearchApi::GitHub,
            status,
            ..
        } if *status == reqwest::StatusCode::NOT_FOUND
    )
}

fn serialize_negative_error(error: ResearchError) -> Result<FetchOutput> {
    if let ResearchError::Upstream {
        status, message, ..
    } = error
    {
        return serialize_cache_entry(
            RepoHealthCacheEntry::Miss {
                status: status.as_u16(),
                message,
            },
            true,
        );
    }

    Err(ResearchError::Internal(
        "attempted to negative-cache a non-upstream repo health error".to_string(),
    ))
}

fn serialize_cache_entry(entry: RepoHealthCacheEntry, is_negative: bool) -> Result<FetchOutput> {
    let data = serde_json::to_value(entry).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to serialize repo health cache entry: {err}"
        ))
    })?;

    Ok(if is_negative {
        FetchOutput::negative(data)
    } else {
        FetchOutput::positive(data)
    })
}

fn deserialize_cache_entry(output: FetchOutput) -> Result<RepoHealth> {
    let entry: RepoHealthCacheEntry = serde_json::from_value(output.data).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to deserialize repo health cache entry: {err}"
        ))
    })?;

    match entry {
        RepoHealthCacheEntry::Hit { health } => Ok(health),
        RepoHealthCacheEntry::Miss { status, message } => {
            let status =
                reqwest::StatusCode::from_u16(status).unwrap_or(reqwest::StatusCode::NOT_FOUND);
            Err(ResearchError::Upstream {
                api: ResearchApi::GitHub,
                status,
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::query_param;

    use crate::ResearchToolkit;
    use crate::config::CacheTtls;
    use crate::config::ResearchConfig;
    use crate::error::ResearchError;
    use crate::tools::test_helpers::build_test_toolkit_with_config;

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_returns_expected_shape() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": { "spdx_id": "MIT" },
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 321,
                "open_issues_count": 14,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header(
                        "link",
                        "<https://api.github.com/repos/openai/codex/releases?per_page=1&page=4>; rel=\"last\"",
                    )
                    .set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "check_runs": [
                    { "status": "completed", "conclusion": "success" }
                ]
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), Some("test-token"));

        let health = toolkit
            .repo_get_health("https://github.com/openai/codex.git/tree/main")
            .await
            .expect("repo_get_health should succeed");

        assert_eq!(health.license, Some("MIT".to_string()));
        assert_eq!(
            health.last_commit_date,
            Some("2026-02-01T01:02:03Z".to_string())
        );
        assert_eq!(health.stars, 321);
        assert_eq!(health.open_issues, 14);
        assert_eq!(health.releases_count, 4);
        assert_eq!(health.ci_passing, Some(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_handles_missing_optional_signals() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": null,
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 100,
                "open_issues_count": 9,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let health = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect("repo_get_health should succeed");

        assert_eq!(health.license, None);
        assert_eq!(health.releases_count, 0);
        assert_eq!(health.ci_passing, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_surfaces_forbidden_check_runs() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": null,
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 100,
                "open_issues_count": 9,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Resource not accessible by integration"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let error = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect_err("forbidden check-runs should fail");

        assert!(
            matches!(
                error,
                ResearchError::Upstream {
                    api: crate::rate_limiter::ResearchApi::GitHub,
                    status,
                    ..
                } if status == reqwest::StatusCode::FORBIDDEN
            ),
            "unexpected error: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_rejects_non_github_urls() {
        let toolkit = build_test_toolkit("https://api.github.com".to_string(), None);
        let error = toolkit
            .repo_get_health("https://gitlab.com/openai/codex")
            .await
            .expect_err("non-github URL should fail");

        assert!(
            matches!(error, ResearchError::InvalidInput(_)),
            "expected invalid input, got {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_negative_caches_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Not Found"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);

        for _ in 0..2 {
            let error = toolkit
                .repo_get_health("https://github.com/openai/missing")
                .await
                .expect_err("missing repo should fail");
            assert!(
                matches!(
                    error,
                    ResearchError::Upstream {
                        api: crate::rate_limiter::ResearchApi::GitHub,
                        status,
                        ..
                    } if status == reqwest::StatusCode::NOT_FOUND
                ),
                "unexpected error: {error}"
            );
        }

        let requests = server
            .received_requests()
            .await
            .expect("request history should be available");
        let metadata_calls = requests
            .iter()
            .filter(|request| request.url.path() == "/repos/openai/missing")
            .count();
        assert_eq!(metadata_calls, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_negative_cache_respects_negative_ttl() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Not Found"
            })))
            .mount(&server)
            .await;

        let mut config = ResearchConfig {
            github_api_base_url: server.uri(),
            ..ResearchConfig::default()
        };
        config.cache_ttls = CacheTtls {
            repo_health: Duration::from_secs(60),
            negative: Duration::from_millis(20),
            ..config.cache_ttls
        };

        let toolkit = build_test_toolkit_with_config(config);

        let first = toolkit
            .repo_get_health("https://github.com/openai/missing")
            .await
            .expect_err("first missing repo request should fail");
        assert!(
            matches!(first, ResearchError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND),
            "unexpected error: {first}"
        );

        tokio::time::sleep(Duration::from_millis(35)).await;

        let second = toolkit
            .repo_get_health("https://github.com/openai/missing")
            .await
            .expect_err("second missing repo request should fail");
        assert!(
            matches!(second, ResearchError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND),
            "unexpected error: {second}"
        );

        let requests = server
            .received_requests()
            .await
            .expect("request history should be available");
        let metadata_calls = requests
            .iter()
            .filter(|request| request.url.path() == "/repos/openai/missing")
            .count();
        assert_eq!(metadata_calls, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_rate_limit_error_is_retryable() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "message": "API rate limit exceeded"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let error = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect_err("rate-limited request should fail");

        assert!(error.is_retryable(), "error should be retryable: {error}");
    }

    fn build_test_toolkit(
        github_api_base_url: String,
        github_token: Option<&str>,
    ) -> ResearchToolkit {
        build_test_toolkit_with_config(ResearchConfig {
            github_api_base_url,
            github_token: github_token.map(ToString::to_string),
            ..ResearchConfig::default()
        })
    }
}
