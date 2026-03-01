use crate::error::WorkspaceError;
use regex::Regex;
use std::sync::LazyLock;

// SAFETY: All three regex patterns are compile-time string literals and are known valid.
#[allow(clippy::expect_used)]
static TOKEN_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)ghp_[A-Za-z0-9_]|gho_[A-Za-z0-9_]|github_pat_[A-Za-z0-9_]")
        .expect("valid regex")
});

#[allow(clippy::expect_used)]
static QUERY_TOKEN_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[?&](token|access_token)=").expect("valid regex"));

#[allow(clippy::expect_used)]
static EMBEDDED_CREDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"://[^/@]+:[^/@]+@").expect("valid regex"));

/// Validate a repository URL: must be HTTPS, no embedded credentials, no token patterns.
pub fn validate_repo_url(url: &str) -> Result<(), WorkspaceError> {
    if !url.starts_with("https://") {
        return Err(WorkspaceError::NotHttps(url.to_string()));
    }
    if EMBEDDED_CREDS.is_match(url) {
        return Err(WorkspaceError::EmbeddedCredentials);
    }
    if QUERY_TOKEN_PARAMS.is_match(url) {
        return Err(WorkspaceError::TokenQueryParams);
    }
    if TOKEN_PATTERNS.is_match(url) {
        return Err(WorkspaceError::GitHubPatDetected);
    }
    Ok(())
}

/// Extract and normalize host from URL (lowercase, strip default port `:443`).
pub fn extract_host(url: &str) -> String {
    let after_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    let host_part = after_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    // Strip userinfo
    let host_part = if let Some((_user, host)) = host_part.rsplit_once('@') {
        host
    } else {
        host_part
    };
    let mut host = host_part.to_lowercase();
    if host.ends_with(":443") {
        host.truncate(host.len() - 4);
    }
    host
}

/// Check host against allowlist. `None` = allow all, empty `[]` = block all.
pub fn check_host_allowlist(
    url: &str,
    allowlist: Option<&[String]>,
) -> Result<(), WorkspaceError> {
    if let Some(allowed) = allowlist {
        let host = extract_host(url);
        let allowed_lower: Vec<String> = allowed.iter().map(|h| h.to_lowercase()).collect();
        if !allowed_lower.contains(&host) {
            return Err(WorkspaceError::HostNotAllowed {
                host,
                allowlist: allowed_lower,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_repo_url() {
        assert!(validate_repo_url("https://github.com/org/repo").is_ok());
        assert!(validate_repo_url("http://github.com/org/repo").is_err());
        assert!(validate_repo_url("https://user:pass@github.com/repo").is_err());
        assert!(validate_repo_url("https://github.com/repo?token=abc").is_err());
        assert!(validate_repo_url("https://ghp_abc123@github.com/repo").is_err());
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://GitHub.COM/org/repo"),
            "github.com"
        );
        assert_eq!(
            extract_host("https://github.com:443/org/repo"),
            "github.com"
        );
        assert_eq!(
            extract_host("https://gitlab.com:8443/org/repo"),
            "gitlab.com:8443"
        );
    }

    #[test]
    fn test_check_host_allowlist() {
        // None = allow all
        assert!(check_host_allowlist("https://any.com/repo", None).is_ok());
        // Empty = block all
        assert!(check_host_allowlist("https://any.com/repo", Some(&[])).is_err());
        // Match
        assert!(check_host_allowlist(
            "https://github.com/repo",
            Some(&["github.com".to_string()])
        )
        .is_ok());
        // No match
        assert!(check_host_allowlist(
            "https://gitlab.com/repo",
            Some(&["github.com".to_string()])
        )
        .is_err());
    }
}
