use crate::agents_md::AgentsMdManager;
use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::config::types::AppsConfigToml;
use crate::config::types::DEFAULT_OTEL_ENVIRONMENT;
use crate::config::types::History;
use crate::config::types::McpServerConfig;
use crate::config::types::McpServerDisabledReason;
use crate::config::types::McpServerTransportConfig;
use crate::config::types::MemoriesConfig;
use crate::config::types::MemoriesToml;
use crate::config::types::ModelAvailabilityNuxConfig;
use crate::config::types::Notice;
use crate::config::types::NotificationMethod;
use crate::config::types::Notifications;
use crate::config::types::OtelConfig;
use crate::config::types::OtelConfigToml;
use crate::config::types::OtelExporterKind;
use crate::config::types::PluginConfig;
use crate::config::types::ReadingViewToml;
use crate::config::types::SandboxWorkspaceWrite;
use crate::config::types::ShellEnvironmentPolicy;
use crate::config::types::ShellEnvironmentPolicyToml;
use crate::config::types::SkillsConfig;
use crate::config::types::Tui;
use crate::config::types::UriBasedFileOpener;
use crate::config::types::VoiceModeToml;
use crate::config::types::WindowsSandboxModeToml;
use crate::config::types::WindowsToml;
use crate::config_loader::CloudRequirementsLoader;
use crate::config_loader::ConfigLayerStack;
use crate::config_loader::ConfigLayerStackOrdering;
use crate::config_loader::ConfigRequirements;
use crate::config_loader::ConstrainedWithSource;
use crate::config_loader::LoaderOverrides;
use crate::config_loader::McpServerIdentity;
use crate::config_loader::McpServerRequirement;
use crate::config_loader::ResidencyRequirement;
use crate::config_loader::Sourced;
use crate::config_loader::load_config_layers_state;
use crate::features::Feature;
use crate::features::FeatureOverrides;
use crate::features::Features;
use crate::features::FeaturesToml;
use crate::git_info::resolve_root_git_project_for_trust;
use crate::memories::memory_root;
use crate::model_provider_info::LEGACY_OLLAMA_CHAT_PROVIDER_ID;
use crate::model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::OLLAMA_CHAT_PROVIDER_REMOVED_ERROR;
use crate::model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use crate::model_provider_info::OPENAI_PROVIDER_ID;
use crate::model_provider_info::built_in_model_providers;
use crate::path_utils::normalize_for_native_workdir;
use crate::project_doc::DEFAULT_PROJECT_DOC_FILENAME;
use crate::project_doc::LOCAL_PROJECT_DOC_FILENAME;
use crate::protocol::AskForApproval;
use crate::protocol::ReadOnlyAccess;
use crate::protocol::SandboxPolicy;
use crate::unified_exec::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::unified_exec::MIN_EMPTY_YIELD_TIME_MS;
use crate::windows_sandbox::WindowsSandboxLevelExt;
use crate::windows_sandbox::resolve_windows_sandbox_mode;
use crate::windows_sandbox::resolve_windows_sandbox_private_desktop;
use codex_app_server_protocol::Tools;
use codex_app_server_protocol::UserSavedConfig;
use codex_protocol::config_types::AltScreenMode;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::config_types::TrustLevel;
use codex_protocol::config_types::Verbosity;
use codex_protocol::config_types::WebSearchConfig;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WebSearchToolConfig;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::ActivePermissionProfileModification;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxEnforcement;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_rmcp_client::OAuthCredentialsStoreMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use crate::config::permissions::compile_permission_profile;
use crate::config::permissions::network_proxy_config_from_profile_network;
use crate::config::profile::ConfigProfile;
use codex_network_proxy::NetworkProxyConfig;
use toml::Value as TomlValue;
use toml_edit::DocumentMut;
use toml_edit::value;

pub(crate) mod agent_roles;
pub mod edit;
mod managed_features;
mod mcp_config;
mod network_proxy_spec;
mod permissions;
pub mod profile;
mod project;
pub mod schema;
pub mod service;
pub mod types;
mod web_search;
pub use codex_config::Constrained;
pub use codex_config::ConstraintError;
pub use codex_config::ConstraintResult;
pub use codex_network_proxy::NetworkProxyAuditMetadata;
use codex_sandboxing::compatibility_sandbox_policy_for_permission_profile;
pub use codex_sandboxing::system_bwrap_warning;
pub use managed_features::ManagedFeatures;
use mcp_config::constrain_mcp_servers;
#[cfg(test)]
use mcp_config::filter_mcp_servers_by_requirements;
pub use mcp_config::load_global_mcp_servers;
pub use network_proxy_spec::NetworkProxySpec;
pub use network_proxy_spec::StartedNetworkProxy;
pub use permissions::FilesystemPermissionToml;
pub use permissions::FilesystemPermissionsToml;
pub use permissions::NetworkToml;
pub use permissions::PermissionProfileToml;
pub use permissions::PermissionsToml;
pub(crate) use permissions::resolve_permission_profile;
pub use project::ProjectConfig;
pub use project::set_project_trust_level;
pub(crate) use project::set_project_trust_level_inner;
pub use service::ConfigService;
pub use service::ConfigServiceError;
pub use types::ApprovalsReviewer;
use web_search::resolve_web_search_config;
use web_search::resolve_web_search_mode;
pub(crate) use web_search::resolve_web_search_mode_for_turn;

/// Well-known prefix for provider-fallback warnings in `startup_warnings`.
/// The TUI checks for this prefix to force the login screen when a fallback
/// to the openai provider occurs (cached tokens are likely stale/absent).
pub const MODEL_PROVIDER_FALLBACK_PREFIX: &str = "Model provider fallback:";

const DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS: i64 = 200;
const DEFAULT_IGNORE_LARGE_UNTRACKED_FILES: i64 = 10 * 1024 * 1024;

/// Compatibility-only config retained so legacy `ghost_snapshot` settings
/// continue to load even though snapshots are no longer produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostSnapshotConfig {
    pub ignore_large_untracked_files: Option<i64>,
    pub ignore_large_untracked_dirs: Option<i64>,
    pub disable_warnings: bool,
}

impl Default for GhostSnapshotConfig {
    fn default() -> Self {
        Self {
            ignore_large_untracked_files: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_FILES),
            ignore_large_untracked_dirs: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS),
            disable_warnings: false,
        }
    }
}

/// Maximum number of bytes of the documentation that will be embedded. Larger
/// files are *silently truncated* to this size so we do not take up too much of
/// the context window.
pub(crate) const PROJECT_DOC_MAX_BYTES: usize = 32 * 1024; // 32 KiB
pub(crate) const DEFAULT_AGENT_MAX_THREADS: Option<usize> = Some(10);
pub(crate) const DEFAULT_AGENT_MAX_DEPTH: i32 = 1;
pub(crate) const DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS: Option<u64> = None;
const LOCAL_DEV_BUILD_VERSION: &str = "0.0.0";

pub const CONFIG_TOML_FILE: &str = "config.toml";
const OPENAI_BASE_URL_ENV_VAR: &str = "OPENAI_BASE_URL";
const RESERVED_MODEL_PROVIDER_IDS: [&str; 3] = [
    OPENAI_PROVIDER_ID,
    OLLAMA_OSS_PROVIDER_ID,
    LMSTUDIO_OSS_PROVIDER_ID,
];

fn resolve_sqlite_home_env(resolved_cwd: &Path) -> Option<PathBuf> {
    let raw = std::env::var(codex_state::SQLITE_HOME_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(resolved_cwd.join(path))
    }
}

fn resolve_cli_auth_credentials_store_mode(
    configured: AuthCredentialsStoreMode,
    package_version: &str,
) -> AuthCredentialsStoreMode {
    match (package_version, configured) {
        (
            LOCAL_DEV_BUILD_VERSION,
            AuthCredentialsStoreMode::Keyring | AuthCredentialsStoreMode::Auto,
        ) => AuthCredentialsStoreMode::File,
        (_, mode) => mode,
    }
}

fn resolve_mcp_oauth_credentials_store_mode(
    configured: OAuthCredentialsStoreMode,
    package_version: &str,
) -> OAuthCredentialsStoreMode {
    match (package_version, configured) {
        (
            LOCAL_DEV_BUILD_VERSION,
            OAuthCredentialsStoreMode::Keyring | OAuthCredentialsStoreMode::Auto,
        ) => OAuthCredentialsStoreMode::File,
        (_, mode) => mode,
    }
}

#[cfg(test)]
pub(crate) fn test_config() -> Config {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        AbsolutePathBuf::from_absolute_path(codex_home.path()).expect("temp dir should resolve"),
    )
    .await
    .expect("load default test config")
}

/// Application configuration loaded from disk and merged with overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Permissions {
    /// Approval policy for executing commands.
    pub approval_policy: Constrained<AskForApproval>,
    /// Effective sandbox policy used for shell/unified exec.
    pub sandbox_policy: Constrained<SandboxPolicy>,
    /// Effective filesystem sandbox policy, including entries that cannot yet
    /// be fully represented by the legacy [`SandboxPolicy`] projection.
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    /// Effective network sandbox policy split out from the legacy
    /// [`SandboxPolicy`] projection.
    pub network_sandbox_policy: NetworkSandboxPolicy,
    /// Effective network configuration applied to all spawned processes.
    pub network: Option<NetworkProxySpec>,
    /// Whether the model may request a login shell for shell-based tools.
    /// Default to `true`
    ///
    /// If `true`, the model may request a login shell (`login = true`), and
    /// omitting `login` defaults to using a login shell.
    /// If `false`, the model can never use a login shell: `login = true`
    /// requests are rejected, and omitting `login` defaults to a non-login
    /// shell.
    pub allow_login_shell: bool,
    /// Policy used to build process environments for shell/unified exec.
    pub shell_environment_policy: ShellEnvironmentPolicy,
    /// Effective Windows sandbox mode derived from `[windows].sandbox` or
    /// legacy feature keys.
    pub windows_sandbox_mode: Option<WindowsSandboxModeToml>,
    /// Whether the final Windows sandboxed child should run on a private desktop.
    pub windows_sandbox_private_desktop: bool,
    /// Optional macOS seatbelt extension profile used to extend default
    /// seatbelt permissions when running under seatbelt.
    pub macos_seatbelt_profile_extensions: Option<MacOsSeatbeltProfileExtensions>,
}

