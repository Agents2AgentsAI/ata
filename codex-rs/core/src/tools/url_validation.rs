use codex_network_proxy::is_non_public_ip;
use codex_network_proxy::normalize_host;
use std::net::IpAddr;
use std::time::Duration;
use thiserror::Error;
use tokio::net::lookup_host;
use tokio::time::timeout;
use url::Url;

const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_FILENAME: &str = "document.pdf";
const MAX_FILENAME_LEN: usize = 120;
pub(crate) const MAX_URL_LENGTH: usize = 8192;

#[derive(Debug, Clone, Copy)]
pub(crate) struct UrlValidationOptions {
    pub(crate) allow_http: bool,
    pub(crate) resolve_dns: bool,
    pub(crate) allow_non_public_hosts: bool,
}

impl Default for UrlValidationOptions {
    fn default() -> Self {
        Self {
            allow_http: false,
            resolve_dns: true,
            allow_non_public_hosts: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ValidatedUrl {
    url: Url,
    normalized_cache_key: String,
}

impl ValidatedUrl {
    pub(crate) fn as_url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn into_url(self) -> Url {
        self.url
    }

    pub(crate) fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub(crate) fn normalized_cache_key(&self) -> &str {
        &self.normalized_cache_key
    }

    pub(crate) fn redacted_for_display(&self) -> String {
        redact_url_for_display(&self.url)
    }

    pub(crate) fn derive_pdf_filename(&self, filename_hint: Option<&str>) -> String {
        derive_pdf_filename(&self.url, filename_hint)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(crate) enum UrlValidationError {
    #[error("url must not be empty")]
    EmptyUrl,
    #[error("invalid URL: {message}")]
    InvalidUrl { message: String },
    #[error("URL exceeds max length of {max_length} characters")]
    UrlTooLong { max_length: usize },
    #[error("unsupported URL scheme `{scheme}`")]
    UnsupportedScheme { scheme: String },
    #[error("URL must not include embedded credentials")]
    CredentialsNotAllowed,
    #[error("URL must include a host")]
    MissingHost,
    #[error("host `{host}` is not allowed")]
    BlockedHost { host: String },
    #[error("URL host resolves to a non-public IP: {ip}")]
    NonPublicIp { ip: IpAddr },
    #[error("DNS resolution failed for host `{host}`: {message}")]
    DnsResolutionFailed { host: String, message: String },
}

pub(crate) async fn validate_url_strict(url: &str) -> Result<ValidatedUrl, UrlValidationError> {
    validate_url(url, UrlValidationOptions::default()).await
}

pub(crate) async fn validate_url(
    url: &str,
    options: UrlValidationOptions,
) -> Result<ValidatedUrl, UrlValidationError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(UrlValidationError::EmptyUrl);
    }
    if trimmed.len() > MAX_URL_LENGTH {
        return Err(UrlValidationError::UrlTooLong {
            max_length: MAX_URL_LENGTH,
        });
    }

    let parsed = Url::parse(trimmed).map_err(|error| UrlValidationError::InvalidUrl {
        message: error.to_string(),
    })?;
    validate_parsed_url(parsed, options).await
}

pub(crate) async fn validate_parsed_url(
    parsed: Url,
    options: UrlValidationOptions,
) -> Result<ValidatedUrl, UrlValidationError> {
    validate_scheme(parsed.scheme(), options.allow_http)?;

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UrlValidationError::CredentialsNotAllowed);
    }

    let host = parsed
        .host_str()
        .ok_or(UrlValidationError::MissingHost)?
        .to_string();
    let normalized_host = normalize_host(&host);
    if normalized_host.is_empty() {
        return Err(UrlValidationError::MissingHost);
    }
    if normalized_host == "localhost" && !options.allow_non_public_hosts {
        return Err(UrlValidationError::BlockedHost {
            host: normalized_host,
        });
    }

    if let Some(ip) = parse_host_ip(&normalized_host) {
        if is_non_public_ip(ip) && !options.allow_non_public_hosts {
            return Err(UrlValidationError::NonPublicIp { ip });
        }
    } else if options.resolve_dns {
        validate_dns(
            &normalized_host,
            parsed.port_or_known_default(),
            options.allow_non_public_hosts,
        )
        .await?;
    }

    let normalized_cache_key = normalize_url_for_cache(&parsed);
    Ok(ValidatedUrl {
        url: parsed,
        normalized_cache_key,
    })
}

pub(crate) fn normalize_url_for_cache(url: &Url) -> String {
    let mut normalized = url.clone();

    let scheme = url.scheme().to_ascii_lowercase();
    let _ = normalized.set_scheme(&scheme);

    if let Some(host) = url.host_str() {
        let normalized_host = normalize_host(host);
        let _ = normalized.set_host(Some(&normalized_host));
    }

    normalized.set_fragment(None);
    if normalized.query() == Some("") {
        normalized.set_query(None);
    }

    normalized.to_string()
}

pub(crate) fn redact_url_for_display(url: &Url) -> String {
    let mut redacted = url.clone();
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

pub(crate) fn redact_url_string_for_display(raw_url: &str) -> String {
    Url::parse(raw_url.trim())
        .ok()
        .map(|url| redact_url_for_display(&url))
        .unwrap_or_else(|| "<invalid-url>".to_string())
}

pub(crate) fn derive_pdf_filename(url: &Url, filename_hint: Option<&str>) -> String {
    let raw = filename_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            url.path_segments()
                .and_then(Iterator::last)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    urlencoding::decode(value)
                        .map(|decoded| decoded.into_owned())
                        .unwrap_or_else(|_| value.to_string())
                })
        });

    let mut sanitized = sanitize_filename(raw.as_deref().unwrap_or(DEFAULT_FILENAME));
    if sanitized.is_empty() {
        return DEFAULT_FILENAME.to_string();
    }
    if !sanitized.to_ascii_lowercase().ends_with(".pdf") {
        sanitized.push_str(".pdf");
    }
    if sanitized.len() > MAX_FILENAME_LEN {
        let ext = ".pdf";
        let keep = MAX_FILENAME_LEN.saturating_sub(ext.len());
        sanitized.truncate(keep);
        sanitized.push_str(ext);
    }
    if sanitized.is_empty() {
        DEFAULT_FILENAME.to_string()
    } else {
        sanitized
    }
}

