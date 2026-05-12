/*
Module: runtimes

Concrete ToolRuntime implementations for specific tools. Each runtime stays
small and focused and reuses the orchestrator for approvals + sandbox + retry.
*/
use crate::exec_env::CODEX_THREAD_ID_ENV_VAR;
use crate::path_utils;
use crate::sandboxing::SandboxPermissions;
use crate::shell::Shell;
use crate::tools::sandboxing::ToolError;
#[cfg(target_os = "macos")]
use codex_network_proxy::CODEX_PROXY_GIT_SSH_COMMAND_MARKER;
use codex_network_proxy::PROXY_ACTIVE_ENV_KEY;
use codex_network_proxy::PROXY_ENV_KEYS;
#[cfg(target_os = "macos")]
use codex_network_proxy::PROXY_GIT_SSH_COMMAND_ENV_KEY;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::FileSystemPermissions;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_sandboxing::SandboxCommand;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::path::Path;

/// When set on a shell exec environment, tells the child `ata` invocation
/// to skip its `prepend_path_entry_for_codex_aliases` PATH munging — we
/// already rewrote the program to the fully-qualified current_exe path,
/// so the PATH helper would just add a redundant entry (and on some
/// runners can recurse into itself).
pub(crate) const CODEX_SKIP_ARG0_PATH_HELPER_ENV_VAR: &str = "CODEX_SKIP_ARG0_PATH_HELPER";

fn is_ata_program(program: &str) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    name.eq_ignore_ascii_case("ata") || name.eq_ignore_ascii_case("ata.exe")
}

fn command_with_current_exe(args: &[String]) -> Option<Vec<String>> {
    let current_exe = std::env::current_exe().ok()?;
    Some(
        std::iter::once(current_exe.to_string_lossy().to_string())
            .chain(args.iter().cloned())
            .collect(),
    )
}

fn command_with_ata_shell_wrapper(command: &[String]) -> Option<Vec<String>> {
    let (_, script) = codex_shell_command::bash::extract_bash_command(command)?;
    let current_exe = std::env::current_exe().ok()?;
    let current_exe = shell_single_quote(current_exe.to_string_lossy().as_ref());
    let rewritten_script = format!("ata() {{ '{current_exe}' \"$@\"; }}\n\n{script}");
    let mut rewritten = command.to_vec();
    rewritten[2] = rewritten_script;
    Some(rewritten)
}

/// Resolve model-invoked `ata` commands to the current executable so agent
/// shell commands always target the same binary as the running session.
///
/// The agent has no reliable PATH lookup for `ata` — it might be
/// installed under `~/.cargo/bin`, `~/.local/bin`, or via a wrapper
/// script — and the workspace-write sandbox often strips the user's
/// shell-init PATH entries. Three rewrite paths:
///
/// 1. **Direct**: `ata <args>` -> `<current_exe> <args>`
/// 2. **Single shell wrap**: `bash -lc "ata <args>"` (only one command in
///    the script) -> `<current_exe> <args>`
/// 3. **Compound shell wrap**: `bash -lc "foo; ata <args>; bar"` -> inject
///    a shell function `ata() { '<current_exe>' "$@"; }` before the
///    script so every `ata` reference in the chain resolves correctly.
///
/// Returns `(rewritten_command, did_rewrite)`. Callers set
/// `CODEX_SKIP_ARG0_PATH_HELPER=1` in the child env when `did_rewrite` is
/// true, so the child binary doesn't run its PATH-prepend helper a
/// second time.
pub(crate) fn resolve_agent_ata_command(command: &[String]) -> (Vec<String>, bool) {
    if let Some((program, args)) = command.split_first()
        && is_ata_program(program)
        && let Some(updated) = command_with_current_exe(args)
    {
        return (updated, true);
    }

    if let Some(commands) = codex_shell_command::bash::parse_shell_lc_plain_commands(command)
        && let [single] = commands.as_slice()
        && let Some((program, args)) = single.split_first()
        && is_ata_program(program)
        && let Some(updated) = command_with_current_exe(args)
    {
        return (updated, true);
    }

    if let Some(command_names) = codex_shell_command::bash::parse_shell_lc_command_names(command)
        && command_names.iter().any(|name| is_ata_program(name))
        && let Some(updated) = command_with_ata_shell_wrapper(command)
    {
        return (updated, true);
    }

    (command.to_vec(), false)
}

pub(crate) mod apply_patch;
pub(crate) mod shell;
pub(crate) mod unified_exec;

/// Shared helper to construct sandbox transform inputs from a tokenized command line.
/// Validates that at least a program is present.
///
/// `workspace_kb_root`, when `Some`, is appended to the command's
/// `additional_permissions.file_system.write` list so the agent can
/// freely write into its per-workspace knowledge-base directory
/// (`~/.ata/<workspace_id>/knowledge-base/...`) without an approval
/// prompt. Resolved by `crate::workspace_kb::kb_writable_root` from the
/// per-turn `CODEX_KB_PATH` env var; threaded through `TurnContext`.
pub(crate) fn build_sandbox_command(
    command: &[String],
    cwd: &AbsolutePathBuf,
    env: &HashMap<String, String>,
    additional_permissions: Option<AdditionalPermissionProfile>,
    workspace_kb_root: Option<&AbsolutePathBuf>,
) -> Result<SandboxCommand, ToolError> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| ToolError::Rejected("command args are empty".to_string()))?;
    let additional_permissions =
        merge_workspace_kb_root(additional_permissions, workspace_kb_root.cloned());
    Ok(SandboxCommand {
        program: program.clone().into(),
        args: args.to_vec(),
        cwd: cwd.clone(),
        env: env.clone(),
        additional_permissions,
    })
}

