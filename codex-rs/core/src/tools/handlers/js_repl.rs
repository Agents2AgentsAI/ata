//! `js_repl` tool: evaluate a JavaScript snippet in an embedded V8 isolate.
//!
//! The actual evaluation lives in the `codex-v8-poc` crate. This module is
//! the v0.129.0-shaped tool handler glue: parse the function-call arguments,
//! dispatch the eval on a blocking thread (V8 isolates are `!Send`), and
//! map the outcome back into a `ResponseInputItem`.
//!
//! The previous Ata tree had a session-scoped REPL kernel with a manager,
//! reset handler, freeform-payload pragma parser, and a `JsRepl` feature
//! flag. v0.129.0's tool-spec system asks every tool to be self-contained
//! and registered through `tools::spec_plan::build_specs_with_toolkits`,
//! so this re-port follows that contract: a single function-typed tool,
//! stateless across calls, always available. State persistence can be
//! layered on later by giving `Session::services` a per-session V8 worker.

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::js_repl_spec::JS_REPL_TOOL_NAME;
use crate::tools::registry::CoreToolRuntime;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use codex_tools::ToolExecutor;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use codex_v8_poc::JsEvalOutcome;
use codex_v8_poc::js_eval;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub struct JsReplHandler {
    pub(crate) spec: ToolSpec,
}

impl JsReplHandler {
    pub(crate) fn new(spec: ToolSpec) -> Self {
        Self { spec }
    }
}

#[derive(Debug, Deserialize)]
struct JsReplArgs {
    code: String,
}

pub struct JsReplToolOutput {
    outcome: JsEvalOutcome,
}

impl ToolOutput for JsReplToolOutput {
    fn log_preview(&self) -> String {
        let mut preview = self.outcome.value.clone();
        if let Some(idx) = preview.find('\n') {
            preview.truncate(idx);
        }
        if self.outcome.success {
            format!("js_repl ok: {preview}")
        } else {
            format!("js_repl failed: {preview}")
        }
    }

    fn success_for_logging(&self) -> bool {
        self.outcome.success
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        let mut output = FunctionCallOutputPayload::from_text(self.outcome.value.clone());
        output.success = Some(self.outcome.success);
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output,
        }
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        let mut map = serde_json::Map::new();
        map.insert(
            "value".to_string(),
            JsonValue::String(self.outcome.value.clone()),
        );
        map.insert("success".to_string(), JsonValue::Bool(self.outcome.success));
        map.insert(
            "elapsed_ms".to_string(),
            JsonValue::Number(serde_json::Number::from(
                self.outcome.elapsed.as_millis() as u64
            )),
        );
        JsonValue::Object(map)
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for JsReplHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(JS_REPL_TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn handle(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let arguments = match invocation.payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "js_repl handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: JsReplArgs = serde_json::from_str(&arguments).map_err(|e| {
            FunctionCallError::RespondToModel(format!("failed to parse js_repl arguments: {e}"))
        })?;

        // V8 isolates are `!Send`; run the eval on a blocking thread so we do
        // not pin a tokio worker for the duration of the script. A panic
        // inside the eval propagates here as a `JoinError` — convert it into
        // a model-visible failure rather than aborting the turn.
        let outcome = tokio::task::spawn_blocking(move || js_eval(&args.code))
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("js_repl execution panicked: {err}"))
            })?;

        Ok(boxed_tool_output(JsReplToolOutput { outcome }))
    }
}

impl CoreToolRuntime for JsReplHandler {}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn make_outcome(success: bool, value: &str) -> JsReplToolOutput {
        JsReplToolOutput {
            outcome: JsEvalOutcome {
                value: value.to_string(),
                success,
                elapsed: std::time::Duration::from_millis(7),
            },
        }
    }

    #[test]
    fn log_preview_summarizes_success() {
        let preview = make_outcome(true, "42").log_preview();
        assert_eq!(preview, "js_repl ok: 42");
    }

    #[test]
    fn log_preview_summarizes_failure() {
        let preview = make_outcome(false, "compile error").log_preview();
        assert_eq!(preview, "js_repl failed: compile error");
    }

    #[test]
    fn log_preview_collapses_multiline_value() {
        let preview = make_outcome(true, "line1\nline2\nline3").log_preview();
        assert_eq!(preview, "js_repl ok: line1");
    }

    #[test]
    fn code_mode_result_includes_value_success_and_elapsed() {
        let value = make_outcome(true, "ok").code_mode_result(&ToolPayload::Function {
            arguments: String::new(),
        });
        let JsonValue::Object(map) = value else {
            panic!("expected object");
        };
        assert_eq!(map.get("value"), Some(&JsonValue::String("ok".to_string())));
        assert_eq!(map.get("success"), Some(&JsonValue::Bool(true)));
        assert!(matches!(map.get("elapsed_ms"), Some(JsonValue::Number(_))));
    }
}
