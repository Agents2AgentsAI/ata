//! Minimal `voice_mode` surface used by `chatwidget_document_reader` while
//! the full `voice_mode.rs` is offline (Wave 9D).
//!
//! Only the two TTS-text normalisers are exposed here. Both are passthroughs
//! that just strip leading/trailing whitespace — the real implementations in
//! `voice_mode.rs` do markdown→spoken-text rewrites (collapse headings, drop
//! list bullets, expand equations). Audio quality is degraded under this
//! stub but the produced text is still safe to feed to a TTS backend.

#[cfg(not(target_os = "linux"))]
pub(crate) fn clean_for_tts(markdown: &str) -> String {
    markdown.trim().to_string()
}

#[cfg(not(target_os = "linux"))]
pub fn clean_for_tts_preserving_equation_markers(text: &str) -> String {
    text.trim().to_string()
}
