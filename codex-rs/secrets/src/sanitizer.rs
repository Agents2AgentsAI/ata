/// Remove secrets and keys from a String, best effort, using the shared secret
/// patterns in [`codex_secret_patterns`]. The pattern definitions live there so
/// every redaction boundary (logs, errors, snapshots, memories) stays in sync.
pub fn redact_secrets(input: String) -> String {
    codex_secret_patterns::redact_secret_values(&input, "[REDACTED_SECRET]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_regex() {
        // The goal of this test is just to compile all the regex to prevent the panic
        let _ = redact_secrets("secret".to_string());
    }

    #[test]
    fn redacts_known_shapes() {
        let out =
            redact_secrets("authorization: Bearer sk-proj-ABCDEF0123456789ghijkl".to_string());
        assert!(!out.contains("ABCDEF"), "{out}");
        assert!(out.contains("[REDACTED_SECRET]"), "{out}");
    }
}
