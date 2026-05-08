use codex_core::AuthManager;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_login::ServerOptions;
use codex_login::complete_device_code_login;
use codex_login::request_device_code;
use codex_login::run_login_server;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;
use tokio::sync::Notify;

use crate::motion::MotionMode;
use crate::motion::shimmer_text;

use super::AuthModeWidget;
use super::ContinueWithDeviceCodeState;
use super::SignInState;
use super::cancel_login_attempt;
use super::mark_url_hyperlink;
use super::onboarding_request_id;

pub(super) fn start_headless_chatgpt_login(widget: &mut AuthModeWidget) {
    let request_id = Uuid::new_v4().to_string();
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::pending(request_id.clone()));
    widget.request_frame.schedule_frame();

    let request_handle = widget.app_server_request_handle.clone();
    let sign_in_state = widget.sign_in_state.clone();
    let request_frame = widget.request_frame.clone();
    let auth_manager = widget.auth_manager.clone();
    let codex_home = widget.codex_home.clone();
    let cancel = begin_device_code_attempt(&sign_in_state, &request_frame);

    tokio::spawn(async move {
        let device_code = match request_device_code(&opts).await {
            Ok(device_code) => device_code,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    let should_fallback = {
                        let guard = sign_in_state.read().unwrap();
                        device_code_attempt_matches(&guard, &cancel)
                    };

                    if !should_fallback {
                        return;
                    }

                    match run_login_server(opts) {
                        Ok(child) => {
                            let auth_url = child.auth_url.clone();
                            {
                                *sign_in_state.write().unwrap() =
                                    SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
                                        auth_url,
                                        shutdown_flag: Some(child.cancel_handle()),
                                    });
                            }
                            request_frame.schedule_frame();
                            let r = child.block_until_done().await;
                            match r {
                                Ok(()) => {
                                    auth_manager.reload();
                                    // Clear model to use remote default for ChatGPT
                                    if let Err(e) = ConfigEditsBuilder::new(&codex_home)
                                        .set_model(None, None, None)
                                        .apply_blocking()
                                    {
                                        tracing::error!(
                                            "failed to clear model on ChatGPT login: {e}"
                                        );
                                    }
                                    *sign_in_state.write().unwrap() =
                                        SignInState::ChatGptSuccessMessage;
                                    request_frame.schedule_frame();
                                }
                                _ => {
                                    *sign_in_state.write().unwrap() = SignInState::PickMode;
                                    request_frame.schedule_frame();
                                }
                            }
                        }
                        Err(_) => {
                            set_device_code_state_for_active_attempt(
                                &sign_in_state,
                                &request_frame,
                                &cancel,
                                SignInState::PickMode,
                            );
                        }
                    }
                } else {
                    cancel_login_attempt(&request_handle, login_id).await;
                }
            }
        };

        if !set_device_code_state_for_active_attempt(
            &sign_in_state,
            &request_frame,
            &cancel,
            SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState {
                device_code: Some(device_code.clone()),
                cancel: Some(cancel.clone()),
            }),
        ) {
            return;
        }

        tokio::select! {
            _ = cancel.notified() => {}
            r = complete_device_code_login(opts, device_code) => {
                match r {
                    Ok(()) => {
                        set_device_code_success_message_for_active_attempt(
                            &sign_in_state,
                            &request_frame,
                            &auth_manager,
                            &codex_home,
                            &cancel,
                        );
                    }
                    Err(_) => {
                        set_device_code_state_for_active_attempt(
                            &sign_in_state,
                            &request_frame,
                            &cancel,
                            SignInState::PickMode,
                        );
                    }
                }
            }
        }
    });
}

pub(super) fn render_device_code_login(
    widget: &AuthModeWidget,
    area: Rect,
    buf: &mut Buffer,
    state: &ContinueWithDeviceCodeState,
) {
    let banner = if state.is_showing_copyable_auth() {
        "Finish signing in via your browser"
    } else {
        "Preparing device code login"
    };

    let mut spans = vec!["  ".into()];
    if widget.animations_enabled && !widget.animations_suppressed.get() {
        widget
            .request_frame
            .schedule_frame_in(std::time::Duration::from_millis(100));
        spans.extend(shimmer_text(banner, MotionMode::Animated));
    } else {
        spans.push(banner.into());
    }

    let mut lines = vec![spans.into(), "".into()];

    let verification_url = if let (Some(verification_url), Some(user_code)) =
        (&state.verification_url, &state.user_code)
    {
        lines.push("  1. Open this link in your browser and sign in".into());
        lines.push("".into());
        lines.push(Line::from(vec![
            "  ".into(),
            verification_url.as_str().cyan().underlined(),
        ]));
        lines.push("".into());
        lines.push(
            "  2. Enter this one-time code after you are signed in (expires in 15 minutes)".into(),
        );
        lines.push("".into());
        lines.push(Line::from(vec![
            "  ".into(),
            user_code.as_str().cyan().bold(),
        ]));
        lines.push("".into());
        lines.push(
            "  Device codes are a common phishing target. Never share this code."
                .dim()
                .into(),
        );
        lines.push("".into());
        Some(verification_url.clone())
    } else {
        lines.push("  Requesting a one-time code...".dim().into());
        lines.push("".into());
        None
    };

    lines.push(Line::from(vec![
        "  Press ".dim(),
        widget.cancel_binding().into(),
        " to cancel".dim(),
    ]));
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(area, buf);

    if let Some(url) = &verification_url {
        mark_url_hyperlink(buf, area, url);
    }
}