/// Application configuration loaded from disk and merged with overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Provenance for how this [`Config`] was derived (merged layers + enforced
    /// requirements).
    pub config_layer_stack: ConfigLayerStack,

    /// Warnings collected during config load that should be shown on startup.
    pub startup_warnings: Vec<String>,

    /// Optional override of model selection.
    pub model: Option<String>,

    /// Effective service tier request id preference for new turns.
    pub service_tier: Option<String>,

    /// Model used specifically for review sessions.
    pub review_model: Option<String>,

    /// Size of the context window for the model, in tokens.
    pub model_context_window: Option<i64>,

    /// Token usage threshold triggering auto-compaction of conversation history.
    pub model_auto_compact_token_limit: Option<i64>,

    /// Key into the model_providers map that specifies which provider to use.
    pub model_provider_id: String,

    /// Info needed to make an API request to the model.
    pub model_provider: ModelProviderInfo,

    /// Optionally specify the personality of the model
    pub personality: Option<Personality>,

    /// Effective permission configuration for shell tool execution.
    pub permissions: Permissions,

    /// Configures who approval requests are routed to for review once they have
    /// been escalated. This does not disable separate safety checks such as
    /// ARC.
    pub approvals_reviewer: ApprovalsReviewer,

    /// enforce_residency means web traffic cannot be routed outside of a
    /// particular geography. HTTP clients should direct their requests
    /// using backend-specific headers or URLs to enforce this.
    pub enforce_residency: Constrained<Option<ResidencyRequirement>>,

    /// When `true`, `AgentReasoning` events emitted by the backend will be
    /// suppressed from the frontend output. This can reduce visual noise when
    /// users are only interested in the final agent responses.
    pub hide_agent_reasoning: bool,

    /// When set to `true`, `AgentReasoningRawContentEvent` events will be shown in the UI/output.
    /// Defaults to `false`.
    pub show_raw_agent_reasoning: bool,

    /// User-provided instructions from AGENTS.md.
    pub user_instructions: Option<String>,

    /// Base instructions override.
    pub base_instructions: Option<String>,

    /// Developer instructions override injected as a separate message.
    pub developer_instructions: Option<String>,

    /// Guardian-specific policy config override from requirements.toml or config.toml.
    /// This is inserted into the fixed guardian prompt template under the
    /// `# Policy Configuration` section rather than replacing the whole
    /// guardian developer prompt.
    pub guardian_policy_config: Option<String>,

    /// Whether to inject the `<permissions instructions>` developer block.
    pub include_permissions_instructions: bool,

    /// Whether to inject the `<apps_instructions>` developer block.
    pub include_apps_instructions: bool,

    /// Whether to inject the `<skills_instructions>` developer block.
    pub include_skill_instructions: bool,

    /// Whether to inject the `<environment_context>` user block.
    pub include_environment_context: bool,

    /// Compact prompt override.
    pub compact_prompt: Option<String>,

    /// Optional commit attribution text for commit message co-author trailers.
    /// This top-level setting only takes effect when `[features].codex_git_commit`
    /// is enabled.
    ///
    /// - `None`: use default attribution (`Codex <noreply@openai.com>`)
    /// - `Some("")` or whitespace-only: disable commit attribution
    /// - `Some("...")`: use the provided attribution text verbatim
    pub commit_attribution: Option<String>,

    /// Optional external notifier command. When set, Codex will spawn this
    /// program after each completed *turn* (i.e. when the agent finishes
    /// processing a user submission). The value must be the full command
    /// broken into argv tokens **without** the trailing JSON argument - Codex
    /// appends one extra argument containing a JSON payload describing the
    /// event.
    ///
    /// Example `~/.codex/config.toml` snippet:
    ///
    /// ```toml
    /// notify = ["notify-send", "Codex"]
    /// ```
    ///
    /// which will be invoked as:
    ///
    /// ```shell
    /// notify-send Codex '{"type":"agent-turn-complete","turn-id":"12345"}'
    /// ```
    ///
    /// If unset the feature is disabled.
    pub notify: Option<Vec<String>>,

    /// TUI notification settings, including enabled events, delivery method, and focus condition.
    pub tui_notifications: TuiNotificationSettings,

    /// Enable ASCII animations and shimmer effects in the TUI.
    pub animations: bool,

    /// Show startup tooltips in the TUI welcome screen.
    pub show_tooltips: bool,

    /// Persisted startup availability NUX state for model tooltips.
    pub model_availability_nux: ModelAvailabilityNuxConfig,

    /// Start the composer in Vim mode (`Normal`) by default.
    pub tui_vim_mode_default: bool,

    /// Start the TUI in raw scrollback mode for copy-friendly transcript output.
    pub tui_raw_output_mode: bool,

    /// Start the TUI in the specified collaboration mode (plan/default).

    /// Controls whether the TUI uses the terminal's alternate screen buffer.
    ///
    /// This is the same `tui.alternate_screen` value from `config.toml`.
    /// - `auto` (default): Disable alternate screen in Zellij, enable elsewhere.
    /// - `always`: Always use alternate screen (original behavior).
    /// - `never`: Never use alternate screen (inline mode, preserves scrollback).
    pub tui_alternate_screen: AltScreenMode,
    /// Ordered list of status line item identifiers for the TUI.
    ///
    /// When unset, the TUI defaults to: `model-with-reasoning` and `current-dir`.
    pub tui_status_line: Option<Vec<String>>,

    /// Whether to color status line items with colors from the active syntax theme.
    pub tui_status_line_use_colors: bool,

    /// Ordered list of terminal title item identifiers for the TUI.
    ///
    /// When unset, the TUI defaults to: `activity` and `project`.
    /// The `activity` item spins while working and shows an action-required
    /// message when blocked on the user.
    pub tui_terminal_title: Option<Vec<String>>,

    /// Syntax highlighting theme override (kebab-case name).
    pub tui_theme: Option<String>,

    /// Preferred layout for resume/fork session picker results.
    pub tui_session_picker_view: SessionPickerViewMode,

    /// Terminal resize-reflow tuning knobs.
    pub terminal_resize_reflow: TerminalResizeReflowConfig,

    /// Keybinding overrides for the TUI.
    ///
    /// Precedence is:
    ///
    /// 1. context table (`tui.keymap.chat`, `tui.keymap.composer`, etc.)
    /// 2. `tui.keymap.global`
    /// 3. built-in defaults
    pub tui_keymap: TuiKeymap,

    /// The absolute directory that should be treated as the current working
    /// directory for the session. All relative paths inside the business-logic
    /// layer are resolved against this path.
    pub cwd: AbsolutePathBuf,

    /// Preferred store for CLI auth credentials.
    /// file (default): Use a file in the Codex home directory.
    /// keyring: Use an OS-specific keyring service.
    /// auto: Use the OS-specific keyring service if available, otherwise use a file.
    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,

    /// Definition for MCP servers that Codex can reach out to for tool calls.
    pub mcp_servers: Constrained<HashMap<String, McpServerConfig>>,

    /// Preferred store for MCP OAuth credentials.
    /// keyring: Use an OS-specific keyring service.
    ///          Credentials stored in the keyring will only be readable by Codex unless the user explicitly grants access via OS-level keyring access.
    ///          https://github.com/openai/codex/blob/main/codex-rs/rmcp-client/src/oauth.rs#L2
    /// file: CODEX_HOME/.credentials.json
    ///       This file will be readable to Codex and other applications running as the same user.
    /// auto (default): keyring if available, otherwise file.
    pub mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode,

    /// Optional fixed port to use for the local HTTP callback server used during MCP OAuth login.
    ///
    /// When unset, Codex will bind to an ephemeral port chosen by the OS.
    pub mcp_oauth_callback_port: Option<u16>,

    /// Optional redirect URI to use during MCP OAuth login.
    ///
    /// When set, this URI is used in the OAuth authorization request instead
    /// of the local listener address. The local callback listener still binds
    /// to 127.0.0.1 (using `mcp_oauth_callback_port` when provided).
    pub mcp_oauth_callback_url: Option<String>,

    /// Combined provider map (defaults plus user-defined providers).
    pub model_providers: HashMap<String, ModelProviderInfo>,

    /// Maximum number of bytes to include from an AGENTS.md project doc file.
    pub project_doc_max_bytes: usize,

    /// Additional filenames to try when looking for project-level docs.
    pub project_doc_fallback_filenames: Vec<String>,

    /// Token budget applied when storing tool/function outputs in the context manager.
    pub tool_output_token_limit: Option<usize>,

    /// Maximum number of agent threads that can be open concurrently.
    pub agent_max_threads: Option<usize>,
    /// Maximum runtime in seconds for agent job workers before they are failed.
    pub agent_job_max_runtime_seconds: Option<u64>,

    /// Whether to record a model-visible message when an agent turn is interrupted.
    pub agent_interrupt_message_enabled: bool,

    /// Maximum nesting depth allowed for spawned agent threads.
    pub agent_max_depth: i32,

    /// User-defined role declarations keyed by role name.
    pub agent_roles: BTreeMap<String, AgentRoleConfig>,

    /// Memories subsystem settings.
    pub memories: MemoriesConfig,

    /// Directory containing all Codex state (defaults to `~/.codex` but can be
    /// overridden by the `CODEX_HOME` environment variable).
    pub codex_home: AbsolutePathBuf,

    /// Directory where Codex stores the SQLite state DB.
    pub sqlite_home: PathBuf,

    /// Directory where Codex writes log files (defaults to `$CODEX_HOME/log`).
    pub log_dir: PathBuf,

    /// Directory where Codex writes effective session config lock files.
    pub config_lock_export_dir: Option<AbsolutePathBuf>,

    /// Whether config lock replay ignores Codex version drift between the
    /// lock metadata and the regenerated lock.
    pub config_lock_allow_codex_version_mismatch: bool,

    /// Whether config lock creation saves values resolved from the model
    /// catalog/session configuration.
    pub config_lock_save_fields_resolved_from_model_catalog: bool,

    /// Effective config lock used for strict replay validation.
    pub config_lock_toml: Option<Arc<ConfigLockfileToml>>,

    /// Settings that govern if and what will be written to `~/.codex/history.jsonl`.
    pub history: History,

    /// When true, session is not persisted on disk. Default to `false`
    pub ephemeral: bool,

    /// Optional URI-based file opener. If set, citations to files in the model
    /// output will be hyperlinked using the specified URI scheme.
    pub file_opener: UriBasedFileOpener,

    /// Path to the current Codex executable. This cannot be set in the config
    /// file: it must be set in code via [`ConfigOverrides`].
    pub codex_self_exe: Option<PathBuf>,

    /// Path to the `codex-linux-sandbox` executable. This must be set if
    /// [`codex_sandboxing::SandboxType::LinuxSeccomp`] is used. Note that this
    /// cannot be set in the config file: it must be set in code via
    /// [`ConfigOverrides`].
    ///
    /// When this program is invoked, arg0 will be set to `codex-linux-sandbox`.
    pub codex_linux_sandbox_exe: Option<PathBuf>,

    /// Path to the `codex-execve-wrapper` executable used for shell
    /// escalation. This cannot be set in the config file: it must be set in
    /// code via [`ConfigOverrides`].
    pub main_execve_wrapper_exe: Option<PathBuf>,

    /// Optional absolute path to patched zsh used by zsh-exec-bridge-backed shell execution.
    pub zsh_path: Option<PathBuf>,

    /// Value to use for `reasoning.effort` when making a request using the
    /// Responses API.
    pub model_reasoning_effort: Option<ReasoningEffort>,
    /// Optional Plan-mode-specific reasoning effort override used by the TUI.
    ///
    /// When unset, Plan mode uses the built-in Plan preset default (currently
    /// `medium`). When explicitly set (including `none`), this overrides the
    /// Plan preset. The `none` value means "no reasoning" (not "inherit the
    /// global default").
    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,

    /// Optional value to use for `reasoning.summary` when making a request
    /// using the Responses API. When unset, the model catalog default is used.
    pub model_reasoning_summary: Option<ReasoningSummary>,

    /// Optional override to force-enable reasoning summaries for the configured model.
    pub model_supports_reasoning_summaries: Option<bool>,

    /// Optional full model catalog loaded from `model_catalog_json`.
    /// When set, this replaces the bundled catalog for the current process.
    pub model_catalog: Option<ModelsResponse>,

    /// Optional verbosity control for GPT-5 models (Responses API `text.verbosity`).
    pub model_verbosity: Option<Verbosity>,

    /// Base URL for requests to ChatGPT (as opposed to the OpenAI API).
    pub chatgpt_base_url: String,

    /// Optional path override for the built-in apps MCP server.
    pub apps_mcp_path_override: Option<String>,

    /// Machine-local realtime audio device preferences used by realtime voice.
    pub realtime_audio: RealtimeAudioConfig,

    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport base URL (the `Op::RealtimeConversation`
    /// `/v1/realtime`
    /// connection) without changing normal provider HTTP requests.
    pub experimental_realtime_ws_base_url: Option<String>,
    /// Experimental / do not use. Selects the realtime websocket model/snapshot
    /// used for the `Op::RealtimeConversation` connection.
    pub experimental_realtime_ws_model: Option<String>,
    /// Experimental / do not use. Realtime websocket session selection.
    /// `version` controls v1/v2 and `type` controls conversational/transcription.
    pub realtime: RealtimeConfig,
    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport instructions (the `Op::RealtimeConversation`
    /// `/ws` session.update instructions) without changing normal prompts.
    pub experimental_realtime_ws_backend_prompt: Option<String>,
    /// Experimental / do not use. Replaces the synthesized realtime startup
    /// context appended to websocket session instructions. An empty string
    /// disables startup context injection entirely.
    pub experimental_realtime_ws_startup_context: Option<String>,
    /// Experimental / do not use. Replaces the built-in realtime start
    /// instructions inserted into developer messages when realtime becomes
    /// active.
    pub experimental_realtime_start_instructions: Option<String>,
    /// When set, restricts ChatGPT login to a specific workspace identifier.
    pub forced_chatgpt_workspace_id: Option<String>,

    /// When set, restricts the login mechanism users may use.
    pub forced_login_method: Option<ForcedLoginMethod>,

    /// Include the `apply_patch` tool for models that benefit from invoking
    /// file edits as a structured tool call. When unset, this falls back to the
    /// model info's default preference.
    pub include_apply_patch_tool: bool,

    /// Explicit or feature-derived web search mode.
    pub web_search_mode: Constrained<WebSearchMode>,

    /// Additional parameters for the web search tool when it is enabled.
    pub web_search_config: Option<WebSearchConfig>,

    /// If set to `true`, used only the experimental unified exec tool.
    pub use_experimental_unified_exec_tool: bool,

    /// Maximum poll window for background terminal output (`write_stdin`), in milliseconds.
    /// Default: `300000` (5 minutes).
    pub background_terminal_max_timeout: u64,

    /// Compatibility-only settings retained for legacy `ghost_snapshot`
    /// config loading.
    pub ghost_snapshot: GhostSnapshotConfig,

    /// Settings specific to the task-path-based multi-agent tool surface.
    pub multi_agent_v2: MultiAgentV2Config,

    /// Centralized feature flags; source of truth for feature gating.
    pub features: ManagedFeatures,

    /// Research tools configuration (Zotero, OpenAlex, etc.).
    pub research: Option<ResearchToolsToml>,

    /// Optional non-secret data tool settings from `[data]`.
    pub data: Option<DataToolsToml>,

    /// Optional KB settings from `[kb]`.
    pub kb: Option<KbToml>,

    /// Agent coordination channel settings from `[coordination]`.
    pub coordination: Option<CoordinationToml>,

    /// When `true`, suppress warnings about unstable (under development) features.
    pub suppress_unstable_features_warning: bool,

    /// The active profile name used to derive this `Config` (if any).
    pub active_profile: Option<String>,

    /// The currently active project config, resolved by checking if cwd:
    /// is (1) part of a git repo, (2) a git worktree, or (3) just using the cwd
    pub active_project: ProjectConfig,

    /// Tracks whether the Windows onboarding screen has been acknowledged.
    pub windows_wsl_setup_acknowledged: bool,

    /// Collection of various notices we show the user
    pub notices: Notice,

    /// When `true`, checks for Codex updates on startup and surfaces update prompts.
    /// Set to `false` only if your Codex updates are centrally managed.
    /// Defaults to `true`.
    pub check_for_update_on_startup: bool,

    /// When true, disables burst-paste detection for typed input entirely.
    /// All characters are inserted as they are received, and no buffering
    /// or placeholder replacement will occur for fast keypress bursts.
    pub disable_paste_burst: bool,

    /// When `false`, disables analytics across Codex product surfaces in this machine.
    /// Voluntarily left as Optional because the default value might depend on the client.
    pub analytics_enabled: Option<bool>,

    /// When `false`, disables feedback collection across Codex product surfaces.
    /// Defaults to `true`.
    pub feedback_enabled: bool,

    /// Configured discoverable tools for tool suggestions.
    pub tool_suggest: ToolSuggestConfig,

    /// OTEL configuration (exporter type, endpoint, headers, etc.).
    pub otel: codex_config::types::OtelConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MultiAgentV2Config {
    pub max_concurrent_threads_per_session: usize,
    pub min_wait_timeout_ms: i64,
    pub usage_hint_enabled: bool,
    pub usage_hint_text: Option<String>,
    pub root_agent_usage_hint_text: Option<String>,
    pub subagent_usage_hint_text: Option<String>,
    pub hide_spawn_agent_metadata: bool,
}

impl Default for MultiAgentV2Config {
    fn default() -> Self {
        Self {
            max_concurrent_threads_per_session:
                DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION,
            min_wait_timeout_ms: DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS,
            usage_hint_enabled: true,
            usage_hint_text: None,
            root_agent_usage_hint_text: None,
            subagent_usage_hint_text: None,
            hide_spawn_agent_metadata: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalResizeReflowMaxRows {
    /// Use the runtime terminal detector to choose a scrollback-sized cap.
    #[default]
    Auto,
    /// Keep all rendered transcript rows during resize reflow.
    Disabled,
    /// Keep at most this many rendered transcript rows during resize reflow.
    Limit(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerminalResizeReflowConfig {
    pub max_rows: TerminalResizeReflowMaxRows,
}

impl AuthManagerConfig for Config {
    fn codex_home(&self) -> PathBuf {
        self.codex_home.to_path_buf()
    }

    fn cli_auth_credentials_store_mode(&self) -> AuthCredentialsStoreMode {
        self.cli_auth_credentials_store_mode
    }

    fn forced_chatgpt_workspace_id(&self) -> Option<String> {
        self.forced_chatgpt_workspace_id.clone()
    }

    fn chatgpt_base_url(&self) -> String {
        self.chatgpt_base_url.clone()
    }
}

#[derive(Clone, Default)]
pub struct ConfigBuilder {
    codex_home: Option<PathBuf>,
    cli_overrides: Option<Vec<(String, TomlValue)>>,
    harness_overrides: Option<ConfigOverrides>,
    loader_overrides: Option<LoaderOverrides>,
    cloud_requirements: CloudRequirementsLoader,
    thread_config_loader: Option<Arc<dyn ThreadConfigLoader>>,
    fallback_cwd: Option<PathBuf>,
}

impl ConfigBuilder {
    pub fn codex_home(mut self, codex_home: PathBuf) -> Self {
        self.codex_home = Some(codex_home);
        self
    }

    pub fn cli_overrides(mut self, cli_overrides: Vec<(String, TomlValue)>) -> Self {
        self.cli_overrides = Some(cli_overrides);
        self
    }

    pub fn harness_overrides(mut self, harness_overrides: ConfigOverrides) -> Self {
        self.harness_overrides = Some(harness_overrides);
        self
    }

    pub fn loader_overrides(mut self, loader_overrides: LoaderOverrides) -> Self {
        self.loader_overrides = Some(loader_overrides);
        self
    }

    pub fn cloud_requirements(mut self, cloud_requirements: CloudRequirementsLoader) -> Self {
        self.cloud_requirements = cloud_requirements;
        self
    }

    pub fn thread_config_loader(
        mut self,
        thread_config_loader: Arc<dyn ThreadConfigLoader>,
    ) -> Self {
        self.thread_config_loader = Some(thread_config_loader);
        self
    }

    pub fn fallback_cwd(mut self, fallback_cwd: Option<PathBuf>) -> Self {
        self.fallback_cwd = fallback_cwd;
        self
    }

    pub async fn build(self) -> std::io::Result<Config> {
        // Keep the large config-loading future off small runtime thread stacks.
        Box::pin(self.build_inner()).await
    }

    async fn build_inner(self) -> std::io::Result<Config> {
        let Self {
            codex_home,
            cli_overrides,
            harness_overrides,
            loader_overrides,
            cloud_requirements,
            thread_config_loader,
            fallback_cwd,
        } = self;
        let codex_home = codex_home.map_or_else(find_codex_home, std::io::Result::Ok)?;
        if let Err(err) = maybe_migrate_smart_approvals_alias(&codex_home).await {
            tracing::warn!(error = %err, "failed to migrate smart_approvals feature alias");
        }
        let cli_overrides = cli_overrides.unwrap_or_default();
        let mut harness_overrides = harness_overrides.unwrap_or_default();
        let loader_overrides = loader_overrides.unwrap_or_default();
        let cwd_override = harness_overrides.cwd.as_deref().or(fallback_cwd.as_deref());
        let cwd = match cwd_override {
            Some(path) => AbsolutePathBuf::relative_to_current_dir(path)?,
            None => AbsolutePathBuf::current_dir()?,
        };
        harness_overrides.cwd = Some(cwd.to_path_buf());
        let config_layer_stack = load_config_layers_state(
            LOCAL_FS.as_ref(),
            &codex_home,
            Some(cwd),
            &cli_overrides,
            loader_overrides,
            cloud_requirements,
            thread_config_loader
                .as_deref()
                .unwrap_or(&codex_config::NoopThreadConfigLoader),
        )
        .await?;
        let merged_toml = config_layer_stack.effective_config();

        // Note that each layer in ConfigLayerStack should have resolved
        // relative paths to absolute paths based on the parent folder of the
        // respective config file, so we should be safe to deserialize without
        // AbsolutePathBufGuard here.
        let config_toml: ConfigToml = match merged_toml.try_into() {
            Ok(config_toml) => config_toml,
            Err(err) => {
                if let Some(config_error) = codex_config::first_layer_config_error::<ConfigToml>(
                    &config_layer_stack,
                    codex_config::CONFIG_TOML_FILE,
                )
                .await
                {
                    return Err(codex_config::io_error_from_config_error(
                        std::io::ErrorKind::InvalidData,
                        config_error,
                        Some(err),
                    ));
                }
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, err));
            }
        };
        let config_lock_settings = config_toml
            .debug
            .as_ref()
            .and_then(|debug| debug.config_lockfile.as_ref());
        if let Some(config_lock_load_path) =
            config_lock_settings.and_then(|config_lock| config_lock.load_path.as_ref())
        {
            let allow_codex_version_mismatch = config_lock_settings
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false);
            let save_fields_resolved_from_model_catalog = config_lock_settings
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true);
            let lockfile_toml = read_config_lock_from_path(config_lock_load_path).await?;
            let expected_lock_config = lockfile_toml.clone();
            let lock_layer = lock_layer_from_config(config_lock_load_path, &lockfile_toml)?;
            let lock_config_toml = config_without_lock_controls(&lockfile_toml.config);
            let lock_config_layer_stack = ConfigLayerStack::new(
                vec![lock_layer],
                config_layer_stack.requirements().clone(),
                config_layer_stack.requirements_toml().clone(),
            )?;
            let mut config = Config::load_config_with_layer_stack(
                LOCAL_FS.as_ref(),
                lock_config_toml,
                harness_overrides,
                codex_home,
                lock_config_layer_stack,
            )
            .await?;
            config.config_lock_toml = Some(Arc::new(expected_lock_config));
            config.config_lock_allow_codex_version_mismatch = allow_codex_version_mismatch;
            config.config_lock_save_fields_resolved_from_model_catalog =
                save_fields_resolved_from_model_catalog;
            return Ok(config);
        }
        Config::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            config_toml,
            harness_overrides,
            codex_home,
            config_layer_stack,
        )
        .await
    }

    #[cfg(test)]
    pub(crate) fn without_managed_config_for_tests() -> Self {
        Self::default().loader_overrides(LoaderOverrides::without_managed_config_for_tests())
    }
}

fn config_scope_segments(scope: &[String], key: &str) -> Vec<String> {
    let mut segments = scope.to_vec();
    segments.push(key.to_string());
    segments
}

fn feature_scope_segments(scope: &[String], feature_key: &str) -> Vec<String> {
    let mut segments = scope.to_vec();
    segments.push("features".to_string());
    segments.push(feature_key.to_string());
    segments
}

fn push_smart_approvals_alias_migration_edits(
    edits: &mut Vec<ConfigEdit>,
    scope: &[String],
    features: &FeaturesToml,
    approvals_reviewer_missing: bool,
) {
    let Some(alias_enabled) = features.entries.get("smart_approvals").copied() else {
        return;
    };
    let canonical_enabled = features
        .entries
        .get("guardian_approval")
        .copied()
        .unwrap_or(alias_enabled);

    if !features.entries.contains_key("guardian_approval") {
        edits.push(ConfigEdit::SetPath {
            segments: feature_scope_segments(scope, "guardian_approval"),
            value: value(alias_enabled),
        });
    }
    if canonical_enabled && approvals_reviewer_missing {
        edits.push(ConfigEdit::SetPath {
            segments: config_scope_segments(scope, "approvals_reviewer"),
            value: value(ApprovalsReviewer::GuardianSubagent.to_string()),
        });
    }
    edits.push(ConfigEdit::ClearPath {
        segments: feature_scope_segments(scope, "smart_approvals"),
    });
}

/// Rewrites the legacy `smart_approvals` feature flag to
/// `guardian_approval` in `config.toml` before normal config loading.
///
/// If the old key is present, this preserves its value by setting
/// `guardian_approval = <alias value>` when the new key is not already present.
/// Because the deprecated flag historically meant "turn guardian review on",
/// this migration also backfills `approvals_reviewer = "guardian_subagent"`
/// in the same scope when that reviewer is not already configured there and the
/// migrated feature value is `true`.
/// In all cases it removes the deprecated `smart_approvals` entry so future
/// loads only see the canonical feature flag name.
async fn maybe_migrate_smart_approvals_alias(codex_home: &Path) -> std::io::Result<bool> {
    let config_path = codex_home.join(CONFIG_TOML_FILE);
    if !tokio::fs::try_exists(&config_path).await? {
        return Ok(false);
    }

    let config_contents = tokio::fs::read_to_string(&config_path).await?;
    let Ok(config_toml) = toml::from_str::<ConfigToml>(&config_contents) else {
        return Ok(false);
    };

    let mut edits = Vec::new();

    let root_scope = Vec::new();
    if let Some(features) = config_toml.features.as_ref() {
        push_smart_approvals_alias_migration_edits(
            &mut edits,
            &root_scope,
            features,
            config_toml.approvals_reviewer.is_none(),
        );
    }

    for (profile_name, profile) in &config_toml.profiles {
        if let Some(features) = profile.features.as_ref() {
            let scope = vec!["profiles".to_string(), profile_name.clone()];
            push_smart_approvals_alias_migration_edits(
                &mut edits,
                &scope,
                features,
                profile.approvals_reviewer.is_none(),
            );
        }
    }

    if edits.is_empty() {
        return Ok(false);
    }

    ConfigEditsBuilder::new(codex_home)
        .with_edits(edits)
        .apply()
        .await
        .map_err(|err| {
            std::io::Error::other(format!("failed to migrate guardian_approval alias: {err}"))
        })?;
    Ok(true)
}

impl Config {
    pub fn legacy_sandbox_policy(&self) -> SandboxPolicy {
        self.permissions.legacy_sandbox_policy(self.cwd.as_path())
    }

    pub fn set_legacy_sandbox_policy(
        &mut self,
        sandbox_policy: SandboxPolicy,
    ) -> ConstraintResult<()> {
        self.permissions
            .set_legacy_sandbox_policy(sandbox_policy, self.cwd.as_path())
    }

    pub fn to_models_manager_config(&self) -> ModelsManagerConfig {
        ModelsManagerConfig {
            model_context_window: self.model_context_window,
            model_auto_compact_token_limit: self.model_auto_compact_token_limit,
            tool_output_token_limit: self.tool_output_token_limit,
            base_instructions: self.base_instructions.clone(),
            personality_enabled: self.features.enabled(Feature::Personality),
            model_supports_reasoning_summaries: self.model_supports_reasoning_summaries,
            model_catalog: self.model_catalog.clone(),
        }
    }

    /// Build the plugin-manager input from the effective config.
    pub fn plugins_config_input(&self) -> PluginsConfigInput {
        PluginsConfigInput::new(
            self.config_layer_stack.clone(),
            self.features.enabled(Feature::Plugins),
            self.features.enabled(Feature::RemotePlugin),
            self.features.enabled(Feature::PluginHooks),
            self.chatgpt_base_url.clone(),
        )
    }

    pub async fn to_mcp_config(
        &self,
        plugins_manager: &codex_core_plugins::PluginsManager,
    ) -> McpConfig {
        let plugins_input = self.plugins_config_input();
        let loaded_plugins = plugins_manager.plugins_for_config(&plugins_input).await;
        let builtin_mcp_servers = configured_builtin_mcp_servers(BuiltinMcpServerOptions {
            codex_self_exe: self.codex_self_exe.as_deref(),
            codex_home: self.codex_home.as_path(),
            memories_enabled: self.features.enabled(Feature::BuiltInMcp)
                && self.features.enabled(Feature::MemoryTool)
                && self.memories.use_memories,
        });
        let mut configured_mcp_servers = self.mcp_servers.get().clone();
        for plugin in loaded_plugins
            .plugins()
            .iter()
            .filter(|plugin| plugin.is_active())
        {
            let mut plugin_mcp_servers = plugin.mcp_servers.clone();
            filter_plugin_mcp_servers_by_requirements(
                &plugin.config_name,
                &mut plugin_mcp_servers,
                self.config_layer_stack.requirements().plugins.as_ref(),
            );
            for (name, plugin_server) in plugin_mcp_servers {
                configured_mcp_servers.entry(name).or_insert(plugin_server);
            }
        }
        if let Some(mcp_requirements) = self.config_layer_stack.requirements().mcp_servers.as_ref()
            && mcp_requirements.value.is_empty()
        {
            // A present empty allowlist bans configurable MCPs, including plugin MCPs merged
            // above. Built-ins are product-owned and stay available regardless of admin
            // allowlists.
            filter_mcp_servers_by_requirements(&mut configured_mcp_servers, Some(mcp_requirements));
        }
        configured_mcp_servers.extend(builtin_mcp_servers);

        McpConfig {
            chatgpt_base_url: self.chatgpt_base_url.clone(),
            apps_mcp_path_override: self.apps_mcp_path_override.clone(),
            codex_home: self.codex_home.to_path_buf(),
            mcp_oauth_credentials_store_mode: self.mcp_oauth_credentials_store_mode,
            mcp_oauth_callback_port: self.mcp_oauth_callback_port,
            mcp_oauth_callback_url: self.mcp_oauth_callback_url.clone(),
            skill_mcp_dependency_install_enabled: self
                .features
                .enabled(Feature::SkillMcpDependencyInstall),
            approval_policy: self.permissions.approval_policy.clone(),
            codex_linux_sandbox_exe: self.codex_linux_sandbox_exe.clone(),
            use_legacy_landlock: self.features.use_legacy_landlock(),
            apps_enabled: self.features.enabled(Feature::Apps),
            configured_mcp_servers,
            plugin_capability_summaries: loaded_plugins.capability_summaries().to_vec(),
        }
    }

    pub async fn rebuild_preserving_session_layers(
        &self,
        refreshed_config: &Config,
    ) -> std::io::Result<Self> {
        let mut layers = refreshed_config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .filter(|layer| !is_session_layer(&layer.name))
            .cloned()
            .collect::<Vec<_>>();
        layers.extend(
            self.config_layer_stack
                .get_layers(
                    ConfigLayerStackOrdering::LowestPrecedenceFirst,
                    /*include_disabled*/ true,
                )
                .into_iter()
                .filter(|layer| is_session_layer(&layer.name))
                .cloned(),
        );
        layers.sort_by_key(|layer| layer.name.precedence());

        let config_layer_stack = ConfigLayerStack::new(
            layers,
            refreshed_config.config_layer_stack.requirements().clone(),
            refreshed_config
                .config_layer_stack
                .requirements_toml()
                .clone(),
        )?
        .with_user_and_project_exec_policy_rules_ignored(
            refreshed_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules(),
        );
        let cfg: ConfigToml = config_layer_stack
            .effective_config()
            .try_into()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            cfg,
            ConfigOverrides {
                cwd: Some(self.cwd.to_path_buf()),
                ..Default::default()
            },
            refreshed_config.codex_home.clone(),
            config_layer_stack,
        )
        .await
    }

    /// This is the preferred way to create an instance of [Config].
    pub async fn load_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .build()
            .await
    }

    /// Load a default configuration when user config files are invalid.
    pub async fn load_default_with_cli_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let codex_home = find_codex_home()?;
        Self::load_default_with_cli_overrides_for_codex_home(
            codex_home.to_path_buf(),
            cli_overrides,
        )
        .await
    }

    /// Load a default configuration for a specific Codex home without reading
    /// user, project, or system config layers.
    pub async fn load_default_with_cli_overrides_for_codex_home(
        codex_home: PathBuf,
        cli_overrides: Vec<(String, TomlValue)>,
    ) -> std::io::Result<Self> {
        let mut merged = toml::Value::try_from(ConfigToml::default()).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to serialize default config: {e}"),
            )
        })?;
        let cli_layer = codex_config::build_cli_overrides_layer(&cli_overrides);
        codex_config::merge_toml_values(&mut merged, &cli_layer);
        let codex_home = AbsolutePathBuf::from_absolute_path_checked(codex_home)?;
        let config_toml = deserialize_config_toml_with_base(merged, &codex_home)?;
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            config_toml,
            ConfigOverrides::default(),
            codex_home,
            ConfigLayerStack::default(),
        )
        .await
    }

    /// This is a secondary way of creating [Config], which is appropriate when
    /// the harness is meant to be used with a specific configuration that
    /// ignores user settings. For example, the `codex exec` subcommand is
    /// designed to use [AskForApproval::Never] exclusively.
    ///
    /// Further, [ConfigOverrides] contains some options that are not supported
    /// in [ConfigToml], such as `cwd`, `codex_self_exe`, `codex_linux_sandbox_exe`, and
    /// `main_execve_wrapper_exe`.
    pub async fn load_with_cli_overrides_and_harness_overrides(
        cli_overrides: Vec<(String, TomlValue)>,
        harness_overrides: ConfigOverrides,
    ) -> std::io::Result<Self> {
        ConfigBuilder::default()
            .cli_overrides(cli_overrides)
            .harness_overrides(harness_overrides)
            .build()
            .await
    }
}

