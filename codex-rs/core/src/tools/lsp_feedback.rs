//! Helper that wraps `ServerRegistry` to provide diagnostic feedback
//! for implicit LSP integration (apply_patch, read_file).

use std::path::Path;
use std::sync::Arc;

use codex_lsp_client::ServerRegistry;
use codex_lsp_client::lsp_types::DiagnosticSeverity;

/// Maximum number of diagnostics reported per file.
const MAX_DIAGNOSTICS_PER_FILE: usize = 20;

/// Wraps the LSP `ServerRegistry` and provides formatted diagnostic feedback.
pub struct LspFeedback {
    pub(crate) registry: Arc<ServerRegistry>,
}

impl LspFeedback {
    pub fn new(registry: Arc<ServerRegistry>) -> Self {
        Self { registry }
    }

    /// Sync a file to all applicable LSP servers, wait for diagnostics,
    /// and return formatted ERROR-level diagnostics as XML.
    /// Returns an empty string if there are no errors.
    pub async fn touch_and_collect_errors(&self, file: &Path) -> String {
        let all_diags = self.registry.touch_file(file, true).await;

        let mut errors = Vec::new();
        for (_server_id, diags) in &all_diags {
            for diag in diags {
                if diag.severity == Some(DiagnosticSeverity::ERROR) {
                    let line = diag.range.start.line + 1;
                    let col = diag.range.start.character + 1;
                    errors.push(format!("ERROR [{}:{}] {}", line, col, diag.message));
                }
            }
        }

        format_diagnostics_xml(file, &errors)
    }

    /// Sync a file without waiting for diagnostics (warmup only).
    pub async fn touch_nowait(&self, file: &Path) {
        let _ = self.registry.touch_file(file, false).await;
    }
}

/// Format a list of error strings as XML diagnostics output.
/// Returns an empty string if `errors` is empty.
fn format_diagnostics_xml(file: &Path, errors: &[String]) -> String {
    if errors.is_empty() {
        return String::new();
    }

    let display_path = file.display();
    let total = errors.len();
    let shown: Vec<_> = errors.iter().take(MAX_DIAGNOSTICS_PER_FILE).collect();
    let remaining = total.saturating_sub(MAX_DIAGNOSTICS_PER_FILE);

    let mut result = format!(
        "\nLSP errors detected in {display_path}, please fix:\n<diagnostics file=\"{display_path}\">\n"
    );
    for line in &shown {
        result.push_str(line);
        result.push('\n');
    }
    if remaining > 0 {
        result.push_str(&format!("... and {remaining} more\n"));
    }
    result.push_str("</diagnostics>");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_no_errors() {
        let formatted = format_diagnostics_xml(Path::new("src/main.rs"), &[]);
        assert!(formatted.is_empty());
    }

    #[test]
    fn format_with_errors() {
        let formatted = format_diagnostics_xml(
            Path::new("src/main.rs"),
            &[
                "ERROR [12:5] Type mismatch".to_string(),
                "ERROR [15:3] Method not found".to_string(),
            ],
        );
        assert!(formatted.contains("<diagnostics file=\"src/main.rs\">"));
        assert!(formatted.contains("ERROR [12:5] Type mismatch"));
        assert!(formatted.contains("ERROR [15:3] Method not found"));
        assert!(formatted.contains("</diagnostics>"));
    }

    #[test]
    fn format_truncates_at_max() {
        let errors: Vec<_> = (0..25)
            .map(|i| format!("ERROR [{i}:0] error {i}"))
            .collect();
        let formatted = format_diagnostics_xml(Path::new("test.rs"), &errors);
        assert!(formatted.contains("... and 5 more"));
    }
}