fn sanitize_filename(candidate: &str) -> String {
    let mut out = String::with_capacity(candidate.len());
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out.trim_matches('.').trim_matches('_').to_string()
}

fn validate_scheme(scheme: &str, allow_http: bool) -> Result<(), UrlValidationError> {
    if scheme == "https" || (allow_http && scheme == "http") {
        return Ok(());
    }
    Err(UrlValidationError::UnsupportedScheme {
        scheme: scheme.to_string(),
    })
}

fn parse_host_ip(host: &str) -> Option<IpAddr> {
    let host = host.split_once('%').map(|(ip, _)| ip).unwrap_or(host);
    host.parse::<IpAddr>().ok()
}

async fn validate_dns(
    host: &str,
    port: Option<u16>,
    allow_non_public_hosts: bool,
) -> Result<(), UrlValidationError> {
    let port = port.unwrap_or(443);
    let lookup = timeout(DNS_LOOKUP_TIMEOUT, lookup_host((host, port)))
        .await
        .map_err(|_| UrlValidationError::DnsResolutionFailed {
            host: host.to_string(),
            message: "lookup timed out".to_string(),
        })?;
    let addrs = lookup.map_err(|error| UrlValidationError::DnsResolutionFailed {
        host: host.to_string(),
        message: error.to_string(),
    })?;

    let mut saw_public = false;
    for addr in addrs {
        let ip = addr.ip();
        if is_non_public_ip(ip) && !allow_non_public_hosts {
            return Err(UrlValidationError::NonPublicIp { ip });
        }
        saw_public = true;
    }

    if !saw_public {
        return Err(UrlValidationError::DnsResolutionFailed {
            host: host.to_string(),
            message: "no resolved addresses".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn validated_url_for_test(url: &str) -> ValidatedUrl {
    let parsed = Url::parse(url).expect("valid test URL");
    let normalized_cache_key = normalize_url_for_cache(&parsed);
    ValidatedUrl {
        url: parsed,
        normalized_cache_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn rejects_non_https_by_default() {
        let error = validate_url_strict("http://example.com/test.pdf")
            .await
            .expect_err("http should be rejected");
        assert_eq!(
            error,
            UrlValidationError::UnsupportedScheme {
                scheme: "http".to_string()
            }
        );
    }

    #[tokio::test]
    async fn rejects_credentials_in_url() {
        let error = validate_url(
            "https://user:pass@example.com/test.pdf",
            UrlValidationOptions {
                allow_http: false,
                resolve_dns: false,
                allow_non_public_hosts: false,
            },
        )
        .await
        .expect_err("credentials should be rejected");
        assert_eq!(error, UrlValidationError::CredentialsNotAllowed);
    }

    #[tokio::test]
    async fn rejects_localhost_and_non_public_ips() {
        let localhost = validate_url(
            "https://localhost/report.pdf",
            UrlValidationOptions {
                allow_http: false,
                resolve_dns: false,
                allow_non_public_hosts: false,
            },
        )
        .await
        .expect_err("localhost should be rejected");
        assert_eq!(
            localhost,
            UrlValidationError::BlockedHost {
                host: "localhost".to_string()
            }
        );

        let loopback = validate_url(
            "https://127.0.0.1/report.pdf",
            UrlValidationOptions {
                allow_http: false,
                resolve_dns: false,
                allow_non_public_hosts: false,
            },
        )
        .await
        .expect_err("loopback should be rejected");
        assert!(matches!(loopback, UrlValidationError::NonPublicIp { .. }));
    }

    #[tokio::test]
    async fn normalizes_cache_key_for_case_fragment_and_empty_query() {
        let validated = validate_url(
            "HTTPS://EXAMPLE.com/Doc.PDF?#page=2",
            UrlValidationOptions {
                allow_http: false,
                resolve_dns: false,
                allow_non_public_hosts: false,
            },
        )
        .await
        .expect("url should validate");

        assert_eq!(
            validated.normalized_cache_key(),
            "https://example.com/Doc.PDF"
        );
    }

    #[test]
    fn derives_pdf_filename_from_hint_or_url() {
        let url =
            Url::parse("https://example.com/files/report%20Q1.PDF?token=abc").expect("valid url");
        assert_eq!(
            derive_pdf_filename(&url, Some(" summary report ")),
            "summary_report.pdf"
        );
        assert_eq!(derive_pdf_filename(&url, None), "report_Q1.PDF");
    }

    #[test]
    fn derives_default_pdf_filename_when_sanitization_removes_everything() {
        let url = Url::parse("https://example.com/files/...").expect("valid url");
        assert_eq!(derive_pdf_filename(&url, None), DEFAULT_FILENAME);
    }

    #[test]
    fn redacts_query_and_fragment_from_display_url() {
        let url =
            Url::parse("https://example.com/path/file.pdf?token=abc#page=3").expect("valid url");
        assert_eq!(
            redact_url_for_display(&url),
            "https://example.com/path/file.pdf"
        );
    }

    #[test]
    fn redacts_or_marks_invalid_raw_url_for_display() {
        assert_eq!(
            redact_url_string_for_display("https://example.com/file.pdf?token=abc#page=2"),
            "https://example.com/file.pdf"
        );
        assert_eq!(
            redact_url_string_for_display("not-a-url"),
            "<invalid-url>".to_string()
        );
    }

    #[tokio::test]
    async fn rejects_url_longer_than_max_length() {
        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH));
        let error = validate_url_strict(&long_url)
            .await
            .expect_err("URL should be rejected");
        assert_eq!(
            error,
            UrlValidationError::UrlTooLong {
                max_length: MAX_URL_LENGTH
            }
        );
    }
}
