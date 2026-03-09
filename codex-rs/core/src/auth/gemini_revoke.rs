use super::*;

pub(super) fn revoke_gemini_oauth_tokens_for_store(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) {
    let storage = create_auth_storage(codex_home.to_path_buf(), auth_credentials_store_mode);
    let refresh_token = match storage.load() {
        Ok(Some(auth_dot_json)) => auth_dot_json
            .get_provider_oauth_credential(PROVIDER_GEMINI)
            .map(|credential| credential.refresh)
            .filter(|token| !token.trim().is_empty()),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                mode = ?auth_credentials_store_mode,
                error = %err,
                "Failed to load auth storage while preparing Gemini OAuth revocation"
            );
            None
        }
    };

    if let Some(refresh_token) = refresh_token
        && let Err(err) = revoke_gemini_refresh_token(&refresh_token)
    {
        tracing::warn!(
            mode = ?auth_credentials_store_mode,
            error = %err,
            "Failed to revoke Gemini OAuth refresh token during logout; proceeding with local credential removal"
        );
    }
}

fn gemini_oauth_revoke_endpoint() -> String {
    std::env::var(GEMINI_OAUTH_REVOKE_URL_OVERRIDE_ENV_VAR)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| GEMINI_OAUTH_REVOKE_URL.to_string())
}

fn revoke_gemini_refresh_token(refresh_token: &str) -> std::io::Result<()> {
    let trimmed_token = refresh_token.trim();
    if trimmed_token.is_empty() {
        return Ok(());
    }

    let endpoint = gemini_oauth_revoke_endpoint();
    let body = format!(
        "token={token}&token_type_hint=refresh_token",
        token = urlencoding::encode(trimmed_token)
    );
    std::thread::spawn(move || revoke_gemini_refresh_token_with_client(endpoint, body))
        .join()
        .map_err(|_| std::io::Error::other("revoke request thread panicked"))?
}

fn revoke_gemini_refresh_token_with_client(endpoint: String, body: String) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| std::io::Error::other(format!("build revoke runtime failed: {err}")))?;

    runtime.block_on(async move {
        let client = crate::default_client::build_reqwest_client_with_timeouts(
            Some(std::time::Duration::from_secs(10)),
            Some(std::time::Duration::from_secs(10)),
        );
        let response = client
            .post(&endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|err| std::io::Error::other(format!("revoke request failed: {err}")))?;

        let status = response.status();
        if status.is_success() || status == reqwest::StatusCode::BAD_REQUEST {
            return Ok(());
        }

        let body = response.text().await.unwrap_or_default();
        Err(std::io::Error::other(format!(
            "revoke endpoint returned {status}: {body}"
        )))
    })
}
