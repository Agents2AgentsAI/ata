//! Canonical definition of "what is a secret" for every output boundary.
//!
//! Secret redaction is a cross-cutting transform applied at several boundaries
//! (the trace log DB, streaming/error display, error JSON, shell snapshots).
//! Each boundary formats its output differently — a log line keeps the header
//! name, a shell snapshot drops the whole export line, an error string swaps in
//! a placeholder — but the *patterns* that decide what counts as a secret must
//! live in one place so a newly recognized secret shape is covered everywhere
//! at once. Boundaries import these patterns instead of re-deriving them.

use regex_lite::Regex;
use std::sync::LazyLock;

/// Detects bare/inline secret *values* by shape: provider key prefixes, AWS
/// access key ids, bearer tokens, masked key fragments, and `key=value` style
/// secret assignments. Used wherever free text might embed a credential
/// (streaming errors, provider error bodies, log spans).
fn value_redactors() -> &'static [(&'static Regex, Replacement)] {
    static REDACTORS: LazyLock<Vec<(Regex, Replacement)>> = LazyLock::new(|| {
        vec![
            // OpenAI-style and other `<prefix>-<chars>` provider keys, masked or
            // not. Covers `sk-`, `pk-`, `rk-`, `api-`/`api_` followed by enough
            // key chars (e.g. `sk-proj-ABCDEF0123456789ghijkl`).
            (
                compile(r"(?i)\b(?:sk|pk|rk|api)[-_][A-Za-z0-9][A-Za-z0-9._\-]{7,}"),
                Replacement::Whole,
            ),
            // AWS access key ids.
            (compile(r"\bAKIA[0-9A-Z]{16}\b"), Replacement::Whole),
            // Masked key fragments: a key-shaped run containing a `**` mask is
            // still credential-derived material, so the whole token is replaced
            // (e.g. `sk-inval***************************test`).
            (
                compile(r"[A-Za-z0-9][A-Za-z0-9._\-]*\*{2,}[A-Za-z0-9._\-]*"),
                Replacement::Whole,
            ),
            // Bearer tokens (the credential after `Bearer `, keeping the scheme).
            (
                compile(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=\-]{8,}"),
                Replacement::Bearer,
            ),
            // `api_key: value`, `token=value`, `secret = "value"` assignments.
            (
                compile(
                    r#"(?i)\b(api[_-]?key|token|secret|password)\b(\s*[:=]\s*)(["']?)[^\s"']{8,}"#,
                ),
                Replacement::Assignment,
            ),
        ]
    });
    static REFS: LazyLock<Vec<(&'static Regex, Replacement)>> =
        LazyLock::new(|| REDACTORS.iter().map(|(re, kind)| (re, *kind)).collect());
    &REFS
}

#[derive(Clone, Copy)]
enum Replacement {
    /// Replace the entire match with the placeholder.
    Whole,
    /// Keep the `Bearer ` scheme, replace the token.
    Bearer,
    /// Keep `key`, separator, and opening quote; replace the value.
    Assignment,
}

/// Redact inline secret *values* from `text`, substituting `placeholder` for
/// each detected credential. Boundaries pass their own placeholder
/// (`[REDACTED_SECRET]`, `[REDACTED_CREDENTIAL]`, `REDACTED`, …) so existing
/// output contracts are preserved while the *patterns* stay shared.
pub fn redact_secret_values(text: &str, placeholder: &str) -> String {
    let mut current = text.to_string();
    for (re, kind) in value_redactors() {
        let replaced = match kind {
            Replacement::Whole => re.replace_all(&current, placeholder).into_owned(),
            Replacement::Bearer => re
                .replace_all(&current, format!("Bearer {placeholder}").as_str())
                .into_owned(),
            Replacement::Assignment => re
                .replace_all(&current, format!("$1$2$3{placeholder}").as_str())
                .into_owned(),
        };
        current = replaced;
    }
    current
}

/// Placeholder used when redacting credential values out of log text.
pub const LOG_REDACTION_PLACEHOLDER: &str = "REDACTED";

/// Redact credential-bearing header/field values and inline secret values from a
/// single line of log text. This is the canonical transform every log sink must
/// apply before the bytes reach disk: TRACE-level HTTP/websocket spans (including
/// third-party library logging such as tungstenite/reqwest) can otherwise emit
/// the cleartext `Authorization` header (an API key or ChatGPT access token) or a
/// `chatgpt-account-id`. The field *name* is kept; only the value is replaced.
///
/// Applied at one seam so the file log layer and the log database layer cannot
/// drift: if a sink formats an event some other way, routing it through here is
/// what keeps secrets off disk.
pub fn redact_log_line(text: &str) -> String {
    if !may_contain_secret(text) {
        return text.to_string();
    }
    // Matches `authorization: <value>` / `cookie=<value>` style pairs in HTTP
    // wire dumps, debug-formatted header maps, and span field assignments. The
    // value ends at a newline (raw or debug-escaped as `\r`/`\n`), quote, or
    // comma so surrounding structure stays intact.
    static SENSITIVE_PAIR_RE: LazyLock<Regex> = LazyLock::new(|| {
        let names = secret_field_name_alternation();
        Regex::new(&format!(r#"(?i)\b({names})("?\s*[:=]\s*"?)[^\r\n",\\]*"#))
            .unwrap_or_else(|err| panic!("sensitive pair regex failed to compile: {err}"))
    });
    let redacted = SENSITIVE_PAIR_RE.replace_all(
        text,
        format!("${{1}}${{2}}{LOG_REDACTION_PLACEHOLDER}").as_str(),
    );
    redact_secret_values(&redacted, LOG_REDACTION_PLACEHOLDER)
}

/// Cheap pre-filter so the redaction regexes only run on text that could
/// plausibly carry a credential. Covers both secret-bearing *field names* and
/// the cheap substrings of inline *value* shapes (provider key prefixes, masked
/// fragments, AWS access key ids), so a masked or prefixed key with no
/// surrounding field name still routes through redaction.
fn may_contain_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        // field names
        "authorization",
        "cookie",
        "api-key",
        "api_key",
        "apikey",
        "api key",
        "bearer",
        "_token",
        "client_secret",
        "secret",
        "password",
        "chatgpt-account-id",
        "account-id",
        // inline value shapes
        "sk-",
        "pk-",
        "rk-",
        "akia",
    ];
    NEEDLES.iter().any(|needle| lower.contains(needle)) || text.contains("**")
}

/// True when a header/JSON field *name* identifies a secret-bearing value
/// (so the boundary can replace the value while keeping the name). Union of the
/// header denylist used by the trace log DB and the error-body JSON keys.
pub fn is_secret_field_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "api-key",
        "apikey",
        "api_key",
        "x-goog-api-key",
        "openai-api-key",
        "anthropic-api-key",
        "x-auth-token",
        "chatgpt-account-id",
        "access_token",
        "refresh_token",
        "id_token",
        "token",
        "authorization_code",
        "code_verifier",
        "client_secret",
        "secret",
        "password",
    ];
    EXACT.contains(&lower.as_str())
        || lower.ends_with("_token")
        || lower.ends_with("_api_key")
        || lower.ends_with("-api-key")
}

