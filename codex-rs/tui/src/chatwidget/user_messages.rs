//! User-message display models and helpers for the chat widget.
//!
//! The app-server preserves user input as structured chunks, while chat history
//! renders a single prompt row. This module owns that display projection and
//! the small compare key used to suppress duplicate rows for pending steers.

use std::path::PathBuf;

use codex_app_server_protocol::UserInput;
use codex_protocol::user_input::ByteRange;
use codex_protocol::user_input::TextElement;

use super::ChatWidget;
use super::append_text_with_rebased_elements;

/// Byte range of `text` that should be shown in chat history. System-injected
/// preambles (voice-mode, reading-view question context) are hidden from
/// display but kept in the model payload.
fn displayable_range(text: &str) -> (usize, usize) {
    let mut start = 0usize;
    let mut end = text.len();

    #[cfg(not(target_os = "linux"))]
    {
        for prefix in crate::chatwidget::voice_mode::voice_mode_instruction_prefixes() {
            if text.starts_with(prefix) {
                start = prefix.len();
                break;
            }
        }
        if start == 0 && text.starts_with(crate::chatwidget::voice_mode::VOICE_MODE_OFF_INSTRUCTION)
        {
            start = crate::chatwidget::voice_mode::VOICE_MODE_OFF_INSTRUCTION.len();
        }
    }

    // Reading-view close feedback ("[The user closed the document reader…]")
    // is a pure system message — no user text inside. Hide the entire body.
    if text[start..].starts_with("[The user closed the document reader") {
        return (start, start);
    }

    // Reading-view Tab-to-ask wraps the user question with a system context
    // block. The agent answers via inline section patches, not chat replies,
    // so hide the question + system context entirely from the chat history —
    // otherwise the chat pane competes with the reading view for screen
    // space and the user sees their own question echoed twice (once in the
    // section, once in chat).
    if text[start..].starts_with("[The user is reading ") {
        return (start, start);
    }
    const READER_INSTR_SEP: &str = "\n\n<!-- READER_TOOL_INSTRUCTIONS -->\n";
    const SELECTION_SUFFIX: &str = "\n\nThe user selected specific text from the section";
    let tail = &text[start..];
    let cut_instr = tail.find(READER_INSTR_SEP);
    let cut_sel = tail.find(SELECTION_SUFFIX);
    let cut = match (cut_instr, cut_sel) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    if let Some(cut) = cut {
        end = start + cut;
    }

    (start, end.max(start))
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct UserMessageDisplay {
    pub(super) message: String,
    pub(super) remote_image_urls: Vec<String>,
    pub(super) local_images: Vec<PathBuf>,
    pub(super) text_elements: Vec<TextElement>,
}

impl UserMessageDisplay {
    /// Compare two displays on text content only, ignoring image
    /// attachments. The optimistic submit-render uses what the user typed
    /// (no auto-injected attachments yet), while the agent's later
    /// committed event includes server-injected attachments (e.g. PDF
    /// pages rasterized for Copilot chat-completions). Both should be
    /// treated as the same bubble for dedup purposes.
    pub(super) fn matches_text_of(&self, other: &UserMessageDisplay) -> bool {
        self.message == other.message && self.text_elements == other.text_elements
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingSteerCompareKey {
    pub(super) message: String,
    pub(super) image_count: usize,
}

impl ChatWidget {
    pub(super) fn user_message_display_from_parts(
        message: String,
        text_elements: Vec<TextElement>,
        local_images: Vec<PathBuf>,
        remote_image_urls: Vec<String>,
    ) -> UserMessageDisplay {
        // Strip system-injected wrappers (voice-mode preambles, reading-view
        // question wrappers, document-closed feedback) so they never reach
        // chat history rendering. The agent still sees the raw message.
        let (vis_start, vis_end) = displayable_range(&message);
        let message = message[vis_start..vis_end].to_string();
        let stripped_elements = text_elements.into_iter().filter_map(|element| {
            let range = element.byte_range;
            if range.end <= vis_start || range.start >= vis_end {
                return None;
            }
            let new_start = range.start.saturating_sub(vis_start);
            let new_end = range.end.min(vis_end).saturating_sub(vis_start);
            Some(element.map_range(|_| ByteRange {
                start: new_start,
                end: new_end,
            }))
        });

        let (message, prompt_request_offset) =
            crate::ide_context::extract_prompt_request_with_offset(&message);
        let prompt_request_end = prompt_request_offset + message.len();
        // Prompt context uses the same delimiter and stripping behavior as the desktop app and IDE
        // extension. The raw user message goes to the agent, but every surface renders only the
        // request after that delimiter, so keep elements inside the visible request and shift their
        // byte ranges to match.
        let text_elements: Vec<TextElement> = stripped_elements
            .filter_map(|element| {
                let range = element.byte_range;
                if range.start < prompt_request_offset || range.end > prompt_request_end {
                    return None;
                }

                Some(element.map_range(|range| ByteRange {
                    start: range.start - prompt_request_offset,
                    end: range.end - prompt_request_offset,
                }))
            })
            .collect();

        UserMessageDisplay {
            message: message.to_string(),
            remote_image_urls,
            local_images,
            text_elements,
        }
    }

    /// Build the compare key for a submitted pending steer without invoking the
    /// expensive request-serialization path. Pending steers only need to match the
    /// committed app-server `UserMessage` item emitted after input drains, which
    /// preserves flattened text and total image count.
    pub(super) fn pending_steer_compare_key_from_items(
        items: &[UserInput],
    ) -> PendingSteerCompareKey {
        let mut message = String::new();
        let mut image_count = 0;

        for item in items {
            match item {
                UserInput::Text { text, .. } => message.push_str(text),
                UserInput::Image { .. } | UserInput::LocalImage { .. } => image_count += 1,
                UserInput::Skill { .. }
                | UserInput::Mention { .. }
                | UserInput::LocalFile { .. }
                | UserInput::UploadedFile { .. } => {}
            }
        }

        PendingSteerCompareKey {
            message,
            image_count,
        }
    }

    pub(super) fn user_message_display_from_inputs(items: &[UserInput]) -> UserMessageDisplay {
        let mut message = String::new();
        let mut remote_image_urls = Vec::new();
        let mut local_images = Vec::new();
        let mut text_elements = Vec::new();

        for item in items {
            match item {
                UserInput::Text {
                    text,
                    text_elements: current_text_elements,
                } => {
                    let (vis_start, vis_end) = displayable_range(text);
                    let display_text = &text[vis_start..vis_end];
                    append_text_with_rebased_elements(
                        &mut message,
                        &mut text_elements,
                        display_text,
                        current_text_elements.iter().filter_map(|element| {
                            let range = element.byte_range.clone();
                            if range.end <= vis_start || range.start >= vis_end {
                                return None;
                            }
                            let start = range.start.saturating_sub(vis_start);
                            let end = range.end.min(vis_end).saturating_sub(vis_start);
                            let shifted = ByteRange { start, end };
                            Some(TextElement::new(
                                shifted,
                                element
                                    .placeholder()
                                    .or_else(|| display_text.get(start..end))
                                    .map(str::to_string),
                            ))
                        }),
                    );
                }
                UserInput::Image { url } => remote_image_urls.push(url.clone()),
                UserInput::LocalImage { path } => local_images.push(path.clone()),
                UserInput::Skill { .. }
                | UserInput::Mention { .. }
                | UserInput::LocalFile { .. }
                | UserInput::UploadedFile { .. } => {}
            }
        }

        Self::user_message_display_from_parts(
            message,
            text_elements,
            local_images,
            remote_image_urls,
        )
    }
}
