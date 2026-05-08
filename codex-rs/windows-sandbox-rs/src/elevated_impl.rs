use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub struct ElevatedSandboxCaptureRequest<'a> {
    pub policy_json_or_preset: &'a str,
    pub sandbox_policy_cwd: &'a Path,
    pub codex_home: &'a Path,
    pub command: Vec<String>,
    pub cwd: &'a Path,
    pub env_map: HashMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub use_private_desktop: bool,
    pub proxy_enforced: bool,
    pub read_roots_override: Option<&'a [PathBuf]>,
    pub read_roots_include_platform_defaults: bool,
    pub write_roots_override: Option<&'a [PathBuf]>,
    pub deny_write_paths_override: &'a [PathBuf],
}

mod windows_impl {
    use super::ElevatedSandboxCaptureRequest;
    use crate::acl::allow_null_device;
    use crate::cap::load_or_create_cap_sids;
    use crate::env::ensure_non_interactive_pager;
    use crate::env::inherit_path_env;
    use crate::env::normalize_null_device_env;
    use crate::identity::require_logon_sandbox_creds;
    use crate::ipc_framed::Message;
    use crate::ipc_framed::OutputStream;
    use crate::ipc_framed::SpawnRequest;
    use crate::ipc_framed::decode_bytes;
    use crate::ipc_framed::read_frame;
    use crate::logging::log_failure;
    use crate::logging::log_start;
    use crate::logging::log_success;
    use crate::policy::SandboxPolicy;
    use crate::policy::parse_policy;
    use crate::runner_client::spawn_runner_transport;
    use crate::sandbox_utils::ensure_codex_home_exists;
    use crate::sandbox_utils::inject_git_safe_directory;
    use crate::token::convert_string_sid_to_sid;
    use anyhow::Result;
    use std::path::Path;

    pub use crate::windows_impl::CaptureResult;

    #[derive(serde::Serialize)]
    struct RunnerPayload {
        policy_json_or_preset: String,
        sandbox_policy_cwd: PathBuf,
        // Writable log dir for sandbox user (.ata in sandbox profile).
        codex_home: PathBuf,
        // Real user's CODEX_HOME for shared data (caps, config).
        real_codex_home: PathBuf,
        cap_sids: Vec<String>,
        request_file: Option<PathBuf>,
        command: Vec<String>,
        cwd: PathBuf,
        env_map: HashMap<String, String>,
        timeout_ms: Option<u64>,
        use_private_desktop: bool,
        stdin_pipe: String,
        stdout_pipe: String,
        stderr_pipe: String,
    }