fn merge_workspace_kb_root(
    additional_permissions: Option<AdditionalPermissionProfile>,
    workspace_kb_root: Option<AbsolutePathBuf>,
) -> Option<AdditionalPermissionProfile> {
    let Some(workspace_kb_root) = workspace_kb_root else {
        return additional_permissions;
    };

    let mut additional_permissions = additional_permissions.unwrap_or_default();
    let file_system = additional_permissions
        .file_system
        .get_or_insert_with(FileSystemPermissions::default);
    let already_present = file_system.entries.iter().any(|entry| match &entry.path {
        FileSystemPath::Path { path } => {
            path == &workspace_kb_root && entry.access == FileSystemAccessMode::Write
        }
        FileSystemPath::GlobPattern { .. } | FileSystemPath::Special { .. } => false,
    });
    if !already_present {
        file_system.entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: workspace_kb_root,
            },
            access: FileSystemAccessMode::Write,
        });
    }
    Some(additional_permissions)
}

pub(crate) fn exec_env_for_sandbox_permissions(
    env: &HashMap<String, String>,
    sandbox_permissions: SandboxPermissions,
) -> HashMap<String, String> {
    let mut env = env.clone();
    if sandbox_permissions.requires_escalated_permissions()
        && env.contains_key(PROXY_ACTIVE_ENV_KEY)
    {
        for key in PROXY_ENV_KEYS {
            env.remove(*key);
        }
        // Only macOS injects a Codex-owned SSH wrapper for the managed SOCKS proxy.
        #[cfg(target_os = "macos")]
        if env
            .get(PROXY_GIT_SSH_COMMAND_ENV_KEY)
            .is_some_and(|command| command.starts_with(CODEX_PROXY_GIT_SSH_COMMAND_MARKER))
        {
            env.remove(PROXY_GIT_SSH_COMMAND_ENV_KEY);
        }
    }
    env
}

/// POSIX-only helper: for commands produced by `Shell::derive_exec_args`
/// for Bash/Zsh/sh of the form `[shell_path, "-lc", "<script>"]`, and
/// when a snapshot is configured on the session shell, rewrite the argv
/// to a single non-login shell that sources the snapshot before running
/// the original script:
///
///   shell -lc "<script>"
///   => user_shell -c ". SNAPSHOT (best effort); exec shell -c <script>"
///
/// This wrapper script uses POSIX constructs (`if`, `.`, `exec`) so it can
/// be run by Bash/Zsh/sh. On non-matching commands, or when command cwd does
/// not match the snapshot cwd, this is a no-op.
///
/// `explicit_env_overrides` and `env` are intentionally separate inputs.
/// `explicit_env_overrides` contains policy-driven shell env overrides that
/// should win after the snapshot is sourced, while `env` is the full live exec
/// environment. We need access to both so snapshot restore logic can preserve
/// runtime-only vars like `CODEX_THREAD_ID` without pretending they came from
/// the explicit override policy.
pub(crate) fn maybe_wrap_shell_lc_with_snapshot(
    command: &[String],
    session_shell: &Shell,
    cwd: &AbsolutePathBuf,
    explicit_env_overrides: &HashMap<String, String>,
    env: &HashMap<String, String>,
) -> Vec<String> {
    if cfg!(windows) {
        return command.to_vec();
    }

    let Some(snapshot) = session_shell.shell_snapshot() else {
        return command.to_vec();
    };

    if !snapshot.path.exists() {
        return command.to_vec();
    }

    if !path_utils::paths_match_after_normalization(snapshot.cwd.as_path(), cwd) {
        return command.to_vec();
    }

    if command.len() < 3 {
        return command.to_vec();
    }

    let flag = command[1].as_str();
    if flag != "-lc" {
        return command.to_vec();
    }

    let snapshot_path = snapshot.path.to_string_lossy();
    let shell_path = session_shell.shell_path.to_string_lossy();
    let original_shell = shell_single_quote(&command[0]);
    let original_script = shell_single_quote(&command[2]);
    let snapshot_path = shell_single_quote(snapshot_path.as_ref());
    let trailing_args = command[3..]
        .iter()
        .map(|arg| format!(" '{}'", shell_single_quote(arg)))
        .collect::<String>();
    let mut override_env = explicit_env_overrides.clone();
    if let Some(thread_id) = env.get(CODEX_THREAD_ID_ENV_VAR) {
        override_env.insert(CODEX_THREAD_ID_ENV_VAR.to_string(), thread_id.clone());
    }
    let (override_captures, override_exports) = build_override_exports(&override_env);
    let (proxy_captures, proxy_exports) = build_proxy_env_exports();
    let override_captures = join_shell_blocks([override_captures, proxy_captures]);
    let override_exports = join_shell_blocks([override_exports, proxy_exports]);
    let rewritten_script = if override_exports.is_empty() {
        format!(
            "if . '{snapshot_path}' >/dev/null 2>&1; then :; fi\n\nexec '{original_shell}' -c '{original_script}'{trailing_args}"
        )
    } else {
        format!(
            "{override_captures}\n\nif . '{snapshot_path}' >/dev/null 2>&1; then :; fi\n\n{override_exports}\n\nexec '{original_shell}' -c '{original_script}'{trailing_args}"
        )
    };

    vec![shell_path.to_string(), "-c".to_string(), rewritten_script]
}

