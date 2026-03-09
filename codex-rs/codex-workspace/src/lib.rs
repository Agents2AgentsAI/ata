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

use clap::Parser;
use error::WorkspaceError;

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

    /// Print workspace manifest.
    Read {
        /// Workspace ID (default: resolved).
        #[arg(long)]
        workspace: Option<String>,
    },

    /// Set active workspace.
    Select {
        /// Workspace ID to select.
        id: String,
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

        Command::Read { workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref())?;
            let manifest = commands::read::run(&wid)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(0)
        }

        Command::Select { id } => {
            commands::select::run(&id)?;
            println!("selected: {id}");
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
    let codex_home = paths::codex_home();
    let cwd = std::env::current_dir().ok();
    let session_id = std::env::var("CODEX_SESSION_ID").ok();
    let thread_id = std::env::var("CODEX_THREAD_ID").ok();

    if let Some(wid) = workspace_resolution::resolve_selected_workspace_for(
        &codex_home,
        cwd.as_deref(),
        explicit,
        session_id.as_deref(),
        thread_id.as_deref(),
    )? {
        return Ok(wid);
    }

    // Create a new workspace from the spec name
    let spec = spec::read_spec(std::path::Path::new(spec_path))?;
    let wid = commands::init::run(&spec.name)?;
    eprintln!("created workspace: {wid}");
    Ok(wid)
}
