use std::cmp::min;

use super::research_output_schema;
use crate::research::tool_names::ResearchToolNames;

#[derive(Debug, Clone)]
pub struct ResearchPromptParams {
    pub task_description: String,
    pub num_solutions: usize,
    pub framework: String,
    pub generate_code: bool,
    pub output_path: String,
    pub tool_names: ResearchToolNames,
    pub has_zotero: bool,
    pub has_paper_search: bool,
    pub has_repo_analysis: bool,
    pub has_web_search: bool,
    pub has_user_codebase: bool,
    pub codebase_path: Option<String>,
    pub prior_results_path: Option<String>,
    pub feedback_path: Option<String>,
    pub iteration_number: u32,
}

pub fn build_research_prompt(params: &ResearchPromptParams) -> String {
    let mut prompt = String::new();
    prompt.push_str("# Research Task\n\n");
    prompt.push_str("## Problem Statement\n\n");
    prompt.push_str(&params.task_description);
    prompt.push_str("\n\n");
    prompt.push_str("Follow the phases below in order.\n\n");
    prompt.push_str(PHASE_1_PROMPT);
    prompt.push_str(&build_phase_2_prompt(params));

    if params.has_repo_analysis {
        prompt.push_str(&build_phase_2b_prompt(params));
    }

    prompt.push_str(&build_phase_3_prompt(params.num_solutions));
    prompt.push_str(&build_phase_3b_prompt());

    if params.generate_code {
        prompt.push_str(&build_phase_4_prompt(
            params.num_solutions,
            params.framework.as_str(),
            params.output_path.as_str(),
        ));
    }

    if params.iteration_number > 0 {
        prompt.push_str(&build_iteration_context_prompt(params));
    }

    prompt.push_str(&build_output_format_prompt(params.output_path.as_str()));
    prompt
}

const PHASE_1_PROMPT: &str = "### Phase 1: Problem Decomposition\n\n\
Identify core challenges, constraints, and scope boundaries. Extract explicit and \
implicit constraints across quality, latency, deployment, observability, privacy, \
cost, and reproducibility.\n\n";

fn build_phase_2_prompt(params: &ResearchPromptParams) -> String {
    let tool = &params.tool_names;
    let mut phase = String::from(
        "### Phase 2: Literature and SOTA Survey\n\n\
For each sub-problem, gather evidence and summarize key findings.\n\
Use sub-agents when available. Each sub-agent should write one artifact to \
`{output_path}/.research_state/subfinding_N.json` using `apply_patch`, then return a compact summary.\n\n",
    );
    phase = phase.replace("{output_path}", params.output_path.as_str());

    if params.has_zotero {
        phase.push_str(&format!(
            "- Search user library via `{}` and inspect relevant collections via `{}`.\n",
            tool.zotero_search, tool.zotero_get_collections,
        ));
    }
    if params.has_paper_search {
        phase.push_str(&format!(
            "- Run academic search via `{}`.\n\
- Expand graph via `{}` and `{}`.\n",
            tool.paper_search, tool.paper_citations, tool.paper_references,
        ));
    }
    if params.has_web_search {
        phase.push_str("- Use web search for recent practical deployments and tutorials.\n");
    }

    phase.push_str(
        "\nTarget at least 15 relevant papers overall and return ranked findings with citations.\n\
Missing `.research_state/subfinding_N.json` files must not block completion; treat them as best-effort artifacts.\n\n",
    );
    phase
}

fn build_phase_2b_prompt(params: &ResearchPromptParams) -> String {
    let tool = &params.tool_names;
    let k = params.num_solutions + 1;
    let mut phase = format!(
        "### Phase 2b: Repo-Grounded Integration Analysis\n\n\
Select top {k} candidate approaches with code repositories. For each candidate:\n\
- Inspect repo entrypoints via `{}` and I/O shapes via `{}`.\n\
- Extract config schema via `{}` and dependencies via `{}`.\n\
- Assess repo health via `{}` and export paths via `{}`.\n",
        tool.repo_find_entrypoints,
        tool.repo_extract_io_shapes,
        tool.repo_extract_config_schema,
        tool.repo_extract_requirements,
        tool.repo_get_health,
        tool.repo_find_export_paths,
    );

    if params.has_user_codebase {
        if let Some(codebase_path) = params.codebase_path.as_deref() {
            phase.push_str(&format!(
                "- Analyze fit against user codebase at `{codebase_path}` using `list_dir`, \
`grep_files`, and `read_file`.\n"
            ));
            phase.push_str(&format!(
                "- Compare dependency deltas via `{}` when requirements files are available.\n",
                tool.repo_diff_requirements
            ));
        }
    } else {
        phase
            .push_str("- Produce a generic integration plan with placeholder module boundaries.\n");
    }

    phase.push_str(&format!(
        "\nWrite per-candidate repo assessments to `{}/.research_state/repo_assessment_N.json` using `apply_patch` and return compact summaries.\n\
Each assessment should include adoption strategy, provenance pinning (commit/license), and repo feasibility.\n\
If cloning/inspection fails, set `repo_unavailable=true` and include a blocked reason.\n\
Missing repo assessment artifacts are best-effort and must not block final output.\n\n",
        params.output_path
    ));
    phase
}