/// DEPRECATED: Use [Config::load_with_cli_overrides()] instead because working
/// with [ConfigToml] directly means that [ConfigRequirements] have not been
/// applied yet, which risks failing to enforce required constraints.
pub async fn load_config_as_toml_with_cli_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
) -> std::io::Result<ConfigToml> {
    if let Err(err) = maybe_migrate_smart_approvals_alias(codex_home).await {
        tracing::warn!(error = %err, "failed to migrate smart_approvals feature alias");
    }
    let config_layer_stack = load_config_layers_state(
        codex_home,
        cwd,
        cli_overrides,
        LoaderOverrides::default(),
    )
    .await
}

pub async fn load_config_as_toml_with_cli_and_loader_overrides(
    codex_home: &Path,
    cwd: Option<&AbsolutePathBuf>,
    cli_overrides: Vec<(String, TomlValue)>,
    loader_overrides: LoaderOverrides,
) -> std::io::Result<ConfigToml> {
    let config_layer_stack = load_config_layers_state(
        LOCAL_FS.as_ref(),
        codex_home,
        cwd.cloned(),
        &cli_overrides,
        loader_overrides,
        CloudRequirementsLoader::default(),
        &codex_config::NoopThreadConfigLoader,
    )
    .await?;

    let merged_toml = config_layer_stack.effective_config();
    let cfg = deserialize_config_toml_with_base(merged_toml, codex_home).map_err(|e| {
        tracing::error!("Failed to deserialize overridden config: {e}");
        e
    })?;

    Ok(cfg)
}

