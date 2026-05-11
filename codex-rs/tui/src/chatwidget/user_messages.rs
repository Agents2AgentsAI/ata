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

/// Length of the leading system-injected prefix to hide from chat history
/// (the model still sees it). Currently covers voice-mode instructions.
fn strip_voice_prefix_len(text: &str) -> usize {
    #[cfg(not(target_os = "linux"))]
    {
        for prefix in crate::chatwidget::voice_mode::voice_mode_instruction_prefixes() {
            if text.starts_with(prefix) {
                return prefix.len();
            }
        }
        if text.starts_with(crate::chatwidget::voice_mode::VOICE_MODE_OFF_INSTRUCTION) {
            return crate::chatwidget::voice_mode::VOICE_MODE_OFF_INSTRUCTION.len();
        }
    }
    let _ = text;
    0
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct UserMessageDisplay {
    pub(super) message: String,
    pub(super) remote_image_urls: Vec<String>,
    pub(super) local_images: Vec<PathBuf>,
    pub(super) text_elements: Vec<TextElement>,
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
        let (message, prompt_request_offset) =
            crate::ide_context::extract_prompt_request_with_offset(&message);
        let prompt_request_end = prompt_request_offset + message.len();
        // Prompt context uses the same delimiter and stripping behavior as the desktop app and IDE
        // extension. The raw user message goes to the agent, but every surface renders only the
        // request after that delimiter, so keep elements inside the visible request and shift their
        // byte ranges to match.
        let text_elements = text_elements
            .into_iter()
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
                    let prefix_len = strip_voice_prefix_len(text);
                    let display_text = &text[prefix_len..];
                    append_text_with_rebased_elements(
                        &mut message,
                        &mut text_elements,
                        display_text,
                        current_text_elements.iter().filter_map(|element| {
                            let range = element.byte_range.clone();
                            if range.end <= prefix_len {
                                return None;
                            }
                            let start = range.start.saturating_sub(prefix_len);
                            let end = range.end.saturating_sub(prefix_len);
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
