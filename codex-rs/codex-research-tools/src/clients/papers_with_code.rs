use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::error::Result;
use crate::http_client::HttpClient;
use crate::rate_limiter::ResearchApi;
use crate::types::CodeRepo;
use crate::types::SotaEntry;
use crate::types::SourceMeta;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PapersWithCodeConfig<'a> {
    pub base_url: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct SotaSearchRequest<'a> {
    pub task: &'a str,
    pub dataset: Option<&'a str>,
    pub limit: u32,
}

pub(crate) async fn search_sota(
    http: &HttpClient,
    config: PapersWithCodeConfig<'_>,
    request: &SotaSearchRequest<'_>,
) -> Result<Vec<SotaEntry>> {
    let task_url = format!(
        "{base_url}/tasks/?q={task}",
        base_url = config.base_url,
        task = urlencoding::encode(request.task),
    );
    let tasks: PwcTaskList = http
        .execute_json(ResearchApi::PapersWithCode, || http.client().get(&task_url))
        .await?;

    let selected_task = select_task(tasks.results, request.task);
    let Some(task) = selected_task else {
        return Ok(Vec::new());
    };

    let evaluations_url = format!(
        "{base_url}/evaluations/?task={task_id}",
        base_url = config.base_url,
        task_id = urlencoding::encode(&task.id),
    );

    let evaluations: PwcEvaluationList = http
        .execute_json(ResearchApi::PapersWithCode, || {
            http.client().get(&evaluations_url)
        })
        .await?;

    let dataset_filter = request.dataset.map(str::to_ascii_lowercase);

    let mut entries = evaluations
        .results
        .into_iter()
        .filter_map(|evaluation| {
            let dataset_name = evaluation
                .dataset
                .as_ref()
                .and_then(|dataset| dataset.name.clone())
                .or(evaluation.dataset_name)
                .unwrap_or_else(|| "Unknown Dataset".to_string());

            if let Some(filter) = dataset_filter.as_ref()
                && !dataset_name.to_ascii_lowercase().contains(filter)
            {
                return None;
            }

            let task_name = evaluation
                .task
                .as_ref()
                .and_then(|task| task.name.clone())
                .unwrap_or_else(|| {
                    task.name
                        .clone()
                        .unwrap_or_else(|| request.task.to_string())
                });

            let metric = evaluation
                .metric
                .clone()
                .or_else(|| {
                    evaluation
                        .sota
                        .as_ref()
                        .and_then(|sota| sota.metric.clone())
                })
                .unwrap_or_else(|| "Unknown Metric".to_string());

            let best_method = evaluation
                .best_method
                .as_ref()
                .and_then(value_to_string)
                .or_else(|| {
                    evaluation
                        .sota
                        .as_ref()
                        .and_then(|sota| sota.model_name.clone())
                })
                .unwrap_or_else(|| "Unknown Method".to_string());

            let best_score = evaluation
                .best_score
                .as_ref()
                .and_then(value_to_f64)
                .or_else(|| {
                    evaluation
                        .sota
                        .as_ref()
                        .and_then(|sota| sota.value.as_ref())
                        .and_then(value_to_f64)
                });

            let paper_title = evaluation.paper_title.clone().or_else(|| {
                evaluation
                    .sota
                    .as_ref()
                    .and_then(|sota| sota.paper.as_ref())
                    .and_then(|paper| paper.title.clone())
            });

            let paper_url = evaluation.paper_url.clone().or_else(|| {
                evaluation
                    .sota
                    .as_ref()
                    .and_then(|sota| sota.paper.as_ref())
                    .and_then(|paper| paper.url.clone())
            });

            let code_url = evaluation.code_url.clone().or_else(|| {
                evaluation
                    .sota
                    .as_ref()
                    .and_then(|sota| sota.code_url.clone())
            });

            Some(SotaEntry {
                task: task_name,
                dataset: dataset_name,
                metric,
                best_method,
                best_score,
                paper_title,
                paper_url,
                code_url,
                source_meta: Some(SourceMeta {
                    source: "papers_with_code".to_string(),
                    api_url: evaluations_url.clone(),
                    fetched_at: Utc::now(),
                    canonical_id: None,
                }),
            })
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .best_score
            .unwrap_or(f64::MIN)
            .partial_cmp(&left.best_score.unwrap_or(f64::MIN))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.task.cmp(&right.task))
    });

    entries.truncate(request.limit as usize);
    Ok(entries)
}

