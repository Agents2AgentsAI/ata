// Modular split of API v2 protocol types. Mirrors upstream's
// `protocol/v2/` directory layout to minimize merge-conflict surface.
//
// Shared imports and the `v2_enum_from_core!` macro live here so each
// submodule simply does `use super::*;` to access them.

use crate::RequestId;
use crate::protocol::common::AuthMode;
use codex_experimental_api_macros::ExperimentalApi;
use codex_protocol::account::PlanType;
use codex_protocol::approvals::ElicitationRequest as CoreElicitationRequest;
use codex_protocol::approvals::ExecApprovalRequestSkillMetadata as CoreExecApprovalRequestSkillMetadata;
use codex_protocol::approvals::ExecPolicyAmendment as CoreExecPolicyAmendment;
use codex_protocol::approvals::NetworkApprovalContext as CoreNetworkApprovalContext;
use codex_protocol::approvals::NetworkApprovalProtocol as CoreNetworkApprovalProtocol;
use codex_protocol::approvals::NetworkPolicyAmendment as CoreNetworkPolicyAmendment;
use codex_protocol::approvals::NetworkPolicyRuleAction as CoreNetworkPolicyRuleAction;
use codex_protocol::config_types::ApprovalsReviewer as CoreApprovalsReviewer;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::CollaborationModeMask as CoreCollaborationModeMask;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::config_types::SandboxMode as CoreSandboxMode;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::config_types::Verbosity;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WebSearchToolConfig;
use codex_protocol::items::AgentMessageContent as CoreAgentMessageContent;
use codex_protocol::items::TurnItem as CoreTurnItem;
use codex_protocol::mcp::Resource as McpResource;
use codex_protocol::mcp::ResourceTemplate as McpResourceTemplate;
use codex_protocol::mcp::Tool as McpTool;
use codex_protocol::models::FileSystemPermissions as CoreFileSystemPermissions;
use codex_protocol::models::MacOsAutomationPermission as CoreMacOsAutomationPermission;
use codex_protocol::models::MacOsContactsPermission as CoreMacOsContactsPermission;
use codex_protocol::models::MacOsPreferencesPermission as CoreMacOsPreferencesPermission;
use codex_protocol::models::MacOsSeatbeltProfileExtensions as CoreMacOsSeatbeltProfileExtensions;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::NetworkPermissions as CoreNetworkPermissions;
use codex_protocol::models::PermissionProfile as CorePermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelAvailabilityNux as CoreModelAvailabilityNux;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::default_input_modalities;
use codex_protocol::parse_command::ParsedCommand as CoreParsedCommand;
use codex_protocol::plan_tool::PlanItemArg as CorePlanItemArg;
use codex_protocol::plan_tool::StepStatus as CorePlanStepStatus;
use codex_protocol::protocol::AgentStatus as CoreAgentStatus;
use codex_protocol::protocol::AskForApproval as CoreAskForApproval;
use codex_protocol::protocol::CodexErrorInfo as CoreCodexErrorInfo;
use codex_protocol::protocol::CreditsSnapshot as CoreCreditsSnapshot;
use codex_protocol::protocol::ExecCommandStatus as CoreExecCommandStatus;
use codex_protocol::protocol::GranularApprovalConfig as CoreGranularApprovalConfig;
use codex_protocol::protocol::GuardianRiskLevel as CoreGuardianRiskLevel;
use codex_protocol::protocol::HookEventName as CoreHookEventName;
use codex_protocol::protocol::HookExecutionMode as CoreHookExecutionMode;
use codex_protocol::protocol::HookHandlerType as CoreHookHandlerType;
use codex_protocol::protocol::HookOutputEntry as CoreHookOutputEntry;
use codex_protocol::protocol::HookOutputEntryKind as CoreHookOutputEntryKind;
use codex_protocol::protocol::HookRunStatus as CoreHookRunStatus;
use codex_protocol::protocol::HookRunSummary as CoreHookRunSummary;
use codex_protocol::protocol::HookScope as CoreHookScope;
use codex_protocol::protocol::ModelRerouteReason as CoreModelRerouteReason;
use codex_protocol::protocol::NetworkAccess as CoreNetworkAccess;
use codex_protocol::protocol::PatchApplyStatus as CorePatchApplyStatus;
use codex_protocol::protocol::RateLimitSnapshot as CoreRateLimitSnapshot;
use codex_protocol::protocol::RateLimitWindow as CoreRateLimitWindow;
use codex_protocol::protocol::ReadOnlyAccess as CoreReadOnlyAccess;
use codex_protocol::protocol::RealtimeAudioFrame as CoreRealtimeAudioFrame;
use codex_protocol::protocol::ReviewDecision as CoreReviewDecision;
use codex_protocol::protocol::SessionSource as CoreSessionSource;
use codex_protocol::protocol::SkillDependencies as CoreSkillDependencies;
use codex_protocol::protocol::SkillErrorInfo as CoreSkillErrorInfo;
use codex_protocol::protocol::SkillInterface as CoreSkillInterface;
use codex_protocol::protocol::SkillMetadata as CoreSkillMetadata;
use codex_protocol::protocol::SkillScope as CoreSkillScope;
use codex_protocol::protocol::SkillToolDependency as CoreSkillToolDependency;
use codex_protocol::protocol::SubAgentSource as CoreSubAgentSource;
use codex_protocol::protocol::TokenUsage as CoreTokenUsage;
use codex_protocol::protocol::TokenUsageInfo as CoreTokenUsageInfo;
use codex_protocol::request_permissions::PermissionGrantScope as CorePermissionGrantScope;
use codex_protocol::user_input::ByteRange as CoreByteRange;
use codex_protocol::user_input::TextElement as CoreTextElement;
use codex_protocol::user_input::UserInput as CoreUserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use ts_rs::TS;

