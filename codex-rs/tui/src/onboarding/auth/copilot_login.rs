//! GitHub Copilot device-code OAuth flow, driven through the app-server
//! protocol.
//!
//! Mirrors `headless_chatgpt_login::start_headless_chatgpt_login` but sends
//! `LoginAccountParams::CopilotDeviceCode` and unwraps the matching
//! `LoginAccountResponse::CopilotDeviceCode { login_id, verification_uri,
//! user_code }`. The TUI never imports `codex_core` directly; OAuth state
//! lives entirely behind the protocol.

use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::LoginAccountParams;
use codex_app_server_protocol::LoginAccountResponse;
use uuid::Uuid;

use super::AuthModeWidget;
use super::ContinueWithDeviceCodeState;
use super::SignInState;
use super::cancel_login_attempt;
use super::onboarding_request_id;

pub(super) fn start_copilot_login(widget: &mut AuthModeWidget) {
    let request_id = Uuid::new_v4().to_string();
    *widget.sign_in_state.write().unwrap() =
        SignInState::CopilotDeviceCode(ContinueWithDeviceCodeState::pending(request_id.clone()));
    widget.request_frame.schedule_frame();

    let request_handle = widget.app_server_request_handle.clone();
    let sign_in_state = widget.sign_in_state.clone();
    let request_frame = widget.request_frame.clone();
    let error = widget.error.clone();
    tokio::spawn(async move {
        match request_handle
            .request_typed::<LoginAccountResponse>(ClientRequest::LoginAccount {
                request_id: onboarding_request_id(),
                params: LoginAccountParams::CopilotDeviceCode,
            })
            .await
        {
            Ok(LoginAccountResponse::CopilotDeviceCode {
                login_id,
                verification_uri,
                user_code,
            }) => {
                let updated = set_state_for_active_attempt(
                    &sign_in_state,
                    &request_frame,
                    &request_id,
                    ContinueWithDeviceCodeState::ready(
                        request_id.clone(),
                        login_id.clone(),
                        verification_uri,
                        user_code,
                    ),
                );
                if updated {
                    *error.write().unwrap() = None;
                } else {
                    cancel_login_attempt(&request_handle, login_id).await;
                }
            }
            Ok(other) => {
                let _updated = set_error_for_active_attempt(
                    &sign_in_state,
                    &request_frame,
                    &error,
                    &request_id,
                    format!("Unexpected account/login/start response: {other:?}"),
                );
            }
            Err(err) => {
                let _updated = set_error_for_active_attempt(
                    &sign_in_state,
                    &request_frame,
                    &error,
                    &request_id,
                    err.to_string(),
                );
            }
        }
    });
}

fn copilot_attempt_matches(state: &SignInState, request_id: &str) -> bool {
    matches!(
        state,
        SignInState::CopilotDeviceCode(state) if state.request_id == request_id
    )
}

fn set_state_for_active_attempt(
    sign_in_state: &std::sync::Arc<std::sync::RwLock<SignInState>>,
    request_frame: &crate::tui::FrameRequester,
    request_id: &str,
    next_state: ContinueWithDeviceCodeState,
) -> bool {
    let mut guard = sign_in_state.write().unwrap();
    if !copilot_attempt_matches(&guard, request_id) {
        return false;
    }
    *guard = SignInState::CopilotDeviceCode(next_state);
    drop(guard);
    request_frame.schedule_frame();
    true
}

fn set_error_for_active_attempt(
    sign_in_state: &std::sync::Arc<std::sync::RwLock<SignInState>>,
    request_frame: &crate::tui::FrameRequester,
    error: &std::sync::Arc<std::sync::RwLock<Option<String>>>,
    request_id: &str,
    message: String,
) -> bool {
    let mut guard = sign_in_state.write().unwrap();
    if !copilot_attempt_matches(&guard, request_id) {
        return false;
    }
    *guard = SignInState::PickMode;
    drop(guard);
    *error.write().unwrap() = Some(message);
    request_frame.schedule_frame();
    true
}
