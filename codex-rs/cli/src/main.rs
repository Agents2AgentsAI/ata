use anyhow::Context;
use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use clap_complete::Shell;
use clap_complete::generate;
use codex_arg0::arg0_dispatch_or_else;
use codex_chatgpt::apply_command::ApplyCommand;
use codex_chatgpt::apply_command::run_apply_command;
use codex_cli::LandlockCommand;
use codex_cli::SeatbeltCommand;
use codex_cli::WindowsCommand;
use codex_cli::login::read_api_key_from_stdin;
use codex_cli::login::run_list_providers;
use codex_cli::login::run_login_status;
use codex_cli::login::run_login_with_chatgpt;
use codex_cli::login::run_login_with_device_code;
use codex_cli::login::run_login_with_provider_api_key;
use codex_cli::login::run_logout_provider;
use codex_cloud_tasks::Cli as CloudTasksCli;
use codex_common::CliConfigOverrides;
use codex_exec::Cli as ExecCli;
use codex_exec::Command as ExecCommand;
use codex_exec::ReviewArgs;
use codex_execpolicy::ExecPolicyCheckCommand;
use codex_responses_api_proxy::Args as ResponsesApiProxyArgs;
use codex_tui::AppExitInfo;
use codex_tui::Cli as TuiCli;
use codex_tui::ExitReason;
use codex_tui::update_action::UpdateAction;
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use supports_color::Stream;

#[cfg(target_os = "macos")]
mod app_cmd;
#[cfg(target_os = "macos")]
mod desktop_app;
mod mcp_cmd;
#[cfg(not(windows))]
mod wsl_paths;

use crate::mcp_cmd::McpCli;

use codex_core::config::Config;
use codex_core::config::ConfigOverrides;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::find_codex_home;
use codex_core::features::Stage;
use codex_core::features::is_known_feature_key;
use codex_core::research::RESEARCHER_SYSTEM_PROMPT;
use codex_core::research::ResearchOutput;
use codex_core::research::ResearchPromptParams;
use codex_core::research::ResearchToolAvailability;
use codex_core::research::ResearchToolContext;
use codex_core::research::ResearchToolNames;
use codex_core::research::build_research_prompt;
use codex_core::research::configured_native_tool_context;
use codex_core::terminal::TerminalName;
use codex_protocol::config_types::WebSearchMode;

/// Codex CLI
///
/// If no subcommand is specified, options will be forwarded to the interactive CLI.
#[derive(Debug, Parser)]
#[clap(
    author,
    version,
    // If a sub‑command is given, ignore requirements of the default args.
    subcommand_negates_reqs = true,
    // The executable is sometimes invoked via a platform‑specific name like
    // `codex-x86_64-unknown-linux-musl`, but the help output should always use
    // the generic `codex` command name that users run.
    bin_name = "codex",
    override_usage = "codex [OPTIONS] [PROMPT]\n       codex [OPTIONS] <COMMAND> [ARGS]"
)]
struct MultitoolCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[clap(flatten)]
    pub feature_toggles: FeatureToggles,

    #[clap(flatten)]
    interactive: TuiCli,

    #[clap(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    /// Run Codex non-interactively.
    #[clap(visible_alias = "e")]
    Exec(ExecCli),

    /// Run a code review non-interactively.
    Review(ReviewArgs),

    /// Run a structured multi-phase research workflow.
    Research(ResearchArgs),

    /// Manage login.
    Login(LoginCommand),

    /// Remove stored authentication credentials.
    Logout(LogoutCommand),

    /// [experimental] Run Codex as an MCP server and manage MCP servers.
    Mcp(McpCli),

    /// [experimental] Run the Codex MCP server (stdio transport).
    McpServer,

    /// [experimental] Run the app server or related tooling.
    AppServer(AppServerCommand),

    /// Launch the Codex desktop app (downloads the macOS installer if missing).
    #[cfg(target_os = "macos")]
    App(app_cmd::AppCommand),

    /// Generate shell completion scripts.
    Completion(CompletionCommand),

    /// Run commands within a Codex-provided sandbox.
    Sandbox(SandboxArgs),

    /// Debugging tools.
    Debug(DebugCommand),

    /// Execpolicy tooling.
    #[clap(hide = true)]
    Execpolicy(ExecpolicyCommand),

    /// Apply the latest diff produced by Codex agent as a `git apply` to your local working tree.
    #[clap(visible_alias = "a")]
    Apply(ApplyCommand),

    /// Resume a previous interactive session (picker by default; use --last to continue the most recent).
    Resume(ResumeCommand),

    /// Fork a previous interactive session (picker by default; use --last to fork the most recent).
    Fork(ForkCommand),

    /// [EXPERIMENTAL] Browse tasks from Codex Cloud and apply changes locally.
    #[clap(name = "cloud", alias = "cloud-tasks")]
    Cloud(CloudTasksCli),

    /// Internal: run the responses API proxy.
    #[clap(hide = true)]
    ResponsesApiProxy(ResponsesApiProxyArgs),

    /// Internal: relay stdio to a Unix domain socket.
    #[clap(hide = true, name = "stdio-to-uds")]
    StdioToUds(StdioToUdsCommand),

    /// Inspect feature flags.
    Features(FeaturesCli),
}

#[derive(Debug, Parser)]
struct CompletionCommand {
    /// Shell to generate completions for
    #[clap(value_enum, default_value_t = Shell::Bash)]
    shell: Shell,
}

