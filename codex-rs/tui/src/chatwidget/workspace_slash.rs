//! `/workspace` slash command dispatcher and `/fast` toggle.
//!
//! `/workspace [current|list|use <selector>]` calls the `codex_workspace`
//! library directly and prints a human-readable summary inline. Workspace
//! creation and the long tail of admin verbs (init, delete, audit, etc.) stay
//! CLI-only — see PLAN.md TR-040.
//!
//! `/fast` toggles the model's Fast service tier and prints a confirmation
//! line so transcript readers can see the new state.

use codex_features::Feature;
use codex_workspace::commands;
use codex_workspace::workspace_resolution;

use crate::chatwidget::ChatWidget;

const WORKSPACE_USAGE: &str = "Usage: /workspace [current|list|use <selector>]";
const WORKSPACE_USAGE_HINT: &str =
    "Run `ata workspace --help` for the full CLI surface (init, delete, audit, etc.).";

impl ChatWidget {
    pub(crate) fn run_workspace_slash_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.add_info_message(
                WORKSPACE_USAGE.to_string(),
                Some(WORKSPACE_USAGE_HINT.to_string()),
            );
            return;
        }

        let split_at = trimmed.find(char::is_whitespace);
        let (verb, rest) = match split_at {
            Some(index) => (&trimmed[..index], trimmed[index..].trim_start()),
            None => (trimmed, ""),
        };

        match verb.to_ascii_lowercase().as_str() {
            "current" | "show" | "status" if rest.is_empty() => self.workspace_current(),
            "list" if rest.is_empty() => self.workspace_list(),
            "use" | "select" if !rest.is_empty() => self.workspace_use(rest),
            _ => {
                self.add_error_message(WORKSPACE_USAGE.to_string());
            }
        }
    }

    fn workspace_current(&mut self) {
        match workspace_resolution::resolve_workspace(None) {
            Ok(id) => {
                let name = commands::read::run(&id)
                    .ok()
                    .map(|m| m.name)
                    .unwrap_or_default();
                let line = if name.is_empty() || name == id {
                    format!("Current workspace: {id}")
                } else {
                    format!("Current workspace: {id} ({name})")
                };
                self.add_info_message(line, None);
            }
            Err(err) => {
                self.add_error_message(format!("Could not resolve current workspace: {err}"));
            }
        }
    }

    fn workspace_list(&mut self) {
        let active = workspace_resolution::resolve_workspace(None).ok();
        match commands::list::run() {
            Ok(summaries) if summaries.is_empty() => {
                self.add_info_message(
                    "Workspaces: (none — create one with `ata workspace init <name>`)"
                        .to_string(),
                    None,
                );
            }
            Ok(summaries) => {
                let mut lines = vec!["Workspaces:".to_string()];
                for summary in summaries {
                    let marker = if active.as_deref() == Some(summary.id.as_str()) {
                        " (current)"
                    } else {
                        ""
                    };
                    let name = if summary.name.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", summary.name)
                    };
                    lines.push(format!("  {id}{name}{marker}", id = summary.id));
                }
                self.add_info_message(lines.join("\n"), None);
            }
            Err(err) => {
                self.add_error_message(format!("Could not list workspaces: {err}"));
            }
        }
    }

    fn workspace_use(&mut self, selector: &str) {
        match commands::select::run(selector) {
            Ok(resolved) => {
                self.add_info_message(format!("Selected workspace: {resolved}"), None);
            }
            Err(err) => {
                let message = err.to_string();
                // Surface "not found"/"unknown" phrasing so it's discoverable
                // for users typing an invalid selector.
                self.add_error_message(format!(
                    "Could not select workspace `{selector}`: workspace not found ({message})"
                ));
            }
        }
    }

    pub(crate) fn run_fast_slash_command(&mut self, _args: &str) {
        if !self.config.features.enabled(Feature::FastMode) {
            self.add_error_message("Fast mode is disabled by feature flag.".to_string());
            return;
        }
        let Some(fast_tier) = self.current_model_fast_service_tier() else {
            self.add_error_message(
                "Fast mode is not available for the current model.".to_string(),
            );
            return;
        };
        let was_fast = self.current_service_tier() == Some(fast_tier.id.as_str());
        self.toggle_fast_mode_from_ui();
        let is_fast_now = self.current_service_tier() == Some(fast_tier.id.as_str());
        let status = if is_fast_now { "on" } else { "off" };
        if was_fast == is_fast_now {
            self.add_info_message(format!("Fast mode is {status}."), None);
        } else {
            self.add_info_message(format!("Fast mode set to {status}."), None);
        }
    }
}
