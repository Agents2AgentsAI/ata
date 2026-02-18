use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFacingMappedError {
    pub user_message: String,
    pub debug_detail: String,
}

#[derive(Debug, Deserialize)]
struct ProviderErrorResponse {
    error: ProviderErrorDetail,
}

/// Provider-agnostic error detail for OpenAI/Anthropic/Gemini error bodies.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
struct ProviderErrorDetail {
    /// OpenAI/Anthropic: "type" (or sometimes "code"). Gemini: "status".
    #[serde(rename = "type", alias = "code", alias = "status")]
    error_code: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorResponse {
    error: GeminiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorDetail {
    #[allow(dead_code)]
    code: Option<u16>,
    message: Option<String>,
    status: Option<String>,
}

fn parse_provider_error_detail(body: &str) -> Option<ProviderErrorDetail> {
    let trimmed = body.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    // OpenAI / Anthropic envelope: {"error": {"type"/"code": "...", "message": "..."}}
    if let Ok(parsed) = serde_json::from_str::<ProviderErrorResponse>(trimmed) {
        return Some(parsed.error);
    }

    // Gemini envelope: {"error": {"code": 400, "message": "...", "status": "INVALID_ARGUMENT"}}
    if let Ok(parsed) = serde_json::from_str::<GeminiErrorResponse>(trimmed) {
        return Some(ProviderErrorDetail {
            error_code: parsed.error.status,
            message: parsed.error.message,
        });
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileErrorKind {
    Encrypted,
    TooManyPages,
    UnsupportedFormat,
    FileReferenceNotFound,
}

impl FileErrorKind {
    fn user_message(self) -> &'static str {
        match self {
            Self::Encrypted => {
                "This PDF appears to be encrypted. Please remove the password protection and try again."
            }
            Self::TooManyPages => {
                "This PDF has too many pages for the current provider. Try a shorter document."
            }
            Self::UnsupportedFormat => {
                "Unsupported file format. Check that the file is a valid PDF."
            }
            Self::FileReferenceNotFound => {
                "File reference is no longer valid. Please re-attach the file."
            }
        }
    }
}

fn classify_file_error(message: &str) -> Option<FileErrorKind> {
    // Fast path: exact match against known user-facing messages.
    // These messages are produced by `FileErrorKind::user_message()` and may be
    // re-classified after upstream code wraps them into `CodexErr::InvalidRequest`.
    // Checking exact strings avoids keyword drift between user-facing copy and
    // the heuristics below.
    for kind in [
        FileErrorKind::Encrypted,
        FileErrorKind::TooManyPages,
        FileErrorKind::UnsupportedFormat,
        FileErrorKind::FileReferenceNotFound,
    ] {
        if message == kind.user_message() {
            return Some(kind);
        }
    }

    let lower = message.to_ascii_lowercase();
    let has_file_context = lower.contains("pdf")
        || lower.contains("input_file")
        || lower.contains("document")
        || lower.contains("file")
        || lower.contains("files/");
    if !has_file_context {
        return None;
    }

    if (lower.contains("not found")
        || lower.contains("no such file")
        || lower.contains("no longer valid"))
        && (lower.contains("file") || lower.contains("files/") || lower.contains("file_id"))
    {
        return Some(FileErrorKind::FileReferenceNotFound);
    }

    let has_password_protection_context = lower.contains("password protected")
        || lower.contains("password-protected")
        || lower.contains("password protection");
    if lower.contains("encrypted")
        || (lower.contains("password")
            && (lower.contains("pdf") || has_password_protection_context))
    {
        return Some(FileErrorKind::Encrypted);
    }

    if lower.contains("too many pages")
        || lower.contains("page limit")
        || (lower.contains("page") && lower.contains("maximum"))
    {
        return Some(FileErrorKind::TooManyPages);
    }

    let invalid_pdf_with_context = if lower.contains("invalid") && lower.contains("pdf") {
        let max_gap = 12;
        let invalid_near_pdf = lower.match_indices("pdf").any(|(idx, _)| {
            let start = idx.saturating_sub(max_gap);
            let end = (idx + "pdf".len() + max_gap).min(lower.len());
            lower
                .get(start..end)
                .is_some_and(|window| window.contains("invalid"))
        });
        let has_format_context = lower.contains("file")
            || lower.contains("format")
            || lower.contains("type")
            || lower.contains("upload")
            || lower.contains("expected")
            || lower.contains("must be")
            || lower.contains("only");
        invalid_near_pdf || has_format_context
    } else {
        false
    };

    if lower.contains("unsupported file type")
        || lower.contains("unsupported file")
        || lower.contains("unsupported format")
        || lower.contains("invalid pdf")
        || lower.contains("not a pdf")
        || invalid_pdf_with_context
    {
        return Some(FileErrorKind::UnsupportedFormat);
    }

    None
}

pub fn map_user_facing_file_error_from_message(message: &str) -> Option<UserFacingMappedError> {
    let kind = classify_file_error(message)?;
    Some(UserFacingMappedError {
        user_message: kind.user_message().to_string(),
        debug_detail: message.to_string(),
    })
}

pub fn map_user_facing_file_error_from_http_body(
    status: u16,
    body: &str,
) -> Option<UserFacingMappedError> {
    let detail = parse_provider_error_detail(body);
    let message = detail
        .as_ref()
        .and_then(|detail| detail.message.as_deref())
        .unwrap_or(body)
        .trim();
    let kind = classify_file_error(message)?;

    let error_code = detail
        .as_ref()
        .and_then(|detail| detail.error_code.as_deref())
        .unwrap_or("");
    let debug_detail = if error_code.is_empty() {
        format!("[{status}] {message}")
    } else {
        format!("[{status}] {error_code}: {message}")
    };

    Some(UserFacingMappedError {
        user_message: kind.user_message().to_string(),
        debug_detail,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_openai_style_error_envelope() {
        let body =
            r#"{"error":{"type":"invalid_request_error","message":"This PDF is encrypted."}}"#;
        let detail = parse_provider_error_detail(body).expect("detail");
        assert_eq!(detail.error_code.as_deref(), Some("invalid_request_error"));
        assert_eq!(detail.message.as_deref(), Some("This PDF is encrypted."));
    }

    #[test]
    fn parses_gemini_error_envelope() {
        let body = r#"{"error":{"code":400,"message":"The PDF is encrypted.","status":"INVALID_ARGUMENT"}}"#;
        let detail = parse_provider_error_detail(body).expect("detail");
        assert_eq!(detail.error_code.as_deref(), Some("INVALID_ARGUMENT"));
        assert_eq!(detail.message.as_deref(), Some("The PDF is encrypted."));
    }

    #[test]
    fn maps_encrypted_pdf_message() {
        let mapped =
            map_user_facing_file_error_from_message("This PDF is encrypted.").expect("should map");
        assert_eq!(
            mapped.user_message,
            "This PDF appears to be encrypted. Please remove the password protection and try again."
        );
    }

    #[test]
    fn maps_too_many_pages_message() {
        let mapped = map_user_facing_file_error_from_message(
            "Document processing failed: too many pages in the PDF.",
        )
        .expect("should map");
        assert_eq!(
            mapped.user_message,
            "This PDF has too many pages for the current provider. Try a shorter document."
        );
    }

    #[test]
    fn maps_invalid_file_reference_message() {
        let mapped = map_user_facing_file_error_from_message("File not found: files/abc123")
            .expect("should map");
        assert_eq!(
            mapped.user_message,
            "File reference is no longer valid. Please re-attach the file."
        );
    }

    #[test]
    fn maps_own_user_facing_file_reference_not_found_message() {
        let message = "File reference is no longer valid. Please re-attach the file.";
        let mapped = map_user_facing_file_error_from_message(message)
            .expect("should map own user-facing message");
        assert_eq!(
            mapped,
            UserFacingMappedError {
                user_message: message.to_string(),
                debug_detail: message.to_string(),
            }
        );
    }

    #[test]
    fn all_user_facing_messages_are_self_classifiable() {
        let kinds = [
            FileErrorKind::Encrypted,
            FileErrorKind::TooManyPages,
            FileErrorKind::UnsupportedFormat,
            FileErrorKind::FileReferenceNotFound,
        ];
        for kind in kinds {
            let message = kind.user_message();
            let mapped = map_user_facing_file_error_from_message(message);
            assert_eq!(
                mapped,
                Some(UserFacingMappedError {
                    user_message: message.to_string(),
                    debug_detail: message.to_string(),
                }),
                "user-facing message for {kind:?} must be self-classifiable"
            );
        }
    }

    #[test]
    fn does_not_map_unrelated_message() {
        assert_eq!(
            map_user_facing_file_error_from_message("Rate limit exceeded."),
            None
        );
    }

    #[test]
    fn does_not_map_unrelated_password_error() {
        assert_eq!(
            map_user_facing_file_error_from_message(
                "Invalid password for file upload authentication."
            ),
            None
        );
    }

    #[test]
    fn does_not_map_message_with_file_word_but_not_pdf_error() {
        assert_eq!(
            map_user_facing_file_error_from_message("File upload service rate limited."),
            None
        );
    }

    #[test]
    fn does_not_map_invalid_request_without_file_context() {
        assert_eq!(
            map_user_facing_file_error_from_message("Invalid JSON in request."),
            None
        );
    }

    #[test]
    fn does_not_map_invalid_response_from_pdf_viewer() {
        assert_eq!(
            map_user_facing_file_error_from_message("The PDF viewer returned an invalid response."),
            None
        );
    }

    #[test]
    fn maps_invalid_file_only_pdf_supported_message() {
        let mapped =
            map_user_facing_file_error_from_message("Invalid file. Only PDF files are supported.")
                .expect("should map");
        assert_eq!(
            mapped.user_message,
            "Unsupported file format. Check that the file is a valid PDF."
        );
    }

    #[test]
    fn maps_from_http_body_using_provider_message() {
        let body =
            r#"{"error":{"type":"invalid_request_error","message":"This PDF is encrypted."}}"#;
        let mapped = map_user_facing_file_error_from_http_body(400, body).expect("should map");
        assert_eq!(
            mapped.user_message,
            "This PDF appears to be encrypted. Please remove the password protection and try again."
        );
        assert!(mapped.debug_detail.contains("[400]"));
        assert!(mapped.debug_detail.contains("invalid_request_error"));
    }
}
