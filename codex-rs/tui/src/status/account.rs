#[derive(Debug, Clone)]
pub(crate) enum StatusAccountDisplay {
    ChatGpt {
        email: Option<String>,
        plan: Option<String>,
    },
    ApiKey,
    Ata {
        email: Option<String>,
    },
}
