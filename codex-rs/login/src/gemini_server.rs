use std::io::Cursor;
use std::io::Write;
use std::io::{self};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use crate::pkce::PkceCodes;
use crate::pkce::generate_pkce;
use crate::server::LoginServer;
use crate::server::ShutdownHandle;
use base64::Engine;
use chrono::Duration;
use chrono::Utc;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::DEFAULT_GEMINI_OAUTH_CLIENT_ID;
use codex_core::auth::DEFAULT_GEMINI_OAUTH_CLIENT_SECRET;
use codex_core::auth::DEFAULT_GEMINI_OAUTH_TOKEN_URL;
use codex_core::auth::GEMINI_OAUTH_CLIENT_ID_ENV_VAR;
use codex_core::auth::GEMINI_OAUTH_CLIENT_SECRET_ENV_VAR;
use codex_core::auth::GEMINI_OAUTH_TOKEN_URL_ENV_VAR;
use codex_core::auth::PROVIDER_GEMINI;
use codex_core::auth::ProviderOauthCredential;
use codex_core::auth::env_string_or_default;
use codex_core::auth::login_with_provider_oauth;
use rand::RngCore;
use tiny_http::Header;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;

const GEMINI_OAUTH_AUTHORIZE_URL_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_AUTHORIZE_URL";
const GEMINI_OAUTH_USERINFO_URL_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_USERINFO_URL";
const GEMINI_OAUTH_CALLBACK_PORT_ENV_VAR: &str = "CODEX_GEMINI_OAUTH_CALLBACK_PORT";

const DEFAULT_GEMINI_OAUTH_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const DEFAULT_GEMINI_OAUTH_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GEMINI_OAUTH_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";

#[derive(Debug, Clone)]
pub struct GeminiServerOptions {
    pub codex_home: PathBuf,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub port: u16,
    pub open_browser: bool,
    pub force_state: Option<String>,
    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
}

impl GeminiServerOptions {
    pub fn new(
        codex_home: PathBuf,
        cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
    ) -> Self {
        Self {
            codex_home,
            client_id: env_string_or_default(
                GEMINI_OAUTH_CLIENT_ID_ENV_VAR,
                DEFAULT_GEMINI_OAUTH_CLIENT_ID,
            ),
            client_secret: env_string_or_default(
                GEMINI_OAUTH_CLIENT_SECRET_ENV_VAR,
                DEFAULT_GEMINI_OAUTH_CLIENT_SECRET,
            ),
            authorize_url: env_string_or_default(
                GEMINI_OAUTH_AUTHORIZE_URL_ENV_VAR,
                DEFAULT_GEMINI_OAUTH_AUTHORIZE_URL,
            ),
            token_url: env_string_or_default(
                GEMINI_OAUTH_TOKEN_URL_ENV_VAR,
                DEFAULT_GEMINI_OAUTH_TOKEN_URL,
            ),
            userinfo_url: env_string_or_default(
                GEMINI_OAUTH_USERINFO_URL_ENV_VAR,
                DEFAULT_GEMINI_OAUTH_USERINFO_URL,
            ),
            port: std::env::var(GEMINI_OAUTH_CALLBACK_PORT_ENV_VAR)
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(0),
            open_browser: true,
            force_state: None,
            cli_auth_credentials_store_mode,
        }
    }
}

pub fn run_gemini_login_server(opts: GeminiServerOptions) -> io::Result<LoginServer> {
    let pkce = generate_pkce();
    let state = opts.force_state.clone().unwrap_or_else(generate_state);

    let server = bind_server(opts.port)?;
    let actual_port = match server.server_addr().to_ip() {
        Some(addr) => addr.port(),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unable to determine the server port",
            ));
        }
    };
    let server = Arc::new(server);

    let redirect_uri = format!("http://127.0.0.1:{actual_port}/auth/callback");
    let auth_url = build_authorize_url(
        &opts.authorize_url,
        &opts.client_id,
        &redirect_uri,
        &pkce,
        &state,
    );

    if opts.open_browser {
        let _ = webbrowser::open(&auth_url);
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Request>(16);
    let _server_handle = {
        let server = server.clone();
        thread::spawn(move || -> io::Result<()> {
            while let Ok(request) = server.recv() {
                tx.blocking_send(request).map_err(|err| {
                    io::Error::other(format!("Failed to send request to channel: {err}"))
                })?;
            }
            Ok(())
        })
    };

    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let server_handle = {
        let shutdown_notify = shutdown_notify.clone();
        let server = server;
        tokio::spawn(async move {
            let result = loop {
                tokio::select! {
                    _ = shutdown_notify.notified() => {
                        break Err(io::Error::other("Login was not completed"));
                    }
                    maybe_req = rx.recv() => {
                        let Some(req) = maybe_req else {
                            break Err(io::Error::other("Login was not completed"));
                        };

                        let url_raw = req.url().to_string();
                        let response = process_request(
                            &url_raw,
                            &opts,
                            &redirect_uri,
                            &pkce,
                            actual_port,
                            &state,
                        )
                        .await;

                        let exit_result = match response {
                            HandledRequest::Response(response) => {
                                let _ = tokio::task::spawn_blocking(move || req.respond(response)).await;
                                None
                            }
                            HandledRequest::ResponseAndExit {
                                headers,
                                body,
                                result,
                            } => {
                                let _ = tokio::task::spawn_blocking(move || {
                                    send_response_with_disconnect(req, headers, body)
                                })
                                .await;
                                Some(result)
                            }
                            HandledRequest::RedirectWithHeader(header) => {
                                let redirect = Response::empty(302).with_header(header);
                                let _ = tokio::task::spawn_blocking(move || req.respond(redirect)).await;
                                None
                            }
                        };

                        if let Some(result) = exit_result {
                            break result;
                        }
                    }
                }
            };

            server.unblock();
            result
        })
    };

    Ok(LoginServer {
        auth_url,
        actual_port,
        server_handle,
        shutdown_handle: ShutdownHandle { shutdown_notify },
    })
}

