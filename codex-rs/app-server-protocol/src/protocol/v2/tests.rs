use codex_protocol::approvals::ElicitationRequest as CoreElicitationRequest;
use codex_protocol::models::MacOsAutomationPermission as CoreMacOsAutomationPermission;
use codex_protocol::models::MacOsContactsPermission as CoreMacOsContactsPermission;
use codex_protocol::models::MacOsPreferencesPermission as CoreMacOsPreferencesPermission;
use codex_protocol::models::MacOsSeatbeltProfileExtensions as CoreMacOsSeatbeltProfileExtensions;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::PermissionProfile as CorePermissionProfile;
use codex_protocol::protocol::AgentStatus as CoreAgentStatus;
use codex_protocol::protocol::AskForApproval as CoreAskForApproval;
use codex_protocol::protocol::GranularApprovalConfig as CoreGranularApprovalConfig;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::collections::HashMap;

use super::*;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::ReasoningItem;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::items::WebSearchItem;
use codex_protocol::models::WebSearchAction as CoreWebSearchAction;
use pretty_assertions::assert_eq;
use serde_json::json;

fn absolute_path_string(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    if cfg!(windows) {
        format!(r"C:\{}", trimmed.replace('/', "\\"))
    } else {
        format!("/{trimmed}")
    }
}

fn absolute_path(path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(absolute_path_string(path)).expect("path must be absolute")
}

fn test_absolute_path() -> AbsolutePathBuf {
    absolute_path("readable")
}

#[test]
fn collab_agent_state_maps_interrupted_status() {
    assert_eq!(
        CollabAgentState::from(CoreAgentStatus::Interrupted),
        CollabAgentState {
            status: CollabAgentStatus::Interrupted,
            message: None,
        }
    );
}

#[test]
fn command_execution_request_approval_rejects_relative_additional_permission_paths() {
    let err = serde_json::from_value::<CommandExecutionRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "command": "cat file",
        "cwd": "/tmp",
        "commandActions": null,
        "reason": null,
        "networkApprovalContext": null,
        "additionalPermissions": {
            "network": null,
            "fileSystem": {
                "read": ["relative/path"],
                "write": null
            },
            "macos": null
        },
        "skillMetadata": null,
        "proposedExecpolicyAmendment": null,
        "proposedNetworkPolicyAmendments": null,
        "availableDecisions": null
    }))
    .expect_err("relative additional permission paths should fail");
    assert!(
        err.to_string()
            .contains("AbsolutePathBuf deserialized without a base path"),
        "unexpected error: {err}"
    );
}

#[test]
fn command_execution_request_approval_accepts_macos_automation_bundle_ids_object() {
    let params = serde_json::from_value::<CommandExecutionRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "command": "cat file",
        "cwd": "/tmp",
        "commandActions": null,
        "reason": null,
        "networkApprovalContext": null,
        "additionalPermissions": {
            "network": null,
            "fileSystem": null,
            "macos": {
                "preferences": "read_only",
                "automations": {
                    "bundle_ids": ["com.apple.Notes"]
                },
                "launchServices": false,
                "accessibility": false,
                "calendar": false,
                "reminders": false,
                "contacts": "read_only"
            }
        },
        "skillMetadata": null,
        "proposedExecpolicyAmendment": null,
        "proposedNetworkPolicyAmendments": null,
        "availableDecisions": null
    }))
    .expect("bundle_ids object should deserialize");

    assert_eq!(
        params
            .additional_permissions
            .and_then(|permissions| permissions.macos)
            .map(|macos| (macos.automations, macos.launch_services, macos.contacts)),
        Some((
            CoreMacOsAutomationPermission::BundleIds(vec!["com.apple.Notes".to_string(),]),
            false,
            CoreMacOsContactsPermission::ReadOnly,
        ))
    );
}

#[test]
fn command_execution_request_approval_accepts_macos_reminders_permission() {
    let params = serde_json::from_value::<CommandExecutionRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "command": "cat file",
        "cwd": "/tmp",
        "commandActions": null,
        "reason": null,
        "networkApprovalContext": null,
        "additionalPermissions": {
            "network": null,
            "fileSystem": null,
            "macos": {
                "preferences": "read_only",
                "automations": "none",
                "launchServices": false,
                "accessibility": false,
                "calendar": false,
                "reminders": true,
                "contacts": "none"
            }
        },
        "skillMetadata": null,
        "proposedExecpolicyAmendment": null,
        "proposedNetworkPolicyAmendments": null,
        "availableDecisions": null
    }))
    .expect("reminders permission should deserialize");

    assert_eq!(
        params
            .additional_permissions
            .and_then(|permissions| permissions.macos)
            .map(|macos| macos.reminders),
        Some(true)
    );
}