pub fn deserialize_config_toml_with_base(
    root_value: TomlValue,
    config_base_dir: &Path,
) -> std::io::Result<ConfigToml> {
    // This guard ensures that any relative paths that is deserialized into an
    // [AbsolutePathBuf] is resolved against `config_base_dir`.
    let _guard = AbsolutePathBufGuard::new(config_base_dir);
    root_value
        .try_into()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Validate user-visible feature settings against managed feature requirements.
pub fn validate_feature_requirements_for_config_toml(
    cfg: &ConfigToml,
    feature_requirements: Option<&Sourced<FeatureRequirementsToml>>,
) -> std::io::Result<()> {
    managed_features::validate_explicit_feature_settings_in_config_toml(cfg, feature_requirements)?;
    managed_features::validate_feature_requirements_in_config_toml(cfg, feature_requirements)
}

fn load_catalog_json(path: &AbsolutePathBuf) -> std::io::Result<ModelsResponse> {
    let file_contents = std::fs::read_to_string(path)?;
    let catalog = serde_json::from_str::<ModelsResponse>(&file_contents).map_err(|err| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "failed to parse model_catalog_json path `{}` as JSON: {err}",
                path.display()
            ),
        )
    })?;
    if catalog.models.is_empty() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "model_catalog_json path `{}` must contain at least one model",
                path.display()
            ),
        ));
    }
    Ok(catalog)
}

fn load_model_catalog(
    model_catalog_json: Option<AbsolutePathBuf>,
) -> std::io::Result<Option<ModelsResponse>> {
    model_catalog_json
        .map(|path| load_catalog_json(&path))
        .transpose()
}

fn apply_requirement_constrained_value<T>(
    field_name: &'static str,
    configured_value: T,
    constrained_value: &mut ConstrainedWithSource<T>,
    startup_warnings: &mut Vec<String>,
) -> std::io::Result<bool>
where
    T: Clone + std::fmt::Debug + Send + Sync,
{
    if let Err(err) = constrained_value.set(configured_value) {
        let fallback_value = constrained_value.get().clone();
        tracing::warn!(
            error = %err,
            ?fallback_value,
            requirement_source = ?constrained_value.source,
            "configured value is disallowed by requirements; falling back to required value for {field_name}"
        );
        let message = format!(
            "Configured value for `{field_name}` is disallowed by requirements; falling back to required value {fallback_value:?}. Details: {err}"
        );
        startup_warnings.push(message);

        constrained_value.set(fallback_value).map_err(|fallback_err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "configured value for `{field_name}` is disallowed by requirements ({err}); fallback to a requirement-compliant value also failed ({fallback_err})"
                ),
            )
        })?;
        return Ok(true);
    }

    Ok(false)
}

/// Save the default OSS provider preference to config.toml
pub fn set_default_oss_provider(codex_home: &Path, provider: &str) -> std::io::Result<()> {
    codex_config::config_toml::validate_oss_provider(provider)?;
    use toml_edit::value;

    let edits = [ConfigEdit::SetPath {
        segments: vec!["oss_provider".to_string()],
        value: value(provider),
    }];

    ConfigEditsBuilder::new(codex_home)
        .with_edits(edits)
        .apply_blocking()
        .map_err(|err| std::io::Error::other(format!("failed to persist config.toml: {err}")))
}

/// Base config deserialized from ~/.codex/config.toml.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ConfigToml {
    /// Optional override of model selection.
    pub model: Option<String>,
    /// Review model override used by the `/review` feature.
    pub review_model: Option<String>,

    /// Provider to use from the model_providers map.
    pub model_provider: Option<String>,

    /// Size of the context window for the model, in tokens.
    pub model_context_window: Option<i64>,

    /// Token usage threshold triggering auto-compaction of conversation history.
    pub model_auto_compact_token_limit: Option<i64>,

    /// Default approval policy for executing commands.
    pub approval_policy: Option<AskForApproval>,

    /// Configures who approval requests are routed to for review once they have
    /// been escalated. This does not disable separate safety checks such as
    /// ARC.
    pub approvals_reviewer: Option<ApprovalsReviewer>,

    #[serde(default)]
    pub shell_environment_policy: ShellEnvironmentPolicyToml,

    /// Whether the model may request a login shell for shell-based tools.
    /// Default to `true`
    ///
    /// If `true`, the model may request a login shell (`login = true`), and
    /// omitting `login` defaults to using a login shell.
    /// If `false`, the model can never use a login shell: `login = true`
    /// requests are rejected, and omitting `login` defaults to a non-login
    /// shell.
    pub allow_login_shell: Option<bool>,

    /// Sandbox mode to use.
    pub sandbox_mode: Option<SandboxMode>,

    /// Sandbox configuration to apply if `sandbox` is `WorkspaceWrite`.
    pub sandbox_workspace_write: Option<SandboxWorkspaceWrite>,

    /// Default named permissions profile to apply from the `[permissions]`
    /// table.
    pub default_permissions: Option<String>,

    /// Named permissions profiles.
    #[serde(default)]
    pub permissions: Option<PermissionsToml>,

    /// Optional external command to spawn for end-user notifications.
    #[serde(default)]
    pub notify: Option<Vec<String>>,

    /// System instructions.
    pub instructions: Option<String>,

    /// Developer instructions inserted as a `developer` role message.
    #[serde(default)]
    pub developer_instructions: Option<String>,

    /// Optional path to a file containing model instructions that will override
    /// the built-in instructions for the selected model. Users are STRONGLY
    /// DISCOURAGED from using this field, as deviating from the instructions
    /// sanctioned by Codex will likely degrade model performance.
    pub model_instructions_file: Option<AbsolutePathBuf>,

    /// Compact prompt used for history compaction.
    pub compact_prompt: Option<String>,

    /// Optional commit attribution text for commit message co-author trailers.
    ///
    /// Set to an empty string to disable automatic commit attribution.
    pub commit_attribution: Option<String>,

    /// When set, restricts ChatGPT login to a specific workspace identifier.
    #[serde(default)]
    pub forced_chatgpt_workspace_id: Option<String>,

    /// When set, restricts the login mechanism users may use.
    #[serde(default)]
    pub forced_login_method: Option<ForcedLoginMethod>,

    /// Preferred backend for storing CLI auth credentials.
    /// file (default): Use a file in the Codex home directory.
    /// keyring: Use an OS-specific keyring service.
    /// auto: Use the keyring if available, otherwise use a file.
    #[serde(default)]
    pub cli_auth_credentials_store: Option<AuthCredentialsStoreMode>,

    /// Definition for MCP servers that Codex can reach out to for tool calls.
    #[serde(default)]
    // Uses the raw MCP input shape (custom deserialization) rather than `McpServerConfig`.
    #[schemars(schema_with = "crate::config::schema::mcp_servers_schema")]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// Preferred backend for storing MCP OAuth credentials.
    /// keyring: Use an OS-specific keyring service.
    ///          https://github.com/openai/codex/blob/main/codex-rs/rmcp-client/src/oauth.rs#L2
    /// file: Use a file in the Codex home directory.
    /// auto (default): Use the OS-specific keyring service if available, otherwise use a file.
    #[serde(default)]
    pub mcp_oauth_credentials_store: Option<OAuthCredentialsStoreMode>,

    /// Optional fixed port for the local HTTP callback server used during MCP OAuth login.
    /// When unset, Codex will bind to an ephemeral port chosen by the OS.
    pub mcp_oauth_callback_port: Option<u16>,

    /// Optional redirect URI to use during MCP OAuth login.
    /// When set, this URI is used in the OAuth authorization request instead
    /// of the local listener address. The local callback listener still binds
    /// to 127.0.0.1 (using `mcp_oauth_callback_port` when provided).
    pub mcp_oauth_callback_url: Option<String>,

    /// User-defined provider entries that extend the built-in list. Built-in
    /// IDs cannot be overridden.
    #[serde(default, deserialize_with = "deserialize_model_providers")]
    pub model_providers: HashMap<String, ModelProviderInfo>,

    /// Maximum number of bytes to include from an AGENTS.md project doc file.
    pub project_doc_max_bytes: Option<usize>,

    /// Ordered list of fallback filenames to look for when AGENTS.md is missing.
    pub project_doc_fallback_filenames: Option<Vec<String>>,

    /// Token budget applied when storing tool/function outputs in the context manager.
    pub tool_output_token_limit: Option<usize>,

    /// Maximum poll window for background terminal output (`write_stdin`), in milliseconds.
    /// Default: `300000` (5 minutes).
    pub background_terminal_max_timeout: Option<u64>,

    /// Optional absolute path to the Node runtime used by `js_repl`.
    pub js_repl_node_path: Option<AbsolutePathBuf>,

    /// Ordered list of directories to search for Node modules in `js_repl`.
    pub js_repl_node_module_dirs: Option<Vec<AbsolutePathBuf>>,

    /// Optional absolute path to patched zsh used by zsh-exec-bridge-backed shell execution.
    pub zsh_path: Option<AbsolutePathBuf>,

    /// Profile to use from the `profiles` map.
    pub profile: Option<String>,

    /// Named profiles to facilitate switching between different configurations.
    #[serde(default)]
    pub profiles: HashMap<String, ConfigProfile>,

    /// Settings that govern if and what will be written to `~/.codex/history.jsonl`.
    #[serde(default)]
    pub history: Option<History>,

    /// Directory where Codex stores the SQLite state DB.
    /// Defaults to `$CODEX_SQLITE_HOME` when set. Otherwise uses `$CODEX_HOME`.
    pub sqlite_home: Option<AbsolutePathBuf>,

    /// Directory where Codex writes log files, for example `codex-tui.log`.
    /// Defaults to `$CODEX_HOME/log`.
    pub log_dir: Option<AbsolutePathBuf>,

    /// Optional URI-based file opener. If set, citations to files in the model
    /// output will be hyperlinked using the specified URI scheme.
    pub file_opener: Option<UriBasedFileOpener>,

    /// Collection of settings that are specific to the TUI.
    pub tui: Option<Tui>,

    /// When set to `true`, `AgentReasoning` events will be hidden from the
    /// UI/output. Defaults to `false`.
    pub hide_agent_reasoning: Option<bool>,

    /// When set to `true`, `AgentReasoningRawContentEvent` events will be shown in the UI/output.
    /// Defaults to `false`.
    pub show_raw_agent_reasoning: Option<bool>,

    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub plan_mode_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    /// Optional verbosity control for GPT-5 models (Responses API `text.verbosity`).
    pub model_verbosity: Option<Verbosity>,

    /// Override to force-enable reasoning summaries for the configured model.
    pub model_supports_reasoning_summaries: Option<bool>,

    /// Optional path to a JSON model catalog (applied on startup only).
    /// Per-thread `config` overrides are accepted but do not reapply this (no-ops).
    pub model_catalog_json: Option<AbsolutePathBuf>,

    /// Optionally specify a personality for the model
    pub personality: Option<Personality>,

    /// Optional explicit service tier preference for new turns (`fast` or `flex`).
    pub service_tier: Option<ServiceTier>,

    /// Base URL for requests to ChatGPT (as opposed to the OpenAI API).
    pub chatgpt_base_url: Option<String>,

    /// Base URL override for the built-in `openai` model provider.
    pub openai_base_url: Option<String>,

    /// Machine-local realtime audio device preferences used by realtime voice.
    #[serde(default)]
    pub audio: Option<RealtimeAudioToml>,

    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport base URL (the `Op::RealtimeConversation`
    /// `/v1/realtime`
    /// connection) without changing normal provider HTTP requests.
    pub experimental_realtime_ws_base_url: Option<String>,
    /// Experimental / do not use. Selects the realtime websocket model/snapshot
    /// used for the `Op::RealtimeConversation` connection.
    pub experimental_realtime_ws_model: Option<String>,
    /// Experimental / do not use. Realtime websocket session selection.
    /// `version` controls v1/v2 and `type` controls conversational/transcription.
    #[serde(default)]
    pub realtime: Option<RealtimeToml>,
    /// Experimental / do not use. Overrides only the realtime conversation
    /// websocket transport instructions (the `Op::RealtimeConversation`
    /// `/ws` session.update instructions) without changing normal prompts.
    pub experimental_realtime_ws_backend_prompt: Option<String>,
    /// Experimental / do not use. Replaces the synthesized realtime startup
    /// context appended to websocket session instructions. An empty string
    /// disables startup context injection entirely.
    pub experimental_realtime_ws_startup_context: Option<String>,
    /// Experimental / do not use. Replaces the built-in realtime start
    /// instructions inserted into developer messages when realtime becomes
    /// active.
    pub experimental_realtime_start_instructions: Option<String>,
    pub projects: Option<HashMap<String, ProjectConfig>>,

    /// Controls the web search tool mode: disabled, cached, or live.
    pub web_search: Option<WebSearchMode>,

    /// Nested tools section for feature toggles
    pub tools: Option<ToolsToml>,

    /// Agent-related settings (thread limits, etc.).
    pub agents: Option<AgentsToml>,

    /// Memories subsystem settings.
    pub memories: Option<MemoriesToml>,

    /// User-level skill config entries keyed by SKILL.md path.
    pub skills: Option<SkillsConfig>,

    /// Research tools configuration (Zotero, OpenAlex, etc.).
    pub research: Option<ResearchToolsToml>,

    /// Optional non-secret data tool settings.
    #[serde(default)]
    pub data: Option<DataToolsToml>,

    /// LSP server integration configuration.
    #[serde(default)]
    pub lsp: Option<serde_json::Value>,

    /// TreeSitter indexing configuration.
    #[serde(default)]
    pub treesitter: Option<serde_json::Value>,

    /// Optional KB settings.
    #[serde(default)]
    pub kb: Option<KbToml>,

    /// User-level plugin config entries keyed by plugin name.
    #[serde(default)]
    pub plugins: HashMap<String, PluginConfig>,

    /// Agent coordination channel settings.
    #[serde(default)]
    pub coordination: Option<CoordinationToml>,

    /// Centralized feature flags (new). Prefer this over individual toggles.
    #[serde(default)]
    // Injects known feature keys into the schema and forbids unknown keys.
    #[schemars(schema_with = "crate::config::schema::features_schema")]
    pub features: Option<FeaturesToml>,

    /// Suppress warnings about unstable (under development) features.
    pub suppress_unstable_features_warning: Option<bool>,

    /// Settings for ghost snapshots (used for undo).
    #[serde(default)]
    pub ghost_snapshot: Option<GhostSnapshotToml>,

    /// Markers used to detect the project root when searching parent
    /// directories for `.codex` folders. Defaults to [".git"] when unset.
    #[serde(default)]
    pub project_root_markers: Option<Vec<String>>,

    /// When `true`, checks for Codex updates on startup and surfaces update prompts.
    /// Set to `false` only if your Codex updates are centrally managed.
    /// Defaults to `true`.
    pub check_for_update_on_startup: Option<bool>,

    /// When true, disables burst-paste detection for typed input entirely.
    /// All characters are inserted as they are received, and no buffering
    /// or placeholder replacement will occur for fast keypress bursts.
    pub disable_paste_burst: Option<bool>,

    /// When `false`, disables analytics across Codex product surfaces in this machine.
    /// Defaults to `true`.
    pub analytics: Option<crate::config::types::AnalyticsConfigToml>,

    /// When `false`, disables feedback collection across Codex product surfaces.
    /// Defaults to `true`.
    pub feedback: Option<crate::config::types::FeedbackConfigToml>,

    /// Settings for app-specific controls.
    #[serde(default)]
    pub apps: Option<AppsConfigToml>,

    /// OTEL configuration.
    pub otel: Option<crate::config::types::OtelConfigToml>,

    /// Windows-specific configuration.
    #[serde(default)]
    pub windows: Option<WindowsToml>,

    /// Tracks whether the Windows onboarding screen has been acknowledged.
    pub windows_wsl_setup_acknowledged: Option<bool>,

    /// Collection of in-product notices (different from notifications)
    /// See [`crate::config::types::Notices`] for more details
    pub notice: Option<Notice>,

    /// Legacy, now use features
    /// Deprecated: ignored. Use `model_instructions_file`.
    #[schemars(skip)]
    pub experimental_instructions_file: Option<AbsolutePathBuf>,
    pub experimental_compact_prompt_file: Option<AbsolutePathBuf>,
    pub experimental_use_unified_exec_tool: Option<bool>,
    pub experimental_use_freeform_apply_patch: Option<bool>,
    /// Preferred OSS provider for local models, e.g. "lmstudio" or "ollama".
    pub oss_provider: Option<String>,

    /// Voice mode settings (mic → STT → agent → TTS → speaker).
    #[serde(default)]
    pub voice_mode: Option<VoiceModeToml>,

    /// Reading view display mode settings.
    #[serde(default)]
    pub reading_view: Option<ReadingViewToml>,
}

