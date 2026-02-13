use anyhow::Context;
use clap::Parser;
use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::research::RESEARCHER_SYSTEM_PROMPT;
use codex_core::research::ResearchOutput;
use codex_core::research::ResearchPromptParams;
use codex_core::research::ResearchToolAvailability;
use codex_core::research::ResearchToolContext;
use codex_core::research::ResearchToolNames;
use codex_core::research::build_research_prompt;
use codex_core::research::configured_native_tool_context;
use codex_protocol::config_types::WebSearchMode;
use codex_tui::Cli as TuiCli;
use codex_tui::ExitReason;
use codex_utils_cli::CliConfigOverrides;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Parser, Clone)]
pub(super) struct ResearchArgs {
    /// Research task description. Use `-` to read from stdin.
    #[arg(value_name = "TASK", value_hint = clap::ValueHint::Other)]
    task: Option<String>,

    /// Number of ranked proposals to generate.
    #[arg(long = "num-solutions", default_value_t = 3)]
    num_solutions: usize,

    /// Maximum parallel research sub-agents.
    #[arg(long = "max-agents", default_value_t = 4)]
    max_agents: usize,

    /// Framework for optional scaffold generation.
    #[arg(long = "framework", default_value = "pytorch")]
    framework: String,

    /// Enable Phase 4 code scaffolding.
    #[arg(long = "generate-code", default_value_t = true)]
    generate_code: bool,

    /// Disable Phase 4 code scaffolding.
    #[arg(long = "no-generate-code", default_value_t = false)]
    no_generate_code: bool,

    /// Output directory for research artifacts.
    #[arg(
        long = "output",
        value_name = "DIR",
        default_value = "./research-output"
    )]
    output_path: PathBuf,

    /// Optional codebase root for Phase 2b fit analysis.
    #[arg(long = "codebase", value_name = "DIR")]
    codebase_path: Option<PathBuf>,

    /// Optional prior `research_results.json` for iteration mode.
    #[arg(long = "prior-results", value_name = "FILE")]
    prior_results_path: Option<PathBuf>,

    /// Optional downstream feedback artifact for iteration mode.
    #[arg(long = "feedback", value_name = "FILE")]
    feedback_path: Option<PathBuf>,

    /// Iteration index. `0` is the first run.
    #[arg(long = "iteration", default_value_t = 0)]
    iteration_number: u32,
}

pub(super) async fn run(
    research_args: ResearchArgs,
    mut interactive: TuiCli,
    root_config_overrides: &CliConfigOverrides,
    codex_linux_sandbox_exe: Option<PathBuf>,
) -> anyhow::Result<()> {
    validate_research_inputs(&research_args)?;
    let resolved_output_path =
        resolve_path_for_session(&research_args.output_path, interactive.cwd.as_deref());
    let runtime_context =
        build_research_prompt_runtime_context(&research_args, &interactive, root_config_overrides)
            .await?;
    let research_prompt =
        build_research_command_prompt(&research_args, &runtime_context, &resolved_output_path)?;
    interactive.prompt = Some(research_prompt);

    for feature_override in [
        "features.research=true",
        "features.collab=true",
        "features.apps=true",
        "features.apply_patch_freeform=true",
    ] {
        interactive
            .config_overrides
            .raw_overrides
            .push(feature_override.to_string());
    }

    interactive
        .config_overrides
        .raw_overrides
        .push(build_research_developer_instruction_override(
            runtime_context.developer_instructions.as_deref(),
        ));

    let max_agents = research_args.max_agents;
    interactive
        .config_overrides
        .raw_overrides
        .push(format!("agent_max_threads={max_agents}"));
    if interactive.web_search {
        interactive
            .config_overrides
            .raw_overrides
            .push("web_search=live".to_string());
    }

    super::prepend_config_flags(
        &mut interactive.config_overrides,
        root_config_overrides.clone(),
    );
    let exit_info = super::run_interactive_tui(interactive, codex_linux_sandbox_exe).await?;
    if !matches!(exit_info.exit_reason, ExitReason::Fatal(_)) {
        run_research_post_session_hook(&resolved_output_path);
    }
    super::handle_app_exit(exit_info)?;

    Ok(())
}