#[derive(Debug, Parser, Clone)]
struct ResearchArgs {
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

#[derive(Debug, Parser)]
struct DebugCommand {
    #[command(subcommand)]
    subcommand: DebugSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum DebugSubcommand {
    /// Tooling: helps debug the app server.
    AppServer(DebugAppServerCommand),
}

#[derive(Debug, Parser)]
struct DebugAppServerCommand {
    #[command(subcommand)]
    subcommand: DebugAppServerSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum DebugAppServerSubcommand {
    // Send message to app server V2.
    SendMessageV2(DebugAppServerSendMessageV2Command),
}

#[derive(Debug, Parser)]
struct DebugAppServerSendMessageV2Command {
    #[arg(value_name = "USER_MESSAGE", required = true)]
    user_message: String,
}

#[derive(Debug, Parser)]
struct ResumeCommand {
    /// Conversation/session id (UUID) or thread name. UUIDs take precedence if it parses.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Continue the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false)]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    #[clap(flatten)]
    config_overrides: TuiCli,
}

#[derive(Debug, Parser)]
struct ForkCommand {
    /// Conversation/session id (UUID). When provided, forks this session.
    /// If omitted, use --last to pick the most recent recorded session.
    #[arg(value_name = "SESSION_ID")]
    session_id: Option<String>,

    /// Fork the most recent session without showing the picker.
    #[arg(long = "last", default_value_t = false, conflicts_with = "session_id")]
    last: bool,

    /// Show all sessions (disables cwd filtering and shows CWD column).
    #[arg(long = "all", default_value_t = false)]
    all: bool,

    #[clap(flatten)]
    config_overrides: TuiCli,
}

#[derive(Debug, Parser)]
struct SandboxArgs {
    #[command(subcommand)]
    cmd: SandboxCommand,
}

#[derive(Debug, clap::Subcommand)]
enum SandboxCommand {
    /// Run a command under Seatbelt (macOS only).
    #[clap(visible_alias = "seatbelt")]
    Macos(SeatbeltCommand),

    /// Run a command under Landlock+seccomp (Linux only).
    #[clap(visible_alias = "landlock")]
    Linux(LandlockCommand),

    /// Run a command under Windows restricted token (Windows only).
    Windows(WindowsCommand),
}

#[derive(Debug, Parser)]
struct ExecpolicyCommand {
    #[command(subcommand)]
    sub: ExecpolicySubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum ExecpolicySubcommand {
    /// Check execpolicy files against a command.
    #[clap(name = "check")]
    Check(ExecPolicyCheckCommand),
}

#[derive(Debug, Parser)]
struct LoginCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,

    #[arg(
        long = "with-api-key",
        help = "Read the API key from stdin (e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`)"
    )]
    with_api_key: bool,

    #[arg(
        long = "api-key",
        value_name = "API_KEY",
        help = "(deprecated) Previously accepted the API key directly; now exits with guidance to use --with-api-key",
        hide = true
    )]
    api_key: Option<String>,

    #[arg(long = "device-auth")]
    use_device_code: bool,

    /// Provider to configure (openai, anthropic, gemini). Defaults to openai.
    #[arg(long = "provider", short = 'P', value_name = "PROVIDER")]
    provider: Option<String>,

    /// EXPERIMENTAL: Use custom OAuth issuer base URL (advanced)
    /// Override the OAuth issuer base URL (advanced)
    #[arg(long = "experimental_issuer", value_name = "URL", hide = true)]
    issuer_base_url: Option<String>,

    /// EXPERIMENTAL: Use custom OAuth client ID (advanced)
    #[arg(long = "experimental_client-id", value_name = "CLIENT_ID", hide = true)]
    client_id: Option<String>,

    #[command(subcommand)]
    action: Option<LoginSubcommand>,
}

#[derive(Debug, clap::Subcommand)]
enum LoginSubcommand {
    /// Show login status.
    Status,
    /// List all configured providers.
    List,
}

#[derive(Debug, Parser)]
struct LogoutCommand {
    #[clap(skip)]
    config_overrides: CliConfigOverrides,

    /// Provider to logout (openai, anthropic, gemini). If not specified, logs out all providers.
    #[arg(value_name = "PROVIDER")]
    provider: Option<String>,
}

#[derive(Debug, Parser)]
struct AppServerCommand {
    /// Omit to run the app server; specify a subcommand for tooling.
    #[command(subcommand)]
    subcommand: Option<AppServerSubcommand>,

    /// Transport endpoint URL. Supported values: `stdio://` (default),
    /// `ws://IP:PORT`.
    #[arg(
        long = "listen",
        value_name = "URL",
        default_value = codex_app_server::AppServerTransport::DEFAULT_LISTEN_URL
    )]
    listen: codex_app_server::AppServerTransport,

    /// Controls whether analytics are enabled by default.
    ///
    /// Analytics are disabled by default for app-server. Users have to explicitly opt in
    /// via the `analytics` section in the config.toml file.
    ///
    /// However, for first-party use cases like the VSCode IDE extension, we default analytics
    /// to be enabled by default by setting this flag. Users can still opt out by setting this
    /// in their config.toml:
    ///
    /// ```toml
    /// [analytics]
    /// enabled = false
    /// ```
    ///
    /// See https://developers.openai.com/codex/config-advanced/#metrics for more details.
    #[arg(long = "analytics-default-enabled")]
    analytics_default_enabled: bool,
}

#[derive(Debug, clap::Subcommand)]
enum AppServerSubcommand {
    /// [experimental] Generate TypeScript bindings for the app server protocol.
    GenerateTs(GenerateTsCommand),

    /// [experimental] Generate JSON Schema for the app server protocol.
    GenerateJsonSchema(GenerateJsonSchemaCommand),
}

#[derive(Debug, Args)]
struct GenerateTsCommand {
    /// Output directory where .ts files will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out_dir: PathBuf,

    /// Optional path to the Prettier executable to format generated files
    #[arg(short = 'p', long = "prettier", value_name = "PRETTIER_BIN")]
    prettier: Option<PathBuf>,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    experimental: bool,
}

#[derive(Debug, Args)]
struct GenerateJsonSchemaCommand {
    /// Output directory where the schema bundle will be written
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out_dir: PathBuf,

    /// Include experimental methods and fields in the generated output
    #[arg(long = "experimental", default_value_t = false)]
    experimental: bool,
}