#[test]
fn command_execution_request_approval_accepts_skill_metadata() {
    let params = serde_json::from_value::<CommandExecutionRequestApprovalParams>(json!({
        "threadId": "thr_123",
        "turnId": "turn_123",
        "itemId": "call_123",
        "command": "cat file",
        "cwd": "/tmp",
        "commandActions": null,
        "reason": null,
        "networkApprovalContext": null,
        "additionalPermissions": null,
        "skillMetadata": {
            "pathToSkillsMd": "/tmp/SKILLS.md"
        },
        "proposedExecpolicyAmendment": null,
        "proposedNetworkPolicyAmendments": null,
        "availableDecisions": null
    }))
    .expect("skill metadata should deserialize");

    assert_eq!(
        params.skill_metadata,
        Some(CommandExecutionRequestApprovalSkillMetadata {
            path_to_skills_md: PathBuf::from("/tmp/SKILLS.md"),
        })
    );
}

#[test]
fn permissions_request_approval_response_accepts_partial_macos_grants() {
    let cases = vec![
        (json!({}), Some(GrantedMacOsPermissions::default()), None),
        (
            json!({
                "preferences": "read_only",
            }),
            Some(GrantedMacOsPermissions {
                preferences: Some(CoreMacOsPreferencesPermission::ReadOnly),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::ReadOnly,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: false,
                macos_accessibility: false,
                macos_calendar: false,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "automations": {
                    "bundle_ids": ["com.apple.Notes"],
                },
            }),
            Some(GrantedMacOsPermissions {
                automations: Some(CoreMacOsAutomationPermission::BundleIds(vec![
                    "com.apple.Notes".to_string(),
                ])),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::BundleIds(vec![
                    "com.apple.Notes".to_string(),
                ]),
                macos_launch_services: false,
                macos_accessibility: false,
                macos_calendar: false,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "launchServices": true,
            }),
            Some(GrantedMacOsPermissions {
                launch_services: Some(true),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: true,
                macos_accessibility: false,
                macos_calendar: false,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "accessibility": true,
            }),
            Some(GrantedMacOsPermissions {
                accessibility: Some(true),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: false,
                macos_accessibility: true,
                macos_calendar: false,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "calendar": true,
            }),
            Some(GrantedMacOsPermissions {
                calendar: Some(true),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: false,
                macos_accessibility: false,
                macos_calendar: true,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "reminders": true,
            }),
            Some(GrantedMacOsPermissions {
                reminders: Some(true),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: false,
                macos_accessibility: false,
                macos_calendar: false,
                macos_reminders: true,
                macos_contacts: CoreMacOsContactsPermission::None,
            }),
        ),
        (
            json!({
                "contacts": "read_only",
            }),
            Some(GrantedMacOsPermissions {
                contacts: Some(CoreMacOsContactsPermission::ReadOnly),
                ..Default::default()
            }),
            Some(CoreMacOsSeatbeltProfileExtensions {
                macos_preferences: CoreMacOsPreferencesPermission::None,
                macos_automation: CoreMacOsAutomationPermission::None,
                macos_launch_services: false,
                macos_accessibility: false,
                macos_calendar: false,
                macos_reminders: false,
                macos_contacts: CoreMacOsContactsPermission::ReadOnly,
            }),
        ),
    ];

    for (macos_json, expected_granted_macos, expected_core_macos) in cases {
        let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(json!({
            "permissions": {
                "macos": macos_json,
            },
        }))
        .expect("partial macos permissions response should deserialize");

        assert_eq!(
            response.permissions,
            GrantedPermissionProfile {
                macos: expected_granted_macos,
                ..Default::default()
            }
        );

        assert_eq!(
            CorePermissionProfile::from(response.permissions),
            CorePermissionProfile {
                macos: expected_core_macos,
                ..Default::default()
            }
        );
    }
}

#[test]
fn permissions_request_approval_response_omits_ungranted_macos_keys_when_serialized() {
    let response = PermissionsRequestApprovalResponse {
        permissions: GrantedPermissionProfile {
            macos: Some(GrantedMacOsPermissions {
                accessibility: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
        scope: PermissionGrantScope::Turn,
    };

    assert_eq!(
        serde_json::to_value(response).expect("response should serialize"),
        json!({
            "permissions": {
                "macos": {
                    "accessibility": true,
                },
            },
            "scope": "turn",
        })
    );
}

#[test]
fn permissions_request_approval_response_defaults_scope_to_turn() {
    let response = serde_json::from_value::<PermissionsRequestApprovalResponse>(json!({
        "permissions": {},
    }))
    .expect("response should deserialize");

    assert_eq!(response.scope, PermissionGrantScope::Turn);
}

#[test]
fn fs_get_metadata_response_round_trips_minimal_fields() {
    let response = FsGetMetadataResponse {
        is_directory: false,
        is_file: true,
        created_at_ms: 123,
        modified_at_ms: 456,
    };

    let value = serde_json::to_value(&response).expect("serialize fs/getMetadata response");
    assert_eq!(
        value,
        json!({
            "isDirectory": false,
            "isFile": true,
            "createdAtMs": 123,
            "modifiedAtMs": 456,
        })
    );

    let decoded = serde_json::from_value::<FsGetMetadataResponse>(value)
        .expect("deserialize fs/getMetadata response");
    assert_eq!(decoded, response);
}

#[test]
fn fs_read_file_response_round_trips_base64_data() {
    let response = FsReadFileResponse {
        data_base64: "aGVsbG8=".to_string(),
    };

    let value = serde_json::to_value(&response).expect("serialize fs/readFile response");
    assert_eq!(
        value,
        json!({
            "dataBase64": "aGVsbG8=",
        })
    );

    let decoded = serde_json::from_value::<FsReadFileResponse>(value)
        .expect("deserialize fs/readFile response");
    assert_eq!(decoded, response);
}

#[test]
fn fs_read_file_params_round_trip() {
    let params = FsReadFileParams {
        path: absolute_path("tmp/example.txt"),
    };

    let value = serde_json::to_value(&params).expect("serialize fs/readFile params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example.txt"),
        })
    );

    let decoded =
        serde_json::from_value::<FsReadFileParams>(value).expect("deserialize fs/readFile params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_create_directory_params_round_trip_with_default_recursive() {
    let params = FsCreateDirectoryParams {
        path: absolute_path("tmp/example"),
        recursive: None,
    };

    let value = serde_json::to_value(&params).expect("serialize fs/createDirectory params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example"),
            "recursive": null,
        })
    );

    let decoded = serde_json::from_value::<FsCreateDirectoryParams>(value)
        .expect("deserialize fs/createDirectory params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_write_file_params_round_trip_with_base64_data() {
    let params = FsWriteFileParams {
        path: absolute_path("tmp/example.bin"),
        data_base64: "AAE=".to_string(),
    };

    let value = serde_json::to_value(&params).expect("serialize fs/writeFile params");
    assert_eq!(
        value,
        json!({
            "path": absolute_path_string("tmp/example.bin"),
            "dataBase64": "AAE=",
        })
    );

    let decoded = serde_json::from_value::<FsWriteFileParams>(value)
        .expect("deserialize fs/writeFile params");
    assert_eq!(decoded, params);
}

