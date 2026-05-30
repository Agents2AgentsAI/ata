//! Tool spec for the `artifacts` freeform handler. Ported from the orphan
//! `tools/spec/javascript.rs::create_artifacts_tool` so the handler can be
//! registered through `spec_plan::add_artifacts_tools` without resurfacing
//! the rest of the dead `tools/spec/` subtree.

use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::ToolSpec;

// @agent-facing
pub(crate) fn create_artifacts_tool() -> ToolSpec {
    const ARTIFACTS_FREEFORM_GRAMMAR: &str = r#"
start: pragma_source | plain_source

pragma_source: PRAGMA_LINE NEWLINE js_source
plain_source: PLAIN_JS_SOURCE

js_source: JS_SOURCE

PRAGMA_LINE: /[ \t]*\/\/ codex-artifacts:[^\r\n]*/ | /[ \t]*\/\/ codex-artifact-tool:[^\r\n]*/
NEWLINE: /\r?\n/
PLAIN_JS_SOURCE: /(?:\s*)(?:[^\s{\"`]|`[^`]|``[^`])[\s\S]*/
JS_SOURCE: /(?:\s*)(?:[^\s{\"`]|`[^`]|``[^`])[\s\S]*/
"#;

    ToolSpec::Freeform(FreeformTool {
        name: "artifacts".to_string(),
        description: "Runs raw JavaScript against the preinstalled Codex @oai/artifact-tool runtime for creating presentations or spreadsheets. This is plain JavaScript executed by a local Node-compatible runtime with top-level await, not TypeScript: do not use type annotations, `interface`, `type`, or `import type`. Author code the same way you would for `import { Presentation, Workbook, PresentationFile, SpreadsheetFile, FileBlob, ... } from \"@oai/artifact-tool\"`, but omit that import line because the package surface is already preloaded. Named exports are available directly on `globalThis`, and the full module is available as `globalThis.artifactTool` (also aliased as `globalThis.artifacts` and `globalThis.codexArtifacts`). Node built-ins such as `node:fs/promises` may still be imported when needed for saving preview bytes. This is a freeform tool: send raw JavaScript source text, optionally with a first-line pragma like `// codex-artifacts: timeout_ms=15000` or `// codex-artifact-tool: timeout_ms=15000`; do not send JSON/quotes/markdown fences."
            .to_string(),
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: ARTIFACTS_FREEFORM_GRAMMAR.to_string(),
        },
    })
}