pub(crate) async fn find_repos_by_identifier(
    http: &HttpClient,
    config: PapersWithCodeConfig<'_>,
    identifier: &PwcPaperIdentifier<'_>,
    canonical_id: Option<String>,
) -> Result<Vec<CodeRepo>> {
    let mut query_url = match identifier {
        PwcPaperIdentifier::ArxivId(arxiv_id) => format!(
            "{base_url}/papers/?arxiv_id={arxiv_id}",
            base_url = config.base_url,
            arxiv_id = urlencoding::encode(arxiv_id),
        ),
        PwcPaperIdentifier::Doi(doi) => format!(
            "{base_url}/papers/?doi={doi}",
            base_url = config.base_url,
            doi = urlencoding::encode(doi),
        ),
    };
    query_url.push_str("&items_per_page=50");

    let papers: PwcPaperList = http
        .execute_json(ResearchApi::PapersWithCode, || {
            http.client().get(&query_url)
        })
        .await?;

    let mut repos = Vec::new();
    for paper in papers.results {
        let repos_url = format!(
            "{base_url}/papers/{paper_id}/repositories/",
            base_url = config.base_url,
            paper_id = urlencoding::encode(&paper.id),
        );

        let repo_response: PwcRepositoryList = http
            .execute_json(ResearchApi::PapersWithCode, || {
                http.client().get(&repos_url)
            })
            .await?;

        for repo in repo_response.results {
            let Some(url) = repo.url() else {
                continue;
            };

            repos.push(CodeRepo {
                url,
                framework: repo.framework,
                stars: repo.stars,
                is_official: repo.is_official,
                description: repo.description,
                source_meta: Some(SourceMeta {
                    source: "papers_with_code".to_string(),
                    api_url: repos_url.clone(),
                    fetched_at: Utc::now(),
                    canonical_id: canonical_id.clone(),
                }),
            });
        }
    }

    Ok(repos)
}

fn select_task(tasks: Vec<PwcTask>, requested_task: &str) -> Option<PwcTask> {
    let requested = requested_task.trim().to_ascii_lowercase();
    if requested.is_empty() {
        return None;
    }

    tasks
        .into_iter()
        .filter_map(|task| {
            let name = task
                .name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let is_partial_match =
                !name.is_empty() && (name.contains(&requested) || requested.contains(&name));
            let score = if name == requested {
                3
            } else if is_partial_match {
                2
            } else {
                0
            };
            (score > 0).then_some((task, score))
        })
        .max_by_key(|(_, score)| *score)
        .map(|(task, _)| task)
}

fn value_to_string(value: &Value) -> Option<String> {
    if let Some(raw) = value.as_str() {
        return Some(raw.to_string());
    }

    value
        .get("name")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn value_to_f64(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return Some(number);
    }

    if let Some(raw) = value.as_str() {
        return raw.parse::<f64>().ok();
    }

    value.get("value").and_then(Value::as_f64).or_else(|| {
        value
            .get("value")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse::<f64>().ok())
    })
}

#[derive(Debug, Clone)]
pub(crate) enum PwcPaperIdentifier<'a> {
    ArxivId(&'a str),
    Doi(&'a str),
}

#[derive(Debug, Deserialize)]
struct PwcTaskList {
    #[serde(default)]
    results: Vec<PwcTask>,
}

#[derive(Debug, Deserialize)]
struct PwcEvaluationList {
    #[serde(default)]
    results: Vec<PwcEvaluation>,
}

#[derive(Debug, Deserialize)]
struct PwcPaperList {
    #[serde(default)]
    results: Vec<PwcPaperRecord>,
}

#[derive(Debug, Deserialize)]
struct PwcRepositoryList {
    #[serde(default)]
    results: Vec<PwcRepository>,
}

#[derive(Debug, Deserialize)]
struct PwcTask {
    id: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PwcEvaluation {
    task: Option<PwcNamed>,
    dataset: Option<PwcNamed>,
    #[serde(default)]
    dataset_name: Option<String>,
    metric: Option<String>,
    #[serde(default)]
    best_method: Option<Value>,
    #[serde(default)]
    best_score: Option<Value>,
    #[serde(default)]
    paper_title: Option<String>,
    #[serde(default)]
    paper_url: Option<String>,
    #[serde(default)]
    code_url: Option<String>,
    #[serde(default)]
    sota: Option<PwcSota>,
}

#[derive(Debug, Deserialize)]
struct PwcNamed {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PwcSota {
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    code_url: Option<String>,
    #[serde(default)]
    paper: Option<PwcPaperSummary>,
}

#[derive(Debug, Deserialize)]
struct PwcPaperSummary {
    title: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PwcPaperRecord {
    id: String,
}

#[derive(Debug, Deserialize)]
struct PwcRepository {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    repository_url: Option<String>,
    #[serde(default)]
    repo_url: Option<String>,
    #[serde(default)]
    github_url: Option<String>,
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    stars: Option<u32>,
    #[serde(default)]
    is_official: Option<bool>,
    #[serde(default)]
    description: Option<String>,
}

impl PwcRepository {
    fn url(&self) -> Option<String> {
        self.url
            .clone()
            .or(self.repository_url.clone())
            .or(self.repo_url.clone())
            .or(self.github_url.clone())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::PwcTask;
    use super::select_task;

    #[test]
    fn select_task_returns_none_when_no_match_exists() {
        let selected = select_task(
            vec![
                PwcTask {
                    id: "task-1".to_string(),
                    name: Some("Machine Translation".to_string()),
                },
                PwcTask {
                    id: "task-2".to_string(),
                    name: Some("Image Classification".to_string()),
                },
            ],
            "Object Detection",
        );

        assert_eq!(selected.map(|task| task.id), None);
    }

    #[test]
    fn select_task_prefers_exact_match() {
        let selected = select_task(
            vec![
                PwcTask {
                    id: "task-1".to_string(),
                    name: Some("Object".to_string()),
                },
                PwcTask {
                    id: "task-2".to_string(),
                    name: Some("Object Detection".to_string()),
                },
            ],
            "Object Detection",
        );

        assert_eq!(selected.map(|task| task.id), Some("task-2".to_string()));
    }
}