#[test]
fn fs_copy_params_round_trip_with_recursive_directory_copy() {
    let params = FsCopyParams {
        source_path: absolute_path("tmp/source"),
        destination_path: absolute_path("tmp/destination"),
        recursive: true,
    };

    let value = serde_json::to_value(&params).expect("serialize fs/copy params");
    assert_eq!(
        value,
        json!({
            "sourcePath": absolute_path_string("tmp/source"),
            "destinationPath": absolute_path_string("tmp/destination"),
            "recursive": true,
        })
    );

    let decoded =
        serde_json::from_value::<FsCopyParams>(value).expect("deserialize fs/copy params");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_default_optional_streaming_flags() {
    let params = serde_json::from_value::<CommandExecParams>(json!({
        "command": ["ls", "-la"],
        "timeoutMs": 1000,
        "cwd": "/tmp"
    }))
    .expect("command/exec payload should deserialize");

    assert_eq!(
        params,
        CommandExecParams {
            command: vec!["ls".to_string(), "-la".to_string()],
            process_id: None,
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            disable_output_cap: false,
            disable_timeout: false,
            timeout_ms: Some(1000),
            cwd: Some(PathBuf::from("/tmp")),
            env: None,
            size: None,
            sandbox_policy: None,
        }
    );
}

#[test]
fn command_exec_params_round_trips_disable_timeout() {
    let params = CommandExecParams {
        command: vec!["sleep".to_string(), "30".to_string()],
        process_id: Some("sleep-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: true,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: None,
        sandbox_policy: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["sleep", "30"],
            "processId": "sleep-1",
            "disableTimeout": true,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": null,
            "sandboxPolicy": null,
            "outputBytesCap": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_round_trips_disable_output_cap() {
    let params = CommandExecParams {
        command: vec!["yes".to_string()],
        process_id: Some("yes-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: true,
        output_bytes_cap: None,
        disable_output_cap: true,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: None,
        sandbox_policy: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["yes"],
            "processId": "yes-1",
            "streamStdoutStderr": true,
            "outputBytesCap": null,
            "disableOutputCap": true,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": null,
            "sandboxPolicy": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_round_trips_env_overrides_and_unsets() {
    let params = CommandExecParams {
        command: vec!["printenv".to_string(), "FOO".to_string()],
        process_id: Some("env-1".to_string()),
        tty: false,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: Some(HashMap::from([
            ("FOO".to_string(), Some("override".to_string())),
            ("BAR".to_string(), Some("added".to_string())),
            ("BAZ".to_string(), None),
        ])),
        size: None,
        sandbox_policy: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["printenv", "FOO"],
            "processId": "env-1",
            "outputBytesCap": null,
            "timeoutMs": null,
            "cwd": null,
            "env": {
                "FOO": "override",
                "BAR": "added",
                "BAZ": null,
            },
            "size": null,
            "sandboxPolicy": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_write_round_trips_close_only_payload() {
    let params = CommandExecWriteParams {
        process_id: "proc-7".to_string(),
        delta_base64: None,
        close_stdin: true,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/write params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-7",
            "deltaBase64": null,
            "closeStdin": true,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecWriteParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_terminate_round_trips() {
    let params = CommandExecTerminateParams {
        process_id: "proc-8".to_string(),
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/terminate params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-8",
        })
    );

    let decoded = serde_json::from_value::<CommandExecTerminateParams>(value)
        .expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_params_round_trip_with_size() {
    let params = CommandExecParams {
        command: vec!["top".to_string()],
        process_id: Some("pty-1".to_string()),
        tty: true,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: None,
        disable_output_cap: false,
        disable_timeout: false,
        timeout_ms: None,
        cwd: None,
        env: None,
        size: Some(CommandExecTerminalSize {
            rows: 40,
            cols: 120,
        }),
        sandbox_policy: None,
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec params");
    assert_eq!(
        value,
        json!({
            "command": ["top"],
            "processId": "pty-1",
            "tty": true,
            "outputBytesCap": null,
            "timeoutMs": null,
            "cwd": null,
            "env": null,
            "size": {
                "rows": 40,
                "cols": 120,
            },
            "sandboxPolicy": null,
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_resize_round_trips() {
    let params = CommandExecResizeParams {
        process_id: "proc-9".to_string(),
        size: CommandExecTerminalSize {
            rows: 50,
            cols: 160,
        },
    };

    let value = serde_json::to_value(&params).expect("serialize command/exec/resize params");
    assert_eq!(
        value,
        json!({
            "processId": "proc-9",
            "size": {
                "rows": 50,
                "cols": 160,
            },
        })
    );

    let decoded =
        serde_json::from_value::<CommandExecResizeParams>(value).expect("deserialize round-trip");
    assert_eq!(decoded, params);
}

#[test]
fn command_exec_output_delta_round_trips() {
    let notification = CommandExecOutputDeltaNotification {
        process_id: "proc-1".to_string(),
        stream: CommandExecOutputStream::Stdout,
        delta_base64: "AQI=".to_string(),
        cap_reached: false,
    };

    let value = serde_json::to_value(&notification)
        .expect("serialize command/exec/outputDelta notification");
    assert_eq!(
        value,
        json!({
            "processId": "proc-1",
            "stream": "stdout",
            "deltaBase64": "AQI=",
            "capReached": false,
        })
    );

    let decoded = serde_json::from_value::<CommandExecOutputDeltaNotification>(value)
        .expect("deserialize round-trip");
    assert_eq!(decoded, notification);
}

#[test]
fn sandbox_policy_round_trips_external_sandbox_network_access() {
    let v2_policy = SandboxPolicy::ExternalSandbox {
        network_access: NetworkAccess::Enabled,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        codex_protocol::protocol::SandboxPolicy::ExternalSandbox {
            network_access: CoreNetworkAccess::Enabled,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn sandbox_policy_round_trips_read_only_access() {
    let readable_root = test_absolute_path();
    let v2_policy = SandboxPolicy::ReadOnly {
        access: ReadOnlyAccess::Restricted {
            include_platform_defaults: false,
            readable_roots: vec![readable_root.clone()],
        },
        network_access: true,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        codex_protocol::protocol::SandboxPolicy::ReadOnly {
            access: CoreReadOnlyAccess::Restricted {
                include_platform_defaults: false,
                readable_roots: vec![readable_root],
            },
            network_access: true,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn ask_for_approval_granular_round_trips_request_permissions_flag() {
    let v2_policy = AskForApproval::Granular {
        sandbox_approval: true,
        rules: false,
        skill_approval: false,
        request_permissions: true,
        mcp_elicitations: false,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        CoreAskForApproval::Granular(CoreGranularApprovalConfig {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: true,
            mcp_elicitations: false,
        })
    );

    let back_to_v2 = AskForApproval::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn ask_for_approval_granular_defaults_missing_optional_flags_to_false() {
    let decoded = serde_json::from_value::<AskForApproval>(serde_json::json!({
        "granular": {
            "sandbox_approval": true,
            "rules": false,
            "mcp_elicitations": true,
        }
    }))
    .expect("granular approval policy should deserialize");

    assert_eq!(
        decoded,
        AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        }
    );
}

#[test]
fn ask_for_approval_granular_is_marked_experimental() {
    let reason =
        crate::experimental_api::ExperimentalApi::experimental_reason(&AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        });

    assert_eq!(reason, Some("askForApproval.granular"));
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&AskForApproval::OnRequest,),
        None
    );
}

#[test]
fn profile_v2_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&ProfileV2 {
        model: None,
        model_provider: None,
        approval_policy: Some(AskForApproval::Granular {
            sandbox_approval: true,
            rules: false,
            skill_approval: false,
            request_permissions: true,
            mcp_elicitations: false,
        }),
        approvals_reviewer: None,
        service_tier: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        web_search: None,
        tools: None,
        chatgpt_base_url: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: Some(AskForApproval::Granular {
            sandbox_approval: false,
            rules: true,
            skill_approval: false,
            request_permissions: false,
            mcp_elicitations: true,
        }),
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::new(),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_approvals_reviewer_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: Some(ApprovalsReviewer::GuardianSubagent),
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::new(),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("config/read.approvalsReviewer"));
}

#[test]
fn config_nested_profile_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::from([(
            "default".to_string(),
            ProfileV2 {
                model: None,
                model_provider: None,
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                approvals_reviewer: None,
                service_tier: None,
                model_reasoning_effort: None,
                model_reasoning_summary: None,
                model_verbosity: None,
                web_search: None,
                tools: None,
                chatgpt_base_url: None,
                additional: HashMap::new(),
            },
        )]),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn config_nested_profile_approvals_reviewer_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(&Config {
        model: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_provider: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_mode: None,
        sandbox_workspace_write: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search: None,
        tools: None,
        profile: None,
        profiles: HashMap::from([(
            "default".to_string(),
            ProfileV2 {
                model: None,
                model_provider: None,
                approval_policy: None,
                approvals_reviewer: Some(ApprovalsReviewer::GuardianSubagent),
                service_tier: None,
                model_reasoning_effort: None,
                model_reasoning_summary: None,
                model_verbosity: None,
                web_search: None,
                tools: None,
                chatgpt_base_url: None,
                additional: HashMap::new(),
            },
        )]),
        instructions: None,
        developer_instructions: None,
        compact_prompt: None,
        model_reasoning_effort: None,
        model_reasoning_summary: None,
        model_verbosity: None,
        service_tier: None,
        analytics: None,
        apps: None,
        additional: HashMap::new(),
    });

    assert_eq!(reason, Some("config/read.approvalsReviewer"));
}

#[test]
fn config_requirements_granular_allowed_approval_policy_is_marked_experimental() {
    let reason =
        crate::experimental_api::ExperimentalApi::experimental_reason(&ConfigRequirements {
            allowed_approval_policies: Some(vec![AskForApproval::Granular {
                sandbox_approval: true,
                rules: true,
                skill_approval: false,
                request_permissions: false,
                mcp_elicitations: false,
            }]),
            allowed_sandbox_modes: None,
            allowed_web_search_modes: None,
            feature_requirements: None,
            enforce_residency: None,
            network: None,
        });

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_start_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadStart {
            request_id: crate::RequestId::Integer(1),
            params: ThreadStartParams {
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: true,
                    mcp_elicitations: false,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_resume_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadResume {
            request_id: crate::RequestId::Integer(2),
            params: ThreadResumeParams {
                thread_id: "thr_123".to_string(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: false,
                    rules: true,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_thread_fork_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::ThreadFork {
            request_id: crate::RequestId::Integer(3),
            params: ThreadForkParams {
                thread_id: "thr_456".to_string(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: true,
                    rules: false,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn client_request_turn_start_granular_approval_policy_is_marked_experimental() {
    let reason = crate::experimental_api::ExperimentalApi::experimental_reason(
        &crate::ClientRequest::TurnStart {
            request_id: crate::RequestId::Integer(4),
            params: TurnStartParams {
                thread_id: "thr_123".to_string(),
                input: Vec::new(),
                approval_policy: Some(AskForApproval::Granular {
                    sandbox_approval: false,
                    rules: true,
                    skill_approval: false,
                    request_permissions: false,
                    mcp_elicitations: true,
                }),
                ..Default::default()
            },
        },
    );

    assert_eq!(reason, Some("askForApproval.granular"));
}

#[test]
fn mcp_server_elicitation_response_round_trips_rmcp_result() {
    let rmcp_result = rmcp::model::CreateElicitationResult {
        action: rmcp::model::ElicitationAction::Accept,
        content: Some(json!({
            "confirmed": true,
        })),
    };

    let v2_response = McpServerElicitationRequestResponse::from(rmcp_result.clone());
    assert_eq!(
        v2_response,
        McpServerElicitationRequestResponse {
            action: McpServerElicitationAction::Accept,
            content: Some(json!({
                "confirmed": true,
            })),
            meta: None,
        }
    );
    assert_eq!(
        rmcp::model::CreateElicitationResult::from(v2_response),
        rmcp_result
    );
}

#[test]
fn mcp_server_elicitation_request_from_core_url_request() {
    let request = McpServerElicitationRequest::try_from(CoreElicitationRequest::Url {
        meta: None,
        message: "Finish sign-in".to_string(),
        url: "https://example.com/complete".to_string(),
        elicitation_id: "elicitation-123".to_string(),
    })
    .expect("URL request should convert");

    assert_eq!(
        request,
        McpServerElicitationRequest::Url {
            meta: None,
            message: "Finish sign-in".to_string(),
            url: "https://example.com/complete".to_string(),
            elicitation_id: "elicitation-123".to_string(),
        }
    );
}

#[test]
fn mcp_server_elicitation_request_from_core_form_request() {
    let request = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: None,
        message: "Allow this request?".to_string(),
        requested_schema: json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "boolean",
                }
            },
            "required": ["confirmed"],
        }),
    })
    .expect("form request should convert");

    let expected_schema: McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {
            "confirmed": {
                "type": "boolean",
            }
        },
        "required": ["confirmed"],
    }))
    .expect("expected schema should deserialize");

    assert_eq!(
        request,
        McpServerElicitationRequest::Form {
            meta: None,
            message: "Allow this request?".to_string(),
            requested_schema: expected_schema,
        }
    );
}

#[test]
fn mcp_elicitation_schema_matches_mcp_2025_11_25_primitives() {
    let schema: McpElicitationSchema = serde_json::from_value(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "email": {
                "type": "string",
                "title": "Email",
                "description": "Work email address",
                "format": "email",
                "default": "dev@example.com",
            },
            "count": {
                "type": "integer",
                "title": "Count",
                "description": "How many items to create",
                "minimum": 1,
                "maximum": 5,
                "default": 3,
            },
            "confirmed": {
                "type": "boolean",
                "title": "Confirm",
                "description": "Approve the pending action",
                "default": true,
            },
            "legacyChoice": {
                "type": "string",
                "title": "Action",
                "description": "Legacy titled enum form",
                "enum": ["allow", "deny"],
                "enumNames": ["Allow", "Deny"],
                "default": "allow",
            },
        },
        "required": ["email", "confirmed"],
    }))
    .expect("schema should deserialize");

    assert_eq!(
        schema,
        McpElicitationSchema {
            schema_uri: Some("https://json-schema.org/draft/2020-12/schema".to_string()),
            type_: McpElicitationObjectType::Object,
            properties: BTreeMap::from([
                (
                    "confirmed".to_string(),
                    McpElicitationPrimitiveSchema::Boolean(McpElicitationBooleanSchema {
                        type_: McpElicitationBooleanType::Boolean,
                        title: Some("Confirm".to_string()),
                        description: Some("Approve the pending action".to_string()),
                        default: Some(true),
                    }),
                ),
                (
                    "count".to_string(),
                    McpElicitationPrimitiveSchema::Number(McpElicitationNumberSchema {
                        type_: McpElicitationNumberType::Integer,
                        title: Some("Count".to_string()),
                        description: Some("How many items to create".to_string()),
                        minimum: Some(1.0),
                        maximum: Some(5.0),
                        default: Some(3.0),
                    }),
                ),
                (
                    "email".to_string(),
                    McpElicitationPrimitiveSchema::String(McpElicitationStringSchema {
                        type_: McpElicitationStringType::String,
                        title: Some("Email".to_string()),
                        description: Some("Work email address".to_string()),
                        min_length: None,
                        max_length: None,
                        format: Some(McpElicitationStringFormat::Email),
                        default: Some("dev@example.com".to_string()),
                    }),
                ),
                (
                    "legacyChoice".to_string(),
                    McpElicitationPrimitiveSchema::Enum(McpElicitationEnumSchema::Legacy(
                        McpElicitationLegacyTitledEnumSchema {
                            type_: McpElicitationStringType::String,
                            title: Some("Action".to_string()),
                            description: Some("Legacy titled enum form".to_string()),
                            enum_: vec!["allow".to_string(), "deny".to_string()],
                            enum_names: Some(vec!["Allow".to_string(), "Deny".to_string(),]),
                            default: Some("allow".to_string()),
                        },
                    )),
                ),
            ]),
            required: Some(vec!["email".to_string(), "confirmed".to_string()]),
        }
    );
}

#[test]
fn mcp_server_elicitation_request_rejects_null_core_form_schema() {
    let result = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: Some(json!({
            "persist": "session",
        })),
        message: "Allow this request?".to_string(),
        requested_schema: JsonValue::Null,
    });

    assert!(result.is_err());
}

#[test]
fn mcp_server_elicitation_request_rejects_invalid_core_form_schema() {
    let result = McpServerElicitationRequest::try_from(CoreElicitationRequest::Form {
        meta: None,
        message: "Allow this request?".to_string(),
        requested_schema: json!({
            "type": "object",
            "properties": {
                "confirmed": {
                    "type": "object",
                }
            },
        }),
    });

    assert!(result.is_err());
}

#[test]
fn mcp_server_elicitation_response_serializes_nullable_content() {
    let response = McpServerElicitationRequestResponse {
        action: McpServerElicitationAction::Decline,
        content: None,
        meta: None,
    };

    assert_eq!(
        serde_json::to_value(response).expect("response should serialize"),
        json!({
            "action": "decline",
            "content": null,
            "_meta": null,
        })
    );
}

#[test]
fn sandbox_policy_round_trips_workspace_write_read_only_access() {
    let readable_root = test_absolute_path();
    let v2_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: vec![],
        read_only_access: ReadOnlyAccess::Restricted {
            include_platform_defaults: false,
            readable_roots: vec![readable_root.clone()],
        },
        network_access: true,
        exclude_tmpdir_env_var: false,
        exclude_slash_tmp: false,
    };

    let core_policy = v2_policy.to_core();
    assert_eq!(
        core_policy,
        codex_protocol::protocol::SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            read_only_access: CoreReadOnlyAccess::Restricted {
                include_platform_defaults: false,
                readable_roots: vec![readable_root],
            },
            network_access: true,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        }
    );

    let back_to_v2 = SandboxPolicy::from(core_policy);
    assert_eq!(back_to_v2, v2_policy);
}

#[test]
fn sandbox_policy_deserializes_legacy_read_only_without_access_field() {
    let policy: SandboxPolicy = serde_json::from_value(json!({
        "type": "readOnly"
    }))
    .expect("read-only policy should deserialize");
    assert_eq!(
        policy,
        SandboxPolicy::ReadOnly {
            access: ReadOnlyAccess::FullAccess,
            network_access: false,
        }
    );
}

#[test]
fn sandbox_policy_deserializes_legacy_workspace_write_without_read_only_access_field() {
    let policy: SandboxPolicy = serde_json::from_value(json!({
        "type": "workspaceWrite",
        "writableRoots": [],
        "networkAccess": false,
        "excludeTmpdirEnvVar": false,
        "excludeSlashTmp": false
    }))
    .expect("workspace-write policy should deserialize");
    assert_eq!(
        policy,
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            read_only_access: ReadOnlyAccess::FullAccess,
            network_access: false,
            exclude_tmpdir_env_var: false,
            exclude_slash_tmp: false,
        }
    );
}

#[test]
fn automatic_approval_review_deserializes_legacy_snake_case_risk_fields() {
    let review: GuardianApprovalReview = serde_json::from_value(json!({
        "status": "denied",
        "risk_score": 91,
        "risk_level": "high",
        "rationale": "too risky"
    }))
    .expect("legacy snake_case automatic review should deserialize");
    assert_eq!(
        review,
        GuardianApprovalReview {
            status: GuardianApprovalReviewStatus::Denied,
            risk_score: Some(91),
            risk_level: Some(GuardianRiskLevel::High),
            rationale: Some("too risky".to_string()),
        }
    );
}

#[test]
fn automatic_approval_review_deserializes_aborted_status() {
    let review: GuardianApprovalReview = serde_json::from_value(json!({
        "status": "aborted",
        "riskScore": null,
        "riskLevel": null,
        "rationale": null
    }))
    .expect("aborted automatic review should deserialize");
    assert_eq!(
        review,
        GuardianApprovalReview {
            status: GuardianApprovalReviewStatus::Aborted,
            risk_score: None,
            risk_level: None,
            rationale: None,
        }
    );
}

#[test]
fn core_turn_item_into_thread_item_converts_supported_variants() {
    let user_item = TurnItem::UserMessage(UserMessageItem {
        id: "user-1".to_string(),
        content: vec![
            CoreUserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            },
            CoreUserInput::Image {
                image_url: "https://example.com/image.png".to_string(),
            },
            CoreUserInput::LocalImage {
                path: PathBuf::from("local/image.png"),
            },
            CoreUserInput::Skill {
                name: "skill-creator".to_string(),
                path: PathBuf::from("/repo/.codex/skills/skill-creator/SKILL.md"),
            },
            CoreUserInput::Mention {
                name: "Demo App".to_string(),
                path: "app://demo-app".to_string(),
            },
        ],
    });

    assert_eq!(
        ThreadItem::from(user_item),
        ThreadItem::UserMessage {
            id: "user-1".to_string(),
            content: vec![
                UserInput::Text {
                    text: "hello".to_string(),
                    text_elements: Vec::new(),
                },
                UserInput::Image {
                    url: "https://example.com/image.png".to_string(),
                },
                UserInput::LocalImage {
                    path: PathBuf::from("local/image.png"),
                },
                UserInput::Skill {
                    name: "skill-creator".to_string(),
                    path: PathBuf::from("/repo/.codex/skills/skill-creator/SKILL.md"),
                },
                UserInput::Mention {
                    name: "Demo App".to_string(),
                    path: "app://demo-app".to_string(),
                },
            ],
        }
    );

    let agent_item = TurnItem::AgentMessage(AgentMessageItem {
        id: "agent-1".to_string(),
        content: vec![
            AgentMessageContent::Text {
                text: "Hello ".to_string(),
            },
            AgentMessageContent::Text {
                text: "world".to_string(),
            },
        ],
        phase: None,
    });

    assert_eq!(
        ThreadItem::from(agent_item),
        ThreadItem::AgentMessage {
            id: "agent-1".to_string(),
            text: "Hello world".to_string(),
            phase: None,
        }
    );

    let agent_item_with_phase = TurnItem::AgentMessage(AgentMessageItem {
        id: "agent-2".to_string(),
        content: vec![AgentMessageContent::Text {
            text: "final".to_string(),
        }],
        phase: Some(MessagePhase::FinalAnswer),
    });

    assert_eq!(
        ThreadItem::from(agent_item_with_phase),
        ThreadItem::AgentMessage {
            id: "agent-2".to_string(),
            text: "final".to_string(),
            phase: Some(MessagePhase::FinalAnswer),
        }
    );

    let reasoning_item = TurnItem::Reasoning(ReasoningItem {
        id: "reasoning-1".to_string(),
        summary_text: vec!["line one".to_string(), "line two".to_string()],
        raw_content: vec![],
    });

    assert_eq!(
        ThreadItem::from(reasoning_item),
        ThreadItem::Reasoning {
            id: "reasoning-1".to_string(),
            summary: vec!["line one".to_string(), "line two".to_string()],
            content: vec![],
        }
    );

    let search_item = TurnItem::WebSearch(WebSearchItem {
        id: "search-1".to_string(),
        query: "docs".to_string(),
        action: CoreWebSearchAction::Search {
            query: Some("docs".to_string()),
            queries: None,
        },
    });

    assert_eq!(
        ThreadItem::from(search_item),
        ThreadItem::WebSearch {
            id: "search-1".to_string(),
            query: "docs".to_string(),
            action: Some(WebSearchAction::Search {
                query: Some("docs".to_string()),
                queries: None,
            }),
        }
    );
}

#[test]
fn skills_list_params_serialization_uses_force_reload() {
    assert_eq!(
        serde_json::to_value(SkillsListParams {
            cwds: Vec::new(),
            force_reload: false,
            per_cwd_extra_user_roots: None,
        })
        .unwrap(),
        json!({
            "perCwdExtraUserRoots": null,
        }),
    );

    assert_eq!(
        serde_json::to_value(SkillsListParams {
            cwds: vec![PathBuf::from("/repo")],
            force_reload: true,
            per_cwd_extra_user_roots: Some(vec![SkillsListExtraRootsForCwd {
                cwd: PathBuf::from("/repo"),
                extra_user_roots: vec![PathBuf::from("/shared/skills"), PathBuf::from("/tmp/x")],
            }]),
        })
        .unwrap(),
        json!({
            "cwds": ["/repo"],
            "forceReload": true,
            "perCwdExtraUserRoots": [
                {
                    "cwd": "/repo",
                    "extraUserRoots": ["/shared/skills", "/tmp/x"],
                }
            ],
        }),
    );
}

#[test]
fn plugin_list_params_serialization_uses_force_remote_sync() {
    assert_eq!(
        serde_json::to_value(PluginListParams {
            cwds: None,
            force_remote_sync: false,
        })
        .unwrap(),
        json!({
            "cwds": null,
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginListParams {
            cwds: None,
            force_remote_sync: true,
        })
        .unwrap(),
        json!({
            "cwds": null,
            "forceRemoteSync": true,
        }),
    );
}

#[test]
fn codex_error_info_serializes_http_status_code_in_camel_case() {
    let value = CodexErrorInfo::ResponseTooManyFailedAttempts {
        http_status_code: Some(401),
    };

    assert_eq!(
        serde_json::to_value(value).unwrap(),
        json!({
            "responseTooManyFailedAttempts": {
                "httpStatusCode": 401
            }
        })
    );
}

#[test]
fn dynamic_tool_response_serializes_content_items() {
    let value = serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: "dynamic-ok".to_string(),
        }],
        success: true,
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "contentItems": [
                {
                    "type": "inputText",
                    "text": "dynamic-ok"
                }
            ],
            "success": true,
        })
    );
}

#[test]
fn dynamic_tool_response_serializes_text_and_image_content_items() {
    let value = serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![
            DynamicToolCallOutputContentItem::InputText {
                text: "dynamic-ok".to_string(),
            },
            DynamicToolCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,AAA".to_string(),
            },
        ],
        success: true,
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "contentItems": [
                {
                    "type": "inputText",
                    "text": "dynamic-ok"
                },
                {
                    "type": "inputImage",
                    "imageUrl": "data:image/png;base64,AAA"
                }
            ],
            "success": true,
        })
    );
}

