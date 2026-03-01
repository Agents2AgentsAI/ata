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
pub mod types;
pub mod url_validation;
pub mod workspace_id;
pub mod workspace_resolution;

use clap::Parser;
use error::WorkspaceError;

/// Workspace management CLI.
#[derive(Debug, Parser)]
#[clap(name = "workspace", visible_alias = "ws")]
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
}

/// Run the workspace CLI and print output to stdout/stderr.
/// Returns the process exit code.
pub fn run_cli(cli: Cli) -> i32 {
    match dispatch(cli) {
        Ok(()) => 0,
        Err(e) => {
            let code = e.exit_code();
            eprintln!("error: {e}");
            code
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), WorkspaceError> {
    match cli.command {
        Command::Init { name } => {
            let wid = commands::init::run(&name)?;
            println!("{wid}");
        }

        Command::List => {
            let results = commands::list::run()?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }

        Command::Read { workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::read::run(&wid)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::Select { id } => {
            commands::select::run(&id)?;
            println!("selected: {id}");
        }

        Command::Delete { id } => {
            commands::delete::run(&id)?;
            println!("deleted: {id}");
        }

        Command::Resolve { spec, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let path = commands::resolve::run(&wid, &spec)?;
            println!("{}", path.display());
        }

        Command::CheckHost { url, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            commands::check_host::run(&wid, &url)?;
            println!("{url}");
        }

        Command::Audit { json, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let entry = commands::audit::run(&wid, &json)?;
            println!("{}", serde_json::to_string_pretty(&entry)?);
        }

        Command::AuditQuery {
            workspace,
            since,
            until,
            ops,
            limit,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let results =
                commands::audit_query::run(&wid, since, until, ops.as_deref(), limit)?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }

        Command::RunLocked {
            level,
            workspace,
            target_id,
            mut command,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            // Strip leading '--' separator
            if command.first().map(String::as_str) == Some("--") {
                command.remove(0);
            }
            let code = commands::run_locked::run(
                &wid,
                &level,
                target_id.as_deref(),
                &command,
            )?;
            std::process::exit(code);
        }

        Command::MirrorPath { url } => {
            let path = commands::mirror_path::run(&url);
            println!("{}", path.display());
        }

        Command::RepoClone {
            url,
            alias,
            workspace,
            full,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let result = commands::repo_clone::run(&wid, &url, &alias, full)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        Command::RepoUpdateState {
            alias,
            head_sha,
            head_ref,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::repo_update_state::run(
                &wid,
                &alias,
                &head_sha,
                head_ref.as_deref(),
            )?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::RepoPin {
            alias,
            sha,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::repo_pin::run(&wid, &alias, &sha)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::RepoUnpin { alias, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::repo_unpin::run(&wid, &alias)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::RepoRemove { alias, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            commands::repo_remove::run(&wid, &alias)?;
            println!("removed: {alias}");
        }

        Command::RunSetup {
            name,
            source_alias,
            strategy,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let result =
                commands::run_setup::run(&wid, &name, &source_alias, &strategy)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        Command::RunUpdateStatus {
            id,
            status,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::run_update_status::run(&wid, &id, &status)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::RunRemove { id, workspace } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            commands::run_remove::run(&wid, &id)?;
            println!("removed: {id}");
        }

        Command::AddEntry {
            collection,
            json,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::add_entry::run(&wid, &collection, &json)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::RemoveEntry {
            collection,
            id,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::remove_entry::run(&wid, &collection, &id)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::SetField {
            path,
            value,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::set_field::run(&wid, &path, &value)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::IndexUpdateStatus {
            id,
            status,
            workspace,
        } => {
            let wid = workspace_resolution::resolve_workspace(workspace.as_deref());
            let manifest = commands::index_update_status::run(&wid, &id, &status)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }

        Command::Recipe { operation } => {
            let text = commands::recipe::run(&operation)?;
            print!("{text}");
        }
    }

    Ok(())
}