#[derive(Debug, Parser)]
struct StdioToUdsCommand {
    /// Path to the Unix domain socket to connect to.
    #[arg(value_name = "SOCKET_PATH")]
    socket_path: PathBuf,
}

fn format_exit_messages(exit_info: AppExitInfo, color_enabled: bool) -> Vec<String> {
    let AppExitInfo {
        token_usage,
        thread_id: conversation_id,
        thread_name,
        ..
    } = exit_info;

    if token_usage.is_zero() {
        return Vec::new();
    }

    let mut lines = vec![format!(
        "{}",
        codex_core::protocol::FinalOutput::from(token_usage)
    )];

    if let Some(resume_cmd) =
        codex_core::util::resume_command(thread_name.as_deref(), conversation_id)
    {
        let command = if color_enabled {
            resume_cmd.cyan().to_string()
        } else {
            resume_cmd
        };
        lines.push(format!("To continue this session, run {command}"));
    }

    lines
}

/// Handle the app exit and print the results. Optionally run the update action.
fn handle_app_exit(exit_info: AppExitInfo) -> anyhow::Result<()> {
    match exit_info.exit_reason {
        ExitReason::Fatal(message) => {
            eprintln!("ERROR: {message}");
            std::process::exit(1);
        }
        ExitReason::UserRequested => { /* normal exit */ }
    }

    let update_action = exit_info.update_action;
    let color_enabled = supports_color::on(Stream::Stdout).is_some();
    for line in format_exit_messages(exit_info, color_enabled) {
        println!("{line}");
    }
    if let Some(action) = update_action {
        run_update_action(action)?;
    }
    Ok(())
}

/// Run the update action and print the result.
fn run_update_action(action: UpdateAction) -> anyhow::Result<()> {
    println!();
    let cmd_str = action.command_str();
    println!("Updating Codex via `{cmd_str}`...");

    let status = {
        #[cfg(windows)]
        {
            // On Windows, run via cmd.exe so .CMD/.BAT are correctly resolved (PATHEXT semantics).
            std::process::Command::new("cmd")
                .args(["/C", &cmd_str])
                .status()?
        }
        #[cfg(not(windows))]
        {
            let (cmd, args) = action.command_args();
            let command_path = crate::wsl_paths::normalize_for_wsl(cmd);
            let normalized_args: Vec<String> = args
                .iter()
                .map(crate::wsl_paths::normalize_for_wsl)
                .collect();
            std::process::Command::new(&command_path)
                .args(&normalized_args)
                .status()?
        }
    };
    if !status.success() {
        anyhow::bail!("`{cmd_str}` failed with status {status}");
    }
    println!("\n🎉 Update ran successfully! Please restart Codex.");
    Ok(())
}

fn run_execpolicycheck(cmd: ExecPolicyCheckCommand) -> anyhow::Result<()> {
    cmd.run()
}

fn run_debug_app_server_command(cmd: DebugAppServerCommand) -> anyhow::Result<()> {
    match cmd.subcommand {
        DebugAppServerSubcommand::SendMessageV2(cmd) => {
            let codex_bin = std::env::current_exe()?;
            codex_app_server_test_client::send_message_v2(&codex_bin, &[], cmd.user_message, &None)
        }
    }
}

#[derive(Debug, Default, Parser, Clone)]
struct FeatureToggles {
    /// Enable a feature (repeatable). Equivalent to `-c features.<name>=true`.
    #[arg(long = "enable", value_name = "FEATURE", action = clap::ArgAction::Append, global = true)]
    enable: Vec<String>,

    /// Disable a feature (repeatable). Equivalent to `-c features.<name>=false`.
    #[arg(long = "disable", value_name = "FEATURE", action = clap::ArgAction::Append, global = true)]
    disable: Vec<String>,
}

impl FeatureToggles {
    fn to_overrides(&self) -> anyhow::Result<Vec<String>> {
        let mut v = Vec::new();
        for feature in &self.enable {
            Self::validate_feature(feature)?;
            v.push(format!("features.{feature}=true"));
        }
        for feature in &self.disable {
            Self::validate_feature(feature)?;
            v.push(format!("features.{feature}=false"));
        }
        Ok(v)
    }

    fn validate_feature(feature: &str) -> anyhow::Result<()> {
        if is_known_feature_key(feature) {
            Ok(())
        } else {
            anyhow::bail!("Unknown feature flag: {feature}")
        }
    }
}

#[derive(Debug, Parser)]
struct FeaturesCli {
    #[command(subcommand)]
    sub: FeaturesSubcommand,
}

#[derive(Debug, Parser)]
enum FeaturesSubcommand {
    /// List known features with their stage and effective state.
    List,
    /// Enable a feature in config.toml.
    Enable(FeatureSetArgs),
    /// Disable a feature in config.toml.
    Disable(FeatureSetArgs),
}

#[derive(Debug, Parser)]
struct FeatureSetArgs {
    /// Feature key to update (for example: unified_exec).
    feature: String,
}

fn stage_str(stage: codex_core::features::Stage) -> &'static str {
    use codex_core::features::Stage;
    match stage {
        Stage::UnderDevelopment => "under development",
        Stage::Experimental { .. } => "experimental",
        Stage::Stable => "stable",
        Stage::Deprecated => "deprecated",
        Stage::Removed => "removed",
    }
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
        developer_instructions: config.developer_instructions.clone(),
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

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(|codex_linux_sandbox_exe| async move {
        cli_main(codex_linux_sandbox_exe).await?;
        Ok(())
    })
}

