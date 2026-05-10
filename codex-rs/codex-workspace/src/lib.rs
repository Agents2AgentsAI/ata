pub mod audit;
pub mod commands;
pub mod error;
pub mod git;
pub mod lock;
pub mod manifest;
pub mod paths;
pub mod recipes;
pub mod resolve;
pub mod selection;
pub mod spec;
pub mod types;
pub mod url_validation;
pub mod workspace_id;
pub mod workspace_resolution;

use clap::Args;
use clap::CommandFactory;
use clap::Parser;
use error::WorkspaceError;

fn parse_positive_usize(input: &str) -> std::result::Result<usize, String> {
    let value = input
        .parse::<usize>()
        .map_err(|err| format!("invalid usize value '{input}': {err}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_string());
    }
    Ok(value)
}

/// Workspace management CLI.
#[derive(Debug, Parser)]
#[clap(name = "workspace")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Create a new workspace.
    Init {
        /// Workspace name.
        name: String,
    },

    /// List all workspaces.
    List,

    /// Search workspace commands and print simplified manuals for the best matches.
    SearchCommands(SearchCommandsArgs),

    /// Print workspace manifest.
    Read {
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Set active workspace.
    Select {
        /// Workspace selector: ID, exact name, or slugified name.
        selector: String,
    },

    /// Remove workspace directory tree.
    Delete {
        /// Workspace ID to delete.
        id: String,
        /// Required for destructive deletion.
        #[arg(long)]
        force: bool,
    },

    /// Resolve @-path alias to absolute path.
    Resolve {
        /// Path spec (e.g., @repo/file.txt).
        spec: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Validate repo URL and check host allowlist.
    CheckHost {
        /// Repository URL to validate.
        url: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Append audit entry.
    Audit {
        /// JSON audit entry.
        json: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Query audit log entries.
    AuditQuery {
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
        /// Filter: entries after unix timestamp.
        #[arg(long)]
        since: Option<i64>,
        /// Filter: entries before unix timestamp.
        #[arg(long)]
        until: Option<i64>,
        /// Filter: comma-separated operation names.
        #[arg(long)]
        ops: Option<String>,
        /// Max entries to return.
        #[arg(long, default_value = "200")]
        limit: usize,
    },

    /// Validate workspace manifest and on-disk repo/run directories.
    Validate {
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Run command under fine-grained lock.
    RunLocked {
        /// Lock level.
        #[arg(long, value_parser = ["workspace", "kb", "run", "index"])]
        level: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
        /// Target ID (required for run/index levels).
        #[arg(long)]
        target_id: Option<String>,
        /// Command to run under lock.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },

    /// Print shared mirror cache path for a repo URL.
    MirrorPath {
        /// Repository URL.
        url: String,
    },

    /// Validate, clone, register, and audit a repo.
    RepoClone {
        /// Repository URL (https:// only).
        url: String,
        /// Local alias for the repo.
        alias: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
        /// Full clone (ignore depth/filter policy).
        #[arg(long)]
        full: bool,
    },

    /// Update repo git state (headSha, headRef).
    RepoUpdateState {
        /// Repo alias.
        #[arg(long)]
        alias: String,
        /// New HEAD SHA.
        #[arg(long)]
        head_sha: String,
        /// New HEAD ref.
        #[arg(long)]
        head_ref: Option<String>,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Pin a repo to a specific SHA.
    RepoPin {
        /// Repo alias.
        #[arg(long)]
        alias: String,
        /// Commit SHA to pin to.
        #[arg(long)]
        sha: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Unpin a repo (switch back to tracking mode).
    RepoUnpin {
        /// Repo alias.
        #[arg(long)]
        alias: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Remove a repo: delete dir, manifest entry, audit.
    RepoRemove {
        /// Repo alias.
        #[arg(long)]
        alias: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Create run directory, materialize code, register, and audit.
    RunSetup {
        /// Human-readable run name.
        name: String,
        /// Source repo alias.
        #[arg(long)]
        source_alias: String,
        /// Code materialization strategy.
        #[arg(long, default_value = "worktree", value_parser = ["worktree", "copy", "clone"])]
        strategy: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Update a run's status.
    RunUpdateStatus {
        /// Run ID.
        #[arg(long)]
        id: String,
        /// New status value.
        #[arg(long)]
        status: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Remove a run: worktree cleanup, delete dir, manifest entry, audit.
    RunRemove {
        /// Run ID.
        #[arg(long)]
        id: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Append a JSON entry to a manifest collection.
    AddEntry {
        /// Collection name (papers, datasets, artifacts, links, snapshots, indexes).
        #[arg(long)]
        collection: String,
        /// JSON object to append.
        #[arg(long)]
        json: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Copy a paper markdown file into the workspace and register it by alias.
    AddPaper {
        /// Path to the extracted markdown/text file for the paper.
        text_md_path: String,
        /// Alias exposed as @papers/<alias>.
        #[arg(long)]
        alias: String,
        /// Optional display title. Defaults to the first markdown heading or file stem.
        #[arg(long)]
        title: Option<String>,
        /// Optional DOI metadata.
        #[arg(long)]
        doi: Option<String>,
        /// Optional path to the original PDF to store alongside the markdown.
        #[arg(long)]
        pdf_path: Option<String>,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Remove an entry by ID from a manifest collection.
    RemoveEntry {
        /// Collection name.
        #[arg(long)]
        collection: String,
        /// Entry ID to remove.
        #[arg(long)]
        id: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Set a field at a dotted path in the manifest.
    SetField {
        /// Dotted path (e.g., policies.repoHostsAllowlist).
        #[arg(long)]
        path: String,
        /// JSON value to set.
        #[arg(long)]
        value: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Update an index entry's status.
    IndexUpdateStatus {
        /// Index ID.
        #[arg(long)]
        id: String,
        /// New status value.
        #[arg(long)]
        status: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Print step-by-step recipe for an operation.
    Recipe {
        /// Operation name or 'list'.
        operation: String,
    },

    /// Materialize a workspace from a spec file.
    Materialize {
        /// Path to workspace-spec.json.
        spec_path: String,
        /// Workspace ID (default: create new from spec name, or resolved).
        #[arg(long)]
        workspace: Option<String>,
        /// Show what would change without executing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Export current workspace repos as a workspace spec file.
    ExportSpec {
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
        /// Output file path (prints to stdout if omitted).
        #[arg(long)]
        output: Option<String>,
    },

    /// Show what materialize would do (repos to add/pin/skip).
    DiffSpec {
        /// Path to workspace-spec.json.
        spec_path: String,
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },
}

#[derive(Debug, Args)]
pub struct SearchCommandsArgs {
    #[arg(value_name = "QUERY", num_args = 1.., required = true)]
    pub query: Vec<String>,

    #[arg(long, default_value_t = 3, value_parser = parse_positive_usize)]
    pub limit: usize,
}

/// Run the workspace CLI and print output to stdout/stderr.
/// Returns the process exit code.
pub fn run_cli(cli: Cli) -> i32 {
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            let code = e.exit_code();
            eprintln!("error: {e}");
            code
        }
    }
}

fn dispatch(cli: Cli) -> Result<i32, WorkspaceError> {
    match cli.command {
        Command::Init { name } => {
            let wid = commands::init::run(&name)?;
            println!("{wid}");
            Ok(0)
        }

        Command::List => {
            let results = commands::list::run()?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(0)
        }

        Command::SearchCommands(args) => {
            let query = args.query.join(" ");
            let matches = search_command_catalog(&query, args.limit);
            if matches.is_empty() {
                println!(
                    "No matching workspace command found for \"{}\".",
                    query.trim()
                );
                return Ok(0);
            }
            let best_match = matches[0];
            let manual = render_command_manual(best_match)?;
            println!("{}", render_search_results(&matches, &manual));
            Ok(0)
        }

        Command::Read { workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::read::run(&wid)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::Select { selector } => {
            let wid = commands::select::run(&selector)?;
            println!("selected: {wid}");
            Ok(0)
        }

        Command::Delete { id, force } => {
            commands::delete::run(&id, force)?;
            println!("deleted: {id}");
            Ok(0)
        }

        Command::Resolve { spec, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let path = commands::resolve::run(&wid, &spec)?;
            println!("{}", path.display());
            Ok(0)
        }

        Command::CheckHost { url, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            commands::check_host::run(&wid, &url)?;
            println!("{url}");
            Ok(0)
        }

        Command::Audit { json, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let entry = commands::audit::run(&wid, &json)?;
            println!("{}", serde_json::to_string_pretty(&entry)?);
            Ok(0)
        }

        Command::AuditQuery {
            workspace,
            since,
            until,
            ops,
            limit,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let results = commands::audit_query::run(&wid, since, until, ops.as_deref(), limit)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
            Ok(0)
        }

        Command::Validate { workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let report = commands::validate::run(&wid)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(if report.ok { 0 } else { 1 })
        }

        Command::RunLocked {
            level,
            workspace,
            target_id,
            mut command,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            // Strip leading '--' separator
            if command.first().map(String::as_str) == Some("--") {
                command.remove(0);
            }
            commands::run_locked::run(&wid, &level, target_id.as_deref(), &command)
        }

        Command::MirrorPath { url } => {
            let path = commands::mirror_path::run(&url);
            println!("{}", path.display());
            Ok(0)
        }

        Command::RepoClone {
            url,
            alias,
            workspace,
            full,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let result = commands::repo_clone::run(&wid, &url, &alias, full)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(0)
        }

        Command::RepoUpdateState {
            alias,
            head_sha,
            head_ref,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest =
                commands::repo_update_state::run(&wid, &alias, &head_sha, head_ref.as_deref())?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::RepoPin {
            alias,
            sha,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::repo_pin::run(&wid, &alias, &sha)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::RepoUnpin { alias, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::repo_unpin::run(&wid, &alias)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::RepoRemove { alias, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            commands::repo_remove::run(&wid, &alias)?;
            println!("removed: {alias}");
            Ok(0)
        }

        Command::RunSetup {
            name,
            source_alias,
            strategy,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let result = commands::run_setup::run(&wid, &name, &source_alias, &strategy)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(0)
        }

        Command::RunUpdateStatus {
            id,
            status,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::run_update_status::run(&wid, &id, &status)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::RunRemove { id, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            commands::run_remove::run(&wid, &id)?;
            println!("removed: {id}");
            Ok(0)
        }

        Command::AddEntry {
            collection,
            json,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::add_entry::run(&wid, &collection, &json)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::AddPaper {
            text_md_path,
            alias,
            title,
            doi,
            pdf_path,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::add_paper::run(
                &wid,
                std::path::Path::new(&text_md_path),
                &alias,
                title.as_deref(),
                doi.as_deref(),
                pdf_path.as_deref().map(std::path::Path::new),
            )?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::RemoveEntry {
            collection,
            id,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::remove_entry::run(&wid, &collection, &id)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::SetField {
            path,
            value,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::set_field::run(&wid, &path, &value)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::IndexUpdateStatus {
            id,
            status,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::index_update_status::run(&wid, &id, &status)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::Recipe { operation } => {
            let text = commands::recipe::run(&operation)?;
            print!("{text}");
            Ok(0)
        }

        Command::Materialize {
            spec_path,
            workspace,
            dry_run,
        } => {
            let wid = resolve_workspace_or_create_from_spec(workspace.as_deref(), &spec_path)?;
            let result =
                commands::materialize::run(&wid, std::path::Path::new(&spec_path), dry_run)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(0)
        }

        Command::ExportSpec { workspace, output } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let json =
                commands::export_spec::run(&wid, output.as_deref().map(std::path::Path::new))?;
            if output.is_none() {
                println!("{json}");
            }
            Ok(0)
        }

        Command::DiffSpec {
            spec_path,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let diff = commands::diff_spec::run(&wid, std::path::Path::new(&spec_path))?;
            print!("{diff}");
            Ok(0)
        }
    }
}

/// Resolve workspace for materialize: use explicit ID, resolved workspace,
/// or create a new one from the spec name.
fn resolve_workspace_or_create_from_spec(
    explicit: Option<&str>,
    spec_path: &str,
) -> Result<String, WorkspaceError> {
    let context = paths::SessionContext::from_env();

    if let Some(wid) = workspace_resolution::resolve_selected_workspace_for(
        &context.codex_home,
        context.cwd.as_deref(),
        explicit,
        context.session_id.as_deref(),
        context.thread_id.as_deref(),
    )? {
        return Ok(wid);
    }

    // Create a new workspace from the spec name
    let spec = spec::read_spec(std::path::Path::new(spec_path))?;
    let wid = commands::init::run(&spec.name)?;
    eprintln!("created workspace: {wid}");
    Ok(wid)
}

#[derive(Debug, Clone)]
struct WorkspaceCommandCatalogEntry {
    command: &'static str,
    description: &'static str,
    core_args: &'static [&'static str],
    aliases: &'static [&'static str],
    tags: &'static [&'static str],
    examples: &'static [&'static str],
}

fn search_command_catalog(query: &str, limit: usize) -> Vec<&'static WorkspaceCommandCatalogEntry> {
    let normalized_query = query.trim().to_lowercase();
    let tokens = tokenize_query(&normalized_query);
    let mut matches = workspace_command_catalog()
        .iter()
        .filter_map(|entry| {
            let score = score_catalog_entry(entry, &normalized_query, &tokens);
            (score > 0).then_some((score, entry))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.command.cmp(right.1.command))
    });
    matches
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect()
}

fn render_command_manual(entry: &WorkspaceCommandCatalogEntry) -> Result<String, WorkspaceError> {
    let mut command = Cli::command()
        .name("workspace")
        .bin_name("ata workspace")
        .disable_help_subcommand(true);
    let mut full_command = String::from("ata workspace");
    for segment in entry.command.split(' ') {
        full_command.push(' ');
        full_command.push_str(segment);
        command = command.find_subcommand(segment).cloned().ok_or_else(|| {
            WorkspaceError::InvalidSpec(format!(
                "missing workspace subcommand `{segment}` for `{full_command}`"
            ))
        })?;
    }
    command = command.bin_name(full_command);
    let arg_ids = command
        .get_arguments()
        .map(|arg| arg.get_id().to_string())
        .collect::<Vec<_>>();
    for arg_id in arg_ids {
        let keep = arg_id == "help" || entry.core_args.iter().any(|candidate| *candidate == arg_id);
        if !keep {
            command = command.mut_arg(&arg_id, |arg| arg.hide(true));
        }
    }
    let mut buffer = Vec::new();
    command.write_long_help(&mut buffer)?;
    let manual = String::from_utf8_lossy(&buffer).into_owned();
    let normalize = |line: &str| {
        line.trim()
            .trim_end_matches(|character: char| ['.', ':'].contains(&character))
            .to_ascii_lowercase()
    };
    let mut lines = manual.lines().peekable();
    while lines.peek().is_some_and(|line| line.trim().is_empty()) {
        lines.next();
    }
    if let Some(line) = lines.peek().copied()
        && normalize(line) == normalize(entry.description)
    {
        lines.next();
        while lines.peek().is_some_and(|line| line.trim().is_empty()) {
            lines.next();
        }
    }
    let manual_body = lines.collect::<Vec<_>>().join("\n");
    Ok(format!(
        "Command: {}\n{}\n\n{}",
        entry.command, entry.description, manual_body
    ))
}

fn render_search_results(matches: &[&WorkspaceCommandCatalogEntry], manual: &str) -> String {
    let shortlist = matches
        .iter()
        .enumerate()
        .map(|(index, entry)| format!("{}. {} — {}", index + 1, entry.command, entry.description))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Matches:\n{shortlist}\n\nBest match manual:\n\n{manual}")
}

fn tokenize_query(query: &str) -> Vec<&str> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

fn score_catalog_entry(
    entry: &WorkspaceCommandCatalogEntry,
    normalized_query: &str,
    tokens: &[&str],
) -> u32 {
    let command = entry.command.to_lowercase();
    let aliases = entry
        .aliases
        .iter()
        .map(|alias| alias.to_lowercase())
        .collect::<Vec<_>>();
    let tags = entry
        .tags
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect::<Vec<_>>();
    let description = entry.description.to_lowercase();
    let examples = entry
        .examples
        .iter()
        .map(|example| example.to_lowercase())
        .collect::<Vec<_>>();
    let search_text = std::iter::once(command.as_str())
        .chain(aliases.iter().map(String::as_str))
        .chain(tags.iter().map(String::as_str))
        .chain(std::iter::once(description.as_str()))
        .chain(examples.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    let mut score = 0;
    if !normalized_query.is_empty() {
        if command == normalized_query {
            score += 500;
        }
        if command.contains(normalized_query) {
            score += 220;
        }
        if aliases.iter().any(|alias| alias == normalized_query) {
            score += 200;
        }
        if aliases.iter().any(|alias| alias.contains(normalized_query)) {
            score += 160;
        }
        if tags.iter().any(|tag| tag == normalized_query) {
            score += 140;
        }
        if description.contains(normalized_query) {
            score += 120;
        }
        if examples
            .iter()
            .any(|example| example.contains(normalized_query))
        {
            score += 100;
        }
    }

    let mut matched_tokens = 0_u32;
    for token in tokens {
        let mut token_score = 0;
        if command
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| word == *token)
        {
            token_score += 55;
        } else if command.contains(token) {
            token_score += 35;
        }
        if aliases.iter().any(|alias| {
            alias
                .split(|character: char| !character.is_ascii_alphanumeric())
                .any(|word| word == *token)
        }) {
            token_score += 45;
        } else if aliases.iter().any(|alias| alias.contains(token)) {
            token_score += 25;
        }
        if tags.iter().any(|tag| tag == token) {
            token_score += 35;
        } else if tags.iter().any(|tag| tag.contains(token)) {
            token_score += 20;
        }
        if description.contains(token) {
            token_score += 18;
        }
        if examples.iter().any(|example| example.contains(token)) {
            token_score += 12;
        }
        if token_score == 0 && search_text.contains(token) {
            token_score += 8;
        }
        if token_score > 0 {
            matched_tokens += 1;
            score += token_score;
        }
    }

    if !tokens.is_empty() && matched_tokens == tokens.len() as u32 {
        score += 90;
    }

    score
}

fn workspace_command_catalog() -> &'static [WorkspaceCommandCatalogEntry] {
    const CATALOG: &[WorkspaceCommandCatalogEntry] = &[
        WorkspaceCommandCatalogEntry {
            command: "init",
            description: "Create a new workspace.",
            core_args: &["name"],
            aliases: &["create-workspace", "new-workspace"],
            tags: &["workspace", "create", "init"],
            examples: &["create a new workspace for a project"],
        },
        WorkspaceCommandCatalogEntry {
            command: "list",
            description: "List all workspaces.",
            core_args: &[],
            aliases: &["list-workspaces"],
            tags: &["workspace", "list"],
            examples: &["show my workspaces"],
        },
        WorkspaceCommandCatalogEntry {
            command: "read",
            description: "Print workspace manifest.",
            core_args: &[],
            aliases: &["show-workspace", "manifest"],
            tags: &["workspace", "manifest", "read"],
            examples: &["read the current workspace manifest"],
        },
        WorkspaceCommandCatalogEntry {
            command: "select",
            description: "Set active workspace.",
            core_args: &["id"],
            aliases: &["switch-workspace", "use-workspace"],
            tags: &["workspace", "select", "switch"],
            examples: &["switch to another workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "delete",
            description: "Remove workspace directory tree.",
            core_args: &["id", "force"],
            aliases: &["remove-workspace", "delete-workspace"],
            tags: &["workspace", "delete", "remove"],
            examples: &["delete a workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "resolve",
            description: "Resolve @-path alias to absolute path.",
            core_args: &["spec"],
            aliases: &["resolve-path", "path-resolve"],
            tags: &["workspace", "paths", "resolve", "aliases"],
            examples: &["resolve @repo/file.txt to a real path"],
        },
        WorkspaceCommandCatalogEntry {
            command: "check-host",
            description: "Validate repo URL and check host allowlist.",
            core_args: &["url"],
            aliases: &["validate-host", "check-url"],
            tags: &["workspace", "repo", "host", "allowlist"],
            examples: &["check whether a repository host is allowed"],
        },
        WorkspaceCommandCatalogEntry {
            command: "audit",
            description: "Append audit entry.",
            core_args: &["json"],
            aliases: &["write-audit", "audit-log"],
            tags: &["workspace", "audit", "logging"],
            examples: &["append an audit entry"],
        },
        WorkspaceCommandCatalogEntry {
            command: "audit-query",
            description: "Query audit log entries.",
            core_args: &[],
            aliases: &["query-audit", "read-audit"],
            tags: &["workspace", "audit", "history"],
            examples: &["query recent audit events"],
        },
        WorkspaceCommandCatalogEntry {
            command: "validate",
            description: "Validate workspace manifest and on-disk repo/run directories.",
            core_args: &[],
            aliases: &["check-workspace", "verify-workspace"],
            tags: &["workspace", "validate", "check"],
            examples: &["validate the current workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "run-locked",
            description: "Run command under fine-grained lock.",
            core_args: &["level", "target_id", "command"],
            aliases: &["with-lock", "locked-run"],
            tags: &["workspace", "locking", "run"],
            examples: &["run a command while holding a workspace lock"],
        },
        WorkspaceCommandCatalogEntry {
            command: "mirror-path",
            description: "Print shared mirror cache path for a repo URL.",
            core_args: &["url"],
            aliases: &["repo-mirror-path"],
            tags: &["workspace", "repo", "mirror", "cache"],
            examples: &["show the mirror cache path for a repo url"],
        },
        WorkspaceCommandCatalogEntry {
            command: "repo-clone",
            description: "Validate, clone, register, and audit a repo.",
            core_args: &["url", "alias"],
            aliases: &["clone-repo", "add-repo"],
            tags: &["workspace", "repo", "clone", "register"],
            examples: &["clone a repository into the workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "repo-update-state",
            description: "Update repo git state (headSha, headRef).",
            core_args: &["alias", "head_sha"],
            aliases: &["update-repo-state"],
            tags: &["workspace", "repo", "git", "state"],
            examples: &["update the tracked head sha for a repo"],
        },
        WorkspaceCommandCatalogEntry {
            command: "repo-pin",
            description: "Pin a repo to a specific SHA.",
            core_args: &["alias", "sha"],
            aliases: &["pin-repo"],
            tags: &["workspace", "repo", "pin", "sha"],
            examples: &["pin a repo to a commit"],
        },
        WorkspaceCommandCatalogEntry {
            command: "repo-unpin",
            description: "Unpin a repo (switch back to tracking mode).",
            core_args: &["alias"],
            aliases: &["unpin-repo"],
            tags: &["workspace", "repo", "unpin"],
            examples: &["remove a repo pin"],
        },
        WorkspaceCommandCatalogEntry {
            command: "repo-remove",
            description: "Remove a repo: delete dir, manifest entry, audit.",
            core_args: &["alias"],
            aliases: &["remove-repo", "delete-repo"],
            tags: &["workspace", "repo", "remove"],
            examples: &["remove a repo from the workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "run-setup",
            description: "Create run directory, materialize code, register, and audit.",
            core_args: &["name", "source_alias"],
            aliases: &["create-run", "setup-run"],
            tags: &["workspace", "run", "setup", "execution"],
            examples: &["create a new execution run from a repo"],
        },
        WorkspaceCommandCatalogEntry {
            command: "run-update-status",
            description: "Update a run's status.",
            core_args: &["id", "status"],
            aliases: &["update-run-status"],
            tags: &["workspace", "run", "status"],
            examples: &["mark a run as completed"],
        },
        WorkspaceCommandCatalogEntry {
            command: "run-remove",
            description: "Remove a run: worktree cleanup, delete dir, manifest entry, audit.",
            core_args: &["id"],
            aliases: &["remove-run", "delete-run"],
            tags: &["workspace", "run", "remove"],
            examples: &["remove an execution run"],
        },
        WorkspaceCommandCatalogEntry {
            command: "add-entry",
            description: "Append a JSON entry to a manifest collection.",
            core_args: &["collection", "json"],
            aliases: &["append-entry", "add-resource"],
            tags: &["workspace", "manifest", "collection", "entry"],
            examples: &["add a paper or dataset entry to the workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "add-paper",
            description: "Copy a paper markdown file into the workspace and register it by alias.",
            core_args: &["text_md_path", "alias"],
            aliases: &["paper-add", "register-paper"],
            tags: &["workspace", "papers", "register", "references"],
            examples: &["add a paper markdown file to the workspace"],
        },
        WorkspaceCommandCatalogEntry {
            command: "remove-entry",
            description: "Remove an entry by ID from a manifest collection.",
            core_args: &["collection", "id"],
            aliases: &["delete-entry"],
            tags: &["workspace", "manifest", "collection", "remove"],
            examples: &["remove an artifact entry by id"],
        },
        WorkspaceCommandCatalogEntry {
            command: "set-field",
            description: "Set a field at a dotted path in the manifest.",
            core_args: &["path", "value"],
            aliases: &["update-field"],
            tags: &["workspace", "manifest", "field", "edit"],
            examples: &["set a manifest field to a new value"],
        },
        WorkspaceCommandCatalogEntry {
            command: "index-update-status",
            description: "Update an index entry's status.",
            core_args: &["id", "status"],
            aliases: &["update-index-status"],
            tags: &["workspace", "index", "status"],
            examples: &["mark an index as ready"],
        },
        WorkspaceCommandCatalogEntry {
            command: "recipe",
            description: "Print step-by-step recipe for an operation.",
            core_args: &["operation"],
            aliases: &["show-recipe", "help-recipe"],
            tags: &["workspace", "recipe", "help"],
            examples: &["show the recipe for repo-remove"],
        },
        WorkspaceCommandCatalogEntry {
            command: "materialize",
            description: "Materialize a workspace from a spec file.",
            core_args: &["spec_path"],
            aliases: &["apply-spec", "workspace-materialize"],
            tags: &["workspace", "spec", "materialize"],
            examples: &["materialize a workspace spec file"],
        },
        WorkspaceCommandCatalogEntry {
            command: "export-spec",
            description: "Export current workspace repos as a workspace spec file.",
            core_args: &[],
            aliases: &["dump-spec", "workspace-export-spec"],
            tags: &["workspace", "spec", "export"],
            examples: &["export the current workspace as a spec"],
        },
        WorkspaceCommandCatalogEntry {
            command: "diff-spec",
            description: "Show what materialize would do (repos to add/pin/skip).",
            core_args: &["spec_path"],
            aliases: &["preview-spec"],
            tags: &["workspace", "spec", "diff"],
            examples: &["preview changes from a workspace spec"],
        },
    ];

    CATALOG
}

#[cfg(test)]
mod tests {
    use super::render_command_manual;
    use super::render_search_results;
    use super::search_command_catalog;
    use pretty_assertions::assert_eq;

    #[test]
    fn search_commands_prefers_repo_clone_for_clone_queries() {
        let result = search_command_catalog("clone repo into workspace", 1);
        assert_eq!(
            result.first().map(|entry| entry.command),
            Some("repo-clone")
        );
    }

    #[test]
    fn render_command_manual_hides_workspace_scoping_noise() {
        let entry = search_command_catalog("clone repo", 1)
            .into_iter()
            .next()
            .expect("expected repo clone match");
        let manual = render_command_manual(entry).expect("expected manual to render");
        assert!(manual.contains("Command: repo-clone"));
        assert!(manual.contains("Validate, clone, register, and audit a repo."));
        assert_eq!(
            manual
                .matches("Validate, clone, register, and audit a repo")
                .count(),
            1
        );
        assert!(manual.contains("Usage: ata workspace repo-clone <URL> <ALIAS>"));
        assert!(manual.contains("<URL>"));
        assert!(manual.contains("<ALIAS>"));
        assert!(!manual.contains("--workspace"));
        assert!(!manual.contains("--full"));
    }

    #[test]
    fn render_search_results_shows_shortlist_then_best_manual() {
        let matches = search_command_catalog("repo", 3);
        let manual = render_command_manual(matches[0]).expect("expected manual");
        let rendered = render_search_results(&matches, &manual);
        assert!(rendered.contains("Matches:"));
        assert!(rendered.contains("1. repo-clone — Validate, clone, register, and audit a repo."));
        assert!(rendered.contains("\n2. "));
        assert!(rendered.contains("\n3. "));
        assert!(rendered.contains("Best match manual:"));
        assert_eq!(rendered.matches("Usage: ata workspace").count(), 1);
    }
}