fn resolve_research_task(task_arg: Option<String>) -> anyhow::Result<String> {
    match task_arg {
        Some(task) if task == "-" => {
            let mut task_from_stdin = String::new();
            std::io::stdin().read_to_string(&mut task_from_stdin)?;
            let trimmed = task_from_stdin.trim();
            if trimmed.is_empty() {
                anyhow::bail!("Research task read from stdin is empty");
            }
            Ok(trimmed.to_string())
        }
        Some(task) => {
            let trimmed = task.trim();
            if trimmed.is_empty() {
                anyhow::bail!("Research task cannot be empty");
            }
            Ok(trimmed.to_string())
        }
        None => Ok(
            "No research task was provided yet. Start by asking clarifying questions to capture \
the exact problem statement, constraints, and success criteria. Once clarified, proceed with Phase 1."
                .to_string(),
        ),
    }
}

fn build_research_command_prompt(
    args: &ResearchArgs,
    runtime_context: &ResearchPromptRuntimeContext,
    resolved_output_path: &Path,
) -> anyhow::Result<String> {
    let task_description = resolve_research_task(args.task.clone())?;
    let params = ResearchPromptParams {
        task_description,
        num_solutions: args.num_solutions,
        framework: args.framework.clone(),
        generate_code: args.generate_code && !args.no_generate_code,
        output_path: resolved_output_path.display().to_string(),
        tool_names: runtime_context.tool_names.clone(),
        has_zotero: runtime_context.tool_availability.has_zotero,
        has_paper_search: runtime_context.tool_availability.has_paper_search,
        has_repo_analysis: runtime_context.tool_availability.has_repo_analysis,
        has_hackernews: runtime_context.tool_availability.has_hackernews,
        has_web_search: runtime_context.has_web_search,
        has_user_codebase: runtime_context.has_user_codebase,
        codebase_path: runtime_context.codebase_path.clone(),
        prior_results_path: args
            .prior_results_path
            .as_ref()
            .map(|path| path.display().to_string()),
        feedback_path: args
            .feedback_path
            .as_ref()
            .map(|path| path.display().to_string()),
        iteration_number: args.iteration_number,
    };

    Ok(build_research_prompt(&params))
}

#[derive(Debug, Clone)]
struct ResearchPromptRuntimeContext {
    tool_names: ResearchToolNames,
    tool_availability: ResearchToolAvailability,
    has_web_search: bool,
    has_user_codebase: bool,
    codebase_path: Option<String>,
    developer_instructions: Option<String>,
}

fn resolve_path_for_session(path: &Path, session_cwd: Option<&Path>) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Some(cwd) = session_cwd {
        return cwd.join(path);
    }
    path.to_path_buf()
}

fn detect_project_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    const PROJECT_ROOT_MARKERS: [&str; 11] = [
        ".git",
        "Cargo.toml",
        "pyproject.toml",
        "package.json",
        "setup.py",
        "setup.cfg",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "CMakeLists.txt",
        "Makefile",
    ];

    PROJECT_ROOT_MARKERS
        .iter()
        .any(|marker| path.join(marker).exists())
}

fn canonical_display_path(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.display().to_string()
}

fn resolve_codebase_path(args: &ResearchArgs, session_cwd: &Path) -> PathBuf {
    match args.codebase_path.as_deref() {
        Some(path) => resolve_path_for_session(path, Some(session_cwd)),
        None => session_cwd.to_path_buf(),
    }
}