async fn cli_main(codex_linux_sandbox_exe: Option<PathBuf>) -> anyhow::Result<()> {
    let MultitoolCli {
        config_overrides: mut root_config_overrides,
        feature_toggles,
        mut interactive,
        subcommand,
    } = MultitoolCli::parse();

    // Fold --enable/--disable into config overrides so they flow to all subcommands.
    let toggle_overrides = feature_toggles.to_overrides()?;
    root_config_overrides.raw_overrides.extend(toggle_overrides);

    match subcommand {
        None => {
            prepend_config_flags(
                &mut interactive.config_overrides,
                root_config_overrides.clone(),
            );
            let exit_info = run_interactive_tui(interactive, codex_linux_sandbox_exe).await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Exec(mut exec_cli)) => {
            prepend_config_flags(
                &mut exec_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_exec::run_main(exec_cli, codex_linux_sandbox_exe).await?;
        }
        Some(Subcommand::Review(review_args)) => {
            let mut exec_cli = ExecCli::try_parse_from(["codex", "exec"])?;
            exec_cli.command = Some(ExecCommand::Review(review_args));
            prepend_config_flags(
                &mut exec_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_exec::run_main(exec_cli, codex_linux_sandbox_exe).await?;
        }
        Some(Subcommand::Research(research_args)) => {
            validate_research_inputs(&research_args)?;
            let resolved_output_path =
                resolve_path_for_session(&research_args.output_path, interactive.cwd.as_deref());
            let runtime_context = build_research_prompt_runtime_context(
                &research_args,
                &interactive,
                &root_config_overrides,
            )
            .await?;
            let research_prompt = build_research_command_prompt(
                &research_args,
                &runtime_context,
                &resolved_output_path,
            )?;
            interactive.prompt = Some(research_prompt);
            interactive
                .config_overrides
                .raw_overrides
                .push("features.research=true".to_string());
            interactive
                .config_overrides
                .raw_overrides
                .push("features.collab=true".to_string());
            interactive
                .config_overrides
                .raw_overrides
                .push("features.apps=true".to_string());
            interactive
                .config_overrides
                .raw_overrides
                .push("features.apply_patch_freeform=true".to_string());
            interactive.config_overrides.raw_overrides.push(
                build_research_developer_instruction_override(
                    runtime_context.developer_instructions.as_deref(),
                ),
            );
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
            prepend_config_flags(
                &mut interactive.config_overrides,
                root_config_overrides.clone(),
            );
            let exit_info = run_interactive_tui(interactive, codex_linux_sandbox_exe).await?;
            if !matches!(exit_info.exit_reason, ExitReason::Fatal(_)) {
                run_research_post_session_hook(&resolved_output_path);
            }
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::McpServer) => {
            codex_mcp_server::run_main(codex_linux_sandbox_exe, root_config_overrides).await?;
        }
        Some(Subcommand::Mcp(mut mcp_cli)) => {
            // Propagate any root-level config overrides (e.g. `-c key=value`).
            prepend_config_flags(&mut mcp_cli.config_overrides, root_config_overrides.clone());
            mcp_cli.run().await?;
        }
        Some(Subcommand::AppServer(app_server_cli)) => match app_server_cli.subcommand {
            None => {
                let transport = app_server_cli.listen;
                codex_app_server::run_main_with_transport(
                    codex_linux_sandbox_exe,
                    root_config_overrides,
                    codex_core::config_loader::LoaderOverrides::default(),
                    app_server_cli.analytics_default_enabled,
                    transport,
                )
                .await?;
            }
            Some(AppServerSubcommand::GenerateTs(gen_cli)) => {
                let options = codex_app_server_protocol::GenerateTsOptions {
                    experimental_api: gen_cli.experimental,
                    ..Default::default()
                };
                codex_app_server_protocol::generate_ts_with_options(
                    &gen_cli.out_dir,
                    gen_cli.prettier.as_deref(),
                    options,
                )?;
            }
            Some(AppServerSubcommand::GenerateJsonSchema(gen_cli)) => {
                codex_app_server_protocol::generate_json_with_experimental(
                    &gen_cli.out_dir,
                    gen_cli.experimental,
                )?;
            }
        },
        #[cfg(target_os = "macos")]
        Some(Subcommand::App(app_cli)) => {
            app_cmd::run_app(app_cli).await?;
        }
        Some(Subcommand::Resume(ResumeCommand {
            session_id,
            last,
            all,
            config_overrides,
        })) => {
            interactive = finalize_resume_interactive(
                interactive,
                root_config_overrides.clone(),
                session_id,
                last,
                all,
                config_overrides,
            );
            let exit_info = run_interactive_tui(interactive, codex_linux_sandbox_exe).await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Fork(ForkCommand {
            session_id,
            last,
            all,
            config_overrides,
        })) => {
            interactive = finalize_fork_interactive(
                interactive,
                root_config_overrides.clone(),
                session_id,
                last,
                all,
                config_overrides,
            );
            let exit_info = run_interactive_tui(interactive, codex_linux_sandbox_exe).await?;
            handle_app_exit(exit_info)?;
        }
        Some(Subcommand::Login(mut login_cli)) => {
            prepend_config_flags(
                &mut login_cli.config_overrides,
                root_config_overrides.clone(),
            );
            match login_cli.action {
                Some(LoginSubcommand::Status) => {
                    run_login_status(login_cli.config_overrides).await;
                }
                Some(LoginSubcommand::List) => {
                    run_list_providers(login_cli.config_overrides).await;
                }
                None => {
                    if login_cli.use_device_code {
                        run_login_with_device_code(
                            login_cli.config_overrides,
                            login_cli.issuer_base_url,
                            login_cli.client_id,
                        )
                        .await;
                    } else if login_cli.api_key.is_some() {
                        eprintln!(
                            "The --api-key flag is no longer supported. Pipe the key instead, e.g. `printenv OPENAI_API_KEY | codex login --with-api-key`."
                        );
                        std::process::exit(1);
                    } else if login_cli.with_api_key {
                        let api_key = read_api_key_from_stdin();
                        run_login_with_provider_api_key(
                            login_cli.config_overrides,
                            api_key,
                            login_cli.provider,
                        )
                        .await;
                    } else {
                        run_login_with_chatgpt(login_cli.config_overrides).await;
                    }
                }
            }
        }
        Some(Subcommand::Logout(mut logout_cli)) => {
            prepend_config_flags(
                &mut logout_cli.config_overrides,
                root_config_overrides.clone(),
            );
            run_logout_provider(logout_cli.config_overrides, logout_cli.provider).await;
        }
        Some(Subcommand::Completion(completion_cli)) => {
            print_completion(completion_cli);
        }
        Some(Subcommand::Cloud(mut cloud_cli)) => {
            prepend_config_flags(
                &mut cloud_cli.config_overrides,
                root_config_overrides.clone(),
            );
            codex_cloud_tasks::run_main(cloud_cli, codex_linux_sandbox_exe).await?;
        }
        Some(Subcommand::Sandbox(sandbox_args)) => match sandbox_args.cmd {
            SandboxCommand::Macos(mut seatbelt_cli) => {
                prepend_config_flags(
                    &mut seatbelt_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_seatbelt(
                    seatbelt_cli,
                    codex_linux_sandbox_exe,
                )
                .await?;
            }
            SandboxCommand::Linux(mut landlock_cli) => {
                prepend_config_flags(
                    &mut landlock_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_landlock(
                    landlock_cli,
                    codex_linux_sandbox_exe,
                )
                .await?;
            }
            SandboxCommand::Windows(mut windows_cli) => {
                prepend_config_flags(
                    &mut windows_cli.config_overrides,
                    root_config_overrides.clone(),
                );
                codex_cli::debug_sandbox::run_command_under_windows(
                    windows_cli,
                    codex_linux_sandbox_exe,
                )
                .await?;
            }
        },
        Some(Subcommand::Debug(DebugCommand { subcommand })) => match subcommand {
            DebugSubcommand::AppServer(cmd) => {
                run_debug_app_server_command(cmd)?;
            }
        },
        Some(Subcommand::Execpolicy(ExecpolicyCommand { sub })) => match sub {
            ExecpolicySubcommand::Check(cmd) => run_execpolicycheck(cmd)?,
        },
        Some(Subcommand::Apply(mut apply_cli)) => {
            prepend_config_flags(
                &mut apply_cli.config_overrides,
                root_config_overrides.clone(),
            );
            run_apply_command(apply_cli, None).await?;
        }
        Some(Subcommand::ResponsesApiProxy(args)) => {
            tokio::task::spawn_blocking(move || codex_responses_api_proxy::run_main(args))
                .await??;
        }
        Some(Subcommand::StdioToUds(cmd)) => {
            let socket_path = cmd.socket_path;
            tokio::task::spawn_blocking(move || codex_stdio_to_uds::run(socket_path.as_path()))
                .await??;
        }
        Some(Subcommand::Features(FeaturesCli { sub })) => match sub {
            FeaturesSubcommand::List => {
                // Respect root-level `-c` overrides plus top-level flags like `--profile`.
                let mut cli_kv_overrides = root_config_overrides
                    .parse_overrides()
                    .map_err(anyhow::Error::msg)?;

                // Honor `--search` via the canonical web_search mode.
                if interactive.web_search {
                    cli_kv_overrides.push((
                        "web_search".to_string(),
                        toml::Value::String("live".to_string()),
                    ));
                }

                // Thread through relevant top-level flags (at minimum, `--profile`).
                let overrides = ConfigOverrides {
                    config_profile: interactive.config_profile.clone(),
                    ..Default::default()
                };

                let config = Config::load_with_cli_overrides_and_harness_overrides(
                    cli_kv_overrides,
                    overrides,
                )
                .await?;
                let mut rows = Vec::with_capacity(codex_core::features::FEATURES.len());
                let mut name_width = 0;
                let mut stage_width = 0;
                for def in codex_core::features::FEATURES.iter() {
                    let name = def.key;
                    let stage = stage_str(def.stage);
                    let enabled = config.features.enabled(def.id);
                    name_width = name_width.max(name.len());
                    stage_width = stage_width.max(stage.len());
                    rows.push((name, stage, enabled));
                }

                for (name, stage, enabled) in rows {
                    println!("{name:<name_width$}  {stage:<stage_width$}  {enabled}");
                }
            }
            FeaturesSubcommand::Enable(FeatureSetArgs { feature }) => {
                enable_feature_in_config(&interactive, &feature).await?;
            }
            FeaturesSubcommand::Disable(FeatureSetArgs { feature }) => {
                disable_feature_in_config(&interactive, &feature).await?;
            }
        },
    }

    Ok(())
}

async fn enable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, true)
        .apply()
        .await?;
    println!("Enabled feature `{feature}` in config.toml.");
    maybe_print_under_development_feature_warning(&codex_home, interactive, feature);
    Ok(())
}

async fn disable_feature_in_config(interactive: &TuiCli, feature: &str) -> anyhow::Result<()> {
    FeatureToggles::validate_feature(feature)?;
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .with_profile(interactive.config_profile.as_deref())
        .set_feature_enabled(feature, false)
        .apply()
        .await?;
    println!("Disabled feature `{feature}` in config.toml.");
    Ok(())
}

fn maybe_print_under_development_feature_warning(
    codex_home: &std::path::Path,
    interactive: &TuiCli,
    feature: &str,
) {
    if interactive.config_profile.is_some() {
        return;
    }

    let Some(spec) = codex_core::features::FEATURES
        .iter()
        .find(|spec| spec.key == feature)
    else {
        return;
    };
    if !matches!(spec.stage, Stage::UnderDevelopment) {
        return;
    }

    let config_path = codex_home.join(codex_core::config::CONFIG_TOML_FILE);
    eprintln!(
        "Under-development features enabled: {feature}. Under-development features are incomplete and may behave unpredictably. To suppress this warning, set `suppress_unstable_features_warning = true` in {}.",
        config_path.display()
    );
}

/// Prepend root-level overrides so they have lower precedence than
/// CLI-specific ones specified after the subcommand (if any).
fn prepend_config_flags(
    subcommand_config_overrides: &mut CliConfigOverrides,
    cli_config_overrides: CliConfigOverrides,
) {
    subcommand_config_overrides
        .raw_overrides
        .splice(0..0, cli_config_overrides.raw_overrides);
}

async fn run_interactive_tui(
    mut interactive: TuiCli,
    codex_linux_sandbox_exe: Option<PathBuf>,
) -> std::io::Result<AppExitInfo> {
    if let Some(prompt) = interactive.prompt.take() {
        // Normalize CRLF/CR to LF so CLI-provided text can't leak `\r` into TUI state.
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    let terminal_info = codex_core::terminal::terminal_info();
    if terminal_info.name == TerminalName::Dumb {
        if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
            return Ok(AppExitInfo::fatal(
                "TERM is set to \"dumb\". Refusing to start the interactive TUI because no terminal is available for a confirmation prompt (stdin/stderr is not a TTY). Run in a supported terminal or unset TERM.",
            ));
        }

        eprintln!(
            "WARNING: TERM is set to \"dumb\". Codex's interactive TUI may not work in this terminal."
        );
        if !confirm("Continue anyway? [y/N]: ")? {
            return Ok(AppExitInfo::fatal(
                "Refusing to start the interactive TUI because TERM is set to \"dumb\". Run in a supported terminal or unset TERM.",
            ));
        }
    }

    codex_tui::run_main(interactive, codex_linux_sandbox_exe).await
}

fn confirm(prompt: &str) -> std::io::Result<bool> {
    eprintln!("{prompt}");

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let answer = input.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}

/// Build the final `TuiCli` for a `codex resume` invocation.
fn finalize_resume_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    resume_cli: TuiCli,
) -> TuiCli {
    // Start with the parsed interactive CLI so resume shares the same
    // configuration surface area as `codex` without additional flags.
    let resume_session_id = session_id;
    interactive.resume_picker = resume_session_id.is_none() && !last;
    interactive.resume_last = last;
    interactive.resume_session_id = resume_session_id;
    interactive.resume_show_all = show_all;

    // Merge resume-scoped flags and overrides with highest precedence.
    merge_interactive_cli_flags(&mut interactive, resume_cli);

    // Propagate any root-level config overrides (e.g. `-c key=value`).
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

/// Build the final `TuiCli` for a `codex fork` invocation.
fn finalize_fork_interactive(
    mut interactive: TuiCli,
    root_config_overrides: CliConfigOverrides,
    session_id: Option<String>,
    last: bool,
    show_all: bool,
    fork_cli: TuiCli,
) -> TuiCli {
    // Start with the parsed interactive CLI so fork shares the same
    // configuration surface area as `codex` without additional flags.
    let fork_session_id = session_id;
    interactive.fork_picker = fork_session_id.is_none() && !last;
    interactive.fork_last = last;
    interactive.fork_session_id = fork_session_id;
    interactive.fork_show_all = show_all;

    // Merge fork-scoped flags and overrides with highest precedence.
    merge_interactive_cli_flags(&mut interactive, fork_cli);

    // Propagate any root-level config overrides (e.g. `-c key=value`).
    prepend_config_flags(&mut interactive.config_overrides, root_config_overrides);

    interactive
}

/// Merge flags provided to `codex resume`/`codex fork` so they take precedence over any
/// root-level flags. Only overrides fields explicitly set on the subcommand-scoped
/// CLI. Also appends `-c key=value` overrides with highest precedence.
fn merge_interactive_cli_flags(interactive: &mut TuiCli, subcommand_cli: TuiCli) {
    if let Some(model) = subcommand_cli.model {
        interactive.model = Some(model);
    }
    if subcommand_cli.oss {
        interactive.oss = true;
    }
    if let Some(profile) = subcommand_cli.config_profile {
        interactive.config_profile = Some(profile);
    }
    if let Some(sandbox) = subcommand_cli.sandbox_mode {
        interactive.sandbox_mode = Some(sandbox);
    }
    if let Some(approval) = subcommand_cli.approval_policy {
        interactive.approval_policy = Some(approval);
    }
    if subcommand_cli.full_auto {
        interactive.full_auto = true;
    }
    if subcommand_cli.dangerously_bypass_approvals_and_sandbox {
        interactive.dangerously_bypass_approvals_and_sandbox = true;
    }
    if let Some(cwd) = subcommand_cli.cwd {
        interactive.cwd = Some(cwd);
    }
    if subcommand_cli.web_search {
        interactive.web_search = true;
    }
    if !subcommand_cli.images.is_empty() {
        interactive.images = subcommand_cli.images;
    }
    if !subcommand_cli.add_dir.is_empty() {
        interactive.add_dir.extend(subcommand_cli.add_dir);
    }
    if let Some(prompt) = subcommand_cli.prompt {
        // Normalize CRLF/CR to LF so CLI-provided text can't leak `\r` into TUI state.
        interactive.prompt = Some(prompt.replace("\r\n", "\n").replace('\r', "\n"));
    }

    interactive
        .config_overrides
        .raw_overrides
        .extend(subcommand_cli.config_overrides.raw_overrides);
}

fn print_completion(cmd: CompletionCommand) {
    let mut app = MultitoolCli::command();
    let name = "codex";
    generate(cmd.shell, &mut app, name, &mut std::io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use codex_core::protocol::TokenUsage;
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;

    fn prompt_runtime_context_for_tests() -> ResearchPromptRuntimeContext {
        ResearchPromptRuntimeContext {
            tool_names: ResearchToolNames::from_available_native(),
            tool_availability: ResearchToolAvailability {
                has_paper_search: true,
                has_zotero: true,
                has_repo_analysis: true,
            },
            has_web_search: true,
            has_user_codebase: false,
            codebase_path: None,
            developer_instructions: None,
        }
    }

    fn finalize_resume_from_args(args: &[&str]) -> TuiCli {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let MultitoolCli {
            interactive,
            config_overrides: root_overrides,
            subcommand,
            feature_toggles: _,
        } = cli;

        let Subcommand::Resume(ResumeCommand {
            session_id,
            last,
            all,
            config_overrides: resume_cli,
        }) = subcommand.expect("resume present")
        else {
            unreachable!()
        };

        finalize_resume_interactive(
            interactive,
            root_overrides,
            session_id,
            last,
            all,
            resume_cli,
        )
    }

    fn finalize_fork_from_args(args: &[&str]) -> TuiCli {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let MultitoolCli {
            interactive,
            config_overrides: root_overrides,
            subcommand,
            feature_toggles: _,
        } = cli;

        let Subcommand::Fork(ForkCommand {
            session_id,
            last,
            all,
            config_overrides: fork_cli,
        }) = subcommand.expect("fork present")
        else {
            unreachable!()
        };

        finalize_fork_interactive(interactive, root_overrides, session_id, last, all, fork_cli)
    }

    #[test]
    fn exec_resume_last_accepts_prompt_positional() {
        let cli =
            MultitoolCli::try_parse_from(["codex", "exec", "--json", "resume", "--last", "2+2"])
                .expect("parse should succeed");

        let Some(Subcommand::Exec(exec)) = cli.subcommand else {
            panic!("expected exec subcommand");
        };
        let Some(codex_exec::Command::Resume(args)) = exec.command else {
            panic!("expected exec resume");
        };

        assert!(args.last);
        assert_eq!(args.session_id, None);
        assert_eq!(args.prompt.as_deref(), Some("2+2"));
    }

    fn app_server_from_args(args: &[&str]) -> AppServerCommand {
        let cli = MultitoolCli::try_parse_from(args).expect("parse");
        let Subcommand::AppServer(app_server) = cli.subcommand.expect("app-server present") else {
            unreachable!()
        };
        app_server
    }

    fn sample_exit_info(conversation_id: Option<&str>, thread_name: Option<&str>) -> AppExitInfo {
        let token_usage = TokenUsage {
            output_tokens: 2,
            total_tokens: 2,
            ..Default::default()
        };
        AppExitInfo {
            token_usage,
            thread_id: conversation_id
                .map(ThreadId::from_string)
                .map(Result::unwrap),
            thread_name: thread_name.map(str::to_string),
            update_action: None,
            exit_reason: ExitReason::UserRequested,
        }
    }

    #[test]
    fn format_exit_messages_skips_zero_usage() {
        let exit_info = AppExitInfo {
            token_usage: TokenUsage::default(),
            thread_id: None,
            thread_name: None,
            update_action: None,
            exit_reason: ExitReason::UserRequested,
        };
        let lines = format_exit_messages(exit_info, false);
        assert!(lines.is_empty());
    }

    #[test]
    fn format_exit_messages_includes_resume_hint_without_color() {
        let exit_info = sample_exit_info(Some("123e4567-e89b-12d3-a456-426614174000"), None);
        let lines = format_exit_messages(exit_info, false);
        assert_eq!(
            lines,
            vec![
                "Token usage: total=2 input=0 output=2".to_string(),
                "To continue this session, run codex resume 123e4567-e89b-12d3-a456-426614174000"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn format_exit_messages_applies_color_when_enabled() {
        let exit_info = sample_exit_info(Some("123e4567-e89b-12d3-a456-426614174000"), None);
        let lines = format_exit_messages(exit_info, true);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("\u{1b}[36m"));
    }

    #[test]
    fn format_exit_messages_prefers_thread_name() {
        let exit_info = sample_exit_info(
            Some("123e4567-e89b-12d3-a456-426614174000"),
            Some("my-thread"),
        );
        let lines = format_exit_messages(exit_info, false);
        assert_eq!(
            lines,
            vec![
                "Token usage: total=2 input=0 output=2".to_string(),
                "To continue this session, run codex resume my-thread".to_string(),
            ]
        );
    }

    #[test]
    fn resume_model_flag_applies_when_no_root_flags() {
        let interactive =
            finalize_resume_from_args(["codex", "resume", "-m", "gpt-5.1-test"].as_ref());

        assert_eq!(interactive.model.as_deref(), Some("gpt-5.1-test"));
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
    }

    #[test]
    fn resume_picker_logic_none_and_not_last() {
        let interactive = finalize_resume_from_args(["codex", "resume"].as_ref());
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
        assert!(!interactive.resume_show_all);
    }

    #[test]
    fn resume_picker_logic_last() {
        let interactive = finalize_resume_from_args(["codex", "resume", "--last"].as_ref());
        assert!(!interactive.resume_picker);
        assert!(interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
        assert!(!interactive.resume_show_all);
    }

    #[test]
    fn resume_picker_logic_with_session_id() {
        let interactive = finalize_resume_from_args(["codex", "resume", "1234"].as_ref());
        assert!(!interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id.as_deref(), Some("1234"));
        assert!(!interactive.resume_show_all);
    }

    #[test]
    fn resume_all_flag_sets_show_all() {
        let interactive = finalize_resume_from_args(["codex", "resume", "--all"].as_ref());
        assert!(interactive.resume_picker);
        assert!(interactive.resume_show_all);
    }

    #[test]
    fn resume_merges_option_flags_and_full_auto() {
        let interactive = finalize_resume_from_args(
            [
                "codex",
                "resume",
                "sid",
                "--oss",
                "--full-auto",
                "--search",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "-m",
                "gpt-5.1-test",
                "-p",
                "my-profile",
                "-C",
                "/tmp",
                "-i",
                "/tmp/a.png,/tmp/b.png",
            ]
            .as_ref(),
        );

        assert_eq!(interactive.model.as_deref(), Some("gpt-5.1-test"));
        assert!(interactive.oss);
        assert_eq!(interactive.config_profile.as_deref(), Some("my-profile"));
        assert_matches!(
            interactive.sandbox_mode,
            Some(codex_common::SandboxModeCliArg::WorkspaceWrite)
        );
        assert_matches!(
            interactive.approval_policy,
            Some(codex_common::ApprovalModeCliArg::OnRequest)
        );
        assert!(interactive.full_auto);
        assert_eq!(
            interactive.cwd.as_deref(),
            Some(std::path::Path::new("/tmp"))
        );
        assert!(interactive.web_search);
        let has_a = interactive
            .images
            .iter()
            .any(|p| p == std::path::Path::new("/tmp/a.png"));
        let has_b = interactive
            .images
            .iter()
            .any(|p| p == std::path::Path::new("/tmp/b.png"));
        assert!(has_a && has_b);
        assert!(!interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id.as_deref(), Some("sid"));
    }

    #[test]
    fn resume_merges_dangerously_bypass_flag() {
        let interactive = finalize_resume_from_args(
            [
                "codex",
                "resume",
                "--dangerously-bypass-approvals-and-sandbox",
            ]
            .as_ref(),
        );
        assert!(interactive.dangerously_bypass_approvals_and_sandbox);
        assert!(interactive.resume_picker);
        assert!(!interactive.resume_last);
        assert_eq!(interactive.resume_session_id, None);
    }

    #[test]
    fn fork_picker_logic_none_and_not_last() {
        let interactive = finalize_fork_from_args(["codex", "fork"].as_ref());
        assert!(interactive.fork_picker);
        assert!(!interactive.fork_last);
        assert_eq!(interactive.fork_session_id, None);
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_picker_logic_last() {
        let interactive = finalize_fork_from_args(["codex", "fork", "--last"].as_ref());
        assert!(!interactive.fork_picker);
        assert!(interactive.fork_last);
        assert_eq!(interactive.fork_session_id, None);
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_picker_logic_with_session_id() {
        let interactive = finalize_fork_from_args(["codex", "fork", "1234"].as_ref());
        assert!(!interactive.fork_picker);
        assert!(!interactive.fork_last);
        assert_eq!(interactive.fork_session_id.as_deref(), Some("1234"));
        assert!(!interactive.fork_show_all);
    }

    #[test]
    fn fork_all_flag_sets_show_all() {
        let interactive = finalize_fork_from_args(["codex", "fork", "--all"].as_ref());
        assert!(interactive.fork_picker);
        assert!(interactive.fork_show_all);
    }

    #[test]
    fn app_server_analytics_default_disabled_without_flag() {
        let app_server = app_server_from_args(["codex", "app-server"].as_ref());
        assert!(!app_server.analytics_default_enabled);
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::Stdio
        );
    }

    #[test]
    fn app_server_analytics_default_enabled_with_flag() {
        let app_server =
            app_server_from_args(["codex", "app-server", "--analytics-default-enabled"].as_ref());
        assert!(app_server.analytics_default_enabled);
    }

    #[test]
    fn app_server_listen_websocket_url_parses() {
        let app_server = app_server_from_args(
            ["codex", "app-server", "--listen", "ws://127.0.0.1:4500"].as_ref(),
        );
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::WebSocket {
                bind_address: "127.0.0.1:4500".parse().expect("valid socket address"),
            }
        );
    }

    #[test]
    fn app_server_listen_stdio_url_parses() {
        let app_server =
            app_server_from_args(["codex", "app-server", "--listen", "stdio://"].as_ref());
        assert_eq!(
            app_server.listen,
            codex_app_server::AppServerTransport::Stdio
        );
    }

    #[test]
    fn app_server_listen_invalid_url_fails_to_parse() {
        let parse_result =
            MultitoolCli::try_parse_from(["codex", "app-server", "--listen", "http://foo"]);
        assert!(parse_result.is_err());
    }

    #[test]
    fn features_enable_parses_feature_name() {
        let cli = MultitoolCli::try_parse_from(["codex", "features", "enable", "unified_exec"])
            .expect("parse should succeed");
        let Some(Subcommand::Features(FeaturesCli { sub })) = cli.subcommand else {
            panic!("expected features subcommand");
        };
        let FeaturesSubcommand::Enable(FeatureSetArgs { feature }) = sub else {
            panic!("expected features enable");
        };
        assert_eq!(feature, "unified_exec");
    }

    #[test]
    fn features_disable_parses_feature_name() {
        let cli = MultitoolCli::try_parse_from(["codex", "features", "disable", "shell_tool"])
            .expect("parse should succeed");
        let Some(Subcommand::Features(FeaturesCli { sub })) = cli.subcommand else {
            panic!("expected features subcommand");
        };
        let FeaturesSubcommand::Disable(FeatureSetArgs { feature }) = sub else {
            panic!("expected features disable");
        };
        assert_eq!(feature, "shell_tool");
    }

    #[test]
    fn feature_toggles_known_features_generate_overrides() {
        let toggles = FeatureToggles {
            enable: vec!["web_search_request".to_string()],
            disable: vec!["unified_exec".to_string()],
        };
        let overrides = toggles.to_overrides().expect("valid features");
        assert_eq!(
            overrides,
            vec![
                "features.web_search_request=true".to_string(),
                "features.unified_exec=false".to_string(),
            ]
        );
    }

    #[test]
    fn feature_toggles_unknown_feature_errors() {
        let toggles = FeatureToggles {
            enable: vec!["does_not_exist".to_string()],
            disable: Vec::new(),
        };
        let err = toggles
            .to_overrides()
            .expect_err("feature should be rejected");
        assert_eq!(err.to_string(), "Unknown feature flag: does_not_exist");
    }

    #[test]
    fn research_subcommand_parses_arguments() {
        let cli = MultitoolCli::try_parse_from([
            "codex",
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
            "codex",
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
            "codex",
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