    /// Launches the command runner under the sandbox user and captures its output.
    #[allow(clippy::too_many_arguments)]
    pub fn run_windows_sandbox_capture(
        policy_json_or_preset: &str,
        sandbox_policy_cwd: &Path,
        codex_home: &Path,
        command: Vec<String>,
        cwd: &Path,
        mut env_map: HashMap<String, String>,
        timeout_ms: Option<u64>,
        use_private_desktop: bool,
    ) -> Result<CaptureResult> {
        let ElevatedSandboxCaptureRequest {
            policy_json_or_preset,
            sandbox_policy_cwd,
            codex_home,
            command,
            cwd,
            mut env_map,
            timeout_ms,
            use_private_desktop,
            proxy_enforced,
            read_roots_override,
            read_roots_include_platform_defaults,
            write_roots_override,
            deny_write_paths_override,
        } = request;
        let policy = parse_policy(policy_json_or_preset)?;
        normalize_null_device_env(&mut env_map);
        ensure_non_interactive_pager(&mut env_map);
        inherit_path_env(&mut env_map);
        inject_git_safe_directory(&mut env_map, cwd);
        // Use a temp-based log dir that the sandbox user can write.
        let sandbox_base = codex_home.join(".sandbox");
        ensure_codex_home_exists(&sandbox_base)?;

        let logs_base_dir: Option<&Path> = Some(sandbox_base.as_path());
        log_start(&command, logs_base_dir);
        let sandbox_creds = require_logon_sandbox_creds(
            &policy,
            sandbox_policy_cwd,
            cwd,
            &env_map,
            codex_home,
            read_roots_override,
            read_roots_include_platform_defaults,
            write_roots_override,
            deny_write_paths_override,
            proxy_enforced,
        )?;
        // Build capability SID for ACL grants.
        if matches!(
            &policy,
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. }
        ) {
            anyhow::bail!("DangerFullAccess and ExternalSandbox are not supported for sandboxing")
        }
        let caps = load_or_create_cap_sids(codex_home)?;
        let (psid_to_use, cap_sids) = match &policy {
            SandboxPolicy::ReadOnly { .. } => {
                #[allow(clippy::unwrap_used)]
                let psid = unsafe { convert_string_sid_to_sid(&caps.readonly).unwrap() };
                (psid, vec![caps.readonly])
            }
            SandboxPolicy::WorkspaceWrite { .. } => {
                #[allow(clippy::unwrap_used)]
                let psid = unsafe { convert_string_sid_to_sid(&caps.workspace).unwrap() };
                (
                    psid,
                    vec![
                        caps.workspace,
                        crate::cap::workspace_cap_sid_for_cwd(codex_home, cwd)?,
                    ],
                )
            }
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. } => {
                unreachable!("DangerFullAccess handled above")
            }
        };

        unsafe {
            allow_null_device(psid_to_use);
        }

        // Prepare named pipes for runner.
        let stdin_name = pipe_name("stdin");
        let stdout_name = pipe_name("stdout");
        let stderr_name = pipe_name("stderr");
        let h_stdin_pipe = create_named_pipe(
            &stdin_name,
            PIPE_ACCESS_DUPLEX | PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        )?;
        let h_stdout_pipe = create_named_pipe(
            &stdout_name,
            PIPE_ACCESS_DUPLEX | PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        )?;
        let h_stderr_pipe = create_named_pipe(
            &stderr_name,
            PIPE_ACCESS_DUPLEX | PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        )?;

        // Launch runner as sandbox user via CreateProcessWithLogonW.
        let runner_exe = find_runner_exe(codex_home, logs_base_dir);
        let runner_cmdline = runner_exe
            .to_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "ata-command-runner.exe".to_string());
        // Write request to a file under the sandbox base dir for the runner to read.
        // TODO(iceweasel) - use a different mechanism for invoking the runner.
        let base_tmp = sandbox_base.join("requests");
        std::fs::create_dir_all(&base_tmp)?;
        let mut rng = SmallRng::from_entropy();
        let req_file = base_tmp.join(format!("request-{:x}.json", rng.gen::<u128>()));
        let payload = RunnerPayload {
            policy_json_or_preset: policy_json_or_preset.to_string(),
            sandbox_policy_cwd: sandbox_policy_cwd.to_path_buf(),
            codex_home: sandbox_base.clone(),
            real_codex_home: codex_home.to_path_buf(),
            cap_sids: cap_sids.clone(),
            request_file: Some(req_file.clone()),
            command: command.clone(),
            cwd: cwd.to_path_buf(),
            env_map: env_map.clone(),
            timeout_ms,
            use_private_desktop,
            stdin_pipe: stdin_name.clone(),
            stdout_pipe: stdout_name.clone(),
            stderr_pipe: stderr_name.clone(),
        };
        let payload_json = serde_json::to_string(&payload)?;
        if let Err(e) = fs::write(&req_file, &payload_json) {
            log_note(
                &format!("error writing request file {}: {}", req_file.display(), e),
                logs_base_dir,
                spawn_request,
            )?;
            let (pipe_write, mut pipe_read) = transport.into_files();
            drop(pipe_write);

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let (exit_code, timed_out) = loop {
                let msg = read_frame(&mut pipe_read)?
                    .ok_or_else(|| anyhow::anyhow!("runner pipe closed before exit"))?;
                match msg.message {
                    Message::SpawnReady { .. } => {}
                    Message::Output { payload } => {
                        let bytes = decode_bytes(&payload.data_b64)?;
                        match payload.stream {
                            OutputStream::Stdout => stdout.extend_from_slice(&bytes),
                            OutputStream::Stderr => stderr.extend_from_slice(&bytes),
                        }
                    }
                    Message::Exit { payload } => break (payload.exit_code, payload.timed_out),
                    Message::Error { payload } => {
                        return Err(anyhow::anyhow!("runner error: {}", payload.message));
                    }
                    other => {
                        return Err(anyhow::anyhow!(
                            "unexpected runner message during capture: {other:?}"
                        ));
                    }
                }
            };

            if exit_code == 0 {
                log_success(&command, logs_base_dir);
            } else {
                log_failure(&command, &format!("exit code {exit_code}"), logs_base_dir);
            }

            Ok(CaptureResult {
                exit_code,
                stdout,
                stderr,
                timed_out,
            })
        })()
    }

    #[cfg(test)]
    mod tests {
        use crate::policy::SandboxPolicy;

        fn workspace_policy(network_access: bool) -> SandboxPolicy {
            SandboxPolicy::WorkspaceWrite {
                writable_roots: Vec::new(),
                network_access,
                exclude_tmpdir_env_var: false,
                exclude_slash_tmp: false,
            }
        }

        #[test]
        fn applies_network_block_when_access_is_disabled() {
            assert!(!workspace_policy(/*network_access*/ false).has_full_network_access());
        }

        #[test]
        fn skips_network_block_when_access_is_allowed() {
            assert!(workspace_policy(/*network_access*/ true).has_full_network_access());
        }

        #[test]
        fn applies_network_block_for_read_only() {
            assert!(!SandboxPolicy::new_read_only_policy().has_full_network_access());
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::run_windows_sandbox_capture;

#[cfg(not(target_os = "windows"))]
mod stub {
    use super::ElevatedSandboxCaptureRequest;
    use anyhow::Result;
    use anyhow::bail;

    #[derive(Debug, Default)]
    pub struct CaptureResult {
        pub exit_code: i32,
        pub stdout: Vec<u8>,
        pub stderr: Vec<u8>,
        pub timed_out: bool,
    }

    /// Stub implementation for non-Windows targets; sandboxing only works on Windows.
    #[allow(clippy::too_many_arguments)]
    pub fn run_windows_sandbox_capture(
        _policy_json_or_preset: &str,
        _sandbox_policy_cwd: &Path,
        _codex_home: &Path,
        _command: Vec<String>,
        _cwd: &Path,
        _env_map: HashMap<String, String>,
        _timeout_ms: Option<u64>,
        _use_private_desktop: bool,
    ) -> Result<CaptureResult> {
        bail!("Windows sandbox is only available on Windows")
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub::run_windows_sandbox_capture;
