use reqwest::header::HeaderMap;
use serde::Deserialize;

use crate::error::ResearchError;
use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::types::RepoHealth;

const GITHUB_HOST: &str = "github.com";
const GITHUB_USER_AGENT: &str = "codex-research-tools";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";
const GITHUB_API_VERSION: &str = "2022-11-28";

#[derive(Debug, Clone, Copy)]
pub(crate) struct GitHubConfig<'a> {
    pub api_base_url: &'a str,
    pub token: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct GitHubRepoRef {
    pub owner: String,
    pub repo: String,
}

impl GitHubRepoRef {
    #[must_use]
    pub fn normalized_id(&self) -> String {
        format!(
            "{}/{}",
            self.owner.to_ascii_lowercase(),
            self.repo.to_ascii_lowercase()
        )
    }
}

pub(crate) fn normalize_repo_url(repo_url: &str) -> Result<GitHubRepoRef> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        return Err(ResearchError::InvalidInput(
            "repo_url must not be empty".to_string(),
        ));
    }

    let parsed = reqwest::Url::parse(trimmed).map_err(|err| {
        ResearchError::InvalidInput(format!(
            "invalid repo_url '{trimmed}': {err}. expected https://github.com/{{owner}}/{{repo}}"
        ))
    })?;

    if parsed.scheme() != "https" {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported repo_url scheme '{}' (expected https)",
            parsed.scheme()
        )));
    }

    let Some(host) = parsed.host_str() else {
        return Err(ResearchError::InvalidInput(
            "repo_url is missing a host".to_string(),
        ));
    };

    if !host.eq_ignore_ascii_case(GITHUB_HOST) {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported repo host '{host}' (expected {GITHUB_HOST})"
        )));
    }

    let mut segments = parsed.path_segments().ok_or_else(|| {
        ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' does not contain owner/repo path segments"
        ))
    })?;
    let owner = segments.next().map(str::trim).unwrap_or_default();
    let repo_segment = segments.next().map(str::trim).unwrap_or_default();

    if owner.is_empty() || repo_segment.is_empty() {
        return Err(ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' must include owner and repo segments"
        )));
    }

    let repo = repo_segment
        .strip_suffix(".git")
        .unwrap_or(repo_segment)
        .trim();
    if repo.is_empty() {
        return Err(ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' contains an empty repository name"
        )));
    }

    Ok(GitHubRepoRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

pub(crate) async fn get_repo_health(
    http: &HttpClient,
    config: GitHubConfig<'_>,
    repo_ref: &GitHubRepoRef,
) -> Result<RepoHealth> {
    let repo_api_url = format!(
        "{}/repos/{}/{}",
        config.api_base_url.trim_end_matches('/'),
        repo_ref.owner,
        repo_ref.repo
    );
    let metadata: GitHubRepoResponse = http
        .execute_json(ResearchApi::GitHub, || {
            github_request(http, config, &repo_api_url)
        })
        .await?;

    let releases_count = fetch_releases_count(http, config, &repo_api_url).await?;
    let ci_passing =
        fetch_ci_passing(http, config, &repo_api_url, &metadata.default_branch).await?;

    Ok(RepoHealth {
        license: map_license(metadata.license),
        last_commit_date: metadata.pushed_at,
        stars: metadata.stargazers_count,
        open_issues: metadata.open_issues_count,
        releases_count,
        ci_passing,
    })
}

fn github_request(
    http: &HttpClient,
    config: GitHubConfig<'_>,
    url: &str,
) -> reqwest::RequestBuilder {
    let mut request = http
        .client()
        .get(url)
        .header("user-agent", GITHUB_USER_AGENT)
        .header("accept", GITHUB_ACCEPT)
        .header("x-github-api-version", GITHUB_API_VERSION);

    if let Some(token) = config.token {
        request = request.bearer_auth(token);
    }

    request
}

async fn fetch_releases_count(
    http: &HttpClient,
    config: GitHubConfig<'_>,
    repo_api_url: &str,
) -> Result<u32> {
    let releases_url = format!("{repo_api_url}/releases?per_page=1");
    let response = http
        .execute_response(ResearchApi::GitHub, || {
            github_request(http, config, &releases_url)
        })
        .await?;

    if let Some(last_page) = releases_count_from_link_header(response.headers()) {
        return Ok(last_page);
    }

    let body: Vec<serde_json::Value> =
        response.json().await.map_err(|err| ResearchError::Parse {
            api: ResearchApi::GitHub,
            message: err.to_string(),
        })?;
    Ok(u32::try_from(body.len()).unwrap_or(u32::MAX))
}

fn releases_count_from_link_header(headers: &HeaderMap) -> Option<u32> {
    let raw = headers.get("link")?.to_str().ok()?;
    parse_last_page_from_link_header(raw)
}

fn parse_last_page_from_link_header(raw: &str) -> Option<u32> {
    for part in raw.split(',') {
        let trimmed = part.trim();
        if !trimmed.contains("rel=\"last\"") {
            continue;
        }

        let start = trimmed.find('<')?;
        let end = trimmed.find('>')?;
        if end <= start + 1 {
            continue;
        }
        let url = &trimmed[start + 1..end];
        let parsed = reqwest::Url::parse(url).ok()?;
        let page = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "page").then_some(value))?
            .parse::<u32>()
            .ok()?;
        return Some(page);
    }

    None
}