fn device_code_attempt_matches(state: &SignInState, request_id: &str) -> bool {
    matches!(
        state,
        SignInState::ChatGptDeviceCode(state) if state.request_id == request_id
    )
}

fn set_device_code_state_for_active_attempt(
    sign_in_state: &std::sync::Arc<std::sync::RwLock<SignInState>>,
    request_frame: &crate::tui::FrameRequester,
    request_id: &str,
    next_state: ContinueWithDeviceCodeState,
) -> bool {
    let mut guard = sign_in_state.write().unwrap();
    if !device_code_attempt_matches(&guard, request_id) {
        return false;
    }

    *guard = SignInState::ChatGptDeviceCode(next_state);
    drop(guard);
    request_frame.schedule_frame();
    true
}

fn set_device_code_success_message_for_active_attempt(
    sign_in_state: &Arc<RwLock<SignInState>>,
    request_frame: &FrameRequester,
    auth_manager: &AuthManager,
    codex_home: &Path,
    cancel: &Arc<Notify>,
) -> bool {
    let mut guard = sign_in_state.write().unwrap();
    if !device_code_attempt_matches(&guard, request_id) {
        return false;
    }

    auth_manager.reload();
    // Clear model to use remote default for ChatGPT
    if let Err(e) = ConfigEditsBuilder::new(codex_home)
        .set_model(None, None, None)
        .apply_blocking()
    {
        tracing::error!("failed to clear model on ChatGPT login: {e}");
    }
    *guard = SignInState::ChatGptSuccessMessage;
    drop(guard);
    *error.write().unwrap() = Some(message);
    request_frame.schedule_frame();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use std::sync::RwLock;

    fn pending_device_code_state(request_id: &str) -> Arc<RwLock<SignInState>> {
        Arc::new(RwLock::new(SignInState::ChatGptDeviceCode(
            ContinueWithDeviceCodeState::pending(request_id.to_string()),
        )))
    }

    #[test]
    fn device_code_attempt_matches_only_for_matching_request_id() {
        let state = SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::pending(
            "request-1".to_string(),
        ));

        assert_eq!(device_code_attempt_matches(&state, "request-1"), true);
        assert_eq!(device_code_attempt_matches(&state, "request-2"), false);
        assert_eq!(
            device_code_attempt_matches(&SignInState::PickMode, "request-1"),
            false
        );
    }

    #[test]
    fn set_device_code_state_for_active_attempt_updates_only_when_active() {
        let request_frame = crate::tui::FrameRequester::test_dummy();
        let sign_in_state = pending_device_code_state("request-1");

        assert_eq!(
            set_device_code_state_for_active_attempt(
                &sign_in_state,
                &request_frame,
                "request-1",
                ContinueWithDeviceCodeState::ready(
                    "request-1".to_string(),
                    "login-1".to_string(),
                    "https://example.com/device".to_string(),
                    "ABCD-EFGH".to_string(),
                ),
            ),
            true
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::ChatGptDeviceCode(state) if state.login_id() == Some("login-1")
        ));

        let sign_in_state = pending_device_code_state("request-2");
        assert_eq!(
            set_device_code_state_for_active_attempt(
                &sign_in_state,
                &request_frame,
                "request-1",
                ContinueWithDeviceCodeState::ready(
                    "request-1".to_string(),
                    "login-1".to_string(),
                    "https://example.com/device".to_string(),
                    "ABCD-EFGH".to_string(),
                ),
            ),
            false
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::ChatGptDeviceCode(state) if state.login_id.is_none()
        ));
    }

    #[test]
    fn set_device_code_error_for_active_attempt_updates_only_when_active() {
        let request_frame = crate::tui::FrameRequester::test_dummy();
        let error = Arc::new(RwLock::new(None));
        let sign_in_state = pending_device_code_state("request-1");

        assert_eq!(
            set_device_code_error_for_active_attempt(
                &sign_in_state,
                &request_frame,
                &error,
                "request-1",
                "device code unavailable".to_string(),
            ),
            true
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::PickMode
        ));
        assert_eq!(
            error.read().unwrap().as_deref(),
            Some("device code unavailable")
        );

        let error = Arc::new(RwLock::new(None));
        let sign_in_state = pending_device_code_state("request-2");
        assert_eq!(
            set_device_code_error_for_active_attempt(
                &sign_in_state,
                &request_frame,
                &cancel,
                SignInState::PickMode,
            ),
            false
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::ChatGptDeviceCode(_)
        ));
    }

    #[test]
    fn set_device_code_success_message_for_active_attempt_updates_only_when_active() {
        let request_frame = FrameRequester::test_dummy();
        let cancel = Arc::new(Notify::new());
        let sign_in_state = device_code_sign_in_state(cancel.clone());
        let temp_dir = TempDir::new().unwrap();
        let codex_home = temp_dir.path().to_path_buf();
        let auth_manager =
            AuthManager::shared(codex_home.clone(), false, AuthCredentialsStoreMode::File);

        assert_eq!(
            set_device_code_success_message_for_active_attempt(
                &sign_in_state,
                &request_frame,
                &auth_manager,
                &codex_home,
                &cancel,
            ),
            true
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccessMessage
        ));

        let sign_in_state = device_code_sign_in_state(Arc::new(Notify::new()));
        assert_eq!(
            set_device_code_success_message_for_active_attempt(
                &sign_in_state,
                &request_frame,
                &auth_manager,
                &codex_home,
                &cancel,
            ),
            false
        );
        assert!(matches!(
            &*sign_in_state.read().unwrap(),
            SignInState::ChatGptDeviceCode(_)
        ));
        assert_eq!(*error.read().unwrap(), None);
    }
}
