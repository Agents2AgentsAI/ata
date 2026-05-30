//! `/workspace` slash command dispatcher and `/fast` toggle.
//!
//! `/workspace [current|list|use <selector>]` is a thin TUI wrapper that
//! shells out to the `ata workspace` CLI subcommand and prints the captured
//! stdout/stderr inline. Workspace creation and the long tail of admin
//! verbs (init, delete, audit, etc.) stay CLI-only — see PLAN.md TR-040.
//!
//! `/fast` toggles the model's Fast service tier and prints a confirmation
//! line so transcript readers can see the new state.

use std::process::Command;

use codex_features::Feature;

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

        let cli_args: Vec<&str> = match verb.to_ascii_lowercase().as_str() {
            "current" | "show" | "status" if rest.is_empty() => vec!["workspace", "current"],
            "list" if rest.is_empty() => vec!["workspace", "list"],
            "use" | "select" if !rest.is_empty() => vec!["workspace", "use", rest],
            _ => {
                self.add_error_message(WORKSPACE_USAGE.to_string());
                return;
            }
        };

        let Ok(exe) = std::env::current_exe() else {
            self.add_error_message(
                "Could not resolve the ata binary path for /workspace.".to_string(),
            );
            return;
        };

        let output = Command::new(&exe).args(&cli_args).output();
        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                if !stdout.is_empty() {
                    self.add_info_message(stdout, None);
                }
                if !stderr.is_empty() {
                    if output.status.success() {
                        self.add_info_message(stderr, None);
                    } else {
                        self.add_error_message(stderr);
                    }
                } else if !output.status.success() {
                    self.add_error_message(format!(
                        "`ata {}` failed with exit code {}.",
                        cli_args.join(" "),
                        output.status.code().unwrap_or(-1)
                    ));
                }
            }
            Err(err) => {
                self.add_error_message(format!(
                    "Failed to run `ata {}`: {err}",
                    cli_args.join(" ")
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
        // Toggle didn't change anything (rare: e.g. no fast tier resolved despite the check above).
        if was_fast == is_fast_now {
            self.add_info_message(format!("Fast mode is {status}."), None);
        } else {
            self.add_info_message(format!("Fast mode set to {status}."), None);
        }
    }
}
