/*
Module: runtimes

Concrete ToolRuntime implementations for specific tools. Each runtime stays
small and focused and reuses the orchestrator for approvals + sandbox + retry.
*/
use crate::exec::ExecExpiration;
use crate::path_utils;
use crate::sandboxing::CommandSpec;
use crate::sandboxing::SandboxPermissions;
use crate::shell::Shell;
use crate::skills::SkillMetadata;
use crate::tools::sandboxing::ToolError;
use codex_protocol::models::PermissionProfile;
use std::collections::HashMap;
use std::path::Path;

pub mod apply_patch;
pub mod shell;
pub mod unified_exec;

pub(crate) const CODEX_SKIP_ARG0_PATH_HELPER_ENV_VAR: &str = "CODEX_SKIP_ARG0_PATH_HELPER";

#[derive(Debug, Clone)]
pub(crate) struct ExecveSessionApproval {
    /// If this execve session approval is associated with a skill script, this
    /// field contains metadata about the skill.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub skill: Option<SkillMetadata>,
}

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
    let (_, script) = crate::bash::extract_bash_command(command)?;
    let current_exe = std::env::current_exe().ok()?;
    let current_exe = shell_single_quote(current_exe.to_string_lossy().as_ref());
    let rewritten_script = format!("ata() {{ '{current_exe}' \"$@\"; }}\n\n{script}");
    let mut rewritten = command.to_vec();
    rewritten[2] = rewritten_script;
    Some(rewritten)
}

/// Resolve model-invoked `ata` commands to the current executable so agent
/// shell commands always target the same binary as the running session.
pub(crate) fn resolve_agent_ata_command(command: &[String]) -> (Vec<String>, bool) {
    if let Some((program, args)) = command.split_first()
        && is_ata_program(program)
        && let Some(updated) = command_with_current_exe(args)
    {
        return (updated, true);
    }

    if let Some(commands) = crate::bash::parse_shell_lc_plain_commands(command)
        && let [single] = commands.as_slice()
        && let Some((program, args)) = single.split_first()
        && is_ata_program(program)
        && let Some(updated) = command_with_current_exe(args)
    {
        return (updated, true);
    }

    if let Some(command_names) = crate::bash::parse_shell_lc_command_names(command)
        && command_names.iter().any(|name| is_ata_program(name))
        && let Some(updated) = command_with_ata_shell_wrapper(command)
    {
        return (updated, true);
    }

    (command.to_vec(), false)
}

/// Shared helper to construct a CommandSpec from a tokenized command line.
/// Validates that at least a program is present.
pub(crate) fn build_command_spec(
    command: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    expiration: ExecExpiration,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<PermissionProfile>,
    justification: Option<String>,
) -> Result<CommandSpec, ToolError> {
    let (command, skip_path_helper) = resolve_agent_ata_command(command);
    let mut env = env.clone();
    if skip_path_helper {
        env.insert(
            CODEX_SKIP_ARG0_PATH_HELPER_ENV_VAR.to_string(),
            "1".to_string(),
        );
    }
    build_command_spec_from_resolved_command(
        &command,
        cwd,
        &env,
        expiration,
        sandbox_permissions,
        additional_permissions,
        justification,
    )
}

/// Shared helper for callers that have already resolved `ata` commands and
/// applied any accompanying env changes.
pub(crate) fn build_command_spec_from_resolved_command(
    command: &[String],
    cwd: &Path,
    env: &HashMap<String, String>,
    expiration: ExecExpiration,
    sandbox_permissions: SandboxPermissions,
    additional_permissions: Option<PermissionProfile>,
    justification: Option<String>,
) -> Result<CommandSpec, ToolError> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| ToolError::Rejected("command args are empty".to_string()))?;
    Ok(CommandSpec {
        program: program.clone(),
        args: args.to_vec(),
        cwd: cwd.to_path_buf(),
        env: env.clone(),
        expiration,
        sandbox_permissions,
        additional_permissions,
        justification,
        workspace_kb_root: None,
    })
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
pub(crate) fn maybe_wrap_shell_lc_with_snapshot(
    command: &[String],
    session_shell: &Shell,
    cwd: &Path,
    explicit_env_overrides: &HashMap<String, String>,
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

    if if let (Ok(snapshot_cwd), Ok(command_cwd)) = (
        path_utils::normalize_for_path_comparison(snapshot.cwd.as_path()),
        path_utils::normalize_for_path_comparison(cwd),
    ) {
        snapshot_cwd != command_cwd
    } else {
        snapshot.cwd != cwd
    } {
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
    let (override_captures, override_exports) = build_override_exports(explicit_env_overrides);
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
        .filter(|key| is_valid_shell_variable_name(key))
        .collect::<Vec<_>>();
    keys.sort_unstable();

    if keys.is_empty() {
        return (String::new(), String::new());
    }

    let captures = keys
        .iter()
        .enumerate()
        .map(|(idx, key)| {
            format!(
                "__CODEX_SNAPSHOT_OVERRIDE_SET_{idx}=\"${{{key}+x}}\"\n__CODEX_SNAPSHOT_OVERRIDE_{idx}=\"${{{key}-}}\""
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let restores = keys
        .iter()
        .enumerate()
        .map(|(idx, key)| {
            format!(
                "if [ -n \"${{__CODEX_SNAPSHOT_OVERRIDE_SET_{idx}}}\" ]; then export {key}=\"${{__CODEX_SNAPSHOT_OVERRIDE_{idx}}}\"; else unset {key}; fi"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    (captures, restores)
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