macro_rules! v2_enum_from_core {
    (
        $(#[$enum_meta:meta])*
        pub enum $Name:ident from $Src:path {
            $( $(#[$variant_meta:meta])* $Variant:ident ),+ $(,)?
        }
    ) => {
        #[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
        $(#[$enum_meta])*
        #[serde(rename_all = "camelCase")]
        #[ts(export_to = "v2/")]
        pub enum $Name {
            $( $(#[$variant_meta])* $Variant ),+
        }

        impl $Name {
            pub fn to_core(self) -> $Src {
                match self { $( $Name::$Variant => <$Src>::$Variant ),+ }
            }
        }

        impl From<$Src> for $Name {
            fn from(value: $Src) -> Self {
                match value { $( <$Src>::$Variant => $Name::$Variant ),+ }
            }
        }
    };
}

// Submodule declarations (mirrors upstream `protocol/v2/`).
mod account;
mod apps;
mod ata_extensions;
mod collaboration_mode;
mod command_exec;
mod config;
mod device_key;
mod experimental_feature;
mod feedback;
mod fs;
mod hook;
mod item;
mod mcp;
mod model;
mod notification;
mod permissions;
mod plugin;
mod process;
mod realtime;
mod review;
mod shared;
mod thread;
mod thread_data;
mod turn;
mod windows_sandbox;

// Re-export every public item to preserve a flat public namespace.
pub use account::*;
pub use apps::*;
pub use ata_extensions::*;
pub use collaboration_mode::*;
pub use command_exec::*;
pub use config::*;
pub use experimental_feature::*;
pub use feedback::*;
pub use fs::*;
pub use hook::*;
pub use item::*;
pub use mcp::*;
pub use model::*;
pub use notification::*;
pub use permissions::*;
pub use plugin::*;
pub use realtime::*;
pub use review::*;
pub use shared::*;
pub use thread::*;
pub use thread_data::*;
pub use turn::*;
pub use windows_sandbox::*;

#[cfg(test)]
mod tests;
