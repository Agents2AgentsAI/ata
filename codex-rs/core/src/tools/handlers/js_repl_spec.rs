use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

/// Tool name surfaced to the model.
pub const JS_REPL_TOOL_NAME: &str = "js_repl";

// @agent-facing
const JS_REPL_TOOL_DESCRIPTION: &str = "Evaluate a JavaScript snippet in an embedded V8 isolate \
    and return the value of the last expression. Useful for quick numeric \
    computations, string transformations, JSON shaping, regex tests, and \
    other one-off scripts where shelling out to `node` would be heavy. \
    \
    Each call runs in a fresh isolate — variables, functions, and side \
    effects do NOT persist across calls. There is no Node API available: \
    no `require`, no filesystem, no network, no `process`. Plain ECMAScript \
    only, plus standard built-ins (Math, JSON, Date, Array, regex, etc.). \
    \
    Each evaluation is bounded by a per-call timeout (default 5s). Long-running \
    or infinite scripts will be terminated and the call returns an error. \
    Output is capped at 16KB; longer values are truncated.";

/// Build the `js_repl` tool spec.
pub fn create_js_repl_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "code".to_string(),
        JsonSchema::string(Some(
            "JavaScript source to evaluate. The value of the last expression \
             becomes the tool result."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: JS_REPL_TOOL_NAME.to_string(),
        description: JS_REPL_TOOL_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["code".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn js_repl_tool_has_required_code_param() {
        let spec = create_js_repl_tool();
        match spec {
            ToolSpec::Function(f) => {
                assert_eq!(f.name, "js_repl");
                let properties = f
                    .parameters
                    .properties
                    .as_ref()
                    .expect("object parameters should expose properties");
                assert!(properties.contains_key("code"));
                assert_eq!(
                    f.parameters.required.as_deref(),
                    Some(["code".to_string()].as_slice())
                );
            }
            other => panic!("expected ToolSpec::Function, got {other:?}"),
        }
    }
}
