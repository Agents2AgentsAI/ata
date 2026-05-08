use super::*;

v2_enum_from_core! {
    pub enum NetworkApprovalProtocol from CoreNetworkApprovalProtocol {
        Http,
        Https,
        Socks5Tcp,
        Socks5Udp,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct NetworkApprovalContext {
    pub host: String,
    pub protocol: NetworkApprovalProtocol,
}

impl From<CoreNetworkApprovalContext> for NetworkApprovalContext {
    fn from(value: CoreNetworkApprovalContext) -> Self {
        Self {
            host: value.host,
            protocol: value.protocol.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalFileSystemPermissions {
    pub read: Option<Vec<AbsolutePathBuf>>,
    pub write: Option<Vec<AbsolutePathBuf>>,
}

impl From<CoreFileSystemPermissions> for AdditionalFileSystemPermissions {
    fn from(value: CoreFileSystemPermissions) -> Self {
        Self {
            read: value.read,
            write: value.write,
        }
    }
}

impl From<AdditionalFileSystemPermissions> for CoreFileSystemPermissions {
    fn from(value: AdditionalFileSystemPermissions) -> Self {
        Self {
            read: value.read,
            write: value.write,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalNetworkPermissions {
    pub enabled: Option<bool>,
}

impl From<CoreNetworkPermissions> for AdditionalNetworkPermissions {
    fn from(value: CoreNetworkPermissions) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

impl From<AdditionalNetworkPermissions> for CoreNetworkPermissions {
    fn from(value: AdditionalNetworkPermissions) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
    pub macos: Option<AdditionalMacOsPermissions>,
}

impl From<CorePermissionProfile> for AdditionalPermissionProfile {
    fn from(value: CorePermissionProfile) -> Self {
        Self {
            network: value.network.map(AdditionalNetworkPermissions::from),
            file_system: value.file_system.map(AdditionalFileSystemPermissions::from),
            macos: value.macos.map(AdditionalMacOsPermissions::from),
        }
    }
}

impl From<AdditionalPermissionProfile> for CorePermissionProfile {
    fn from(value: AdditionalPermissionProfile) -> Self {
        Self {
            network: value.network.map(CoreNetworkPermissions::from),
            file_system: value.file_system.map(CoreFileSystemPermissions::from),
            macos: value.macos.map(CoreMacOsSeatbeltProfileExtensions::from),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GrantedPermissionProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<AdditionalNetworkPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub file_system: Option<AdditionalFileSystemPermissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub macos: Option<GrantedMacOsPermissions>,
}

impl From<GrantedPermissionProfile> for CorePermissionProfile {
    fn from(value: GrantedPermissionProfile) -> Self {
        let macos = value.macos.and_then(|macos| {
            if macos.preferences.is_none()
                && macos.automations.is_none()
                && macos.launch_services.is_none()
                && macos.accessibility.is_none()
                && macos.calendar.is_none()
                && macos.reminders.is_none()
                && macos.contacts.is_none()
            {
                None
            } else {
                Some(CoreMacOsSeatbeltProfileExtensions::from(macos))
            }
        });

        Self {
            network: value.network.map(CoreNetworkPermissions::from),
            file_system: value.file_system.map(CoreFileSystemPermissions::from),
            macos,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum NetworkAccess {
    #[default]
    Restricted,
    Enabled,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum SandboxPolicy {
    DangerFullAccess,
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ReadOnly {
        #[serde(default)]
        access: ReadOnlyAccess,
        #[serde(default)]
        network_access: bool,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    ExternalSandbox {
        #[serde(default)]
        network_access: NetworkAccess,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    WorkspaceWrite {
        #[serde(default)]
        writable_roots: Vec<AbsolutePathBuf>,
        #[serde(default)]
        read_only_access: ReadOnlyAccess,
        #[serde(default)]
        network_access: bool,
        #[serde(default)]
        exclude_tmpdir_env_var: bool,
        #[serde(default)]
        exclude_slash_tmp: bool,
    },
}

impl SandboxPolicy {
    pub fn to_core(&self) -> codex_protocol::protocol::SandboxPolicy {
        match self {
            SandboxPolicy::DangerFullAccess => {
                codex_protocol::protocol::SandboxPolicy::DangerFullAccess
            }
            SandboxPolicy::ReadOnly {
                access,
                network_access,
            } => codex_protocol::protocol::SandboxPolicy::ReadOnly {
                access: access.to_core(),
                network_access: *network_access,
            },
            SandboxPolicy::ExternalSandbox { network_access } => {
                codex_protocol::protocol::SandboxPolicy::ExternalSandbox {
                    network_access: match network_access {
                        NetworkAccess::Restricted => CoreNetworkAccess::Restricted,
                        NetworkAccess::Enabled => CoreNetworkAccess::Enabled,
                    },
                }
            }
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            } => codex_protocol::protocol::SandboxPolicy::WorkspaceWrite {
                writable_roots: writable_roots.clone(),
                read_only_access: read_only_access.to_core(),
                network_access: *network_access,
                exclude_tmpdir_env_var: *exclude_tmpdir_env_var,
                exclude_slash_tmp: *exclude_slash_tmp,
            },
        }
    }
}

impl From<codex_protocol::protocol::SandboxPolicy> for SandboxPolicy {
    fn from(value: codex_protocol::protocol::SandboxPolicy) -> Self {
        match value {
            codex_protocol::protocol::SandboxPolicy::DangerFullAccess => {
                SandboxPolicy::DangerFullAccess
            }
            codex_protocol::protocol::SandboxPolicy::ReadOnly {
                access,
                network_access,
            } => SandboxPolicy::ReadOnly {
                access: ReadOnlyAccess::from(access),
                network_access,
            },
            codex_protocol::protocol::SandboxPolicy::ExternalSandbox { network_access } => {
                SandboxPolicy::ExternalSandbox {
                    network_access: match network_access {
                        CoreNetworkAccess::Restricted => NetworkAccess::Restricted,
                        CoreNetworkAccess::Enabled => NetworkAccess::Enabled,
                    },
                }
            }
            codex_protocol::protocol::SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access,
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            } => SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access: ReadOnlyAccess::from(read_only_access),
                network_access,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(transparent)]
#[ts(type = "Array<string>", export_to = "v2/")]
pub struct ExecPolicyAmendment {
    pub command: Vec<String>,
}

impl ExecPolicyAmendment {
    pub fn into_core(self) -> CoreExecPolicyAmendment {
        CoreExecPolicyAmendment::new(self.command)
    }
}

impl From<CoreExecPolicyAmendment> for ExecPolicyAmendment {
    fn from(value: CoreExecPolicyAmendment) -> Self {
        Self {
            command: value.command().to_vec(),
        }
    }
}

v2_enum_from_core!(
    pub enum NetworkPolicyRuleAction from CoreNetworkPolicyRuleAction {
        Allow, Deny
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct NetworkPolicyAmendment {
    pub host: String,
    pub action: NetworkPolicyRuleAction,
}

impl NetworkPolicyAmendment {
    pub fn into_core(self) -> CoreNetworkPolicyAmendment {
        CoreNetworkPolicyAmendment {
            host: self.host,
            action: self.action.to_core(),
        }
    }
}

impl From<CoreNetworkPolicyAmendment> for NetworkPolicyAmendment {
    fn from(value: CoreNetworkPolicyAmendment) -> Self {
        Self {
            host: value.host,
            action: NetworkPolicyRuleAction::from(value.action),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionsRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub reason: Option<String>,
    pub permissions: AdditionalPermissionProfile,
}

v2_enum_from_core!(
    #[derive(Default)]
    pub enum PermissionGrantScope from CorePermissionGrantScope {
        #[default]
        Turn,
        Session
    }
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PermissionsRequestApprovalResponse {
    pub permissions: GrantedPermissionProfile,
    #[serde(default)]
    pub scope: PermissionGrantScope,
}
