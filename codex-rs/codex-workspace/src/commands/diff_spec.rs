use crate::commands::materialize::{plan, ActionKind};
use crate::error::WorkspaceError;
use crate::spec::read_spec;
use std::path::Path;

/// Diff a workspace spec against the current workspace state.
///
/// Shows what `materialize` would do: repos to add, pin, or skip.
pub fn run(workspace_id: &str, spec_path: &Path) -> Result<String, WorkspaceError> {
    let spec = read_spec(spec_path)?;
    let actions = plan(workspace_id, &spec)?;

    let mut adds = Vec::new();
    let mut pins = Vec::new();
    let mut skips = Vec::new();

    for action in &actions {
        match &action.action {
            ActionKind::Add => adds.push(action.alias.as_str()),
            ActionKind::Pin {
                current_sha,
                target_sha,
            } => pins.push((action.alias.as_str(), current_sha.as_str(), target_sha.as_str())),
            ActionKind::Skip => skips.push(action.alias.as_str()),
        }
    }

    let mut output = String::new();

    if !adds.is_empty() {
        output.push_str(&format!("Add ({}):\n", adds.len()));
        for alias in &adds {
            output.push_str(&format!("  + {alias}\n"));
        }
    }

    if !pins.is_empty() {
        output.push_str(&format!("Pin ({}):\n", pins.len()));
        for (alias, current, target) in &pins {
            output.push_str(&format!("  ~ {alias}: {current} → {target}\n"));
        }
    }

    if !skips.is_empty() {
        output.push_str(&format!("Skip ({}):\n", skips.len()));
        for alias in &skips {
            output.push_str(&format!("  = {alias}\n"));
        }
    }

    if adds.is_empty() && pins.is_empty() {
        output.push_str("No changes needed.\n");
    } else {
        output.push_str(&format!(
            "\nSummary: {} add, {} pin, {} skip\n",
            adds.len(),
            pins.len(),
            skips.len()
        ));
    }

    Ok(output)
}