enum HandledRequest {
    Response(Response<Cursor<Vec<u8>>>),
    RedirectWithHeader(Header),
    ResponseAndExit {
        headers: Vec<Header>,
        body: Vec<u8>,
        result: io::Result<()>,
    },
}

async fn process_request(
    url_raw: &str,
    opts: &GeminiServerOptions,
    redirect_uri: &str,
    pkce: &PkceCodes,
    actual_port: u16,
    state: &str,
) -> HandledRequest {
    let parsed_url = match url::Url::parse(&format!("http://localhost{url_raw}")) {
        Ok(url) => url,
        Err(err) => {
            return HandledRequest::Response(
                Response::from_string(format!("Invalid callback URL: {err}")).with_status_code(400),
            );
        }
    };

    match parsed_url.path() {
        "/auth/callback" => {
            let params: std::collections::HashMap<String, String> =
                parsed_url.query_pairs().into_owned().collect();
            if params.get("state").map(String::as_str) != Some(state) {
                return HandledRequest::Response(
                    Response::from_string("State mismatch").with_status_code(400),
                );
            }

            if let Some(error) = params.get("error").filter(|error| !error.trim().is_empty()) {
                let mut message = format!("OAuth authorization failed: {error}");
                if let Some(description) = params
                    .get("error_description")
                    .filter(|description| !description.trim().is_empty())
                {
                    message = format!("{message} ({description})");
                }
                return HandledRequest::Response(
                    Response::from_string(message).with_status_code(400),
                );
            }

            let code = match params.get("code") {
                Some(code) if !code.is_empty() => code.clone(),
                _ => {
                    return HandledRequest::Response(
                        Response::from_string("Missing authorization code").with_status_code(400),
                    );
                }
            };

            match exchange_code_for_tokens(opts, redirect_uri, pkce, &code).await {
                Ok(exchanged) => {
                    let email = match fetch_user_email(opts, &exchanged.access_token).await {
                        Ok(email) => email,
                        Err(err) => {
                            return HandledRequest::Response(
                                Response::from_string(format!("Failed to fetch user email: {err}"))
                                    .with_status_code(500),
                            );
                        }
                    };

                    let oauth_credential = ProviderOauthCredential {
                        access: exchanged.access_token,
                        refresh: exchanged.refresh_token,
                        expires: exchanged
                            .expires_in
                            .map(|expires_in| Utc::now() + Duration::seconds(expires_in)),
                        email: Some(email),
                        project_id: None,
                        managed_project_id: None,
                    };

                    if let Err(err) = login_with_provider_oauth(
                        &opts.codex_home,
                        PROVIDER_GEMINI,
                        oauth_credential,
                        opts.cli_auth_credentials_store_mode,
                    ) {
                        return HandledRequest::Response(
                            Response::from_string(format!("Unable to persist auth file: {err}"))
                                .with_status_code(500),
                        );
                    }

                    let success_url = format!("http://127.0.0.1:{actual_port}/success");
                    match Header::from_bytes(&b"Location"[..], success_url.as_bytes()) {
                        Ok(header) => HandledRequest::RedirectWithHeader(header),
                        Err(_) => HandledRequest::Response(
                            Response::from_string("Internal Server Error").with_status_code(500),
                        ),
                    }
                }
                Err(err) => HandledRequest::Response(
                    Response::from_string(format!("Token exchange failed: {err}"))
                        .with_status_code(500),
                ),
            }
        }
        "/success" => HandledRequest::ResponseAndExit {
            headers: match Header::from_bytes(
                &b"Content-Type"[..],
                &b"text/html; charset=utf-8"[..],
            ) {
                Ok(header) => vec![header],
                Err(_) => Vec::new(),
            },
            body: GEMINI_SUCCESS_PAGE.as_bytes().to_vec(),
            result: Ok(()),
        },
        "/cancel" => HandledRequest::ResponseAndExit {
            headers: Vec::new(),
            body: b"Login cancelled".to_vec(),
            result: Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Login cancelled",
            )),
        },
        _ => HandledRequest::Response(Response::from_string("Not Found").with_status_code(404)),
    }
}