fn build_override_exports(explicit_env_overrides: &HashMap<String, String>) -> (String, String) {
    let mut keys = explicit_env_overrides
        .keys()
        .map(String::as_str)
        .filter(|key| is_valid_shell_variable_name(key))
        .collect::<Vec<_>>();
    keys.sort_unstable();

    build_override_exports_for_keys("__CODEX_SNAPSHOT_OVERRIDE", &keys)
}

fn build_proxy_env_exports() -> (String, String) {
    let mut keys = PROXY_ENV_KEYS
        .iter()
        .copied()
        .filter(|key| is_valid_shell_variable_name(key))
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();

    let (captures, restores) =
        build_override_exports_for_keys("__CODEX_SNAPSHOT_PROXY_OVERRIDE", &keys);
    let key = PROXY_ACTIVE_ENV_KEY;
    let proxy_blocks = (
        format!("{captures}\n__CODEX_SNAPSHOT_PROXY_ENV_SET=\"${{{key}+x}}\""),
        format!(
            "if [ -n \"$__CODEX_SNAPSHOT_PROXY_ENV_SET\" ] || [ -n \"${{{key}+x}}\" ]; then\n{restores}\nfi"
        ),
    );
    let git_blocks = build_codex_proxy_git_ssh_command_exports();
    (
        join_shell_blocks([proxy_blocks.0, git_blocks.0]),
        join_shell_blocks([proxy_blocks.1, git_blocks.1]),
    )
}

#[cfg(target_os = "macos")]
fn build_codex_proxy_git_ssh_command_exports() -> (String, String) {
    let key = PROXY_GIT_SSH_COMMAND_ENV_KEY;
    let marker_pattern = format!("{}\\ *", CODEX_PROXY_GIT_SSH_COMMAND_MARKER.trim_end());
    (
        format!(
            "__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_SET=\"${{{key}+x}}\"\n__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND=\"${{{key}-}}\"\ncase \"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND\" in\n  {marker_pattern}) __CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_LIVE_MARKED=1 ;;\n  *) __CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_LIVE_MARKED= ;;\nesac"
        ),
        format!(
            "case \"${{{key}-}}\" in\n  {marker_pattern}) __CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_AFTER_MARKED=1 ;;\n  *) __CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_AFTER_MARKED= ;;\nesac\nif [ -n \"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_LIVE_MARKED\" ]; then\n  if [ -z \"${{{key}+x}}\" ] || [ -n \"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_AFTER_MARKED\" ]; then\n    export {key}=\"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND\"\n  fi\nelif [ -n \"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_AFTER_MARKED\" ]; then\n  if [ -n \"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND_SET\" ]; then\n    export {key}=\"$__CODEX_SNAPSHOT_PROXY_GIT_SSH_COMMAND\"\n  else\n    unset {key}\n  fi\nfi"
        ),
    )
}

#[cfg(not(target_os = "macos"))]
fn build_codex_proxy_git_ssh_command_exports() -> (String, String) {
    (String::new(), String::new())
}

fn build_override_exports_for_keys(variable_prefix: &str, keys: &[&str]) -> (String, String) {
    if keys.is_empty() {
        return (String::new(), String::new());
    }

    let captures = keys
        .iter()
        .enumerate()
        .map(|(idx, key)| {
            let set_var = format!("{variable_prefix}_SET_{idx}");
            let value_var = format!("{variable_prefix}_{idx}");
            format!("{set_var}=\"${{{key}+x}}\"\n{value_var}=\"${{{key}-}}\"")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let restores = keys
        .iter()
        .enumerate()
        .map(|(idx, key)| {
            let set_var = format!("{variable_prefix}_SET_{idx}");
            let value_var = format!("{variable_prefix}_{idx}");
            format!(
                "if [ -n \"${{{set_var}}}\" ]; then export {key}=\"${{{value_var}}}\"; else unset {key}; fi"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    (captures, restores)
}

fn join_shell_blocks(blocks: impl IntoIterator<Item = String>) -> String {
    blocks
        .into_iter()
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_valid_shell_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn shell_single_quote(input: &str) -> String {
    input.replace('\'', r#"'"'"'"#)
}

#[cfg(all(test, unix))]
#[path = "mod_tests.rs"]
mod tests;