async fn build_research_prompt_runtime_context(
    args: &ResearchArgs,
    interactive: &TuiCli,
    root_config_overrides: &CliConfigOverrides,
) -> anyhow::Result<ResearchPromptRuntimeContext> {
    let mut cli_kv_overrides = root_config_overrides
        .parse_overrides()
        .map_err(anyhow::Error::msg)?;
    if interactive.web_search {
        cli_kv_overrides.push((
            "web_search".to_string(),
            toml::Value::String("live".to_string()),
        ));
    }

    let harness_overrides = ConfigOverrides {
        config_profile: interactive.config_profile.clone(),
        cwd: interactive.cwd.clone(),
        ..Default::default()
    };
    let config =
        Config::load_with_cli_overrides_and_harness_overrides(cli_kv_overrides, harness_overrides)
            .await?;
    let tool_context: ResearchToolContext = configured_native_tool_context(
        config.research.as_ref(),
        config.codex_home.as_path(),
        config.cwd.as_path(),
    );
    let has_web_search = matches!(
        config.web_search_mode.value(),
        WebSearchMode::Cached | WebSearchMode::Live
    );

    let codebase_path = resolve_codebase_path(args, config.cwd.as_path());
    let has_user_codebase = detect_project_root(&codebase_path);
    let codebase_path = if has_user_codebase {
        Some(canonical_display_path(&codebase_path))
    } else {
        None
    };

    Ok(ResearchPromptRuntimeContext {
        tool_names: tool_context.names,
        tool_availability: tool_context.availability,
        has_web_search,
        has_user_codebase,
        codebase_path,
        developer_instructions: config.developer_instructions,
    })
}

fn ensure_existing_file(path: &Path, flag: &str) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("`{flag}` path does not exist: {}", path.display());
    }
    if !path.is_file() {
        anyhow::bail!("`{flag}` path must be a file: {}", path.display());
    }
    Ok(())
}

fn validate_research_inputs(args: &ResearchArgs) -> anyhow::Result<()> {
    if args.num_solutions == 0 {
        anyhow::bail!("`--num-solutions` must be greater than 0");
    }

    if args.iteration_number > 0 && args.prior_results_path.is_none() {
        anyhow::bail!("`--prior-results` must be provided when `--iteration` is > 0");
    }

    if args.prior_results_path.is_some() && args.iteration_number == 0 {
        anyhow::bail!("`--iteration` must be > 0 when `--prior-results` is provided");
    }

    if let Some(prior_results_path) = args.prior_results_path.as_deref() {
        ensure_existing_file(prior_results_path, "--prior-results")?;
        let prior_results_raw = std::fs::read_to_string(prior_results_path).with_context(|| {
            format!(
                "failed to read prior results from {}",
                prior_results_path.display()
            )
        })?;
        let prior_results: ResearchOutput =
            serde_json::from_str(&prior_results_raw).with_context(|| {
                format!(
                    "`--prior-results` is not a valid research output file: {}",
                    prior_results_path.display()
                )
            })?;

        if let Some(iteration_context) = prior_results.iteration_context.as_ref() {
            let expected_iteration = iteration_context.iteration_number.saturating_add(1);
            if args.iteration_number != expected_iteration {
                eprintln!(
                    "Warning: `--iteration={}` does not match prior results expectation `{}`.",
                    args.iteration_number, expected_iteration
                );
            }
        }
    }

    if let Some(feedback_path) = args.feedback_path.as_deref() {
        ensure_existing_file(feedback_path, "--feedback")?;
        let is_json_feedback = feedback_path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if is_json_feedback {
            let feedback_raw = std::fs::read_to_string(feedback_path).with_context(|| {
                format!("failed to read feedback file {}", feedback_path.display())
            })?;
            serde_json::from_str::<serde_json::Value>(&feedback_raw).with_context(|| {
                format!("`--feedback` is invalid JSON: {}", feedback_path.display())
            })?;
        }
    }

    Ok(())
}

fn build_research_developer_instruction_override(
    existing_developer_instructions: Option<&str>,
) -> String {
    let composed_instructions = existing_developer_instructions
        .filter(|instructions| !instructions.trim().is_empty())
        .map_or_else(
            || RESEARCHER_SYSTEM_PROMPT.to_string(),
            |instructions| format!("{instructions}\n\n{RESEARCHER_SYSTEM_PROMPT}"),
        );
    let serialized_prompt = toml::Value::String(composed_instructions);
    format!("developer_instructions={serialized_prompt}")
}

fn run_research_post_session_hook(output_path: &Path) {
    if let Err(error) = run_research_post_session_hook_inner(output_path) {
        eprintln!("Warning: research post-session hook failed: {error}");
    }
}