/// Pipe-joined alternation of [`is_secret_field_name`]'s exact header names, for
/// boundaries that match field names with a regex over wire text. Keeping the
/// list here means the regex denylist cannot drift from [`is_secret_field_name`].
pub fn secret_field_name_alternation() -> &'static str {
    "authorization|proxy-authorization|cookie|set-cookie|x-api-key|api-key|apikey|api_key|\
x-goog-api-key|openai-api-key|anthropic-api-key|x-auth-token|chatgpt-account-id|access_token|\
refresh_token|id_token|client_secret"
}

/// True when an environment-variable *name* identifies a secret value (used to
/// drop export lines from shell snapshots before they are written to disk).
pub fn is_secret_env_var_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    const EXACT: &[&str] = &["API_KEY", "APIKEY", "TOKEN", "SECRET", "PASSWORD"];
    const PREFIXES: &[&str] = &[
        "AWS_",
        "OPENAI_",
        "ANTHROPIC_",
        "GOOGLE_API",
        "GEMINI_API",
        "KAGGLE_",
        "ZOTERO_API",
        "AZURE_OPENAI",
    ];
    const SUFFIXES: &[&str] = &[
        "_API_KEY",
        "_APIKEY",
        "_TOKEN",
        "_SECRET",
        "_PASSWORD",
        "_PASSWD",
        "_ACCESS_KEY",
        "_SECRET_KEY",
        "_PRIVATE_KEY",
        "_CREDENTIALS",
    ];
    EXACT.contains(&upper.as_str())
        || PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        || SUFFIXES.iter().any(|suffix| upper.ends_with(suffix))
}

fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|err| panic!("invalid secret regex `{pattern}`: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "[REDACTED]";

    #[test]
    fn compiles_all_value_patterns() {
        let _ = redact_secret_values("warm up", P);
    }

    #[test]
    fn redacts_openai_key() {
        let out = redact_secret_values("key sk-proj-ABCDEF0123456789ghijkl here", P);
        assert!(!out.contains("ABCDEF"), "{out}");
        assert!(out.contains(P), "{out}");
    }

    #[test]
    fn redacts_other_provider_prefixes() {
        for key in ["pk-ABCDEFGH12345", "rk-ABCDEFGH12345", "api-ABCDEFGH12345"] {
            let out = redact_secret_values(&format!("k {key} x"), P);
            assert!(!out.contains("ABCDEFGH"), "{key} -> {out}");
        }
    }

    #[test]
    fn redacts_masked_fragment() {
        let out = redact_secret_values("sk-inval***************************test", P);
        assert!(!out.contains('*'), "{out}");
        assert!(!out.contains("sk-inval"), "{out}");
    }

    #[test]
    fn redacts_aws_access_key() {
        let out = redact_secret_values("id AKIAIOSFODNN7EXAMPLE end", P);
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
    }

    #[test]
    fn redacts_bearer_keeping_scheme() {
        let out = redact_secret_values("authorization: Bearer sk-proj-supersecret1234", P);
        assert!(!out.contains("supersecret"), "{out}");
        assert!(out.contains("Bearer [REDACTED]"), "{out}");
    }

    #[test]
    fn redacts_assignment_keeping_key() {
        let out = redact_secret_values("token=eyJhbGciOiJSUzI1NiJ9payload", P);
        assert!(!out.contains("eyJhbGci"), "{out}");
        assert!(out.contains("token="), "{out}");
    }

    #[test]
    fn leaves_normal_text_untouched() {
        let text = "Workspace is not authorized in this region.";
        assert_eq!(redact_secret_values(text, P), text);
    }

    #[test]
    fn field_name_denylist() {
        assert!(is_secret_field_name("Authorization"));
        assert!(is_secret_field_name("x-api-key"));
        assert!(is_secret_field_name("session_token"));
        assert!(is_secret_field_name("openai_api_key"));
        assert!(!is_secret_field_name("user-agent"));
        assert!(!is_secret_field_name("accept"));
    }

    #[test]
    fn env_var_denylist() {
        assert!(is_secret_env_var_name("OPENAI_API_KEY"));
        assert!(is_secret_env_var_name("AWS_SECRET_ACCESS_KEY"));
        assert!(is_secret_env_var_name("MY_TOKEN"));
        assert!(!is_secret_env_var_name("PATH"));
        assert!(!is_secret_env_var_name("HOME"));
    }

    /// Every boundary (log DB, error display, error body, shell snapshot) calls
    /// [`redact_secret_values`] for inline values. This pins the union of secret
    /// shapes the boundaries depend on: extend it when a new shape is added, and
    /// the new shape is then redacted at every boundary that routes through here.
    #[test]
    fn covers_every_boundary_secret_shape() {
        let cases = [
            "sk-proj-ABCDEF0123456789ghijkl",
            "pk-ABCDEFGH12345678",
            "AKIAIOSFODNN7EXAMPLE",
            "sk-inval***************************test",
            "Bearer sk-proj-supersecretvalue1234",
            "token=eyJhbGciOiJSUzI1NiJ9payload",
        ];
        for case in cases {
            let out = redact_secret_values(case, P);
            assert!(out.contains(P), "no redaction for `{case}` -> `{out}`");
        }
    }

    #[test]
    fn alternation_matches_exact_names() {
        for name in secret_field_name_alternation().split('|') {
            assert!(is_secret_field_name(name), "{name} not in denylist");
        }
    }

    #[test]
    fn redact_log_line_strips_bearer_authorization_header() {
        let line = "request headers: {\"authorization\": \"Bearer eyJhbGciOiJSUzI1NiJ9.payloadsegment.signaturesegment\", \"user-agent\": \"ata\"}";
        let out = redact_log_line(line);
        assert!(!out.contains("eyJhbGciOiJSUzI1NiJ9"), "{out}");
        assert!(!out.contains("payloadsegment"), "{out}");
        assert!(out.contains("user-agent"), "{out}");
        assert!(out.contains(LOG_REDACTION_PLACEHOLDER), "{out}");
    }

    #[test]
    fn redact_log_line_strips_chatgpt_account_id() {
        let line = "chatgpt-account-id: 11112222-3333-4444-5555-666677778888\r\nuser-agent: ata";
        let out = redact_log_line(line);
        assert!(
            !out.contains("11112222-3333-4444-5555-666677778888"),
            "{out}"
        );
        assert!(out.contains("chatgpt-account-id"), "{out}");
        assert!(out.contains("user-agent"), "{out}");
    }

    #[test]
    fn redact_log_line_redacts_masked_and_prefixed_values_without_field_name() {
        // No secret field name is present, so only the value-shape pre-filter
        // and value redactors can catch these. The pre-filter must not gate them
        // out (regression: it previously only matched field names).
        for case in [
            "error: Incorrect API key provided: sk-inval***************************test",
            "key sk-proj-ABCDEF0123456789ghijkl in env",
            "id AKIAIOSFODNN7EXAMPLE end",
        ] {
            let out = redact_log_line(case);
            assert!(
                out.contains(LOG_REDACTION_PLACEHOLDER),
                "no redaction: {out}"
            );
            assert!(!out.contains("ABCDEF"), "{out}");
            assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
            assert!(!out.contains('*'), "{out}");
        }
    }

    #[test]
    fn redact_log_line_leaves_ordinary_text_untouched() {
        let line = "websocket connected to remote app server";
        assert_eq!(redact_log_line(line), line);
    }
}
