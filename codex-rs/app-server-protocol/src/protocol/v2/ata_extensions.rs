use super::shared::default_include_platform_defaults;
use super::*;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AdditionalMacOsPermissions {
    pub preferences: CoreMacOsPreferencesPermission,
    pub automations: CoreMacOsAutomationPermission,
    pub launch_services: bool,
    pub accessibility: bool,
    pub calendar: bool,
    pub reminders: bool,
    pub contacts: CoreMacOsContactsPermission,
}

impl From<CoreMacOsSeatbeltProfileExtensions> for AdditionalMacOsPermissions {
    fn from(value: CoreMacOsSeatbeltProfileExtensions) -> Self {
        Self {
            preferences: value.macos_preferences,
            automations: value.macos_automation,
            launch_services: value.macos_launch_services,
            accessibility: value.macos_accessibility,
            calendar: value.macos_calendar,
            reminders: value.macos_reminders,
            contacts: value.macos_contacts,
        }
    }
}

impl From<AdditionalMacOsPermissions> for CoreMacOsSeatbeltProfileExtensions {
    fn from(value: AdditionalMacOsPermissions) -> Self {
        Self {
            macos_preferences: value.preferences,
            macos_automation: value.automations,
            macos_launch_services: value.launch_services,
            macos_accessibility: value.accessibility,
            macos_calendar: value.calendar,
            macos_reminders: value.reminders,
            macos_contacts: value.contacts,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GrantedMacOsPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferences: Option<CoreMacOsPreferencesPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub automations: Option<CoreMacOsAutomationPermission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub launch_services: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub accessibility: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub calendar: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reminders: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub contacts: Option<CoreMacOsContactsPermission>,
}

impl From<GrantedMacOsPermissions> for CoreMacOsSeatbeltProfileExtensions {
    fn from(value: GrantedMacOsPermissions) -> Self {
        Self {
            macos_preferences: value
                .preferences
                .unwrap_or(CoreMacOsPreferencesPermission::None),
            macos_automation: value
                .automations
                .unwrap_or(CoreMacOsAutomationPermission::None),
            macos_launch_services: value.launch_services.unwrap_or(false),
            macos_accessibility: value.accessibility.unwrap_or(false),
            macos_calendar: value.calendar.unwrap_or(false),
            macos_reminders: value.reminders.unwrap_or(false),
            macos_contacts: value.contacts.unwrap_or(CoreMacOsContactsPermission::None),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum ReadOnlyAccess {
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Restricted {
        #[serde(default = "default_include_platform_defaults")]
        include_platform_defaults: bool,
        #[serde(default)]
        readable_roots: Vec<AbsolutePathBuf>,
    },
    #[default]
    FullAccess,
}

impl ReadOnlyAccess {
    pub fn to_core(&self) -> CoreReadOnlyAccess {
        match self {
            ReadOnlyAccess::Restricted {
                include_platform_defaults,
                readable_roots,
            } => CoreReadOnlyAccess::Restricted {
                include_platform_defaults: *include_platform_defaults,
                readable_roots: readable_roots.clone(),
            },
            ReadOnlyAccess::FullAccess => CoreReadOnlyAccess::FullAccess,
        }
    }
}

impl From<CoreReadOnlyAccess> for ReadOnlyAccess {
    fn from(value: CoreReadOnlyAccess) -> Self {
        match value {
            CoreReadOnlyAccess::Restricted {
                include_platform_defaults,
                readable_roots,
            } => ReadOnlyAccess::Restricted {
                include_platform_defaults,
                readable_roots,
            },
            CoreReadOnlyAccess::FullAccess => ReadOnlyAccess::FullAccess,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SkillsRemoteReadParams {
    #[serde(default)]
    pub hazelnut_scope: HazelnutScope,
    #[serde(default)]
    pub product_surface: ProductSurface,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS, Default)]
#[serde(rename_all = "kebab-case")]
#[ts(rename_all = "kebab-case")]
#[ts(export_to = "v2/")]
pub enum HazelnutScope {
    #[default]
    Example,
    WorkspaceShared,
    AllShared,
    Personal,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum ProductSurface {
    Chatgpt,
    #[default]
    Codex,
    Api,
    Atlas,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct RemoteSkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SkillsRemoteReadResponse {
    pub data: Vec<RemoteSkillSummary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SkillsRemoteWriteParams {
    pub hazelnut_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct SkillsRemoteWriteResponse {
    pub id: String,
    pub path: PathBuf,
}

impl From<CoreExecApprovalRequestSkillMetadata> for CommandExecutionRequestApprovalSkillMetadata {
    fn from(value: CoreExecApprovalRequestSkillMetadata) -> Self {
        Self {
            path_to_skills_md: value.path_to_skills_md,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct CommandExecutionRequestApprovalSkillMetadata {
    pub path_to_skills_md: PathBuf,
}