fn run_research_post_session_hook_inner(output_path: &Path) -> anyhow::Result<()> {
    let results_path = output_path.join("research_results.json");
    let report_path = output_path.join("RESEARCH_REPORT.md");
    let validation_errors_path = output_path.join("research_results.validation_errors.txt");

    if !results_path.exists() {
        eprintln!(
            "Warning: expected `{}` but file is missing. Validation skipped.",
            results_path.display()
        );
        return Ok(());
    }
    if !results_path.is_file() {
        eprintln!(
            "Warning: expected `{}` to be a file. Validation skipped.",
            results_path.display()
        );
        return Ok(());
    }

    let raw_results = std::fs::read_to_string(&results_path)
        .with_context(|| format!("failed reading {}", results_path.display()))?;
    let parsed_results = match serde_json::from_str::<ResearchOutput>(&raw_results) {
        Ok(parsed_results) => parsed_results,
        Err(validation_error) => {
            let validation_payload = format!(
                "Validation failed for `{}`.\nError: {validation_error}\n\nRaw content:\n{}",
                results_path.display(),
                raw_results
            );
            std::fs::write(&validation_errors_path, validation_payload).with_context(|| {
                format!(
                    "failed writing validation errors to {}",
                    validation_errors_path.display()
                )
            })?;
            eprintln!(
                "Warning: `{}` failed validation. See `{}` for details.",
                results_path.display(),
                validation_errors_path.display()
            );
            return Ok(());
        }
    };

    if !parsed_results
        .proposals
        .iter()
        .any(|proposal| proposal.name == parsed_results.champion.proposal_name)
    {
        let validation_payload = format!(
            "Validation failed for `{}`.\nError: champion.proposal_name `{}` does not match any proposal.name.\n",
            results_path.display(),
            parsed_results.champion.proposal_name
        );
        std::fs::write(&validation_errors_path, validation_payload).with_context(|| {
            format!(
                "failed writing validation errors to {}",
                validation_errors_path.display()
            )
        })?;
        eprintln!(
            "Warning: `{}` failed validation. See `{}` for details.",
            results_path.display(),
            validation_errors_path.display()
        );
        return Ok(());
    }

    if report_path.exists() {
        return Ok(());
    }

    let rendered_report = render_fallback_research_report(&parsed_results);
    std::fs::write(&report_path, rendered_report)
        .with_context(|| format!("failed writing {}", report_path.display()))?;
    eprintln!(
        "Info: wrote fallback report to `{}` because it was missing.",
        report_path.display()
    );
    Ok(())
}

fn render_fallback_research_report(results: &ResearchOutput) -> String {
    let mut report = String::from(
        "# Research Report\n\n\
Generated by the deterministic fallback renderer because `RESEARCH_REPORT.md` was missing.\n\n",
    );

    let champion_name = results.champion.proposal_name.as_str();
    let champion_summary = results
        .proposals
        .iter()
        .find(|proposal| proposal.name == results.champion.proposal_name)
        .map(|proposal| proposal.summary.as_str())
        .unwrap_or("No proposal summary was available.");

    report.push_str("## Executive Summary\n\n");
    report.push_str(&format!(
        "Champion proposal: **{}**.\n\n{}\n\n",
        markdown_escape(champion_name),
        champion_summary
    ));

    report.push_str("## Proposals\n\n");
    report.push_str("| Rank | Name | Status | Summary |\n");
    report.push_str("| --- | --- | --- | --- |\n");
    for (index, proposal) in results.proposals.iter().enumerate() {
        let rank = proposal.rank.unwrap_or((index as u32) + 1);
        let status = proposal.status.as_deref().unwrap_or("unspecified");
        report.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            rank,
            markdown_escape(&proposal.name),
            markdown_escape(status),
            markdown_escape(&proposal.summary)
        ));
    }
    report.push('\n');

    report.push_str("## Champion\n\n");
    report.push_str(&format!(
        "- Name: {}\n- Justification: {}\n",
        markdown_escape(&results.champion.proposal_name),
        markdown_escape(&results.champion.justification)
    ));
    if let Some(promotion_criteria) = results.champion.promotion_criteria.as_deref() {
        report.push_str(&format!(
            "- Promotion criteria: {}\n",
            markdown_escape(promotion_criteria)
        ));
    }
    report.push('\n');

    append_markdown_list_section(&mut report, "## Next Steps", results.next_steps.as_deref());
    append_markdown_list_section(
        &mut report,
        "## Open Questions",
        results.open_questions.as_deref(),
    );

    report.push_str("## Bibliography\n\n");
    let mut ordered_paper_ids = Vec::new();
    let mut seen_paper_ids = std::collections::HashSet::new();
    if let Some(paper_ids_ranked) = results.literature_review.paper_ids_ranked.as_ref() {
        for paper_id in paper_ids_ranked {
            if seen_paper_ids.insert(paper_id.clone()) {
                ordered_paper_ids.push(paper_id.clone());
            }
        }
    }
    let mut remaining_ids = results
        .literature_review
        .papers_by_id
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    remaining_ids.sort();
    for paper_id in remaining_ids {
        if seen_paper_ids.insert(paper_id.clone()) {
            ordered_paper_ids.push(paper_id);
        }
    }

    if ordered_paper_ids.is_empty() {
        report.push_str("- No papers listed.\n");
        return report;
    }

    for paper_id in ordered_paper_ids {
        if let Some(paper) = results.literature_review.papers_by_id.get(&paper_id) {
            let mut line = format!(
                "- [{}] {}",
                markdown_escape(&paper_id),
                markdown_escape(&paper.title)
            );
            if !paper.authors.trim().is_empty() {
                line.push_str(&format!(" - {}", markdown_escape(&paper.authors)));
            }
            if let Some(year) = paper.year {
                line.push_str(&format!(" ({year})"));
            }
            if let Some(url) = paper
                .doi
                .as_ref()
                .map(|doi| format!("https://doi.org/{doi}"))
                .or_else(|| paper.url.clone())
                .or_else(|| paper.pdf_url.clone())
            {
                line.push_str(&format!(" - {}", markdown_escape(&url)));
            }
            report.push_str(&line);
            report.push('\n');
        }
    }

    report
}

