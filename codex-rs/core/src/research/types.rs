use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;
use serde::de;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResearchOutput {
    pub problem_decomposition: ProblemDecomposition,
    pub literature_review: LiteratureReview,
    pub proposals: Vec<Proposal>,
    pub comparison_table: Option<String>,
    pub champion: Champion,
    pub decision_rules: Option<Vec<DecisionRule>>,
    pub iteration_context: Option<IterationContext>,
    pub next_steps: Option<Vec<String>>,
    pub open_questions: Option<Vec<String>>,
    pub run_warnings: Option<Vec<String>>,
    pub file_manifest: Option<Vec<FileManifestEntry>>,
    pub metadata: Option<ResearchMetadata>,
}

impl<'de> Deserialize<'de> for ResearchOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = ResearchOutputRaw::deserialize(deserializer)?;
        let proposals = if raw.proposals.is_empty() {
            raw.solutions
        } else {
            raw.proposals
        };
        if proposals.is_empty() {
            return Err(de::Error::custom(
                "missing `proposals` (or deprecated `solutions`)",
            ));
        }

        let champion = raw
            .champion
            .or_else(|| {
                raw.recommended_solution
                    .map(Champion::from_recommended_solution)
            })
            .ok_or_else(|| {
                de::Error::custom("missing `champion` (or deprecated `recommended_solution`)")
            })?;

        Ok(Self {
            problem_decomposition: raw.problem_decomposition,
            literature_review: raw.literature_review,
            proposals,
            comparison_table: raw.comparison_table,
            champion,
            decision_rules: raw.decision_rules,
            iteration_context: raw.iteration_context,
            next_steps: raw.next_steps,
            open_questions: raw.open_questions,
            run_warnings: raw.run_warnings,
            file_manifest: raw.file_manifest,
            metadata: raw.metadata,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ResearchOutputRaw {
    pub problem_decomposition: ProblemDecomposition,
    pub literature_review: LiteratureReview,
    #[serde(default)]
    pub proposals: Vec<Proposal>,
    #[serde(default)]
    pub solutions: Vec<Proposal>,
    pub comparison_table: Option<String>,
    pub champion: Option<Champion>,
    pub recommended_solution: Option<String>,
    pub decision_rules: Option<Vec<DecisionRule>>,
    pub iteration_context: Option<IterationContext>,
    pub next_steps: Option<Vec<String>>,
    pub open_questions: Option<Vec<String>>,
    pub run_warnings: Option<Vec<String>>,
    pub file_manifest: Option<Vec<FileManifestEntry>>,
    pub metadata: Option<ResearchMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Champion {
    pub proposal_name: String,
    pub justification: String,
    pub promotion_criteria: Option<String>,
}

impl Champion {
    #[must_use]
    pub fn from_recommended_solution(proposal_name: String) -> Self {
        Self {
            proposal_name,
            justification:
                "Migrated from deprecated `recommended_solution`; no explicit justification provided."
                    .to_string(),
            promotion_criteria: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRule {
    pub condition: String,
    pub action: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IterationContext {
    pub iteration_number: u32,
    pub prior_proposal_ids: Option<Vec<String>>,
    pub feedback_received: Option<Vec<FeedbackEntry>>,
    pub changes_from_prior: Option<Vec<String>>,
    pub dead_approaches: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedbackEntry {
    pub source_agent: String,
    pub feedback_type: String,
    pub summary: String,
    pub affected_proposals: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileManifestEntry {
    pub path: String,
    pub kind: String,
    pub solution_rank: Option<u32>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProblemDecomposition {
    pub problem_statement: String,
    pub core_challenges: Vec<Challenge>,
    pub constraints: Vec<Constraint>,
    pub scope: Option<Scope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Challenge {
    pub name: String,
    pub description: String,
    pub domains: Option<Vec<String>>,
    pub search_keywords: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub category: String,
    pub description: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    pub in_scope: Option<Vec<String>>,
    pub out_of_scope: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteratureReview {
    pub total_papers_found: u32,
    pub papers_by_id: HashMap<String, Paper>,
    pub paper_ids_ranked: Option<Vec<String>>,
    pub papers: Option<Vec<Paper>>,
    pub sota_results: Option<Vec<SotaResult>>,
    pub code_repositories: Option<Vec<CodeRepository>>,
    pub key_insights: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paper {
    pub paper_id: Option<String>,
    pub title: String,
    pub authors: String,
    pub year: Option<u32>,
    pub venue: Option<String>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub code_url: Option<String>,
    pub citation_count: Option<u32>,
    pub abstract_text: Option<String>,
    pub relevance_score: Option<u32>,
    pub key_contribution: Option<String>,
    pub relevant_to_subproblems: Option<Vec<String>>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SotaResult {
    pub task: String,
    pub dataset: String,
    pub metric: String,
    pub best_method: String,
    pub best_score: Option<f64>,
    pub paper_title: Option<String>,
    pub paper_url: Option<String>,
    pub code_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeRepository {
    pub url: String,
    pub name: String,
    pub framework: Option<String>,
    pub stars: Option<u32>,
    pub is_official: Option<bool>,
    pub related_paper: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub rank: Option<u32>,
    pub name: String,
    pub status: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub key_papers: Vec<SolutionPaper>,
    pub technical_approach: Option<TechnicalApproach>,
    pub justification: Option<String>,
    pub acceptance_criteria: Option<Vec<AcceptanceCriterion>>,
    pub experiment_plan: Option<ExperimentPlan>,
    pub risk_register: Option<Vec<RiskEntry>>,
    pub instrumentation_requirements: Option<InstrumentationRequirements>,
    pub deployment_constraints: Option<DeploymentConstraints>,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub weaknesses: Vec<String>,
    pub failure_modes: Option<Vec<String>>,
    pub data_requirements: Option<DataRequirements>,
    pub compute_requirements: Option<ComputeRequirements>,
    pub expertise_required: Option<Vec<String>>,
    pub risk_assessment: Option<RiskAssessment>,
    pub evidence: Option<Vec<Evidence>>,
    pub comparison_to_alternatives: Option<String>,
    pub implementation_files: Option<Vec<String>>,
    pub implementation_packet: Option<ImplementationPacket>,
    pub repo_assessments: Option<Vec<RepoAssessment>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub metric: String,
    pub threshold: String,
    pub priority: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentPlan {
    pub dataset_subset: Option<String>,
    pub expected_training_time: Option<String>,
    pub key_metric: Option<String>,
    pub expected_range: Option<String>,
    pub go_no_go: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskEntry {
    pub risk: String,
    pub likelihood: String,
    pub impact: String,
    pub mitigation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentationRequirements {
    pub training_metrics: Option<Vec<String>>,
    pub deployment_metrics: Option<Vec<String>>,
    pub alerting_thresholds: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeploymentConstraints {
    pub target_hardware: Option<String>,
    pub model_format: Option<String>,
    pub runtime: Option<String>,
    pub memory_budget: Option<String>,
    pub rollback_strategy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub r#type: String,
    pub source_tool: Option<String>,
    pub source_ref: String,
    pub extracted_text_snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolutionPaper {
    pub citation: String,
    pub title: String,
    pub url: Option<String>,
    pub relevance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TechnicalApproach {
    pub architecture: Option<String>,
    pub training_procedure: Option<String>,
    pub inference_pipeline: Option<String>,
    pub integration_approach: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataRequirements {
    pub description: Option<String>,
    pub estimated_volume: Option<String>,
    pub availability: Option<String>,
    pub labeling_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeRequirements {
    pub training_gpu: Option<String>,
    pub training_time: Option<String>,
    pub inference_device: Option<String>,
    pub inference_latency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub level: String,
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoAssessment {
    pub repo_url: String,
    pub commit_sha: Option<String>,
    pub license: Option<String>,
    pub health: Option<RepoHealthSummary>,
    pub entrypoints: Option<Vec<RepoEntrypoint>>,
    pub io_shapes: Option<RepoIOShapes>,
    pub dependencies: Option<Vec<String>>,
    pub config_schema: Option<RepoConfigSchema>,
    pub export_paths: Option<Vec<RepoExportPath>>,
    pub reproduction_feasibility: Option<ReproductionFeasibility>,
    pub caveats: Option<Vec<String>>,
    pub files_to_ignore: Option<Vec<String>>,
    pub repo_unavailable: Option<bool>,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoHealthSummary {
    pub stars: Option<u32>,
    pub last_commit_date: Option<String>,
    pub open_issues: Option<u32>,
    pub releases_count: Option<u32>,
    pub ci_passing: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoEntrypoint {
    pub file_path: String,
    pub kind: String,
    pub cli_args_summary: Option<String>,
    pub config_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoIOShapes {
    pub input: Option<String>,
    pub output: Option<String>,
    pub dtype: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoConfigSchema {
    pub file: Option<String>,
    pub format: Option<String>,
    pub key_params: Option<Vec<RepoConfigParam>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoConfigParam {
    pub name: String,
    pub default: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoExportPath {
    pub file: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReproductionFeasibility {
    pub has_readme_instructions: Option<bool>,
    pub has_docker_or_env: Option<bool>,
    pub has_pretrained_weights: Option<bool>,
    pub has_test_suite: Option<bool>,
    pub estimated_repro_effort: Option<String>,
    pub blockers: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplementationPacket {
    pub adoption_strategy: AdoptionStrategy,
    pub repo_provenance: Vec<RepoProvenance>,
    pub repro_plan: Option<ReproPlan>,
    pub integration_plan: Option<IntegrationPlan>,
    pub codebase_fit: Option<CodebaseFit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdoptionStrategy {
    pub strategy: String,
    pub rationale: String,
    pub source_repos: Option<Vec<SourceRepo>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRepo {
    pub url: String,
    pub commit: Option<String>,
    pub subset_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoProvenance {
    pub url: String,
    pub pinned_commit: Option<String>,
    pub license: String,
    pub health_summary: Option<String>,
    pub caveats: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReproPlan {
    pub steps: Vec<String>,
    pub expected_baseline_metric: Option<String>,
    pub expected_time: Option<String>,
    pub config_overrides: Option<serde_json::Value>,
    pub success_criteria: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntegrationPlan {
    pub target_modules: Option<Vec<String>>,
    pub required_interfaces: Option<Vec<String>>,
    pub data_format_mappings: Option<Vec<DataFormatMapping>>,
    pub config_mapping: Option<Vec<ConfigMapping>>,
    pub test_plan: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataFormatMapping {
    pub source_format: String,
    pub target_format: String,
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigMapping {
    pub repo_param: String,
    pub user_config_param: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodebaseFit {
    pub target_location: Option<String>,
    pub existing_abstractions: Option<Vec<String>>,
    pub constraints_satisfied: Option<Vec<String>>,
    pub constraints_violated: Option<Vec<String>>,
    pub required_refactoring: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchMetadata {
    pub research_date: Option<String>,
    pub model_used: Option<String>,
    pub total_papers_reviewed: Option<u32>,
    pub num_sub_agents: Option<u32>,
    pub framework: Option<String>,
    pub codex_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn deserializes_legacy_solutions_as_proposals() {
        let json = serde_json::json!({
            "problem_decomposition": {
                "problem_statement": "x",
                "core_challenges": [],
                "constraints": []
            },
            "literature_review": {
                "total_papers_found": 1,
                "papers_by_id": {}
            },
            "solutions": [{
                "name": "legacy",
                "summary": "legacy summary"
            }],
            "recommended_solution": "legacy"
        });

        let parsed: ResearchOutput = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(parsed.proposals.len(), 1);
        assert_eq!(parsed.proposals[0].name, "legacy");
        assert_eq!(parsed.champion.proposal_name, "legacy");
    }

    #[test]
    fn proposal_field_wins_when_both_aliases_are_present() {
        let json = serde_json::json!({
            "problem_decomposition": {
                "problem_statement": "x",
                "core_challenges": [],
                "constraints": []
            },
            "literature_review": {
                "total_papers_found": 1,
                "papers_by_id": {}
            },
            "proposals": [{
                "name": "canonical",
                "summary": "canonical summary"
            }],
            "solutions": [{
                "name": "legacy",
                "summary": "legacy summary"
            }],
            "champion": {
                "proposal_name": "canonical",
                "justification": "better"
            }
        });

        let parsed: ResearchOutput = serde_json::from_value(json).expect("should deserialize");
        assert_eq!(parsed.proposals.len(), 1);
        assert_eq!(parsed.proposals[0].name, "canonical");
    }

    #[test]
    fn missing_papers_by_id_is_rejected() {
        let json = serde_json::json!({
            "problem_decomposition": {
                "problem_statement": "x",
                "core_challenges": [],
                "constraints": []
            },
            "literature_review": {
                "total_papers_found": 1,
                "papers": []
            },
            "proposals": [{
                "name": "canonical",
                "summary": "canonical summary"
            }],
            "champion": {
                "proposal_name": "canonical",
                "justification": "better"
            }
        });

        let error = serde_json::from_value::<ResearchOutput>(json).expect_err("should fail");
        assert_eq!(error.to_string(), "missing field `papers_by_id`");
    }
}