async fn fetch_ci_passing(
    http: &HttpClient,
    config: GitHubConfig<'_>,
    repo_api_url: &str,
    default_branch: &Option<String>,
) -> Result<Option<bool>> {
    let branch = default_branch
        .as_deref()
        .filter(|branch| !branch.is_empty());
    let Some(branch) = branch else {
        return Ok(None);
    };
    let url = format!(
        "{repo_api_url}/commits/{branch}/check-runs?per_page=1",
        branch = urlencoding::encode(branch),
    );

    let checks: GitHubCheckRunsResponse = match http
        .execute_json(ResearchApi::GitHub, || github_request(http, config, &url))
        .await
    {
        Ok(checks) => checks,
        Err(ResearchError::Upstream {
            status: reqwest::StatusCode::NOT_FOUND,
            ..
        }) => return Ok(None),
        Err(err) => {
            tracing::debug!(repo_api_url, error = %err, "github check-runs request failed");
            return Err(err);
        }
    };

    Ok(checks.check_runs.first().and_then(classify_ci_check_run))
}

fn classify_ci_check_run(run: &GitHubCheckRun) -> Option<bool> {
    if let Some(conclusion) = run.conclusion.as_deref() {
        return Some(conclusion.eq_ignore_ascii_case("success"));
    }

    if let Some(status) = run.status.as_deref()
        && status.eq_ignore_ascii_case("completed")
    {
        return Some(false);
    }

    None
}

fn map_license(license: Option<GitHubLicense>) -> Option<String> {
    let candidate = license
        .and_then(|license| license.spdx_id.or(license.name))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;

    if candidate.eq_ignore_ascii_case("noassertion") {
        return None;
    }

    Some(candidate)
}

#[derive(Debug, Deserialize)]
struct GitHubRepoResponse {
    license: Option<GitHubLicense>,
    pushed_at: Option<String>,
    stargazers_count: u32,
    open_issues_count: u32,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubLicense {
    spdx_id: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubCheckRunsResponse {
    #[serde(default)]
    check_runs: Vec<GitHubCheckRun>,
}

#[derive(Debug, Deserialize)]
struct GitHubCheckRun {
    status: Option<String>,
    conclusion: Option<String>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::GitHubRepoRef;
    use super::normalize_repo_url;
    use super::parse_last_page_from_link_header;

    #[test]
    fn normalize_repo_url_accepts_common_variants() {
        let cases = [
            (
                "https://github.com/Agents2AgentsAI/ata",
                GitHubRepoRef {
                    owner: "Agents2AgentsAI".to_string(),
                    repo: "ata".to_string(),
                },
            ),
            (
                "https://github.com/Agents2AgentsAI/ata.git",
                GitHubRepoRef {
                    owner: "Agents2AgentsAI".to_string(),
                    repo: "ata".to_string(),
                },
            ),
            (
                "https://github.com/Agents2AgentsAI/ata/tree/main",
                GitHubRepoRef {
                    owner: "Agents2AgentsAI".to_string(),
                    repo: "ata".to_string(),
                },
            ),
            (
                "https://github.com/Agents2AgentsAI/Ata.git/issues/1",
                GitHubRepoRef {
                    owner: "Agents2AgentsAI".to_string(),
                    repo: "Ata".to_string(),
                },
            ),
        ];

        for (input, expected) in cases {
            let parsed = normalize_repo_url(input).expect("url should parse");
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn normalize_repo_url_rejects_unsupported_values() {
        let cases = [
            "",
            "github.com/Agents2AgentsAI/ata",
            "http://github.com/Agents2AgentsAI/ata",
            "https://gitlab.com/Agents2AgentsAI/ata",
            "https://github.com/Agents2AgentsAI",
            "https://github.com/Agents2AgentsAI/",
            "git@github.com:Agents2AgentsAI/ata.git",
        ];

        for input in cases {
            let error = normalize_repo_url(input).expect_err("url should fail");
            assert!(
                matches!(error, crate::error::ResearchError::InvalidInput(_)),
                "unexpected error for {input}: {error}"
            );
        }
    }

    #[test]
    fn parse_last_page_from_link_header_extracts_page_number() {
        let raw = "<https://api.github.com/repos/Agents2AgentsAI/ata/releases?per_page=1&page=2>; rel=\"next\", <https://api.github.com/repos/Agents2AgentsAI/ata/releases?per_page=1&page=17>; rel=\"last\"";
        assert_eq!(parse_last_page_from_link_header(raw), Some(17));
    }

    #[test]
    fn parse_last_page_from_link_header_returns_none_without_last_rel() {
        let raw = "<https://api.github.com/repos/Agents2AgentsAI/ata/releases?per_page=1&page=2>; rel=\"next\"";
        assert_eq!(parse_last_page_from_link_header(raw), None);
    }
}