fn append_markdown_list_section(report: &mut String, heading: &str, entries: Option<&[String]>) {
    report.push_str(heading);
    report.push_str("\n\n");

    match entries {
        Some(values) if !values.is_empty() => {
            for value in values {
                report.push_str(&format!("- {}\n", markdown_escape(value)));
            }
            report.push('\n');
        }
        _ => {
            report.push_str("- None listed.\n\n");
        }
    }
}

fn markdown_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' | '|' | '*' | '_' | '[' | ']' | '`' => {
                escaped.push('\\');
                escaped.push(c);
            }
            '\n' => escaped.push(' '),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MultitoolCli;
    use crate::Subcommand;
    use pretty_assertions::assert_eq;

    fn prompt_runtime_context_for_tests() -> ResearchPromptRuntimeContext {
        ResearchPromptRuntimeContext {
            tool_names: ResearchToolNames::from_available_native(),
            tool_availability: ResearchToolAvailability {
                has_paper_search: true,
                has_zotero: true,
                has_repo_analysis: true,
                has_hackernews: true,
            },
            has_web_search: true,
            has_user_codebase: false,
            codebase_path: None,
            developer_instructions: None,
        }
    }

    #[test]
    fn research_subcommand_parses_arguments() {
        let cli = MultitoolCli::try_parse_from([
            "ata",
            "research",
            "Investigate robust offline RL for robot pick-and-place",
            "--num-solutions",
            "4",
            "--max-agents",
            "7",
            "--framework",
            "jax",
            "--generate-code",
            "--output",
            "./results",
            "--codebase",
            "./repo",
            "--iteration",
            "2",
        ])
        .expect("parse should succeed");

        let Some(Subcommand::Research(args)) = cli.subcommand else {
            panic!("expected research subcommand");
        };

        assert_eq!(
            args.task.as_deref(),
            Some("Investigate robust offline RL for robot pick-and-place")
        );
        assert_eq!(args.num_solutions, 4);
        assert_eq!(args.max_agents, 7);
        assert_eq!(args.framework, "jax");
        assert!(args.generate_code);
        assert!(!args.no_generate_code);
        assert_eq!(args.output_path, PathBuf::from("./results"));
        assert_eq!(args.codebase_path, Some(PathBuf::from("./repo")));
        assert_eq!(args.iteration_number, 2);
    }

    #[test]
    fn research_subcommand_generate_code_defaults_to_true() {
        let cli = MultitoolCli::try_parse_from([
            "ata",
            "research",
            "Investigate robust offline RL for robot pick-and-place",
        ])
        .expect("parse should succeed");

        let Some(Subcommand::Research(args)) = cli.subcommand else {
            panic!("expected research subcommand");
        };

        assert!(args.generate_code);
        assert!(!args.no_generate_code);
        assert_eq!(args.max_agents, 4);
    }

    #[test]
    fn research_subcommand_allows_disabling_code_generation() {
        let cli = MultitoolCli::try_parse_from([
            "ata",
            "research",
            "Investigate robust offline RL for robot pick-and-place",
            "--no-generate-code",
        ])
        .expect("parse should succeed");

        let Some(Subcommand::Research(args)) = cli.subcommand else {
            panic!("expected research subcommand");
        };

        assert!(args.generate_code);
        assert!(args.no_generate_code);
    }

    #[test]
    fn research_prompt_builder_wraps_persona_and_task_prompt() {
        let args = ResearchArgs {
            task: Some("Find production-ready visual tracking approaches".to_string()),
            num_solutions: 3,
            max_agents: 4,
            framework: "pytorch".to_string(),
            generate_code: false,
            no_generate_code: false,
            output_path: PathBuf::from("./research-output"),
            codebase_path: None,
            prior_results_path: None,
            feedback_path: None,
            iteration_number: 0,
        };

        let prompt = build_research_command_prompt(
            &args,
            &prompt_runtime_context_for_tests(),
            args.output_path.as_path(),
        )
        .expect("prompt should build");
        assert!(!prompt.starts_with(RESEARCHER_SYSTEM_PROMPT));
        assert!(prompt.contains("Find production-ready visual tracking approaches"));
        assert!(prompt.contains("### Phase 1: Problem Decomposition"));
    }

    #[test]
    fn developer_instruction_override_composes_with_existing_value() {
        let override_value =
            build_research_developer_instruction_override(Some("Use terse bullet lists."));
        assert!(override_value.contains("Use terse bullet lists."));
        assert!(override_value.contains(RESEARCHER_SYSTEM_PROMPT));
    }

    #[test]
    fn detect_project_root_uses_known_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!detect_project_root(dir.path()));

        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n")
            .expect("write marker");
        assert!(detect_project_root(dir.path()));
    }

    #[test]
    fn resolve_research_task_allows_interactive_kickoff_without_task() {
        let kickoff = resolve_research_task(None).expect("task should default");
        assert!(kickoff.contains("No research task was provided"));

        let err = resolve_research_task(Some("   ".to_string()))
            .expect_err("blank task should be rejected");
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    fn validate_research_inputs_requires_iteration_when_prior_results_is_provided() {
        let prior_results_dir = tempfile::tempdir().expect("tempdir");
        let prior_results_path = prior_results_dir.path().join("research_results.json");
        std::fs::write(
            &prior_results_path,
            r#"{
  "problem_decomposition": {
    "problem_statement": "x",
    "core_challenges": [],
    "constraints": []
  },
  "literature_review": {
    "total_papers_found": 0,
    "papers_by_id": {}
  },
  "proposals": [
    {
      "name": "p",
      "summary": "s"
    }
  ],
  "champion": {
    "proposal_name": "p",
    "justification": "j"
  }
}"#,
        )
        .expect("write prior results");

        let args = ResearchArgs {
            task: None,
            num_solutions: 3,
            max_agents: 4,
            framework: "pytorch".to_string(),
            generate_code: false,
            no_generate_code: false,
            output_path: PathBuf::from("./research-output"),
            codebase_path: None,
            prior_results_path: Some(prior_results_path),
            feedback_path: None,
            iteration_number: 0,
        };
        let error = validate_research_inputs(&args).expect_err("must fail");
        assert!(error.to_string().contains("--iteration"));
    }

    #[test]
    fn validate_research_inputs_rejects_zero_num_solutions() {
        let args = ResearchArgs {
            task: None,
            num_solutions: 0,
            max_agents: 4,
            framework: "pytorch".to_string(),
            generate_code: false,
            no_generate_code: false,
            output_path: PathBuf::from("./research-output"),
            codebase_path: None,
            prior_results_path: None,
            feedback_path: None,
            iteration_number: 0,
        };
        let error = validate_research_inputs(&args).expect_err("must fail");
        assert!(error.to_string().contains("--num-solutions"));
    }

    #[test]
    fn validate_research_inputs_rejects_nonzero_iteration_without_prior_results() {
        let args = ResearchArgs {
            task: None,
            num_solutions: 3,
            max_agents: 4,
            framework: "pytorch".to_string(),
            generate_code: false,
            no_generate_code: false,
            output_path: PathBuf::from("./research-output"),
            codebase_path: None,
            prior_results_path: None,
            feedback_path: None,
            iteration_number: 2,
        };
        let error = validate_research_inputs(&args).expect_err("must fail");
        assert!(error.to_string().contains("--prior-results"));
    }

    #[test]
    fn post_session_hook_writes_fallback_report_from_valid_results() {
        let output_dir = tempfile::tempdir().expect("tempdir");
        let results_path = output_dir.path().join("research_results.json");
        std::fs::write(
            &results_path,
            r#"{
  "problem_decomposition": {
    "problem_statement": "x",
    "core_challenges": [],
    "constraints": []
  },
  "literature_review": {
    "total_papers_found": 1,
    "papers_by_id": {
      "paper-1": {
        "title": "Paper One",
        "authors": "A. Author",
        "year": 2024,
        "url": "https://example.com/paper"
      }
    },
    "paper_ids_ranked": ["paper-1"]
  },
  "proposals": [
    {
      "rank": 1,
      "name": "Proposal A",
      "summary": "Summary A"
    }
  ],
  "champion": {
    "proposal_name": "Proposal A",
    "justification": "Best overall fit"
  },
  "next_steps": ["Run experiment 1"],
  "open_questions": ["Need more edge-case data"]
}"#,
        )
        .expect("write results");

        run_research_post_session_hook_inner(output_dir.path()).expect("hook should succeed");

        let report_path = output_dir.path().join("RESEARCH_REPORT.md");
        let report = std::fs::read_to_string(report_path).expect("report");
        assert!(report.contains("## Proposals"));
        assert!(report.contains("Proposal A"));
        assert!(report.contains("## Bibliography"));
    }

    #[test]
    fn post_session_hook_writes_validation_errors_for_invalid_results() {
        let output_dir = tempfile::tempdir().expect("tempdir");
        let results_path = output_dir.path().join("research_results.json");
        std::fs::write(&results_path, "{ invalid json").expect("write invalid");

        run_research_post_session_hook_inner(output_dir.path()).expect("hook should not fail");

        let errors_path = output_dir
            .path()
            .join("research_results.validation_errors.txt");
        let errors = std::fs::read_to_string(errors_path).expect("errors");
        assert!(errors.contains("Validation failed"));
    }

    #[test]
    fn post_session_hook_rejects_champion_missing_from_proposals() {
        let output_dir = tempfile::tempdir().expect("tempdir");
        let results_path = output_dir.path().join("research_results.json");
        std::fs::write(
            &results_path,
            r#"{
  "problem_decomposition": {
    "problem_statement": "x",
    "core_challenges": [],
    "constraints": []
  },
  "literature_review": {
    "total_papers_found": 0,
    "papers_by_id": {}
  },
  "proposals": [
    {
      "name": "Proposal A",
      "summary": "Summary A"
    }
  ],
  "champion": {
    "proposal_name": "Proposal Z",
    "justification": "Best overall fit"
  }
}"#,
        )
        .expect("write results");

        run_research_post_session_hook_inner(output_dir.path()).expect("hook should not fail");

        let errors_path = output_dir
            .path()
            .join("research_results.validation_errors.txt");
        let errors = std::fs::read_to_string(errors_path).expect("errors");
        assert!(errors.contains("champion.proposal_name"));
    }

    #[test]
    fn markdown_escape_covers_table_and_common_markdown_tokens() {
        let escaped = markdown_escape(
            r#"a|b *_[]`\ test
line2"#,
        );
        assert_eq!(escaped, r#"a\|b \*\_\[\]\`\\ test line2"#);
    }
}
