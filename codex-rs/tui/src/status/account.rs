#[derive(Debug, Clone)]
pub(crate) enum StatusAccountDisplay {
    ChatGpt {
        email: Option<String>,
        plan: Option<String>,
    },
    ApiKey,
    #[allow(dead_code)]
    Ata {
        email: Option<String>,
    },
    /// Authenticated via the GitHub Copilot device-code OAuth flow. The wire
    /// protocol does not expose the GitHub identity, so we render generic
    /// "GitHub Copilot" text in the status card.
    Copilot,
}