fn build_phase_3_prompt(num_solutions: usize) -> String {
    format!(
        "### Phase 3: Proposal Synthesis\n\n\
Generate exactly {num_solutions} ranked proposals. Each proposal must include:\n\
- acceptance criteria\n\
- minimum viable experiment\n\
- risk register\n\
- instrumentation requirements\n\
- champion/challenger status\n\
- implementation packet (adoption strategy, provenance, reproduction, integration plan)\n\n"
    )
}

fn build_phase_3b_prompt() -> String {
    "### Phase 3b: Skeptic Pass\n\n\
Before finalizing outputs, verify:\n\
- every key claim maps to explicit evidence\n\
- benchmark numbers match retrieved sources\n\
- unverifiable claims are marked `[UNVERIFIED]`\n\
- strengths/weaknesses are evidence-backed\n\
- there are no citation contradictions across proposals\n\n"
        .to_string()
}

fn build_phase_4_prompt(num_solutions: usize, framework: &str, output_path: &str) -> String {
    let top_n = min(num_solutions, 2);
    format!(
        "### Phase 4: Reproducible Pipeline Scaffolding\n\n\
For the top {top_n} proposal(s), generate executable {framework} scaffolds in `{output_path}`:\n\
- `model.py`, `dataset.py`, `train.py`, `evaluate.py`\n\
- `config.yaml`, `requirements.txt`, `README.md`\n\
Ensure structured metrics output and explicit go/no-go rules.\n\n"
    )
}

fn build_iteration_context_prompt(params: &ResearchPromptParams) -> String {
    let mut section = format!(
        "### Iteration Context\n\n\
This is iteration {}. Incorporate prior evidence and downstream feedback.\n",
        params.iteration_number
    );
    if let Some(path) = params.prior_results_path.as_deref() {
        section.push_str(&format!("- Prior results: `{path}`\n"));
    }
    if let Some(path) = params.feedback_path.as_deref() {
        section.push_str(&format!("- Downstream feedback: `{path}`\n"));
    }
    section.push_str(
        "Do not re-propose known-dead approaches unless you provide new evidence-backed fixes.\n\n",
    );
    section
}

fn build_output_format_prompt(output_path: &str) -> String {
    let output_schema = serde_json::to_string_pretty(&research_output_schema())
        .unwrap_or_else(|error| format!(r#"{{"schema_serialization_error":"{error}"}}"#));

    format!(
        "### Output Format\n\n\
Write these files with `apply_patch`:\n\
1. `{output_path}/research_results.json`\n\
2. `{output_path}/RESEARCH_REPORT.md`\n\
3. `{output_path}/.research_state/subfinding_N.json` and `{output_path}/.research_state/repo_assessment_N.json` (best-effort)\n\
4. `{output_path}/proposal_N/` scaffold folders when Phase 4 runs.\n\n\
`{output_path}/research_results.json` must conform to this JSON Schema:\n\
```json\n\
{output_schema}\n\
```\n\n\
If `.research_state/*` artifacts are missing, continue and complete the final output files.\n\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ResearchPromptParams {
        ResearchPromptParams {
            task_description: "Design a robust manipulation stack.".to_string(),
            num_solutions: 3,
            framework: "pytorch".to_string(),
            generate_code: true,
            output_path: "./research-output".to_string(),
            tool_names: ResearchToolNames::default(),
            has_zotero: true,
            has_paper_search: true,
            has_repo_analysis: true,
            has_web_search: true,
            has_user_codebase: true,
            codebase_path: Some("/workspace/project".to_string()),
            prior_results_path: None,
            feedback_path: None,
            iteration_number: 0,
        }
    }

    #[test]
    fn prompt_includes_selected_tools_and_phases() {
        let rendered = build_research_prompt(&params());
        assert!(rendered.contains("`zotero_search`"));
        assert!(rendered.contains("`paper_search`"));
        assert!(rendered.contains("`repo_find_entrypoints`"));
        assert!(rendered.contains("### Phase 4: Reproducible Pipeline Scaffolding"));
        assert!(rendered.contains("### Phase 3b: Skeptic Pass"));
        assert!(rendered.contains(".research_state/subfinding_N.json"));
    }

    #[test]
    fn prompt_omits_repo_phase_when_repo_analysis_is_disabled() {
        let mut p = params();
        p.has_repo_analysis = false;
        let rendered = build_research_prompt(&p);
        assert!(!rendered.contains("### Phase 2b: Repo-Grounded Integration Analysis"));
    }

    #[test]
    fn prompt_adds_iteration_context_when_iteration_is_nonzero() {
        let mut p = params();
        p.iteration_number = 2;
        p.prior_results_path = Some("./v1/research_results.json".to_string());
        p.feedback_path = Some("./v1/eval_report.json".to_string());
        let rendered = build_research_prompt(&p);
        assert!(rendered.contains("### Iteration Context"));
        assert!(rendered.contains("`./v1/research_results.json`"));
        assert!(rendered.contains("`./v1/eval_report.json`"));
    }

    #[test]
    fn phase_4_uses_top_two_limit() {
        let rendered = build_phase_4_prompt(8, "pytorch", "./out");
        assert!(rendered.contains("For the top 2 proposal"));
        assert!(!rendered.contains("top 8"));
    }

    #[test]
    fn output_format_includes_embedded_json_schema() {
        let rendered = build_output_format_prompt("./out");
        assert!(rendered.contains("must conform to this JSON Schema"));
        assert!(rendered.contains("\"title\": \"ResearchOutput\""));
        assert!(rendered.contains("\"$defs\":"));
    }
}