fn send_response_with_disconnect(
    req: Request,
    mut headers: Vec<Header>,
    body: Vec<u8>,
) -> io::Result<()> {
    let status = StatusCode(200);
    let mut writer = req.into_writer();
    let reason = status.default_reason_phrase();
    write!(writer, "HTTP/1.1 {} {}\r\n", status.0, reason)?;
    headers.retain(|header| !header.field.equiv("Connection"));
    if let Ok(close_header) = Header::from_bytes(&b"Connection"[..], &b"close"[..]) {
        headers.push(close_header);
    }

    let content_length_value = format!("{}", body.len());
    if let Ok(content_length_header) =
        Header::from_bytes(&b"Content-Length"[..], content_length_value.as_bytes())
    {
        headers.push(content_length_header);
    }

    for header in headers {
        write!(
            writer,
            "{}: {}\r\n",
            header.field.as_str(),
            header.value.as_str()
        )?;
    }
    writer.write_all(b"\r\n")?;
    writer.write_all(&body)?;
    writer.flush()
}

fn bind_server(port: u16) -> io::Result<Server> {
    let bind_address = if port == 0 {
        "127.0.0.1:0".to_string()
    } else {
        format!("127.0.0.1:{port}")
    };
    Server::http(&bind_address).map_err(io::Error::other)
}

fn build_authorize_url(
    authorize_url: &str,
    client_id: &str,
    redirect_uri: &str,
    pkce: &PkceCodes,
    state: &str,
) -> String {
    let query = vec![
        ("response_type".to_string(), "code".to_string()),
        ("client_id".to_string(), client_id.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
        ("scope".to_string(), GEMINI_OAUTH_SCOPES.to_string()),
        ("access_type".to_string(), "offline".to_string()),
        ("prompt".to_string(), "consent".to_string()),
        (
            "code_challenge".to_string(),
            pkce.code_challenge.to_string(),
        ),
        ("code_challenge_method".to_string(), "S256".to_string()),
        ("state".to_string(), state.to_string()),
    ];
    let query_string = query
        .into_iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{authorize_url}?{query_string}")
}

fn generate_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Debug)]
struct ExchangedTokens {
    access_token: String,
    refresh_token: String,
    expires_in: Option<i64>,
}

#[derive(serde::Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

async fn exchange_code_for_tokens(
    opts: &GeminiServerOptions,
    redirect_uri: &str,
    pkce: &PkceCodes,
    code: &str,
) -> io::Result<ExchangedTokens> {
    let client = reqwest::Client::new();
    let response = client
        .post(&opts.token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(format!(
            "grant_type=authorization_code&client_id={client_id}&client_secret={client_secret}&code={code}&redirect_uri={redirect_uri}&code_verifier={code_verifier}",
            client_id = urlencoding::encode(&opts.client_id),
            client_secret = urlencoding::encode(&opts.client_secret),
            code = urlencoding::encode(code),
            redirect_uri = urlencoding::encode(redirect_uri),
            code_verifier = urlencoding::encode(&pkce.code_verifier),
        ))
        .send()
        .await
        .map_err(io::Error::other)?;

    let status = response.status();
    let body = response.text().await.map_err(io::Error::other)?;
    if !status.is_success() {
        return Err(io::Error::other(format!(
            "token endpoint returned status {status}: {body}"
        )));
    }

    let tokens: GoogleTokenResponse = serde_json::from_str(&body).map_err(io::Error::other)?;
    let refresh_token = tokens
        .refresh_token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            io::Error::other("Google OAuth token response did not include refresh_token")
        })?;

    Ok(ExchangedTokens {
        access_token: tokens.access_token,
        refresh_token,
        expires_in: tokens.expires_in,
    })
}

#[derive(serde::Deserialize)]
struct GoogleUserInfoResponse {
    #[serde(default)]
    email: Option<String>,
}

async fn fetch_user_email(opts: &GeminiServerOptions, access_token: &str) -> io::Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .get(&opts.userinfo_url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(io::Error::other)?;

    let status = response.status();
    let body = response.text().await.map_err(io::Error::other)?;
    if !status.is_success() {
        return Err(io::Error::other(format!(
            "userinfo endpoint returned status {status}: {body}"
        )));
    }

    let userinfo: GoogleUserInfoResponse = serde_json::from_str(&body).map_err(io::Error::other)?;
    userinfo
        .email
        .map(|email| email.trim().to_string())
        .filter(|email| !email.is_empty())
        .ok_or_else(|| io::Error::other("Google userinfo response did not include an email"))
}

const GEMINI_SUCCESS_PAGE: &str = r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Gemini Login Complete</title>
  </head>
  <body>
    <h1>Gemini login complete</h1>
    <p>You can close this tab and return to Ata.</p>
  </body>
</html>
"#;
