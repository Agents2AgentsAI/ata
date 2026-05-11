use super::*;

// Duration before a browser ChatGPT login attempt is abandoned.
const LOGIN_CHATGPT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
#[cfg(debug_assertions)]
const LOGIN_ISSUER_OVERRIDE_ENV_VAR: &str = "CODEX_APP_SERVER_LOGIN_ISSUER";

enum ActiveLogin {
    Browser {
        shutdown_handle: ShutdownHandle,
        login_id: Uuid,
    },
    DeviceCode {
        cancel: CancellationToken,
        login_id: Uuid,
    },
}

impl ActiveLogin {
    fn login_id(&self) -> Uuid {
        match self {
            ActiveLogin::Browser { login_id, .. } | ActiveLogin::DeviceCode { login_id, .. } => {
                *login_id
            }
        }
    }

    fn cancel(&self) {
        match self {
            ActiveLogin::Browser {
                shutdown_handle, ..
            } => shutdown_handle.shutdown(),
            ActiveLogin::DeviceCode { cancel, .. } => cancel.cancel(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CancelLoginError {
    NotFound,
}

enum RefreshTokenRequestOutcome {
    NotAttemptedOrSucceeded,
    FailedTransiently,
    FailedPermanently,
}

impl Drop for ActiveLogin {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone)]
pub(crate) struct AccountRequestProcessor {
    auth_manager: Arc<AuthManager>,
    thread_manager: Arc<ThreadManager>,
    outgoing: Arc<OutgoingMessageSender>,
    config: Arc<Config>,
    config_manager: ConfigManager,
    active_login: Arc<Mutex<Option<ActiveLogin>>>,
}

impl AccountRequestProcessor {
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        thread_manager: Arc<ThreadManager>,
        outgoing: Arc<OutgoingMessageSender>,
        config: Arc<Config>,
        config_manager: ConfigManager,
    ) -> Self {
        Self {
            auth_manager,
            thread_manager,
            outgoing,
            config,
            config_manager,
            active_login: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) async fn login_account(
        &self,
        request_id: ConnectionRequestId,
        params: LoginAccountParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.login_v2(request_id, params).await.map(|()| None)
    }

    pub(crate) async fn logout_account(
        &self,
        request_id: ConnectionRequestId,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.logout_v2(request_id).await.map(|()| None)
    }

    pub(crate) async fn cancel_login_account(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.cancel_login_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn get_account(
        &self,
        params: GetAccountParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.get_account_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn get_auth_status(
        &self,
        params: GetAuthStatusParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.get_auth_status_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn get_account_rate_limits(
        &self,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.get_account_rate_limits_response()
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn send_add_credits_nudge_email(
        &self,
        params: SendAddCreditsNudgeEmailParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.send_add_credits_nudge_email_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn cancel_active_login(&self) {
        let mut guard = self.active_login.lock().await;
        if let Some(active_login) = guard.take() {
            drop(active_login);
        }
    }

    pub(crate) fn clear_external_auth(&self) {
        self.auth_manager.clear_external_auth();
    }

    fn current_account_updated_notification(&self) -> AccountUpdatedNotification {
        let auth = self.auth_manager.auth_cached();
        AccountUpdatedNotification {
            auth_mode: auth.as_ref().map(CodexAuth::api_auth_mode),
            plan_type: auth.as_ref().and_then(CodexAuth::account_plan_type),
        }
    }

    async fn maybe_refresh_remote_installed_plugins_cache_for_current_config(
        config_manager: &ConfigManager,
        thread_manager: &Arc<ThreadManager>,
        auth: Option<CodexAuth>,
    ) {
        match config_manager
            .load_latest_config(/*fallback_cwd*/ None)
            .await
        {
            Ok(config) => {
                let refresh_thread_manager = Arc::clone(thread_manager);
                let refresh_config_manager = config_manager.clone();
                thread_manager
                    .plugins_manager()
                    .maybe_start_remote_installed_plugins_cache_refresh(
                        &config.plugins_config_input(),
                        auth,
                        Some(Arc::new(move || {
                            Self::spawn_effective_plugins_changed_task(
                                Arc::clone(&refresh_thread_manager),
                                refresh_config_manager.clone(),
                            );
                        })),
                    );
            }
            Err(err) => {
                warn!(
                    "failed to reload config after account changed, skipping remote installed plugins cache refresh: {err}"
                );
            }
        }
    }

    fn spawn_effective_plugins_changed_task(
        thread_manager: Arc<ThreadManager>,
        config_manager: ConfigManager,
    ) {
        tokio::spawn(async move {
            thread_manager.plugins_manager().clear_cache();
            thread_manager.skills_manager().clear_cache();
            if thread_manager.list_thread_ids().await.is_empty() {
                return;
            }
            crate::mcp_refresh::queue_best_effort_refresh(&thread_manager, &config_manager).await;
        });
    }

    async fn login_v2(
        &self,
        request_id: ConnectionRequestId,
        params: LoginAccountParams,
    ) -> Result<(), JSONRPCErrorError> {
        match params {
            LoginAccountParams::ApiKey { api_key } => {
                self.login_api_key_v2(request_id, LoginApiKeyParams { api_key })
                    .await;
            }
            LoginAccountParams::Chatgpt {
                codex_streamlined_login,
            } => {
                self.login_chatgpt_v2(request_id, codex_streamlined_login)
                    .await;
            }
            LoginAccountParams::ChatgptDeviceCode => {
                self.login_chatgpt_device_code_v2(request_id).await;
            }
            LoginAccountParams::CopilotDeviceCode => {
                self.login_copilot_device_code_v2(request_id).await;
            }
            LoginAccountParams::ChatgptAuthTokens {
                access_token,
                chatgpt_account_id,
                chatgpt_plan_type,
            } => {
                self.login_chatgpt_auth_tokens(
                    request_id,
                    access_token,
                    chatgpt_account_id,
                    chatgpt_plan_type,
                )
                .await;
            }
            // === ATA: provider-specific login flows ===
            LoginAccountParams::ProviderApiKey {
                provider_id,
                api_key,
            } => {
                self.login_provider_api_key_v2(request_id, provider_id, api_key)
                    .await;
            }
            LoginAccountParams::GeminiOauth => {
                self.login_gemini_oauth_v2(request_id).await;
            }
            LoginAccountParams::AtaSendOtp { email } => {
                self.ata_send_otp_v2(request_id, email).await;
            }
            LoginAccountParams::AtaVerifyOtp { email, otp } => {
                self.ata_verify_otp_v2(request_id, email, otp).await;
            }
            LoginAccountParams::AtaLogout => {
                self.ata_logout_v2(request_id).await;
            }
        }
        Ok(())
    }

    fn external_auth_active_error(&self) -> JSONRPCErrorError {
        invalid_request(
            "External auth is active. Use account/login/start (chatgptAuthTokens) to update it or account/logout to clear it.",
        )
    }

    async fn login_api_key_common(
        &self,
        params: &LoginApiKeyParams,
    ) -> std::result::Result<(), JSONRPCErrorError> {
        if self.auth_manager.is_external_chatgpt_auth_active() {
            return Err(self.external_auth_active_error());
        }

        if matches!(
            self.config.forced_login_method,
            Some(ForcedLoginMethod::Chatgpt)
        ) {
            return Err(invalid_request(
                "API key login is disabled. Use ChatGPT login instead.",
            ));
        }

        // Cancel any active login attempt.
        {
            let mut guard = self.active_login.lock().await;
            if let Some(active) = guard.take() {
                drop(active);
            }
        }

        match login_with_api_key(
            &self.config.codex_home,
            &params.api_key,
            self.config.cli_auth_credentials_store_mode,
        ) {
            Ok(()) => {
                self.auth_manager.reload().await;
                Ok(())
            }
            Err(err) => Err(internal_error(format!("failed to save api key: {err}"))),
        }
    }

    async fn login_api_key_v2(&self, request_id: ConnectionRequestId, params: LoginApiKeyParams) {
        let result = self
            .login_api_key_common(&params)
            .await
            .map(|()| LoginAccountResponse::ApiKey {});
        let logged_in = result.is_ok();
        self.outgoing.send_result(request_id, result).await;

        if logged_in {
            self.send_login_success_notifications(/*login_id*/ None)
                .await;
        }
    }

    // Build options for a ChatGPT login attempt; performs validation.
    async fn login_chatgpt_common(
        &self,
        codex_streamlined_login: bool,
    ) -> std::result::Result<LoginServerOptions, JSONRPCErrorError> {
        let config = self.config.as_ref();

        if self.auth_manager.is_external_chatgpt_auth_active() {
            return Err(self.external_auth_active_error());
        }

        if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
            return Err(invalid_request(
                "ChatGPT login is disabled. Use API key login instead.",
            ));
        }

        let opts = LoginServerOptions {
            open_browser: false,
            codex_streamlined_login,
            ..LoginServerOptions::new(
                config.codex_home.to_path_buf(),
                CLIENT_ID.to_string(),
                config.forced_chatgpt_workspace_id.clone(),
                config.cli_auth_credentials_store_mode,
            )
        };
        #[cfg(debug_assertions)]
        let opts = {
            let mut opts = opts;
            if let Ok(issuer) = std::env::var(LOGIN_ISSUER_OVERRIDE_ENV_VAR)
                && !issuer.trim().is_empty()
            {
                opts.issuer = issuer;
            }
            opts
        };

        Ok(opts)
    }

    fn login_chatgpt_device_code_start_error(err: IoError) -> JSONRPCErrorError {
        let is_not_found = err.kind() == std::io::ErrorKind::NotFound;
        if is_not_found {
            invalid_request(err.to_string())
        } else {
            internal_error(format!("failed to request device code: {err}"))
        }
    }

    async fn login_chatgpt_v2(
        &self,
        request_id: ConnectionRequestId,
        codex_streamlined_login: bool,
    ) {
        let result = self.login_chatgpt_response(codex_streamlined_login).await;
        self.outgoing.send_result(request_id, result).await;
    }

    async fn login_chatgpt_response(
        &self,
        codex_streamlined_login: bool,
    ) -> Result<LoginAccountResponse, JSONRPCErrorError> {
        let opts = self.login_chatgpt_common(codex_streamlined_login).await?;
        let server = run_login_server(opts)
            .map_err(|err| internal_error(format!("failed to start login server: {err}")))?;
        let login_id = Uuid::new_v4();
        let shutdown_handle = server.cancel_handle();

        // Replace active login if present.
        {
            let mut guard = self.active_login.lock().await;
            if let Some(existing) = guard.take() {
                drop(existing);
            }
            *guard = Some(ActiveLogin::Browser {
                shutdown_handle: shutdown_handle.clone(),
                login_id,
            });
        }

        let outgoing_clone = self.outgoing.clone();
        let config_manager = self.config_manager.clone();
        let thread_manager = Arc::clone(&self.thread_manager);
        let chatgpt_base_url = self.config.chatgpt_base_url.clone();
        let active_login = self.active_login.clone();
        let auth_url = server.auth_url.clone();
        tokio::spawn(async move {
            let (success, error_msg) = match tokio::time::timeout(
                LOGIN_CHATGPT_TIMEOUT,
                server.block_until_done(),
            )
            .await
            {
                Ok(Ok(())) => (true, None),
                Ok(Err(err)) => (false, Some(format!("Login server error: {err}"))),
                Err(_elapsed) => {
                    shutdown_handle.shutdown();
                    (false, Some("Login timed out".to_string()))
                }
            };

            Self::send_chatgpt_login_completion_notifications(
                &outgoing_clone,
                config_manager,
                thread_manager,
                chatgpt_base_url,
                login_id,
                success,
                error_msg,
            )
            .await;

            // Clear the active login if it matches this attempt. It may have been replaced or cancelled.
            let mut guard = active_login.lock().await;
            if guard.as_ref().map(ActiveLogin::login_id) == Some(login_id) {
                *guard = None;
            }
        });

        Ok(LoginAccountResponse::Chatgpt {
            login_id: login_id.to_string(),
            auth_url,
        })
    }

    async fn login_chatgpt_device_code_v2(&self, request_id: ConnectionRequestId) {
        let result = self.login_chatgpt_device_code_response().await;
        self.outgoing.send_result(request_id, result).await;
    }

    async fn login_copilot_device_code_v2(&self, request_id: ConnectionRequestId) {
        let result = self.login_copilot_device_code_response().await;
        self.outgoing.send_result(request_id, result).await;
    }

    async fn login_copilot_device_code_response(
        &self,
    ) -> Result<LoginAccountResponse, JSONRPCErrorError> {
        // Cancel any active login (mirrors the ChatGPT device-code path).
        {
            let mut guard = self.active_login.lock().await;
            if let Some(existing) = guard.take() {
                drop(existing);
            }
        }

        let device = start_copilot_device_flow()
            .await
            .map_err(|err| internal_error(format!("failed to start GitHub device flow: {err}")))?;

        let login_id = Uuid::new_v4();
        let cancel = CancellationToken::new();

        {
            let mut guard = self.active_login.lock().await;
            *guard = Some(ActiveLogin::DeviceCode {
                cancel: cancel.clone(),
                login_id,
            });
        }

        let user_code = device.user_code.clone();
        let verification_uri = device.verification_uri.clone();

        let outgoing_clone = self.outgoing.clone();
        let auth_manager = Arc::clone(&self.auth_manager);
        let codex_home = self.config.codex_home.clone();
        let store_mode = self.config.cli_auth_credentials_store_mode;
        let active_login = self.active_login.clone();
        let device_for_task = device.clone();
        // Captured so the spawned completion task can rebuild the
        // models_manager once `model_provider = "copilot"` lands in
        // config.toml. Without this, the picker keeps showing the OpenAI
        // catalog this session — only the next launch picks up Copilot.
        let thread_manager_for_task = Arc::clone(&self.thread_manager);
        let config_manager_for_task = self.config_manager.clone();

        tokio::spawn(async move {
            let (mut success, mut error_msg) = tokio::select! {
                _ = cancel.cancelled() => {
                    (false, Some("Login was not completed".to_string()))
                }
                result = async {
                    let token = poll_copilot_access_token(&device_for_task).await?;
                    complete_copilot_login(&codex_home, store_mode, token).await
                } => {
                    match result {
                        Ok(()) => (true, None),
                        Err(err) => (false, Some(err.to_string())),
                    }
                }
            };

            if success {
                // Persist `model = "gpt-4.1"` and `model_provider = "copilot"`
                // so the next launch defaults to Copilot without manual
                // `-c model=...` flags. `gpt-4.1` is GitHub's current
                // recommended default and appears in the bundled Copilot
                // catalog (see codex-models-manager/copilot_models.json) —
                // `gpt-4o` is no longer listed on
                // https://docs.github.com/en/copilot/reference/ai-models/supported-models
                // so newly logged-in users were landing on a slug that
                // wasn't in the /model picker.
                //
                // If this write fails we *must* roll back the just-saved
                // Copilot OAuth credential. Leaving creds without the
                // matching provider config strands the user: the next launch
                // re-enters bootstrap with `model_provider = "openai"`,
                // `requires_openai_auth = true`, and an auth.json holding
                // only `providers.copilot` — which used to surface as
                // "email and plan type are required for chatgpt
                // authentication" during TUI bootstrap.
                let codex_home_for_edit = codex_home.clone();
                let edit_result = tokio::task::spawn_blocking(move || {
                    ConfigEditsBuilder::new(&codex_home_for_edit)
                        .set_model(Some("gpt-4.1"), None, Some("copilot".to_string()))
                        .apply_blocking()
                })
                .await;
                let edit_err = match edit_result {
                    Ok(Ok(())) => None,
                    Ok(Err(err)) => Some(err.to_string()),
                    Err(join_err) => Some(format!("config edit task panicked: {join_err}")),
                };
                if let Some(err) = edit_err {
                    warn!(
                        "failed to persist model_provider=copilot to config.toml after login: {err}"
                    );
                    if let Err(rollback_err) = copilot_logout(&codex_home, store_mode) {
                        warn!(
                            "failed to roll back Copilot credentials after config write failure: {rollback_err}"
                        );
                    }
                    success = false;
                    error_msg = Some(format!(
                        "Signed in to GitHub Copilot but could not update config: {err}. Please retry."
                    ));
                }

                // Reload regardless: on success the manager picks up new
                // creds; on rollback it picks up the now-empty auth.json.
                auth_manager.reload().await;

                // === ATA: rebuild the in-memory models catalog so the
                // `/model` picker shows Copilot models *this* session,
                // instead of waiting for the next ata launch. Re-reads
                // config.toml (now `model_provider = "copilot"`) and
                // installs a fresh `models_manager` on the live
                // `ThreadManager`. Best-effort: failures here are
                // non-fatal (the picker will catch up on relaunch).
                if success {
                    match config_manager_for_task
                        .load_latest_config(/*fallback_cwd*/ None)
                        .await
                    {
                        Ok(fresh_config) => {
                            let new_manager = codex_core::build_models_manager(
                                &fresh_config,
                                Arc::clone(&auth_manager),
                            );
                            thread_manager_for_task.set_models_manager(new_manager);
                        }
                        Err(err) => {
                            warn!("failed to reload config for models_manager refresh: {err}");
                        }
                    }
                }
            }

            outgoing_clone
                .send_server_notification(ServerNotification::AccountLoginCompleted(
                    AccountLoginCompletedNotification {
                        login_id: Some(login_id.to_string()),
                        success,
                        error: error_msg,
                    },
                ))
                .await;

            let mut guard = active_login.lock().await;
            if guard.as_ref().map(ActiveLogin::login_id) == Some(login_id) {
                *guard = None;
            }
        });

        Ok(LoginAccountResponse::CopilotDeviceCode {
            login_id: login_id.to_string(),
            verification_uri,
            user_code,
        })
    }

    async fn login_chatgpt_device_code_response(
        &self,
    ) -> Result<LoginAccountResponse, JSONRPCErrorError> {
        let opts = self
            .login_chatgpt_common(/*codex_streamlined_login*/ false)
            .await?;
        let device_code = request_device_code(&opts)
            .await
            .map_err(Self::login_chatgpt_device_code_start_error)?;
        let login_id = Uuid::new_v4();
        let cancel = CancellationToken::new();

        {
            let mut guard = self.active_login.lock().await;
            if let Some(existing) = guard.take() {
                drop(existing);
            }
            *guard = Some(ActiveLogin::DeviceCode {
                cancel: cancel.clone(),
                login_id,
            });
        }

        let verification_url = device_code.verification_url.clone();
        let user_code = device_code.user_code.clone();

        let outgoing_clone = self.outgoing.clone();
        let config_manager = self.config_manager.clone();
        let thread_manager = Arc::clone(&self.thread_manager);
        let chatgpt_base_url = self.config.chatgpt_base_url.clone();
        let active_login = self.active_login.clone();
        tokio::spawn(async move {
            let (success, error_msg) = tokio::select! {
                _ = cancel.cancelled() => {
                    (false, Some("Login was not completed".to_string()))
                }
                r = complete_device_code_login(opts, device_code) => {
                    match r {
                        Ok(()) => (true, None),
                        Err(err) => (false, Some(err.to_string())),
                    }
                }
            };

            Self::send_chatgpt_login_completion_notifications(
                &outgoing_clone,
                config_manager,
                thread_manager,
                chatgpt_base_url,
                login_id,
                success,
                error_msg,
            )
            .await;

            let mut guard = active_login.lock().await;
            if guard.as_ref().map(ActiveLogin::login_id) == Some(login_id) {
                *guard = None;
            }
        });

        Ok(LoginAccountResponse::ChatgptDeviceCode {
            login_id: login_id.to_string(),
            verification_url,
            user_code,
        })
    }

    async fn cancel_login_chatgpt_common(
        &self,
        login_id: Uuid,
    ) -> std::result::Result<(), CancelLoginError> {
        let mut guard = self.active_login.lock().await;
        if guard.as_ref().map(ActiveLogin::login_id) == Some(login_id) {
            if let Some(active) = guard.take() {
                drop(active);
            }
            Ok(())
        } else {
            Err(CancelLoginError::NotFound)
        }
    }

    async fn cancel_login_response(
        &self,
        params: CancelLoginAccountParams,
    ) -> Result<CancelLoginAccountResponse, JSONRPCErrorError> {
        let login_id = params.login_id;
        let uuid = Uuid::parse_str(&login_id)
            .map_err(|_| invalid_request(format!("invalid login id: {login_id}")))?;
        let status = match self.cancel_login_chatgpt_common(uuid).await {
            Ok(()) => CancelLoginAccountStatus::Canceled,
            Err(CancelLoginError::NotFound) => CancelLoginAccountStatus::NotFound,
        };
        Ok(CancelLoginAccountResponse { status })
    }

    async fn login_chatgpt_auth_tokens(
        &self,
        request_id: ConnectionRequestId,
        access_token: String,
        chatgpt_account_id: String,
        chatgpt_plan_type: Option<String>,
    ) {
        let result = self
            .login_chatgpt_auth_tokens_response(access_token, chatgpt_account_id, chatgpt_plan_type)
            .await;
        let logged_in = result.is_ok();
        self.outgoing.send_result(request_id, result).await;

        if logged_in {
            self.send_login_success_notifications(/*login_id*/ None)
                .await;
        }
    }

    async fn login_chatgpt_auth_tokens_response(
        &self,
        access_token: String,
        chatgpt_account_id: String,
        chatgpt_plan_type: Option<String>,
    ) -> Result<LoginAccountResponse, JSONRPCErrorError> {
        if matches!(
            self.config.forced_login_method,
            Some(ForcedLoginMethod::Api)
        ) {
            return Err(invalid_request(
                "External ChatGPT auth is disabled. Use API key login instead.",
            ));
        }

        // Cancel any active login attempt to avoid persisting managed auth state.
        {
            let mut guard = self.active_login.lock().await;
            if let Some(active) = guard.take() {
                drop(active);
            }
        }

        if let Some(expected_workspace) = self.config.forced_chatgpt_workspace_id.as_deref()
            && chatgpt_account_id != expected_workspace
        {
            return Err(invalid_request(format!(
                "External auth must use workspace {expected_workspace}, but received {chatgpt_account_id:?}."
            )));
        }

        login_with_chatgpt_auth_tokens(
            &self.config.codex_home,
            &access_token,
            &chatgpt_account_id,
            chatgpt_plan_type.as_deref(),
        )
        .map_err(|err| internal_error(format!("failed to set external auth: {err}")))?;
        self.auth_manager.reload().await;
        self.config_manager.replace_cloud_requirements_loader(
            self.auth_manager.clone(),
            self.config.chatgpt_base_url.clone(),
        );
        self.config_manager
            .sync_default_client_residency_requirement()
            .await;

        Ok(LoginAccountResponse::ChatgptAuthTokens {})
    }

    async fn send_login_success_notifications(&self, login_id: Option<Uuid>) {
        Self::maybe_refresh_remote_installed_plugins_cache_for_current_config(
            &self.config_manager,
            &self.thread_manager,
            self.auth_manager.auth_cached(),
        )
        .await;

        let payload_login_completed = AccountLoginCompletedNotification {
            login_id: login_id.map(|id| id.to_string()),
            success: true,
            error: None,
        };
        self.outgoing
            .send_server_notification(ServerNotification::AccountLoginCompleted(
                payload_login_completed,
            ))
            .await;

        self.outgoing
            .send_server_notification(ServerNotification::AccountUpdated(
                self.current_account_updated_notification(),
            ))
            .await;
    }

    async fn send_chatgpt_login_completion_notifications(
        outgoing: &OutgoingMessageSender,
        config_manager: ConfigManager,
        thread_manager: Arc<ThreadManager>,
        chatgpt_base_url: String,
        login_id: Uuid,
        success: bool,
        error_msg: Option<String>,
    ) {
        let payload_v2 = AccountLoginCompletedNotification {
            login_id: Some(login_id.to_string()),
            success,
            error: error_msg,
        };
        outgoing
            .send_server_notification(ServerNotification::AccountLoginCompleted(payload_v2))
            .await;

        if success {
            let auth_manager = thread_manager.auth_manager();
            auth_manager.reload().await;
            config_manager
                .replace_cloud_requirements_loader(auth_manager.clone(), chatgpt_base_url);
            config_manager
                .sync_default_client_residency_requirement()
                .await;

            let auth = auth_manager.auth_cached();
            Self::maybe_refresh_remote_installed_plugins_cache_for_current_config(
                &config_manager,
                &thread_manager,
                auth.clone(),
            )
            .await;
            let payload_v2 = AccountUpdatedNotification {
                auth_mode: auth.as_ref().map(CodexAuth::api_auth_mode),
                plan_type: auth.as_ref().and_then(CodexAuth::account_plan_type),
            };
            outgoing
                .send_server_notification(ServerNotification::AccountUpdated(payload_v2))
                .await;
        }
    }

    async fn logout_common(&self) -> std::result::Result<Option<AuthMode>, JSONRPCErrorError> {
        // Cancel any active login attempt.
        {
            let mut guard = self.active_login.lock().await;
            if let Some(active) = guard.take() {
                drop(active);
            }
        }

        match self.auth_manager.logout_with_revoke().await {
            Ok(_) => {}
            Err(err) => {
                return Err(internal_error(format!("logout failed: {err}")));
            }
        }

        // For providers that don't use OpenAI-style auth (today: GitHub
        // Copilot), the TUI's login screen is gated behind
        // `requires_openai_auth`. Leaving `model_provider = "copilot"` in
        // config.toml after logout means the next launch skips onboarding
        // entirely and drops the user into chat with no credentials — they
        // observe this as "/logout didn't log me out". Clearing `model` and
        // `model_provider` here lets the default provider take over so the
        // login screen surfaces and the user can re-pick their provider
        // (including Copilot again).
        if !self.config.model_provider.requires_openai_auth {
            let codex_home = self.config.codex_home.clone();
            let edit_result = tokio::task::spawn_blocking(move || {
                ConfigEditsBuilder::new(&codex_home)
                    .clear_model_and_provider()
                    .apply_blocking()
            })
            .await;
            match edit_result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    warn!(
                        "failed to clear model/model_provider from config.toml after logout: {err}"
                    );
                }
                Err(join_err) => {
                    warn!("config-clear task panicked during logout: {join_err}");
                }
            }
        }

        Self::maybe_refresh_remote_installed_plugins_cache_for_current_config(
            &self.config_manager,
            &self.thread_manager,
            self.auth_manager.auth_cached(),
        )
        .await;

        // === ATA: rebuild the in-memory models catalog so a Copilot
        // logout (which just cleared `model_provider="copilot"`) reverts
        // the `/model` picker to the default OpenAI catalog this session.
        // Best-effort — failures here are non-fatal.
        match self
            .config_manager
            .load_latest_config(/*fallback_cwd*/ None)
            .await
        {
            Ok(fresh_config) => {
                let new_manager =
                    codex_core::build_models_manager(&fresh_config, Arc::clone(&self.auth_manager));
                self.thread_manager.set_models_manager(new_manager);
            }
            Err(err) => {
                warn!("failed to reload config for models_manager refresh after logout: {err}");
            }
        }

        // Reflect the current auth method after logout (likely None).
        Ok(self
            .auth_manager
            .auth_cached()
            .as_ref()
            .map(CodexAuth::api_auth_mode))
    }

    async fn logout_v2(&self, request_id: ConnectionRequestId) -> Result<(), JSONRPCErrorError> {
        let result = self.logout_common().await;
        let account_updated =
            result
                .as_ref()
                .ok()
                .cloned()
                .map(|auth_mode| AccountUpdatedNotification {
                    auth_mode,
                    plan_type: None,
                });
        self.outgoing
            .send_result(request_id, result.map(|_| LogoutAccountResponse {}))
            .await;

        if let Some(payload) = account_updated {
            self.outgoing
                .send_server_notification(ServerNotification::AccountUpdated(payload))
                .await;
        }
        Ok(())
    }

    async fn refresh_token_if_requested(&self, do_refresh: bool) -> RefreshTokenRequestOutcome {
        if self.auth_manager.is_external_chatgpt_auth_active() {
            return RefreshTokenRequestOutcome::NotAttemptedOrSucceeded;
        }
        if do_refresh && let Err(err) = self.auth_manager.refresh_token().await {
            let failed_reason = err.failed_reason();
            if failed_reason.is_none() {
                tracing::warn!("failed to refresh token while getting account: {err}");
                return RefreshTokenRequestOutcome::FailedTransiently;
            }
            return RefreshTokenRequestOutcome::FailedPermanently;
        }
        RefreshTokenRequestOutcome::NotAttemptedOrSucceeded
    }

    async fn get_auth_status_response(
        &self,
        params: GetAuthStatusParams,
    ) -> Result<GetAuthStatusResponse, JSONRPCErrorError> {
        let include_token = params.include_token.unwrap_or(false);
        let do_refresh = params.refresh_token.unwrap_or(false);

        self.refresh_token_if_requested(do_refresh).await;

        // Determine whether auth is required based on the active model provider.
        // If a custom provider is configured with `requires_openai_auth == false`,
        // then no auth step is required; otherwise, default to requiring auth.
        let requires_openai_auth = self.config.model_provider.requires_openai_auth;

        let response = if !requires_openai_auth {
            GetAuthStatusResponse {
                auth_method: None,
                auth_token: None,
                requires_openai_auth: Some(false),
            }
        } else {
            let auth = if do_refresh {
                self.auth_manager.auth_cached()
            } else {
                self.auth_manager.auth().await
            };
            match auth {
                Some(auth) => {
                    let permanent_refresh_failure =
                        self.auth_manager.refresh_failure_for_auth(&auth).is_some();
                    let auth_mode = auth.api_auth_mode();
                    let (reported_auth_method, token_opt) =
                        if matches!(auth, CodexAuth::AgentIdentity(_))
                            || include_token && permanent_refresh_failure
                        {
                            (Some(auth_mode), None)
                        } else {
                            match auth.get_token() {
                                Ok(token) if !token.is_empty() => {
                                    let tok = if include_token { Some(token) } else { None };
                                    (Some(auth_mode), tok)
                                }
                                Ok(_) => (None, None),
                                Err(err) => {
                                    tracing::warn!("failed to get token for auth status: {err}");
                                    (None, None)
                                }
                            }
                        };
                    GetAuthStatusResponse {
                        auth_method: reported_auth_method,
                        auth_token: token_opt,
                        requires_openai_auth: Some(true),
                    }
                }
                None => GetAuthStatusResponse {
                    auth_method: None,
                    auth_token: None,
                    requires_openai_auth: Some(true),
                },
            }
        };

        Ok(response)
    }

    async fn get_account_response(
        &self,
        params: GetAccountParams,
    ) -> Result<GetAccountResponse, JSONRPCErrorError> {
        let do_refresh = params.refresh_token;

        self.refresh_token_if_requested(do_refresh).await;

        let provider = create_model_provider(
            self.config.model_provider.clone(),
            Some(self.auth_manager.clone()),
        );
        let account_state = match provider.account_state() {
            Ok(account_state) => account_state,
            Err(ProviderAccountError::MissingChatgptAccountDetails) => {
                return Err(invalid_request(
                    "email and plan type are required for chatgpt authentication",
                ));
            }
        };
        let mut account = account_state.account.map(Account::from);

        // The standard ChatGPT/ApiKey/Bedrock account_state path doesn't know
        // about Copilot. If the active provider is Copilot and the OAuth
        // credential is present, surface that as the active account so the
        // TUI's "Signed in with GitHub Copilot" view survives a restart.
        if account.is_none()
            && matches!(
                self.config.model_provider.wire_api,
                codex_model_provider_info::WireApi::CopilotInline
            )
            && get_provider_oauth_credential(
                &self.config.codex_home,
                PROVIDER_COPILOT,
                self.config.cli_auth_credentials_store_mode,
            )
            .is_some()
        {
            account = Some(Account::Copilot {});
        }

        Ok(GetAccountResponse {
            account,
            requires_openai_auth: account_state.requires_openai_auth,
        })
    }

    async fn get_account_rate_limits_response(
        &self,
    ) -> Result<GetAccountRateLimitsResponse, JSONRPCErrorError> {
        self.fetch_account_rate_limits()
            .await
            .map(
                |(rate_limits, rate_limits_by_limit_id)| GetAccountRateLimitsResponse {
                    rate_limits: rate_limits.into(),
                    rate_limits_by_limit_id: Some(
                        rate_limits_by_limit_id
                            .into_iter()
                            .map(|(limit_id, snapshot)| (limit_id, snapshot.into()))
                            .collect(),
                    ),
                },
            )
    }

    async fn send_add_credits_nudge_email_response(
        &self,
        params: SendAddCreditsNudgeEmailParams,
    ) -> Result<SendAddCreditsNudgeEmailResponse, JSONRPCErrorError> {
        self.send_add_credits_nudge_email_inner(params)
            .await
            .map(|status| SendAddCreditsNudgeEmailResponse { status })
    }

    async fn send_add_credits_nudge_email_inner(
        &self,
        params: SendAddCreditsNudgeEmailParams,
    ) -> Result<AddCreditsNudgeEmailStatus, JSONRPCErrorError> {
        let Some(auth) = self.auth_manager.auth().await else {
            return Err(invalid_request(
                "codex account authentication required to notify workspace owner",
            ));
        };

        if !auth.uses_codex_backend() {
            return Err(invalid_request(
                "chatgpt authentication required to notify workspace owner",
            ));
        }

        let client = BackendClient::from_auth(self.config.chatgpt_base_url.clone(), &auth)
            .map_err(|err| internal_error(format!("failed to construct backend client: {err}")))?;

        match client
            .send_add_credits_nudge_email(Self::backend_credit_type(params.credit_type))
            .await
        {
            Ok(()) => Ok(AddCreditsNudgeEmailStatus::Sent),
            Err(err) if err.status().is_some_and(|status| status.as_u16() == 429) => {
                Ok(AddCreditsNudgeEmailStatus::CooldownActive)
            }
            Err(err) => Err(internal_error(format!(
                "failed to notify workspace owner: {err}"
            ))),
        }
    }

    fn backend_credit_type(value: AddCreditsNudgeCreditType) -> BackendAddCreditsNudgeCreditType {
        match value {
            AddCreditsNudgeCreditType::Credits => BackendAddCreditsNudgeCreditType::Credits,
            AddCreditsNudgeCreditType::UsageLimit => BackendAddCreditsNudgeCreditType::UsageLimit,
        }
    }

    async fn fetch_account_rate_limits(
        &self,
    ) -> Result<
        (
            CoreRateLimitSnapshot,
            HashMap<String, CoreRateLimitSnapshot>,
        ),
        JSONRPCErrorError,
    > {
        let Some(auth) = self.auth_manager.auth().await else {
            return Err(invalid_request(
                "codex account authentication required to read rate limits",
            ));
        };

        if !auth.uses_codex_backend() {
            return Err(invalid_request(
                "chatgpt authentication required to read rate limits",
            ));
        }

        let client = BackendClient::from_auth(self.config.chatgpt_base_url.clone(), &auth)
            .map_err(|err| internal_error(format!("failed to construct backend client: {err}")))?;

        let snapshots = client
            .get_rate_limits_many()
            .await
            .map_err(|err| internal_error(format!("failed to fetch codex rate limits: {err}")))?;
        if snapshots.is_empty() {
            return Err(internal_error(
                "failed to fetch codex rate limits: no snapshots returned",
            ));
        }

        let rate_limits_by_limit_id: HashMap<String, CoreRateLimitSnapshot> = snapshots
            .iter()
            .cloned()
            .map(|snapshot| {
                let limit_id = snapshot
                    .limit_id
                    .clone()
                    .unwrap_or_else(|| "codex".to_string());
                (limit_id, snapshot)
            })
            .collect();

        let primary = snapshots
            .iter()
            .find(|snapshot| snapshot.limit_id.as_deref() == Some("codex"))
            .cloned()
            .unwrap_or_else(|| snapshots[0].clone());

        Ok((primary, rate_limits_by_limit_id))
    }
}

// === ATA: provider-specific login flows ===
//
// Handlers backing the four-option onboarding picker: per-provider API key
// (OpenAI/Anthropic/Gemini/Copilot), Gemini OAuth (Code Assist), and the
// Supabase email-OTP "Sign in with ATA account" flow.
//
// Kept in a separate impl block so future upstream changes to the main
// `impl AccountRequestProcessor` block above merge cleanly. Each `_v2`
// method follows the same convention as the existing `login_*_v2` methods:
// run the underlying work, send a JSON-RPC result, and fire
// `AccountUpdated` / `AccountLoginCompleted` notifications where relevant.
impl AccountRequestProcessor {
    async fn login_provider_api_key_v2(
        &self,
        request_id: ConnectionRequestId,
        provider_id: String,
        api_key: String,
    ) {
        let result = self
            .login_provider_api_key_common(provider_id.as_str(), api_key.as_str())
            .await
            .map(|()| LoginAccountResponse::ProviderApiKey {});
        let logged_in = result.is_ok();
        self.outgoing.send_result(request_id, result).await;

        if logged_in {
            self.send_login_success_notifications(/*login_id*/ None)
                .await;
        }
    }

    async fn login_provider_api_key_common(
        &self,
        provider_id: &str,
        api_key: &str,
    ) -> std::result::Result<(), JSONRPCErrorError> {
        if api_key.trim().is_empty() {
            return Err(invalid_request("API key must not be empty."));
        }
        match login_with_provider_api_key(
            &self.config.codex_home,
            provider_id,
            api_key,
            self.config.cli_auth_credentials_store_mode,
        ) {
            Ok(()) => {
                self.auth_manager.reload().await;
                Ok(())
            }
            Err(err) => Err(internal_error(format!(
                "failed to save {provider_id} api key: {err}"
            ))),
        }
    }

    async fn login_gemini_oauth_v2(&self, request_id: ConnectionRequestId) {
        // Boots the Gemini Code Assist OAuth callback server. The TUI opens
        // the returned `auth_url` in a browser; the user finishes the OAuth
        // flow there; the server persists the resulting credential and the
        // task below fires `AccountLoginCompleted` with the matching
        // `login_id`. Mirrors the Copilot device-code path.
        let opts = codex_login::GeminiServerOptions::new(
            self.config.codex_home.to_path_buf(),
            self.config.cli_auth_credentials_store_mode,
        );
        let server = match codex_login::run_gemini_login_server(opts) {
            Ok(server) => server,
            Err(err) => {
                let resp: Result<LoginAccountResponse, JSONRPCErrorError> = Err(internal_error(
                    format!("failed to start Gemini OAuth server: {err}"),
                ));
                self.outgoing.send_result(request_id, resp).await;
                return;
            }
        };

        let login_id = Uuid::new_v4();
        let auth_url = server.auth_url.clone();
        let response = LoginAccountResponse::GeminiOauthContinueInBrowser {
            login_id: login_id.to_string(),
            auth_url: auth_url.clone(),
        };
        let result: Result<LoginAccountResponse, JSONRPCErrorError> = Ok(response);
        self.outgoing.send_result(request_id, result).await;

        // Drive the server to completion in a background task so the request
        // returns immediately. Completion / failure fires
        // AccountLoginCompletedNotification with the same login_id.
        let outgoing = self.outgoing.clone();
        tokio::spawn(async move {
            let (success, error_msg) = match server.block_until_done().await {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err.to_string())),
            };
            outgoing
                .send_server_notification(ServerNotification::AccountLoginCompleted(
                    AccountLoginCompletedNotification {
                        login_id: Some(login_id.to_string()),
                        success,
                        error: error_msg,
                    },
                ))
                .await;
        });
    }

    async fn ata_send_otp_v2(&self, request_id: ConnectionRequestId, email: String) {
        let result = self
            .ata_send_otp_common(email.as_str())
            .await
            .map(|()| LoginAccountResponse::AtaSendOtp {});
        self.outgoing.send_result(request_id, result).await;
    }

    async fn ata_send_otp_common(&self, email: &str) -> std::result::Result<(), JSONRPCErrorError> {
        if email.trim().is_empty() {
            return Err(invalid_request("Email must not be empty."));
        }
        let ata_config = ata_account_config_from_env_or_default();
        let client = SupabaseClient::new(ata_config.supabase_url, ata_config.supabase_anon_key);
        let auth = SupabaseAuth::new(client);
        match auth.sign_in_with_otp(email).await {
            Ok(()) => Ok(()),
            Err(SupabaseError::Api { status, message }) => Err(invalid_request(format!(
                "Supabase rejected OTP request ({status}): {message}"
            ))),
            Err(err) => Err(internal_error(format!("failed to send OTP: {err}"))),
        }
    }

    async fn ata_verify_otp_v2(&self, request_id: ConnectionRequestId, email: String, otp: String) {
        let result = self
            .ata_verify_otp_common(email.as_str(), otp.as_str())
            .await
            .map(|email| LoginAccountResponse::AtaVerifyOtp { email });
        let verified = result.is_ok();
        self.outgoing.send_result(request_id, result).await;
        if verified {
            // Reuse the existing account-updated emit so the TUI's
            // /account view and onboarding success screen see the new
            // ATA session immediately.
            let payload = self.current_account_updated_notification();
            self.outgoing
                .send_server_notification(ServerNotification::AccountUpdated(payload))
                .await;
        }
    }

    async fn ata_verify_otp_common(
        &self,
        email: &str,
        otp: &str,
    ) -> std::result::Result<String, JSONRPCErrorError> {
        if email.trim().is_empty() {
            return Err(invalid_request("Email must not be empty."));
        }
        if otp.trim().is_empty() {
            return Err(invalid_request("OTP must not be empty."));
        }
        let ata_config = ata_account_config_from_env_or_default();
        let client = SupabaseClient::new(ata_config.supabase_url, ata_config.supabase_anon_key);
        let auth = SupabaseAuth::new(client);
        match auth.exchange_code_for_session(email, otp).await {
            Ok(session) => {
                let user_email = session.user.email.clone();
                if let Err(err) = save_ata_session(&self.config.codex_home, &session) {
                    return Err(internal_error(format!(
                        "failed to persist ATA session: {err}"
                    )));
                }
                Ok(user_email)
            }
            Err(SupabaseError::Api { status, message }) => Err(invalid_request(format!(
                "Supabase rejected OTP ({status}): {message}"
            ))),
            Err(err) => Err(internal_error(format!("failed to verify OTP: {err}"))),
        }
    }

    async fn ata_logout_v2(&self, request_id: ConnectionRequestId) {
        let result = match delete_ata_session(&self.config.codex_home) {
            Ok(_) => Ok(LoginAccountResponse::AtaLogout {}),
            Err(err) => Err(internal_error(format!(
                "failed to clear ATA session: {err}"
            ))),
        };
        self.outgoing.send_result(request_id, result).await;
    }
}

/// Resolve the active `AtaAccountConfig` for ATA Supabase calls.
///
/// `AtaAccountConfig::default()` returns the public supabase project that
/// ships with the binary. Once we plumb the field through `Config` proper
/// (it lives under `config.toml` as `[ata_account]`) this helper can read
/// from `self.config` instead.
fn ata_account_config_from_env_or_default() -> AtaAccountConfig {
    AtaAccountConfig::default()
}
