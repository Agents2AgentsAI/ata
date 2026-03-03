use env_flags::env_flags;

env_flags! {
    /// Fixture path for offline tests (see client.rs).
    pub CODEX_RS_SSE_FIXTURE: Option<&str> = None;
    /// Disable automatic LSP server downloads/installations.
    pub CODEX_DISABLE_LSP_DOWNLOAD: bool = false;
}
