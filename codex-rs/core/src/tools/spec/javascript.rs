use super::*;

pub(super) fn create_js_repl_tool() -> ToolSpec {
    const JS_REPL_FREEFORM_GRAMMAR: &str = r#"
start: pragma_source | plain_source

pragma_source: PRAGMA_LINE NEWLINE js_source
plain_source: PLAIN_JS_SOURCE

js_source: JS_SOURCE

PRAGMA_LINE: /[ \t]*\/\/ codex-js-repl:[^\r\n]*/
NEWLINE: /\r?\n/
PLAIN_JS_SOURCE: /(?:\s*)(?:[^\s{\"`]|`[^`]|``[^`])[\s\S]*/
JS_SOURCE: /(?:\s*)(?:[^\s{\"`]|`[^`]|``[^`])[\s\S]*/
"#;

    ToolSpec::Freeform(FreeformTool {
        name: "js_repl".to_string(),
        description: "Runs JavaScript in a persistent Node kernel with top-level await. This is a freeform tool: send raw JavaScript source text, optionally with a first-line pragma like `// codex-js-repl: timeout_ms=15000`; do not send JSON/quotes/markdown fences."
            .to_string(),
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: JS_REPL_FREEFORM_GRAMMAR.to_string(),
        },
    })
}

pub(super) fn create_artifacts_tool() -> ToolSpec {
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

pub(super) fn create_js_repl_reset_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "js_repl_reset".to_string(),
        description:
            "Restarts the js_repl kernel for this run and clears persisted top-level bindings."
                .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties: BTreeMap::new(),
            required: None,
            additional_properties: Some(false.into()),
        },
    })
}