impl From<ConfigToml> for UserSavedConfig {
    fn from(config_toml: ConfigToml) -> Self {
        let profiles = config_toml
            .profiles
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect();

        Self {
            approval_policy: config_toml.approval_policy,
            sandbox_mode: config_toml.sandbox_mode,
            sandbox_settings: config_toml.sandbox_workspace_write.map(From::from),
            forced_chatgpt_workspace_id: config_toml.forced_chatgpt_workspace_id,
            forced_login_method: config_toml.forced_login_method,
            model: config_toml.model,
            model_reasoning_effort: config_toml.model_reasoning_effort,
            model_reasoning_summary: config_toml.model_reasoning_summary,
            model_verbosity: config_toml.model_verbosity,
            tools: config_toml.tools.map(From::from),
            profile: config_toml.profile,
            profiles,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealtimeAudioConfig {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeWsMode {
    #[default]
    Conversational,
    Transcription,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeWsVersion {
    #[default]
    V1,
    V2,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RealtimeConfig {
    pub version: RealtimeWsVersion,
    #[serde(rename = "type")]
    pub session_type: RealtimeWsMode,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RealtimeToml {
    pub version: Option<RealtimeWsVersion>,
    #[serde(rename = "type")]
    pub session_type: Option<RealtimeWsMode>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RealtimeAudioToml {
    pub microphone: Option<String>,
    pub speaker: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ToolsToml {
    #[serde(
        default,
        deserialize_with = "deserialize_optional_web_search_tool_config"
    )]
    pub web_search: Option<WebSearchToolConfig>,

    /// Enable the `view_image` tool that lets the agent attach local images.
    #[serde(default)]
    pub view_image: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WebSearchToolConfigInput {
    Enabled(bool),
    Config(WebSearchToolConfig),
}

fn deserialize_optional_web_search_tool_config<'de, D>(
    deserializer: D,
) -> Result<Option<WebSearchToolConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<WebSearchToolConfigInput>::deserialize(deserializer)?;

    Ok(match value {
        None => None,
        Some(WebSearchToolConfigInput::Enabled(enabled)) => {
            let _ = enabled;
            None
        }
        Some(WebSearchToolConfigInput::Config(config)) => Some(config),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AgentsToml {
    /// Maximum number of agent threads that can be open concurrently.
    /// When unset, no limit is enforced.
    #[schemars(range(min = 1))]
    pub max_threads: Option<usize>,
    /// Maximum nesting depth allowed for spawned agent threads.
    /// Root sessions start at depth 0.
    #[schemars(range(min = 1))]
    pub max_depth: Option<i32>,
    /// Default maximum runtime in seconds for agent job workers.
    #[schemars(range(min = 1))]
    pub job_max_runtime_seconds: Option<u64>,

    /// User-defined role declarations keyed by role name.
    ///
    /// Example:
    /// ```toml
    /// [agents.researcher]
    /// description = "Research-focused role."
    /// config_file = "./agents/researcher.toml"
    /// nickname_candidates = ["Herodotus", "Ibn Battuta"]
    /// ```
    #[serde(default, flatten)]
    pub roles: BTreeMap<String, AgentRoleToml>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentRoleConfig {
    /// Human-facing role documentation used in spawn tool guidance.
    /// Required for loaded user-defined roles after deprecated/new metadata precedence resolves.
    pub description: Option<String>,
    /// Path to a role-specific config layer.
    pub config_file: Option<PathBuf>,
    /// Candidate nicknames for agents spawned with this role.
    pub nickname_candidates: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct AgentRoleToml {
    /// Human-facing role documentation used in spawn tool guidance.
    /// Required unless supplied by the referenced agent role file.
    pub description: Option<String>,

    /// Path to a role-specific config layer.
    /// Relative paths are resolved relative to the `config.toml` that defines them.
    pub config_file: Option<AbsolutePathBuf>,

    /// Candidate nicknames for agents spawned with this role.
    pub nickname_candidates: Option<Vec<String>>,
}

impl From<ToolsToml> for Tools {
    fn from(tools_toml: ToolsToml) -> Self {
        Self {
            web_search: tools_toml.web_search.is_some().then_some(true),
            view_image: tools_toml.view_image,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ResearchToolsToml {
    pub zotero_api_key: Option<String>,
    pub zotero_user_id: Option<String>,
    pub openalex_email: Option<String>,
    pub zotero_library_type: Option<String>,
    pub zotero_group_id: Option<String>,
    pub zotero_base_url: Option<String>,
    pub zotero_storage_dir: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DataToolsToml {
    pub huggingface_token: Option<String>,
    pub kaggle_username: Option<String>,
    pub kaggle_key: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct KbToml {
    /// Path to the knowledge base directory. Defaults to
    /// `<codex_home>/workspaces/<active-workspace>/knowledge-base`.
    pub kb_path: Option<String>,
}

/// Agent coordination channel settings.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
pub struct CoordinationToml {
    /// URL for the relay server.
    pub relay_url: Option<String>,
    /// Shared secret for authenticating with the relay server.
    pub relay_secret: Option<String>,
    /// Redis connection URL for the relay server backend.
    pub redis_url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct GhostSnapshotToml {
    /// Exclude untracked files larger than this many bytes from ghost snapshots.
    #[serde(alias = "ignore_untracked_files_over_bytes")]
    pub ignore_large_untracked_files: Option<i64>,
    /// Ignore untracked directories that contain this many files or more.
    /// (Still emits a warning unless warnings are disabled.)
    #[serde(alias = "large_untracked_dir_warning_threshold")]
    pub ignore_large_untracked_dirs: Option<i64>,
    /// Disable all ghost snapshot warning events.
    pub disable_warnings: Option<bool>,
}

impl ConfigToml {
    /// Derive the effective sandbox policy from the configuration.
    fn derive_sandbox_policy(
        &self,
        sandbox_mode_override: Option<SandboxMode>,
        profile_sandbox_mode: Option<SandboxMode>,
        windows_sandbox_level: WindowsSandboxLevel,
        resolved_cwd: &Path,
        sandbox_policy_constraint: Option<&Constrained<SandboxPolicy>>,
    ) -> SandboxPolicy {
        let sandbox_mode_was_explicit = sandbox_mode_override.is_some()
            || profile_sandbox_mode.is_some()
            || self.sandbox_mode.is_some();
        let resolved_sandbox_mode = sandbox_mode_override
            .or(profile_sandbox_mode)
            .or(self.sandbox_mode)
            .or_else(|| {
                // If no sandbox_mode is set but this directory has a trust decision,
                // default to workspace-write except on unsandboxed Windows where we
                // default to read-only.
                self.get_active_project(resolved_cwd).and_then(|p| {
                    if p.is_trusted() || p.is_untrusted() {
                        if cfg!(target_os = "windows")
                            && windows_sandbox_level
                                == codex_protocol::config_types::WindowsSandboxLevel::Disabled
                        {
                            Some(SandboxMode::ReadOnly)
                        } else {
                            Some(SandboxMode::WorkspaceWrite)
                        }
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let mut sandbox_policy = match resolved_sandbox_mode {
            SandboxMode::ReadOnly => SandboxPolicy::new_read_only_policy(),
            SandboxMode::WorkspaceWrite => match self.sandbox_workspace_write.as_ref() {
                Some(SandboxWorkspaceWrite {
                    writable_roots,
                    additional_writable_roots,
                    network_access,
                    exclude_tmpdir_env_var,
                    exclude_slash_tmp,
                }) => {
                    let mut roots = writable_roots.clone();
                    // Expand ~ and resolve relative paths from
                    // additional_writable_roots in the config.
                    for path in additional_writable_roots {
                        match AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd) {
                            Ok(abs) => {
                                if !roots.iter().any(|existing| existing == &abs) {
                                    roots.push(abs);
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Ignoring invalid additional_writable_root {path:?}: {e}",
                                );
                            }
                        }
                    }
                    SandboxPolicy::WorkspaceWrite {
                        writable_roots: roots,
                        read_only_access: ReadOnlyAccess::FullAccess,
                        network_access: *network_access,
                        exclude_tmpdir_env_var: *exclude_tmpdir_env_var,
                        exclude_slash_tmp: *exclude_slash_tmp,
                    }
                }
                None => SandboxPolicy::new_workspace_write_policy(),
            },
            SandboxMode::DangerFullAccess => SandboxPolicy::DangerFullAccess,
        };
        let downgrade_workspace_write_if_unsupported = |policy: &mut SandboxPolicy| {
            if cfg!(target_os = "windows")
                // If the experimental Windows sandbox is enabled, do not force a downgrade.
                && windows_sandbox_level
                    == codex_protocol::config_types::WindowsSandboxLevel::Disabled
                && matches!(&*policy, SandboxPolicy::WorkspaceWrite { .. })
            {
                *policy = SandboxPolicy::new_read_only_policy();
            }
        };
        if !file_system_sandbox_policy
            .entries
            .iter()
            .any(|existing| existing == &deny_entry)
        {
            tracing::warn!(
                error = %err,
                "default sandbox policy is disallowed by requirements; falling back to required default"
            );
            sandbox_policy = constraint.get().clone();
            downgrade_workspace_write_if_unsupported(&mut sandbox_policy);
        }
        sandbox_policy
    }

    pub fn get_config_profile(
        &self,
        override_profile: Option<String>,
    ) -> Result<ConfigProfile, std::io::Error> {
        let profile = override_profile.or_else(|| self.profile.clone());

        match profile {
            Some(key) => {
                if let Some(profile) = self.profiles.get(key.as_str()) {
                    return Ok(profile.clone());
                }

                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("config profile `{key}` not found"),
                ))
            }
            None => Ok(ConfigProfile::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionConfigSyntax {
    Legacy,
    Profiles,
}

#[derive(Debug, Deserialize, Default)]
struct PermissionSelectionToml {
    default_permissions: Option<String>,
    sandbox_mode: Option<SandboxMode>,
}

fn resolve_permission_config_syntax(
    config_layer_stack: &ConfigLayerStack,
    cfg: &ConfigToml,
    sandbox_mode_override: Option<SandboxMode>,
    profile_sandbox_mode: Option<SandboxMode>,
) -> Option<PermissionConfigSyntax> {
    if sandbox_mode_override.is_some() || profile_sandbox_mode.is_some() {
        return Some(PermissionConfigSyntax::Legacy);
    }

    let mut selection = None;
    for layer in
        config_layer_stack.get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false)
    {
        let Ok(layer_selection) = layer.config.clone().try_into::<PermissionSelectionToml>() else {
            continue;
        };

        if layer_selection.sandbox_mode.is_some() {
            selection = Some(PermissionConfigSyntax::Legacy);
        }
        if layer_selection.default_permissions.is_some() {
            selection = Some(PermissionConfigSyntax::Profiles);
        }
    }

    selection.or_else(|| {
        if cfg.default_permissions.is_some() {
            Some(PermissionConfigSyntax::Profiles)
        } else if cfg.sandbox_mode.is_some() {
            Some(PermissionConfigSyntax::Legacy)
        } else {
            None
        }
    })
}

fn add_additional_file_system_writes(
    file_system_sandbox_policy: &mut FileSystemSandboxPolicy,
    additional_writable_roots: &[AbsolutePathBuf],
) {
    for path in additional_writable_roots {
        let exists = file_system_sandbox_policy.entries.iter().any(|entry| {
            matches!(
                &entry.path,
                codex_protocol::permissions::FileSystemPath::Path { path: existing }
                    if existing == path && entry.access == codex_protocol::permissions::FileSystemAccessMode::Write
            )
        });
        if !exists {
            file_system_sandbox_policy.entries.push(
                codex_protocol::permissions::FileSystemSandboxEntry {
                    path: codex_protocol::permissions::FileSystemPath::Path { path: path.clone() },
                    access: codex_protocol::permissions::FileSystemAccessMode::Write,
                },
            );
        }
    }
}

/// Optional overrides for user configuration (e.g., from CLI flags).
#[derive(Default, Debug, Clone)]
pub struct ConfigOverrides {
    pub model: Option<String>,
    pub review_model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub approval_policy: Option<AskForApproval>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub sandbox_mode: Option<SandboxMode>,
    pub permission_profile: Option<PermissionProfile>,
    pub default_permissions: Option<String>,
    pub model_provider: Option<String>,
    pub service_tier: Option<Option<String>>,
    pub config_profile: Option<String>,
    pub codex_self_exe: Option<PathBuf>,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub main_execve_wrapper_exe: Option<PathBuf>,
    pub zsh_path: Option<PathBuf>,
    pub base_instructions: Option<String>,
    pub developer_instructions: Option<String>,
    pub personality: Option<Personality>,
    pub compact_prompt: Option<String>,
    pub include_apply_patch_tool: Option<bool>,
    pub show_raw_agent_reasoning: Option<bool>,
    pub tools_web_search_request: Option<bool>,
    pub ephemeral: Option<bool>,
    /// Additional directories that should be treated as writable roots for this session.
    pub additional_writable_roots: Vec<PathBuf>,
}

fn validate_reserved_model_provider_ids(
    model_providers: &HashMap<String, ModelProviderInfo>,
) -> Result<(), String> {
    let mut conflicts = model_providers
        .keys()
        .filter(|key| RESERVED_MODEL_PROVIDER_IDS.contains(&key.as_str()))
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>();
    conflicts.sort_unstable();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "model_providers contains reserved built-in provider IDs: {}. \
Built-in providers cannot be overridden. Rename your custom provider (for example, `openai-custom`).",
            conflicts.join(", ")
        ))
    }
}

fn deserialize_model_providers<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModelProviderInfo>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let model_providers = HashMap::<String, ModelProviderInfo>::deserialize(deserializer)?;
    validate_reserved_model_provider_ids(&model_providers).map_err(serde::de::Error::custom)?;
    Ok(model_providers)
}

/// Resolves the OSS provider from CLI override, profile config, or global config.
/// Returns `None` if no provider is configured at any level.
pub fn resolve_oss_provider(
    explicit_provider: Option<&str>,
    config_toml: &ConfigToml,
    config_profile: Option<String>,
) -> Option<String> {
    if let Some(provider) = explicit_provider {
        // Explicit provider specified (e.g., via --local-provider)
        Some(provider.to_string())
    } else {
        // Check profile config first, then global config
        let profile = config_toml.get_config_profile(config_profile).ok();
        if let Some(profile) = &profile {
            // Check if profile has an oss provider
            if let Some(profile_oss_provider) = &profile.oss_provider {
                Some(profile_oss_provider.clone())
            }
            // If not then check if the toml has an oss provider
            else {
                config_toml.oss_provider.clone()
            }
        } else {
            config_toml.oss_provider.clone()
        }
    }
}

impl Config {
    #[cfg(test)]
    async fn load_from_base_config_with_overrides(
        cfg: ConfigToml,
        overrides: ConfigOverrides,
        codex_home: AbsolutePathBuf,
    ) -> std::io::Result<Self> {
        // Note this ignores requirements.toml enforcement for tests.
        let config_layer_stack = ConfigLayerStack::default();
        Self::load_config_with_layer_stack(
            LOCAL_FS.as_ref(),
            cfg,
            overrides,
            codex_home,
            config_layer_stack,
        )
        .await
    }

    pub(crate) async fn load_config_with_layer_stack(
        fs: &dyn ExecutorFileSystem,
        cfg: ConfigToml,
        overrides: ConfigOverrides,
        codex_home: AbsolutePathBuf,
        config_layer_stack: ConfigLayerStack,
    ) -> std::io::Result<Self> {
        validate_reserved_model_provider_ids(&cfg.model_providers)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
        // Ensure that every field of ConfigRequirements is applied to the final
        // Config.
        let ConfigRequirements {
            approval_policy: mut constrained_approval_policy,
            approvals_reviewer: mut constrained_approvals_reviewer,
            permission_profile: mut constrained_permission_profile,
            web_search_mode: mut constrained_web_search_mode,
            feature_requirements,
            managed_hooks: _,
            mcp_servers,
            plugins: _,
            exec_policy: _,
            enforce_residency,
            network: network_requirements,
            filesystem: filesystem_requirements,
            guardian_policy_config_source: _,
        } = config_layer_stack.requirements().clone();

        let user_instructions = AgentsMdManager::load_global_instructions(Some(&codex_home))
            .map(|loaded| loaded.contents);
        let mut startup_warnings = config_layer_stack
            .startup_warnings()
            .unwrap_or_default()
            .to_vec();

        // Destructure ConfigOverrides fully to ensure all overrides are applied.
        let ConfigOverrides {
            model,
            review_model: override_review_model,
            cwd,
            approval_policy: approval_policy_override,
            approvals_reviewer: approvals_reviewer_override,
            sandbox_mode,
            permission_profile,
            default_permissions: default_permissions_override,
            model_provider,
            service_tier: service_tier_override,
            config_profile: config_profile_key,
            codex_self_exe,
            codex_linux_sandbox_exe,
            main_execve_wrapper_exe,
            zsh_path: zsh_path_override,
            base_instructions,
            developer_instructions,
            personality,
            compact_prompt,
            include_apply_patch_tool: include_apply_patch_tool_override,
            show_raw_agent_reasoning,
            tools_web_search_request: override_tools_web_search_request,
            ephemeral,
            additional_writable_roots,
        } = overrides;

        if sandbox_mode.is_some() && permission_profile.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`sandbox_mode` and `permission_profile` overrides cannot both be set",
            ));
        }
        if sandbox_mode.is_some() && default_permissions_override.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`sandbox_mode` and `default_permissions` overrides cannot both be set",
            ));
        }
        if permission_profile.is_some() && default_permissions_override.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "`permission_profile` and `default_permissions` overrides cannot both be set",
            ));
        }

        let active_profile_name = config_profile_key
            .as_ref()
            .or(cfg.profile.as_ref())
            .cloned();
        let config_profile = match active_profile_name.as_ref() {
            Some(key) => cfg
                .profiles
                .get(key)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("config profile `{key}` not found"),
                    )
                })?
                .clone(),
            None => ConfigProfile::default(),
        };
        let feature_overrides = FeatureOverrides {
            include_apply_patch_tool: include_apply_patch_tool_override,
            web_search_request: override_tools_web_search_request,
        };

        let configured_features = Features::from_sources(
            FeatureConfigSource {
                features: cfg.features.as_ref(),
                include_apply_patch_tool: None,
                experimental_use_freeform_apply_patch: cfg.experimental_use_freeform_apply_patch,
                experimental_use_unified_exec_tool: cfg.experimental_use_unified_exec_tool,
            },
            FeatureConfigSource {
                features: config_profile.features.as_ref(),
                include_apply_patch_tool: config_profile.include_apply_patch_tool,
                experimental_use_freeform_apply_patch: config_profile
                    .experimental_use_freeform_apply_patch,
                experimental_use_unified_exec_tool: config_profile
                    .experimental_use_unified_exec_tool,
            },
            feature_overrides,
        );
        let features = ManagedFeatures::from_configured_with_warnings(
            configured_features,
            feature_requirements,
            &mut startup_warnings,
        )?;
        let windows_sandbox_mode = resolve_windows_sandbox_mode(&cfg, &config_profile);
        let windows_sandbox_private_desktop =
            resolve_windows_sandbox_private_desktop(&cfg, &config_profile);
        let resolved_cwd = normalize_for_native_workdir({
            use std::env;

            match cwd {
                None => {
                    tracing::info!("cwd not set, using current dir");
                    env::current_dir()?
                }
                Some(p) if p.is_absolute() => p,
                Some(p) => {
                    // Resolve relative path against the current working directory.
                    tracing::info!("cwd is relative, resolving against current dir");
                    let mut current = env::current_dir()?;
                    current.push(p);
                    current
                }
            }
        });
        let mut additional_writable_roots: Vec<AbsolutePathBuf> = additional_writable_roots
            .into_iter()
            .map(|path| AbsolutePathBuf::resolve_path_against_base(path, resolved_cwd.as_path()))
            .collect();
        let requested_additional_writable_roots = additional_writable_roots.clone();
        let repo_root = resolve_root_git_project_for_trust(fs, &resolved_cwd).await;
        let active_project = cfg
            .get_active_project(
                resolved_cwd.as_path(),
                repo_root.as_ref().map(AbsolutePathBuf::as_path),
            )
            .unwrap_or(ProjectConfig { trust_level: None });
        let permission_config_syntax = resolve_permission_config_syntax(
            &config_layer_stack,
            &cfg,
            sandbox_mode,
            config_profile.sandbox_mode,
        );
        let has_permission_profiles = cfg
            .permissions
            .as_ref()
            .is_some_and(|profiles| !profiles.is_empty());
        if has_permission_profiles
            && !matches!(
                permission_config_syntax,
                Some(PermissionConfigSyntax::Legacy)
            )
            && cfg.default_permissions.is_none()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "config defines `[permissions]` profiles but does not set `default_permissions`",
            ));
        }

        let windows_sandbox_level = match windows_sandbox_mode {
            Some(WindowsSandboxModeToml::Elevated) => WindowsSandboxLevel::Elevated,
            Some(WindowsSandboxModeToml::Unelevated) => WindowsSandboxLevel::RestrictedToken,
            None => WindowsSandboxLevel::from_features(&features),
        };
        let memories_root = memory_root(&codex_home);
        std::fs::create_dir_all(&memories_root)?;
        let memories_root = AbsolutePathBuf::from_absolute_path(&memories_root)?;
        if !additional_writable_roots
            .iter()
            .any(|existing| existing == &memories_root)
        {
            additional_writable_roots.push(memories_root);
        }

        let profiles_are_active = matches!(
            permission_config_syntax,
            Some(PermissionConfigSyntax::Profiles)
        ) || (permission_config_syntax.is_none()
            && has_permission_profiles);
        let (
            configured_network_proxy_config,
            sandbox_policy,
            file_system_sandbox_policy,
            network_sandbox_policy,
        ) = if profiles_are_active {
            let permissions = cfg.permissions.as_ref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "default_permissions requires a `[permissions]` table",
                )
            })?;
            let default_permissions = cfg.default_permissions.as_deref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "default_permissions requires a named permissions profile",
                )
            })?;
            let profile = resolve_permission_profile(permissions, default_permissions)?;
            let configured_network_proxy_config =
                network_proxy_config_from_profile_network(profile.network.as_ref());
            let (mut file_system_sandbox_policy, network_sandbox_policy) =
                compile_permission_profile(
                    permissions,
                    default_permissions,
                    &mut startup_warnings,
                )?;
            let mut sandbox_policy = file_system_sandbox_policy
                .to_legacy_sandbox_policy(network_sandbox_policy, &resolved_cwd)?;
            if matches!(sandbox_policy, SandboxPolicy::WorkspaceWrite { .. }) {
                add_additional_file_system_writes(
                    &mut file_system_sandbox_policy,
                    &additional_writable_roots,
                );
                sandbox_policy = file_system_sandbox_policy
                    .to_legacy_sandbox_policy(network_sandbox_policy, &resolved_cwd)?;
            }
            (
                configured_network_proxy_config,
                sandbox_policy,
                file_system_sandbox_policy,
                network_sandbox_policy,
            )
        } else {
            let configured_network_proxy_config = NetworkProxyConfig::default();
            let mut sandbox_policy = cfg.derive_sandbox_policy(
                sandbox_mode,
                config_profile.sandbox_mode,
                windows_sandbox_level,
                &resolved_cwd,
                Some(&constrained_sandbox_policy),
            );
            if let SandboxPolicy::WorkspaceWrite { writable_roots, .. } = &mut sandbox_policy {
                for path in &additional_writable_roots {
                    if !writable_roots.iter().any(|existing| existing == path) {
                        writable_roots.push(path.clone());
                    }
                }
            }
            let file_system_sandbox_policy =
                FileSystemSandboxPolicy::from_legacy_sandbox_policy(&sandbox_policy, &resolved_cwd);
            let network_sandbox_policy = NetworkSandboxPolicy::from(&sandbox_policy);
            (
                configured_network_proxy_config,
                sandbox_policy,
                file_system_sandbox_policy,
                network_sandbox_policy,
            )
        };
        let approval_policy_was_explicit = approval_policy_override.is_some()
            || config_profile.approval_policy.is_some()
            || cfg.approval_policy.is_some();
        let mut approval_policy = approval_policy_override
            .or(config_profile.approval_policy)
            .or(cfg.approval_policy)
            .unwrap_or_else(|| {
                if active_project.is_trusted() {
                    AskForApproval::OnRequest
                } else if active_project.is_untrusted() {
                    AskForApproval::UnlessTrusted
                } else {
                    AskForApproval::default()
                }
            });
        if !approval_policy_was_explicit
            && let Err(err) = constrained_approval_policy.can_set(&approval_policy)
        {
            tracing::warn!(
                error = %err,
                "default approval policy is disallowed by requirements; falling back to required default"
            );
            approval_policy = constrained_approval_policy.value();
        }
        let approvals_reviewer = approvals_reviewer_override
            .or(config_profile.approvals_reviewer)
            .or(cfg.approvals_reviewer)
            .unwrap_or(ApprovalsReviewer::User);
        let web_search_mode = resolve_web_search_mode(&cfg, &config_profile, &features)
            .unwrap_or(WebSearchMode::Cached);
        let web_search_config = resolve_web_search_config(&cfg, &config_profile);

        let agent_roles =
            agent_roles::load_agent_roles(&cfg, &config_layer_stack, &mut startup_warnings)?;

        let openai_base_url = cfg
            .openai_base_url
            .clone()
            .filter(|value| !value.is_empty());
        let openai_base_url_from_env = std::env::var(OPENAI_BASE_URL_ENV_VAR)
            .ok()
            .filter(|value| !value.is_empty());
        if openai_base_url_from_env.is_some() {
            if openai_base_url.is_some() {
                tracing::warn!(
                    env_var = OPENAI_BASE_URL_ENV_VAR,
                    "deprecated env var is ignored because `openai_base_url` is set in config.toml"
                );
            } else {
                startup_warnings.push(format!(
                    "`{OPENAI_BASE_URL_ENV_VAR}` is deprecated. Set `openai_base_url` in config.toml instead."
                ));
            }
        }
        let effective_openai_base_url = openai_base_url.or(openai_base_url_from_env);

        let mut model_providers = built_in_model_providers(effective_openai_base_url);
        // Merge user-defined providers into the built-in list.
        for (key, provider) in cfg.model_providers.into_iter() {
            model_providers.entry(key).or_insert(provider);
        }

        let mut model_provider_id = model_provider
            .or(config_profile.model_provider)
            .or(cfg.model_provider)
            .unwrap_or_else(|| "openai".to_string());
        let model_provider = match model_providers.get(&model_provider_id) {
            Some(provider) => provider.clone(),
            None => {
                let warning = if model_provider_id == LEGACY_OLLAMA_CHAT_PROVIDER_ID {
                    OLLAMA_CHAT_PROVIDER_REMOVED_ERROR.to_string()
                } else {
                    format!("Model provider `{model_provider_id}` not found")
                };
                tracing::warn!("{warning}; falling back to openai");
                startup_warnings.push(format!(
                    "{MODEL_PROVIDER_FALLBACK_PREFIX} {warning}. Using the default openai provider."
                ));
                model_provider_id = "openai".to_string();
                model_providers
                    .get("openai")
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "built-in openai provider is missing",
                        )
                    })?
                    .clone()
            }
        };

        let shell_environment_policy = cfg.shell_environment_policy.into();
        let allow_login_shell = cfg.allow_login_shell.unwrap_or(true);

        let history = cfg.history.unwrap_or_default();

        if multi_agent_v2.max_concurrent_threads_per_session == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.max_concurrent_threads_per_session must be at least 1",
            ));
        }
        if multi_agent_v2.min_wait_timeout_ms <= 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "features.multi_agent_v2.min_wait_timeout_ms must be at least 1",
            ));
        }
        if multi_agent_v2.min_wait_timeout_ms > MAX_MULTI_AGENT_V2_WAIT_TIMEOUT_MS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "features.multi_agent_v2.min_wait_timeout_ms must be at most {MAX_MULTI_AGENT_V2_WAIT_TIMEOUT_MS}"
                ),
            ));
        }
        let agent_max_threads_from_config = cfg.agents.as_ref().and_then(|agents| agents.max_threads);
        let agent_max_threads = if features.enabled(Feature::MultiAgentV2) {
            if agent_max_threads_from_config.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "agents.max_threads cannot be set when multi_agent_v2 is enabled",
                ));
            }
            Some(
                multi_agent_v2
                    .max_concurrent_threads_per_session
                    .saturating_sub(1),
            )
        } else {
            let agent_max_threads = agent_max_threads_from_config.or(DEFAULT_AGENT_MAX_THREADS);
            if agent_max_threads == Some(0) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "agents.max_threads must be at least 1",
                ));
            }
            agent_max_threads
        };
        let agent_max_depth = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.max_depth)
            .unwrap_or(DEFAULT_AGENT_MAX_DEPTH);
        if agent_max_depth < 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.max_depth must be at least 1",
            ));
        }
        let agent_job_max_runtime_seconds = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.job_max_runtime_seconds)
            .or(DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS);
        if agent_job_max_runtime_seconds == Some(0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.job_max_runtime_seconds must be at least 1",
            ));
        }
        if let Some(max_runtime_seconds) = agent_job_max_runtime_seconds
            && max_runtime_seconds > i64::MAX as u64
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "agents.job_max_runtime_seconds must fit within a 64-bit signed integer",
            ));
        }
        let agent_interrupt_message_enabled = cfg
            .agents
            .as_ref()
            .and_then(|agents| agents.interrupt_message)
            .unwrap_or(true);
        let background_terminal_max_timeout = cfg
            .background_terminal_max_timeout
            .unwrap_or(DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
            .max(MIN_EMPTY_YIELD_TIME_MS);

        let ghost_snapshot = {
            let mut config = GhostSnapshotConfig::default();
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(ignore_over_bytes) = ghost_snapshot.ignore_large_untracked_files
            {
                config.ignore_large_untracked_files = if ignore_over_bytes > 0 {
                    Some(ignore_over_bytes)
                } else {
                    None
                };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(threshold) = ghost_snapshot.ignore_large_untracked_dirs
            {
                config.ignore_large_untracked_dirs =
                    if threshold > 0 { Some(threshold) } else { None };
            }
            if let Some(ghost_snapshot) = cfg.ghost_snapshot.as_ref()
                && let Some(disable_warnings) = ghost_snapshot.disable_warnings
            {
                config.disable_warnings = disable_warnings;
            }
            config
        };

        let include_apply_patch_tool_flag = features.enabled(Feature::ApplyPatchFreeform);
        let use_experimental_unified_exec_tool = features.enabled(Feature::UnifiedExec);

        let forced_chatgpt_workspace_id =
            cfg.forced_chatgpt_workspace_id.as_ref().and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            });

        let forced_login_method = cfg.forced_login_method;

        let model = model.or(config_profile.model).or(cfg.model);
        let mut notices = cfg.notice.unwrap_or_default();
        let service_tier = match service_tier_override {
            Some(Some(service_tier)) => Some(service_tier),
            Some(None) => {
                // Preserve explicit standard/clear intent after the nested override
                // collapses into `Config.service_tier = None`.
                notices.fast_default_opt_out = Some(true);
                None
            }
            None => config_profile
                .service_tier
                .or(cfg.service_tier)
                .map(|service_tier| service_tier.request_value().to_string()),
        };
        let service_tier = service_tier.and_then(|service_tier| {
            match ServiceTier::from_request_value(&service_tier) {
                Some(ServiceTier::Fast) => features
                    .enabled(Feature::FastMode)
                    .then(|| ServiceTier::Fast.request_value().to_string()),
                Some(ServiceTier::Flex) => Some(ServiceTier::Flex.request_value().to_string()),
                None => Some(service_tier),
            }
        });

        let compact_prompt = compact_prompt.or(cfg.compact_prompt).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let commit_attribution = cfg.commit_attribution;

        // Load base instructions override from a file if specified. If the
        // path is relative, resolve it against the effective cwd so the
        // behaviour matches other path-like config values.
        let model_instructions_path = config_profile
            .model_instructions_file
            .as_ref()
            .or(cfg.model_instructions_file.as_ref());
        let file_base_instructions = Self::try_read_non_empty_file(
            fs,
            model_instructions_path,
            "model instructions file",
        )
        .await?;
        let base_instructions = base_instructions
            .or(file_base_instructions)
            .or(cfg.instructions.clone());
        let developer_instructions = developer_instructions.or(cfg.developer_instructions);
        let include_permissions_instructions = config_profile
            .include_permissions_instructions
            .or(cfg.include_permissions_instructions)
            .unwrap_or(true);
        let include_apps_instructions = config_profile
            .include_apps_instructions
            .or(cfg.include_apps_instructions)
            .unwrap_or(true);
        let include_skill_instructions = cfg
            .skills
            .as_ref()
            .and_then(|skills| skills.include_instructions)
            .unwrap_or(true);
        let include_environment_context = config_profile
            .include_environment_context
            .or(cfg.include_environment_context)
            .unwrap_or(true);
        let guardian_policy_config =
            guardian_policy_config_from_requirements(config_layer_stack.requirements_toml())
                .or_else(|| {
                    cfg.auto_review
                        .as_ref()
                        .and_then(|auto_review| normalize_guardian_policy_config(
                            auto_review.policy.as_deref(),
                        ))
                });
        let personality = personality
            .or(config_profile.personality)
            .or(cfg.personality)
            .or_else(|| {
                features
                    .enabled(Feature::Personality)
                    .then_some(Personality::Pragmatic)
            });

        let experimental_compact_prompt_path = config_profile
            .experimental_compact_prompt_file
            .as_ref()
            .or(cfg.experimental_compact_prompt_file.as_ref());
        let file_compact_prompt = Self::try_read_non_empty_file(
            fs,
            experimental_compact_prompt_path,
            "experimental compact prompt file",
        )
        .await?;
        let compact_prompt = compact_prompt.or(file_compact_prompt);
        let zsh_path = zsh_path_override
            .or(config_profile.zsh_path.map(Into::into))
            .or(cfg.zsh_path.map(Into::into));

        let review_model = override_review_model.or(cfg.review_model);

        let check_for_update_on_startup = cfg.check_for_update_on_startup.unwrap_or(true);
        let model_catalog = load_model_catalog(
            config_profile
                .model_catalog_json
                .clone()
                .or(cfg.model_catalog_json.clone()),
        )?;

        let log_dir = cfg
            .log_dir
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .unwrap_or_else(|| codex_home.join("log").to_path_buf());
        let sqlite_home = cfg
            .sqlite_home
            .as_ref()
            .map(AbsolutePathBuf::to_path_buf)
            .or_else(|| resolve_sqlite_home_env(&resolved_cwd))
            .unwrap_or_else(|| codex_home.to_path_buf());
        let original_sandbox_policy = sandbox_policy.clone();

        apply_requirement_constrained_value(
            "approval_policy",
            approval_policy,
            &mut constrained_approval_policy,
            &mut startup_warnings,
        )?;
        if let Some(Sourced {
            value: filesystem_requirements,
            source: filesystem_requirements_source,
        }) = filesystem_requirements.as_ref()
            && !filesystem_requirements.deny_read.is_empty()
        {
            let requirement_source = filesystem_requirements_source.clone();
            constrained_permission_profile
                .value
                .add_validator(move |permission_profile| {
                    let mode = sandbox_mode_requirement_for_permission_profile(permission_profile);
                    match mode {
                        SandboxModeRequirement::ReadOnly
                        | SandboxModeRequirement::WorkspaceWrite => Ok(()),
                        SandboxModeRequirement::DangerFullAccess
                        | SandboxModeRequirement::ExternalSandbox => {
                            Err(ConstraintError::InvalidValue {
                                field_name: "sandbox_mode",
                                candidate: format!("{mode:?}"),
                                allowed: "[read-only, workspace-write]".to_string(),
                                requirement_source: requirement_source.clone(),
                            })
                        }
                    }
                })
                .map_err(std::io::Error::from)?;

            if cfg!(target_os = "windows") {
                startup_warnings.push(format!(
                    "managed filesystem deny_read from {filesystem_requirements_source} is only enforced for direct file tools on Windows; shell subprocess reads are not sandboxed"
                ));
            }
        }
        apply_requirement_constrained_value(
            "approvals_reviewer",
            approvals_reviewer,
            &mut constrained_approvals_reviewer,
            &mut startup_warnings,
        )?;
        let permission_profile_was_constrained = apply_requirement_constrained_value(
            "permission_profile",
            permission_profile,
            &mut constrained_permission_profile,
            &mut startup_warnings,
        )?;
        if permission_profile_was_constrained {
            // The selected profile no longer describes the effective
            // permissions after requirements forced a fallback.
            active_permission_profile = None;
        }
        apply_requirement_constrained_value(
            "web_search_mode",
            web_search_mode,
            &mut constrained_web_search_mode,
            &mut startup_warnings,
        )?;

        let mcp_servers = constrain_mcp_servers(cfg.mcp_servers.clone(), mcp_servers.as_ref())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;

        let (network_requirements, network_requirements_source) = match network_requirements {
            Some(Sourced { value, source }) => (Some(value), Some(source)),
            None => (None, None),
        };
        let has_network_requirements = network_requirements.is_some();
        let network_permission_profile = constrained_permission_profile.get().clone();
        let network = NetworkProxySpec::from_config_and_constraints(
            configured_network_proxy_config,
            network_requirements,
            constrained_sandbox_policy.get(),
        )
        .map_err(|err| {
            if let Some(source) = network_requirements_source.as_ref() {
                std::io::Error::new(
                    err.kind(),
                    format!("failed to build managed network proxy from {source}: {err}"),
                )
            } else {
                err
            }
        })?;
        let network = if has_network_requirements {
            Some(network)
        } else {
            network.enabled().then_some(network)
        };
        let effective_sandbox_policy = constrained_sandbox_policy.value.get().clone();
        let effective_file_system_sandbox_policy =
            if effective_sandbox_policy == original_sandbox_policy {
                file_system_sandbox_policy
            } else {
                FileSystemSandboxPolicy::from_legacy_sandbox_policy(
                    &effective_sandbox_policy,
                    &resolved_cwd,
                )
            };
        let effective_network_sandbox_policy =
            if effective_sandbox_policy == original_sandbox_policy {
                network_sandbox_policy
            } else {
                NetworkSandboxPolicy::from(&effective_sandbox_policy)
            };

        let config = Self {
            model,
            service_tier,
            review_model,
            model_context_window: cfg.model_context_window,
            model_auto_compact_token_limit: cfg.model_auto_compact_token_limit,
            model_provider_id,
            model_provider,
            cwd: resolved_cwd,
            startup_warnings,
            permissions: Permissions {
                approval_policy: constrained_approval_policy.value,
                sandbox_policy: constrained_sandbox_policy.value,
                file_system_sandbox_policy: effective_file_system_sandbox_policy,
                network_sandbox_policy: effective_network_sandbox_policy,
                network,
                allow_login_shell,
                shell_environment_policy,
                windows_sandbox_mode,
                windows_sandbox_private_desktop,
                macos_seatbelt_profile_extensions: None,
            },
            approvals_reviewer,
            enforce_residency: enforce_residency.value,
            notify: cfg.notify,
            user_instructions,
            base_instructions,
            personality,
            developer_instructions,
            compact_prompt,
            commit_attribution,
            include_permissions_instructions,
            include_apps_instructions,
            include_skill_instructions,
            include_environment_context,
            // The config.toml omits "_mode" because it's a config file. However, "_mode"
            // is important in code to differentiate the mode from the store implementation.
            cli_auth_credentials_store_mode: resolve_cli_auth_credentials_store_mode(
                cfg.cli_auth_credentials_store.unwrap_or_default(),
                env!("CARGO_PKG_VERSION"),
            ),
            mcp_servers,
            // The config.toml omits "_mode" because it's a config file. However, "_mode"
            // is important in code to differentiate the mode from the store implementation.
            mcp_oauth_credentials_store_mode: resolve_mcp_oauth_credentials_store_mode(
                cfg.mcp_oauth_credentials_store.unwrap_or_default(),
                env!("CARGO_PKG_VERSION"),
            ),
            mcp_oauth_callback_port: cfg.mcp_oauth_callback_port,
            mcp_oauth_callback_url: cfg.mcp_oauth_callback_url.clone(),
            model_providers,
            project_doc_max_bytes: cfg.project_doc_max_bytes.unwrap_or(AGENTS_MD_MAX_BYTES),
            project_doc_fallback_filenames: cfg
                .project_doc_fallback_filenames
                .unwrap_or_default()
                .into_iter()
                .filter_map(|name| {
                    let trimmed = name.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect(),
            tool_output_token_limit: cfg.tool_output_token_limit,
            agent_max_threads,
            agent_max_depth,
            agent_roles,
            memories: cfg.memories.unwrap_or_default().into(),
            agent_job_max_runtime_seconds,
            agent_interrupt_message_enabled,
            codex_home,
            sqlite_home,
            log_dir,
            config_lock_export_dir: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.export_dir.clone()),
            config_lock_allow_codex_version_mismatch: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.allow_codex_version_mismatch)
                .unwrap_or(false),
            config_lock_save_fields_resolved_from_model_catalog: cfg
                .debug
                .as_ref()
                .and_then(|debug| debug.config_lockfile.as_ref())
                .and_then(|config_lock| config_lock.save_fields_resolved_from_model_catalog)
                .unwrap_or(true),
            config_lock_toml: None,
            config_layer_stack,
            history,
            ephemeral: ephemeral.unwrap_or_default(),
            file_opener: cfg.file_opener.unwrap_or(UriBasedFileOpener::VsCode),
            codex_self_exe,
            codex_linux_sandbox_exe,
            main_execve_wrapper_exe,
            zsh_path,

            hide_agent_reasoning: cfg.hide_agent_reasoning.unwrap_or(false),
            show_raw_agent_reasoning: cfg
                .show_raw_agent_reasoning
                .or(show_raw_agent_reasoning)
                .unwrap_or(false),
            guardian_policy_config,
            model_reasoning_effort: config_profile
                .model_reasoning_effort
                .or(cfg.model_reasoning_effort),
            plan_mode_reasoning_effort: config_profile
                .plan_mode_reasoning_effort
                .or(cfg.plan_mode_reasoning_effort),
            model_reasoning_summary: config_profile
                .model_reasoning_summary
                .or(cfg.model_reasoning_summary),
            model_supports_reasoning_summaries: cfg.model_supports_reasoning_summaries,
            model_catalog,
            model_verbosity: config_profile.model_verbosity.or(cfg.model_verbosity),
            chatgpt_base_url: config_profile
                .chatgpt_base_url
                .or(cfg.chatgpt_base_url)
                .unwrap_or("https://chatgpt.com/backend-api/".to_string()),
            apps_mcp_path_override,
            realtime_audio: cfg
                .audio
                .map_or_else(RealtimeAudioConfig::default, |audio| RealtimeAudioConfig {
                    microphone: audio.microphone,
                    speaker: audio.speaker,
                }),
            experimental_realtime_ws_base_url: cfg.experimental_realtime_ws_base_url,
            experimental_realtime_ws_model: cfg.experimental_realtime_ws_model,
            realtime: cfg
                .realtime
                .map_or_else(RealtimeConfig::default, |realtime| RealtimeConfig {
                    version: realtime.version.unwrap_or_default(),
                    session_type: realtime.session_type.unwrap_or_default(),
                }),
            experimental_realtime_ws_backend_prompt: cfg.experimental_realtime_ws_backend_prompt,
            experimental_realtime_ws_startup_context: cfg.experimental_realtime_ws_startup_context,
            experimental_realtime_start_instructions: cfg.experimental_realtime_start_instructions,
            forced_chatgpt_workspace_id,
            forced_login_method,
            include_apply_patch_tool: include_apply_patch_tool_flag,
            web_search_mode: constrained_web_search_mode.value,
            web_search_config,
            use_experimental_unified_exec_tool,
            background_terminal_max_timeout,
            ghost_snapshot,
            multi_agent_v2,
            features,
            research: cfg.research,
            data: cfg.data,
            kb: cfg.kb,
            coordination: cfg.coordination,
            suppress_unstable_features_warning: cfg
                .suppress_unstable_features_warning
                .unwrap_or(false),
            active_profile: active_profile_name,
            active_project,
            windows_wsl_setup_acknowledged: cfg.windows_wsl_setup_acknowledged.unwrap_or(false),
            notices,
            check_for_update_on_startup,
            disable_paste_burst: cfg.disable_paste_burst.unwrap_or(false),
            analytics_enabled: config_profile
                .analytics
                .as_ref()
                .and_then(|a| a.enabled)
                .or(cfg.analytics.as_ref().and_then(|a| a.enabled))
                .or(Some(false)),
            feedback_enabled: cfg
                .feedback
                .as_ref()
                .and_then(|feedback| feedback.enabled)
                .unwrap_or(true),
            tool_suggest,
            tui_notifications: cfg
                .tui
                .as_ref()
                .map(|t| t.notification_settings.clone())
                .unwrap_or_default(),
            animations: cfg.tui.as_ref().map(|t| t.animations).unwrap_or(true),
            show_tooltips: cfg.tui.as_ref().map(|t| t.show_tooltips).unwrap_or(true),
            model_availability_nux: cfg
                .tui
                .as_ref()
                .map(|t| t.model_availability_nux.clone())
                .unwrap_or_default(),
            tui_vim_mode_default: cfg
                .tui
                .as_ref()
                .map(|t| t.vim_mode_default)
                .unwrap_or(false),
            tui_raw_output_mode: cfg
                .tui
                .as_ref()
                .map(|t| t.raw_output_mode)
                .unwrap_or(false),
            tui_alternate_screen: cfg
                .tui
                .as_ref()
                .map(|t| t.alternate_screen)
                .unwrap_or_default(),
            tui_status_line: cfg.tui.as_ref().and_then(|t| t.status_line.clone()),
            tui_status_line_use_colors: cfg
                .tui
                .as_ref()
                .map(|t| t.status_line_use_colors)
                .unwrap_or(true),
            tui_terminal_title: cfg.tui.as_ref().and_then(|t| t.terminal_title.clone()),
            tui_theme: cfg.tui.as_ref().and_then(|t| t.theme.clone()),
            tui_session_picker_view: config_profile
                .tui
                .as_ref()
                .and_then(|t| t.session_picker_view)
                .or_else(|| cfg.tui.as_ref().and_then(|t| t.session_picker_view))
                .unwrap_or_default(),
            terminal_resize_reflow,
            tui_keymap: cfg
                .tui
                .as_ref()
                .map(|t| t.keymap.clone())
                .unwrap_or_default(),
            otel: {
                let t: OtelConfigToml = cfg.otel.unwrap_or_default();
                let log_user_prompt = t.log_user_prompt.unwrap_or(false);
                let environment = t
                    .environment
                    .unwrap_or(DEFAULT_OTEL_ENVIRONMENT.to_string());
                let exporter = t.exporter.unwrap_or(OtelExporterKind::None);
                // OTLP HTTP endpoints are signal-specific in our config, so
                // enabling log export must not implicitly send spans to a
                // /v1/logs endpoint.
                let trace_exporter = t.trace_exporter.unwrap_or(OtelExporterKind::None);
                let metrics_exporter = t.metrics_exporter.unwrap_or(OtelExporterKind::Statsig);
                OtelConfig {
                    log_user_prompt,
                    environment,
                    exporter,
                    trace_exporter,
                    metrics_exporter,
                }
            },
        };
        Ok(config)
        })
        .await
    }

    /// If `path` is `Some`, attempts to read the file at the given path and
    /// returns its contents as a trimmed `String`. If the file is empty, or
    /// is `Some` but cannot be read, returns an `Err`.
    async fn try_read_non_empty_file(
        fs: &dyn ExecutorFileSystem,
        path: Option<&AbsolutePathBuf>,
        context: &str,
    ) -> std::io::Result<Option<String>> {
        let Some(path) = path else {
            return Ok(None);
        };

        let contents = fs
            .read_file_text(path, /*sandbox*/ None)
            .await
            .map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("failed to read {context} {}: {e}", path.display()),
                )
            })?;

        let s = contents.trim().to_string();
        if s.is_empty() {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{context} is empty: {}", path.display()),
            ))
        } else {
            Ok(Some(s))
        }
    }

    pub fn set_windows_sandbox_enabled(&mut self, value: bool) {
        self.permissions.windows_sandbox_mode = if value {
            Some(WindowsSandboxModeToml::Unelevated)
        } else if matches!(
            self.permissions.windows_sandbox_mode,
            Some(WindowsSandboxModeToml::Unelevated)
        ) {
            None
        } else {
            self.permissions.windows_sandbox_mode
        };
    }

    pub fn set_windows_elevated_sandbox_enabled(&mut self, value: bool) {
        self.permissions.windows_sandbox_mode = if value {
            Some(WindowsSandboxModeToml::Elevated)
        } else if matches!(
            self.permissions.windows_sandbox_mode,
            Some(WindowsSandboxModeToml::Elevated)
        ) {
            None
        } else {
            self.permissions.windows_sandbox_mode
        };
    }

    pub fn managed_network_requirements_enabled(&self) -> bool {
        !matches!(
            self.permissions.permission_profile.get(),
            PermissionProfile::Disabled
        ) && self
            .config_layer_stack
            .requirements_toml()
            .network
            .is_some()
    }

    pub fn bundled_skills_enabled(&self) -> bool {
        crate::skills::manager::bundled_skills_enabled_from_stack(&self.config_layer_stack)
    }
}

