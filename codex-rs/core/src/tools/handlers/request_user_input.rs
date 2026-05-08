use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::request_user_input_spec::REQUEST_USER_INPUT_TOOL_NAME;
use crate::tools::handlers::request_user_input_spec::normalize_request_user_input_args;
use crate::tools::handlers::request_user_input_spec::request_user_input_unavailable_message;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use async_trait::async_trait;
use codex_protocol::config_types::ModeKind;
use codex_protocol::request_user_input::RequestUserInputArgs;

fn request_user_input_is_available(mode: ModeKind, default_mode_request_user_input: bool) -> bool {
    mode.allows_request_user_input()
        || (default_mode_request_user_input && mode == ModeKind::Default)
}

fn format_allowed_modes(default_mode_request_user_input: bool) -> String {
    let mode_names: Vec<&str> = TUI_VISIBLE_COLLABORATION_MODES
        .into_iter()
        .filter(|mode| request_user_input_is_available(*mode, default_mode_request_user_input))
        .map(ModeKind::display_name)
        .collect();

    match mode_names.as_slice() {
        [] => "no modes".to_string(),
        [mode] => format!("{mode} mode"),
        [first, second] => format!("{first} or {second} mode"),
        [..] => format!("modes: {}", mode_names.join(",")),
    }
}

pub(crate) fn request_user_input_unavailable_message(
    mode: ModeKind,
    default_mode_request_user_input: bool,
) -> Option<String> {
    if request_user_input_is_available(mode, default_mode_request_user_input) {
        None
    } else {
        let mode_name = mode.display_name();
        Some(format!(
            "request_user_input is unavailable in {mode_name} mode"
        ))
    }
}

// @agent-facing
pub(crate) fn request_user_input_tool_description(default_mode_request_user_input: bool) -> String {
    let allowed_modes = format_allowed_modes(default_mode_request_user_input);
    format!(
        "Request user input for one to three short questions and wait for the response. This tool is only available in {allowed_modes}."
    )
}

pub struct RequestUserInputHandler {
    pub available_modes: Vec<ModeKind>,
}

impl ToolHandler for RequestUserInputHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "{REQUEST_USER_INPUT_TOOL_NAME} handler received unsupported payload"
                )));
            }
        };

        if turn.session_source.is_non_root_agent() {
            return Err(FunctionCallError::RespondToModel(
                "request_user_input can only be used by the root thread".to_string(),
            ));
        }

        let mode = session.collaboration_mode().await.mode;
        if let Some(message) = request_user_input_unavailable_message(mode, &self.available_modes) {
            return Err(FunctionCallError::RespondToModel(message));
        }

        let args: RequestUserInputArgs = parse_arguments(&arguments)?;
        let args =
            normalize_request_user_input_args(args).map_err(FunctionCallError::RespondToModel)?;
        let response = session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
            .ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "{REQUEST_USER_INPUT_TOOL_NAME} was cancelled before receiving a response"
                ))
            })?;

        let content = serde_json::to_string(&response).map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize {REQUEST_USER_INPUT_TOOL_NAME} response: {err}"
            ))
        })?;

        Ok(FunctionToolOutput::from_text(content, Some(true)))
    }
}

#[cfg(test)]
#[path = "request_user_input_tests.rs"]
mod tests;