#[test]
fn dynamic_tool_spec_deserializes_defer_loading() {
    let value = json!({
        "name": "lookup_ticket",
        "description": "Fetch a ticket",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string" }
            }
        },
        "deferLoading": true,
    });

    let actual: DynamicToolSpec = serde_json::from_value(value).expect("deserialize");

    assert_eq!(
        actual,
        DynamicToolSpec {
            name: "lookup_ticket".to_string(),
            description: "Fetch a ticket".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                }
            }),
            defer_loading: true,
        }
    );
}

#[test]
fn dynamic_tool_spec_legacy_expose_to_context_inverts_to_defer_loading() {
    let value = json!({
        "name": "lookup_ticket",
        "description": "Fetch a ticket",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "exposeToContext": false,
    });

    let actual: DynamicToolSpec = serde_json::from_value(value).expect("deserialize");

    assert!(actual.defer_loading);
}

#[test]
fn thread_start_params_preserve_explicit_null_service_tier() {
    let params: ThreadStartParams =
        serde_json::from_value(json!({ "serviceTier": null })).expect("params should deserialize");
    assert_eq!(params.service_tier, Some(None));

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(
        serialized.get("serviceTier"),
        Some(&serde_json::Value::Null)
    );

    let serialized_without_override =
        serde_json::to_value(ThreadStartParams::default()).expect("params should serialize");
    assert_eq!(serialized_without_override.get("serviceTier"), None);
}

#[test]
fn turn_start_params_preserve_explicit_null_service_tier() {
    let params: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
        "serviceTier": null
    }))
    .expect("params should deserialize");
    assert_eq!(params.service_tier, Some(None));

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(
        serialized.get("serviceTier"),
        Some(&serde_json::Value::Null)
    );

    let without_override = TurnStartParams {
        thread_id: "thread_123".to_string(),
        input: vec![],
        cwd: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_policy: None,
        model: None,
        service_tier: None,
        effort: None,
        summary: None,
        output_schema: None,
        collaboration_mode: None,
        personality: None,
    };
    let serialized_without_override =
        serde_json::to_value(&without_override).expect("params should serialize");
    assert_eq!(serialized_without_override.get("serviceTier"), None);
}