pub(crate) fn uses_deprecated_instructions_file(config_layer_stack: &ConfigLayerStack) -> bool {
    config_layer_stack
        .layers_high_to_low()
        .into_iter()
        .any(|layer| toml_uses_deprecated_instructions_file(&layer.config))
}

fn guardian_policy_config_from_requirements(
    requirements_toml: &ConfigRequirementsToml,
) -> Option<String> {
    normalize_guardian_policy_config(requirements_toml.guardian_policy_config.as_deref())
}

fn normalize_guardian_policy_config(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn toml_uses_deprecated_instructions_file(value: &TomlValue) -> bool {
    let Some(table) = value.as_table() else {
        return false;
    };
    if table.contains_key("experimental_instructions_file") {
        return true;
    }
    let Some(profiles) = table.get("profiles").and_then(TomlValue::as_table) else {
        return false;
    };
    profiles.values().any(|profile| {
        profile.as_table().is_some_and(|profile_table| {
            profile_table.contains_key("experimental_instructions_file")
        })
    })
}

/// Returns the path to the Codex configuration directory, which can be
/// specified by the `CODEX_HOME` environment variable. If not set, defaults to
/// `~/.codex`.
///
/// - If `CODEX_HOME` is set, the value must exist and be a directory. The
///   value will be canonicalized and this function will Err otherwise.
/// - If `CODEX_HOME` is not set, this function does not verify that the
///   directory exists.
pub fn find_codex_home() -> std::io::Result<AbsolutePathBuf> {
    codex_utils_home_dir::find_codex_home()
}

/// Returns the path to the folder where Codex logs are stored. Does not verify
/// that the directory exists.
pub fn log_dir(cfg: &Config) -> std::io::Result<PathBuf> {
    Ok(cfg.log_dir.clone())
}

#[cfg(test)]
mod tests;
