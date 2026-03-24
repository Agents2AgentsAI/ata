//! Dump the fully assembled initial context that the agent receives on
//! startup.  This is intended for debugging and understanding agent behavior
//! without starting a full session.

use crate::commit_attribution::commit_message_trailer_instruction;
use crate::config::Config;
use crate::environment_context::EnvironmentContext;
use crate::features::Feature;
use crate::instructions::UserInstructions;
use crate::models_manager::collaboration_mode_presets::CollaborationModesConfig;
use crate::models_manager::collaboration_mode_presets::builtin_collaboration_mode_presets;
use crate::models_manager::model_info::model_info_from_slug;
use crate::models_manager::model_info::with_config_overrides;
use crate::project_doc::get_user_instructions;
use crate::shell::default_user_shell;
use codex_execpolicy::Policy;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::models::DeveloperInstructions;

/// The assembled initial context broken into its constituent parts.
pub struct DumpResult {
    /// The base instructions (the `instructions` field sent to the API).
    pub base_instructions: String,
    /// The developer message assembled from policy, collaboration mode, etc.
    pub developer_message: String,
    /// The contextual user message (user instructions + environment context).
    pub user_message: String,
    /// A short summary of the tool configuration.
    pub tools_summary: String,
}

impl DumpResult {
    /// Approximate token count using the same heuristic as `core/src/truncate.rs`:
    /// `ceil(bytes / 4)`.
    fn approx_tokens(text: &str) -> usize {
        text.len().div_ceil(4)
    }
}

/// Assemble the initial context that would be sent to the model, using the
/// same building blocks as `Session::build_initial_context()`.
pub async fn dump_initial_context(config: &Config) -> anyhow::Result<DumpResult> {
    // ── 1. Model info ──────────────────────────────────────────────────
    let model_slug = config.model.as_deref().unwrap_or("gpt-4.1");
    let raw_model_info = model_info_from_slug(model_slug);
    let model_info = with_config_overrides(raw_model_info, config);

    // ── 2. Base instructions ───────────────────────────────────────────
    let base_instructions = config
        .base_instructions
        .clone()
        .unwrap_or_else(|| model_info.get_model_instructions(config.personality));

    // ── 3. Developer message sections ──────────────────────────────────
    let mut developer_sections = Vec::<String>::with_capacity(8);

    // 3a. Policy-based instructions
    let sandbox_policy = config.permissions.sandbox_policy.get().clone();
    let approval_policy = config.permissions.approval_policy.value();
    let exec_policy = Policy::empty();
    let exec_permission_approvals_enabled = config
        .features
        .get()
        .enabled(Feature::ExecPermissionApprovals);
    let request_permissions_tool_enabled = config
        .features
        .get()
        .enabled(Feature::RequestPermissionsTool);
    developer_sections.push(
        DeveloperInstructions::from_policy(
            &sandbox_policy,
            approval_policy,
            &exec_policy,
            &config.cwd,
            exec_permission_approvals_enabled,
            request_permissions_tool_enabled,
        )
        .into_text(),
    );

    // 3b. Config developer instructions
    if let Some(developer_instructions) = config.developer_instructions.as_deref() {
        developer_sections.push(developer_instructions.to_string());
    }

    // 3c. Collaboration mode instructions
    let model_name = config
        .model
        .clone()
        .unwrap_or_else(|| model_slug.to_string());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model: model_name,
            reasoning_effort: config
                .model_reasoning_effort
                .or(model_info.default_reasoning_level),
            developer_instructions: None,
        },
    };
    let collaboration_modes_config = CollaborationModesConfig::default();
    let presets = builtin_collaboration_mode_presets(collaboration_modes_config);
    // Apply the default mode's developer instructions from the presets (same
    // as `DeveloperInstructions::from_collaboration_mode` does at runtime).
    // The initial session always starts in Default mode, so we resolve
    // against the presets to find matching developer instructions.
    let mut resolved_collab = collaboration_mode.clone();
    for preset in &presets {
        if preset.name == resolved_collab.mode.display_name()
            && let Some(Some(instructions)) = &preset.developer_instructions
        {
            resolved_collab.settings.developer_instructions = Some(instructions.clone());
        }
    }
    if let Some(collab_instructions) =
        DeveloperInstructions::from_collaboration_mode(&resolved_collab)
    {
        developer_sections.push(collab_instructions.into_text());
    }

    // 3d. Commit attribution
    if config.features.get().enabled(Feature::CodexGitCommit)
        && let Some(commit_instruction) =
            commit_message_trailer_instruction(config.commit_attribution.as_deref())
    {
        developer_sections.push(commit_instruction);
    }

    let developer_message = developer_sections.join("\n\n");

    // ── 4. User message ────────────────────────────────────────────────
    let mut contextual_user_sections = Vec::<String>::with_capacity(2);

    // 4a. User instructions (AGENTS.md, skills, etc.)
    if let Some(user_instructions) = get_user_instructions(config).await {
        contextual_user_sections.push(
            UserInstructions {
                text: user_instructions,
                directory: config.cwd.to_string_lossy().into_owned(),
            }
            .serialize_to_text(),
        );
    }

    // 4b. Environment context
    let shell = default_user_shell();
    let (current_date, timezone) = local_time_context();
    let env_context = EnvironmentContext::new(
        Some(config.cwd.clone()),
        shell,
        Some(current_date),
        Some(timezone),
        None,
        None,
    );
    contextual_user_sections.push(env_context.serialize_to_xml());

    let user_message = contextual_user_sections.join("\n\n");

    // ── 5. Tools summary ───────────────────────────────────────────────
    let tools_summary =
        "Tool listing requires a running session. Use `codex --full-auto` to see tools in action."
            .to_string();

    Ok(DumpResult {
        base_instructions,
        developer_message,
        user_message,
        tools_summary,
    })
}

/// Format a `DumpResult` into a human-readable multi-section string.
pub fn format_dump_result(result: &DumpResult) -> String {
    let base_tokens = DumpResult::approx_tokens(&result.base_instructions);
    let dev_tokens = DumpResult::approx_tokens(&result.developer_message);
    let user_tokens = DumpResult::approx_tokens(&result.user_message);
    let total_tokens = base_tokens + dev_tokens + user_tokens;

    format!(
        "\
\u{2550}\u{2550}\u{2550} BASE INSTRUCTIONS (instructions field) \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}
Tokens: ~{base_tokens}

{base_instructions}

\u{2550}\u{2550}\u{2550} DEVELOPER MESSAGE \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}
Tokens: ~{dev_tokens}

{developer_message}

\u{2550}\u{2550}\u{2550} CONTEXTUAL USER MESSAGE \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}
Tokens: ~{user_tokens}

{user_message}

\u{2550}\u{2550}\u{2550} TOTAL \u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}
Base instructions:     ~{base_tokens} tokens
Developer message:     ~{dev_tokens} tokens
User message:          ~{user_tokens} tokens
\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}
Total initial context: ~{total_tokens} tokens
",
        base_instructions = result.base_instructions,
        developer_message = result.developer_message,
        user_message = result.user_message,
    )
}

fn local_time_context() -> (String, String) {
    match iana_time_zone::get_timezone() {
        Ok(timezone) => (
            chrono::Local::now().format("%Y-%m-%d").to_string(),
            timezone,
        ),
        Err(_) => (
            chrono::Utc::now().format("%Y-%m-%d").to_string(),
            "Etc/UTC".to_string(),
        ),
    }
}
