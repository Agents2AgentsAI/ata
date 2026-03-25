// Document reader integration for ChatWidget.
//
// This file is included into `chatwidget.rs` via `include!` to keep
// fork-specific document reader code in a separate file, reducing merge
// conflicts when syncing with upstream.

fn browser_extract_tag_attr(tag: &str, attr: &str) -> Option<String> {
    let tag_lower = tag.to_ascii_lowercase();
    let attr_marker = format!("{attr}=");
    let start = tag_lower.find(&attr_marker)? + attr_marker.len();
    let rest = &tag[start..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &rest[1..];
    let end = value.find(quote)?;
    Some(value[..end].to_string())
}

fn browser_latex_spoken_fallback(latex: &str) -> String {
    if latex.is_empty() {
        String::new()
    } else {
        crate::text_formatting::latex_to_plain_text(&format!("${latex}$"))
    }
}

fn browser_strip_voice_tags_only(markdown: &str) -> String {
    if !markdown.contains('<') {
        return markdown.to_string();
    }

    let mut result = String::with_capacity(markdown.len());
    let mut remaining = markdown;

    while let Some(start) = remaining.find('<') {
        result.push_str(&remaining[..start]);
        let tag_region = &remaining[start..];
        let tag_lower = tag_region.to_ascii_lowercase();

        if tag_lower.starts_with("<voice")
            && let Some(end) = tag_region.find('>')
        {
            remaining = &tag_region[end + 1..];
            continue;
        }
        if tag_lower.starts_with("</voice>") {
            remaining = &tag_region["</voice>".len()..];
            continue;
        }

        result.push('<');
        remaining = &tag_region[1..];
    }

    result.push_str(remaining);
    result
}

fn browser_preserve_spoken_tags(markdown: &str) -> String {
    if !markdown.contains('<') {
        return markdown.to_string();
    }

    let mut result = String::with_capacity(markdown.len());
    let mut remaining = markdown;

    while let Some(start) = remaining.find('<') {
        result.push_str(&remaining[..start]);
        let tag_region = &remaining[start..];
        let tag_lower = tag_region.to_ascii_lowercase();

        if tag_lower.starts_with("<voice")
            && let Some(end) = tag_region.find('>')
        {
            remaining = &tag_region[end + 1..];
            continue;
        }
        if tag_lower.starts_with("</voice>") {
            remaining = &tag_region["</voice>".len()..];
            continue;
        }
        if tag_lower.starts_with("<eq")
            && let Some(end) = tag_region.find('>')
        {
            let tag = &tag_region[..=end];
            let tag_lower = tag.to_ascii_lowercase();
            let speak = browser_extract_tag_attr(tag, "speak");
            let latex = browser_extract_tag_attr(tag, "latex").unwrap_or_default();
            let is_self_closing = tag_lower.ends_with("/>");

            if is_self_closing {
                let spoken = speak.unwrap_or_else(|| browser_latex_spoken_fallback(&latex));
                result.push_str(&spoken);
                remaining = &tag_region[end + 1..];
                continue;
            }

            if let Some(close) = tag_lower
                .ends_with('>')
                .then(|| tag_region.to_ascii_lowercase().find("</eq>"))
                .flatten()
            {
                let inner = tag_region[end + 1..close].trim();
                let spoken = speak
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| {
                        if inner.is_empty() {
                            browser_latex_spoken_fallback(&latex)
                        } else {
                            inner.to_string()
                        }
                    });
                result.push_str(&spoken);
                remaining = &tag_region[close + "</eq>".len()..];
                continue;
            }

            remaining = &tag_region[end + 1..];
            continue;
        }
        if tag_lower.starts_with("</eq>") {
            remaining = &tag_region["</eq>".len()..];
            continue;
        }

        result.push('<');
        remaining = &tag_region[1..];
    }

    result.push_str(remaining);
    result
}

fn browser_normalized_spoken_text(markdown: &str) -> String {
    use std::sync::LazyLock;

    static RE_WHITESPACE: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"\s+") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_WHITESPACE regex: {e}"),
        });

    let spoken = browser_read_aloud_text(markdown);
    RE_WHITESPACE
        .replace_all(spoken.trim(), " ")
        .trim()
        .to_string()
}

fn browser_parse_table_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.matches('|').count() < 2 {
        return None;
    }
    let cells: Vec<String> = trimmed
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect();
    if cells.is_empty() || cells.iter().all(std::string::String::is_empty) {
        return None;
    }
    Some(cells)
}

fn browser_is_table_separator(line: &str) -> bool {
    let Some(cells) = browser_parse_table_row(line) else {
        return false;
    };
    cells.iter().all(|cell| {
        !cell.is_empty()
            && cell
                .chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
    })
}

fn browser_linearize_tables(markdown: &str) -> String {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let Some(header_cells) = browser_parse_table_row(line) else {
            out.push(line.to_string());
            i += 1;
            continue;
        };

        if i + 1 >= lines.len() || !browser_is_table_separator(lines[i + 1]) {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        out.extend(
            header_cells
                .into_iter()
                .filter(|cell| !cell.is_empty()),
        );
        i += 2;

        while i < lines.len() {
            let Some(row_cells) = browser_parse_table_row(lines[i]) else {
                break;
            };
            if browser_is_table_separator(lines[i]) {
                i += 1;
                continue;
            }
            out.extend(
                row_cells
                    .into_iter()
                    .filter(|cell| !cell.is_empty()),
            );
            i += 1;
        }
    }

    out.join("\n")
}

fn browser_expand_image_alt_text(markdown: &str) -> String {
    if !markdown.contains("![") {
        return markdown.to_string();
    }

    let mut result = String::with_capacity(markdown.len());
    let mut remaining = markdown;

    while let Some(start) = remaining.find("![") {
        result.push_str(&remaining[..start]);

        let after_marker = &remaining[start + 2..];
        let Some(alt_end) = after_marker.find(']') else {
            result.push_str("![");
            remaining = after_marker;
            continue;
        };

        let alt_text = after_marker[..alt_end].trim();
        let after_alt = &after_marker[alt_end + 1..];
        if let Some(after_paren) = after_alt.strip_prefix('(')
            && let Some(url_end) = after_paren.find(')')
        {
            if !alt_text.is_empty() {
                result.push_str(alt_text);
            }
            remaining = &after_paren[url_end + 1..];
            continue;
        }

        result.push_str("![");
        remaining = after_marker;
    }

    result.push_str(remaining);
    result
}

fn browser_prepare_read_aloud_markdown(markdown: &str) -> String {
    use std::sync::LazyLock;

    static RE_BLOCKQUOTE_PREFIX: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"^[ \t]*(?:>\s*)+") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_BLOCKQUOTE_PREFIX regex: {e}"),
        });
    static RE_LIST_TASK_MARKER: LazyLock<regex_lite::Regex> =
        LazyLock::new(
            || match regex_lite::Regex::new(r"^([ \t]*(?:[-+]|\d{1,3}\.)\s*)\[(?: |x|X)\]\s+") {
                Ok(r) => r,
                Err(e) => panic!("invalid RE_LIST_TASK_MARKER regex: {e}"),
            },
        );
    static RE_BARE_TASK_MARKER: LazyLock<regex_lite::Regex> =
        LazyLock::new(|| match regex_lite::Regex::new(r"^[ \t]*\[(?: |x|X)\]\s+") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_BARE_TASK_MARKER regex: {e}"),
        });

    let expanded_images = browser_expand_image_alt_text(markdown);
    let linearized_tables = browser_linearize_tables(&expanded_images);
    linearized_tables
        .lines()
        .map(|line| {
            let without_quote = RE_BLOCKQUOTE_PREFIX.replace(line, "");
            let without_list_task = RE_LIST_TASK_MARKER.replace(&without_quote, "$1");
            RE_BARE_TASK_MARKER
                .replace(&without_list_task, "")
                .into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn browser_finalize_read_aloud_text(cleaned: String) -> String {
    use std::sync::LazyLock;

    static RE_LIST_PREFIX: LazyLock<regex_lite::Regex> = LazyLock::new(|| {
        match regex_lite::Regex::new(r"^[ \t]*(?:[-+]|\d{1,3}\.)\s+") {
            Ok(r) => r,
            Err(e) => panic!("invalid RE_LIST_PREFIX regex: {e}"),
        }
    });

    cleaned
        .lines()
        .map(|line| RE_LIST_PREFIX.replace(line, "").into_owned())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn append_browser_reading_view_debug_log(message: &str) {
    let Ok(home) = codex_core::config::find_codex_home() else {
        return;
    };
    let path = home.join("logs/browser-reading-view.log");
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::io::Write::write_all(&mut file, format!("[{ts}] {message}\n").as_bytes());
}

fn browser_log_preview(text: &str) -> String {
    const MAX_CHARS: usize = 160;

    let normalized = text.replace('\n', "\\n");
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn browser_selection_word_offset(section_markdown: &str, selected_text: &str) -> Option<usize> {
    let full = browser_normalized_spoken_text(section_markdown);
    let selected = browser_normalized_spoken_text(selected_text);
    if full.is_empty() || selected.is_empty() {
        return None;
    }
    let start = full.find(&selected)?;
    Some(full[..start].split_whitespace().count())
}

fn browser_read_aloud_text(markdown: &str) -> String {
    let normalized = browser_prepare_read_aloud_markdown(&browser_preserve_spoken_tags(markdown));
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    let cleaned = crate::chatwidget::voice_mode::clean_for_tts(&normalized);
    #[cfg(not(all(not(target_os = "linux"), feature = "voice-input")))]
    let cleaned = normalized;
    browser_finalize_read_aloud_text(cleaned)
}

pub fn browser_read_aloud_markup(markdown: &str) -> String {
    let normalized = browser_prepare_read_aloud_markdown(&browser_strip_voice_tags_only(markdown));
    #[cfg(all(not(target_os = "linux"), feature = "voice-input"))]
    let cleaned =
        crate::chatwidget::voice_mode::clean_for_tts_preserving_equation_markers(&normalized);
    #[cfg(not(all(not(target_os = "linux"), feature = "voice-input")))]
    let cleaned = normalized;
    browser_finalize_read_aloud_text(cleaned)
}

fn browser_resume_command(thread_name: Option<&str>, thread_id: Option<ThreadId>) -> String {
    if let Some(thread_id) = thread_id {
        return format!("ata --yolo resume {thread_id}");
    }

    codex_core::util::resume_command(thread_name, None)
        .map(|command| command.replacen("ata ", "ata --yolo ", 1))
        .unwrap_or_else(|| "ata --yolo resume <session-id>".to_string())
}

impl ChatWidget {
    /// Returns `true` when the document reader is active and streaming output
    /// (agent messages, reasoning) should be suppressed from the chat history.
    fn is_suppressing_streaming_for_reader(&self) -> bool {
        self.bottom_pane.is_document_reader_active()
    }

    /// Returns `true` when the document reader view is the active bottom pane view.
    pub(crate) fn is_document_reader_active(&self) -> bool {
        self.bottom_pane.is_document_reader_active()
    }

    /// Whether the reading view feature is enabled and not set to Disabled mode.
    /// Also disabled in Plan mode so the reading view doesn't interfere with planning.
    fn is_reading_view_enabled(&self) -> bool {
        self.config.features.enabled(Feature::ReadingView)
            && self.reading_view_mode != crate::app_event::ReadingViewMode::Disabled
            && self.active_mode_kind() != ModeKind::Plan
    }

    /// Whether the reading view is in browser mode.
    fn is_reading_view_browser_mode(&self) -> bool {
        self.reading_view_mode == crate::app_event::ReadingViewMode::Browser
    }

    /// Split full markdown content on `## ` headings into `(heading, content)`
    /// pairs for the browser reading view.
    fn reading_view_sections_parsed(content: &str) -> Vec<(String, String)> {
        let mut sections: Vec<(String, String)> = Vec::new();
        let mut current_heading = String::new();
        let mut current_body = String::new();

        for line in content.lines() {
            if let Some(h) = line.strip_prefix("## ") {
                // Flush previous section.
                if !current_heading.is_empty() || !current_body.is_empty() {
                    sections.push((
                        current_heading,
                        current_body.trim_end().to_string(),
                    ));
                }
                current_heading = h.trim().to_string();
                current_body = String::new();
            } else if !current_body.is_empty() || !line.is_empty() {
                if !current_body.is_empty() {
                    current_body.push('\n');
                }
                current_body.push_str(line);
            }
        }
        // Flush last section.
        if !current_heading.is_empty() || !current_body.is_empty() {
            sections.push((
                current_heading,
                current_body.trim_end().to_string(),
            ));
        }

        sections
    }

    /// Start the reading-view server (if not already running) and open in
    /// the browser. Returns the server reference for event forwarding.
    fn ensure_reading_view_server(&mut self) {
        if self.reading_view_server.is_some() {
            return;
        }
        let tx = self.app_event_tx.clone();
        let assets_root = Some(
            self.config
                .codex_home
                .join("knowledge-base")
                .join("assets"),
        );

        // Create a channel for incoming browser messages (follow-up questions, etc.)
        let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let tx_for_incoming = self.app_event_tx.clone();

        // Spawn a task to forward incoming browser messages to the app event loop.
        tokio::spawn(async move {
            while let Some(msg) = incoming_rx.recv().await {
                tx_for_incoming.send(AppEvent::ReadingViewBrowserMessage(msg));
            }
        });

        tokio::spawn(async move {
            match codex_reading_view_server::ReadingViewServer::start(assets_root, Some(incoming_tx)).await {
                Ok(server) => {
                    let url = server.url();
                    // Open in browser.
                    #[cfg(target_os = "macos")]
                    {
                        // Use -n to open a new browser instance/tab and -g to not
                        // bring the browser to foreground (user stays in terminal).
                        // Fall back to plain open if -n fails (some browsers don't support it).
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                    }
                    #[cfg(target_os = "linux")]
                    {
                        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                    }
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("cmd")
                            .args(["/C", "start", &url])
                            .spawn();
                    }
                    tx.send(AppEvent::ReadingViewServerStarted(server));
                }
                Err(e) => {
                    tracing::error!("Failed to start reading-view server: {e}");
                }
            }
        });
    }

    /// Store the server handle once the async startup completes,
    /// and flush any events that were queued while the server was starting.
    ///
    /// Immediate events (presentDocument outline, initial streaming-state
    /// indicator) are flushed synchronously so the browser gets the document
    /// skeleton right away.  Section content events (updateSection +
    /// streamingState pairs) are streamed with small per-section delays via a
    /// spawned async task so that sections appear progressively in the browser
    /// rather than all arriving at once.
    pub(crate) fn set_reading_view_server(
        &mut self,
        server: codex_reading_view_server::ReadingViewServer,
    ) {
        // Flush "immediate" events (presentDocument skeleton, initial
        // streaming-state indicator) so the browser sees the outline first.
        for queued in self.reading_view_pending_events.drain(..) {
            server.send_event(&queued);
        }

        // Stream section content one-by-one with a short delay between each
        // so the browser renders them progressively.  ReadingViewServer is
        // Clone (cheap — all clones share the same broadcast channel and
        // event buffer), so we can move a clone into the async task.
        let section_events: Vec<String> = self.reading_view_pending_section_updates.drain(..).collect();
        if !section_events.is_empty() {
            let server_for_task = server.clone();
            tokio::spawn(async move {
                for json in section_events {
                    // Small delay so each section fades in visibly.
                    tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
                    server_for_task.send_event(&json);
                }
            });
        }

        self.reading_view_server = Some(server);

        if self.reading_view_pending_browser_info {
            self.reading_view_pending_browser_info = false;
            if let Some(ref server) = self.reading_view_server {
                let title = &self.reading_view_browser_title;
                self.add_info_message(
                    format!("Reading view opened in browser: {title} — {}", server.url()),
                    None,
                );
                self.request_redraw();
            }
        }
    }

    /// Forward a JSON event to the browser reading-view server, if running.
    /// If the server handle is not yet available (async startup in progress),
    /// the event is queued and will be flushed when `set_reading_view_server`
    /// is called.
    fn forward_to_reading_view_server(&mut self, json: &str) {
        if let Some(ref server) = self.reading_view_server {
            server.send_event(json);
        } else {
            self.reading_view_pending_events.push(json.to_string());
        }
    }

    /// Queue a section-content event (updateSection + streamingState pair) to
    /// be streamed to the browser with a per-section delay once the server is
    /// ready.  If the server is already running, the event is sent immediately
    /// (no delay needed since we're in the live streaming path).
    fn queue_section_update_to_reading_view_server(&mut self, json: &str) {
        if let Some(ref server) = self.reading_view_server {
            // Server is already running — send immediately (live update path).
            server.send_event(json);
        } else {
            // Server is still starting — add to the section-update queue so
            // `set_reading_view_server` can stream them with delays.
            self.reading_view_pending_section_updates.push(json.to_string());
        }
    }

    /// Handle a JSON message received from a browser WebSocket client.
    ///
    /// Currently supported messages:
    /// - `{"type": "followUpQuestion", "text": "..."}` — submit a follow-up
    ///   question about the current document, routed through the agent.
    /// - `{"type": "requestReadAloud", "sectionIndex": N}` — request TTS
    ///   narration of the specified section.
    /// - `{"type": "requestSelectionReadAloud", "sectionIndex": N,
    ///    "selectedText": "...", "selectionStartWord": W}` — request TTS
    ///   narration of just the selected text within the section.
    pub(crate) fn handle_reading_view_browser_message(&mut self, json: &str) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(json) else {
            tracing::warn!("Failed to parse browser message: {json}");
            return;
        };

        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        append_browser_reading_view_debug_log(&format!(
            "browser_message type={msg_type} payload={}",
            browser_log_preview(json)
        ));
        match msg_type {
            "followUpQuestion" => {
                let text = msg
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if text.is_empty() {
                    return;
                }
                let section_index = msg.get("sectionIndex").and_then(serde_json::Value::as_u64).map(|v| v as usize);
                let selected_text = msg
                    .get("selectedText")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                self.handle_browser_follow_up_question(text, section_index, selected_text);
            }
            "requestReadAloud" => {
                let section_index = msg
                    .get("sectionIndex")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                self.handle_browser_request_read_aloud(section_index);
            }
            "visibleSectionChanged" => {
                let section_index = msg
                    .get("sectionIndex")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                self.handle_browser_visible_section_changed(section_index);
            }
            "requestSelectionReadAloud" => {
                let section_index = msg
                    .get("sectionIndex")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let selection_start_word = msg
                    .get("selectionStartWord")
                    .and_then(serde_json::Value::as_u64)
                    .map(|v| v as usize);
                let selected_text = msg
                    .get("selectedText")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                if let Some(selected_text) = selected_text {
                    self.handle_browser_request_selection_read_aloud(
                        section_index,
                        selected_text,
                        selection_start_word,
                    );
                }
            }
            #[cfg(not(target_os = "linux"))]
            "requestPause" => {
                self.on_voice_pause_tts();
            }
            #[cfg(not(target_os = "linux"))]
            "requestResume" => {
                self.on_voice_resume_tts();
            }
            #[cfg(not(target_os = "linux"))]
            "requestStop" => {
                self.on_voice_interrupt_tts();
            }
            #[cfg(not(target_os = "linux"))]
            "karaokeSeek" => {
                let word_index = msg
                    .get("wordIndex")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                self.handle_browser_karaoke_seek(word_index);
            }
            #[cfg(not(target_os = "linux"))]
            "seekToProgress" => {
                let fraction = msg
                    .get("fraction")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                self.handle_browser_seek_to_progress(fraction);
            }
            other => {
                tracing::debug!("Unknown browser message type: {other}");
            }
        }
    }

    /// Handle a follow-up question from the browser reading view.
    ///
    /// Submits the question as a `CodexOp(Op::UserInput)` with reading-view
    /// context so the agent can provide an appropriate response and update
    /// the document sections.
    fn handle_browser_follow_up_question(
        &mut self,
        text: String,
        section_index: Option<usize>,
        selected_text: Option<String>,
    ) {
        use codex_protocol::protocol::Op;
        use codex_protocol::user_input::UserInput;

        let title = &self.reading_view_browser_title;
        let doc_id = &self.reading_view_browser_doc_id;

        if title.is_empty() || doc_id.is_empty() {
            tracing::warn!(
                "Browser follow-up question received but no document is loaded"
            );
            // Fall back to a plain user message.
            let user_msg = UserMessage::from(text);
            self.submit_user_message(user_msg);
            return;
        }

        // Extract section heading and content from the cached sections.
        let (section_heading, section_content) = section_index
            .and_then(|idx| self.reading_view_browser_sections.get(idx))
            .map(|(heading, content)| (heading.as_str(), content.as_str()))
            .unwrap_or(("", ""));

        // Use the actual section index in tool call instructions, or fall back
        // to a placeholder when no section was specified by the browser.
        let section_idx_str = section_index
            .map(|idx| idx.to_string())
            .unwrap_or_else(|| "<most relevant section>".to_string());

        // Truncate section content to ~2000 chars for context.
        let content_preview = if section_content.len() > 2000 {
            &section_content[..2000]
        } else {
            section_content
        };

        let selection_guidance = codex_core::reading_view_selection_follow_up_guidance(
            codex_core::ReadingViewDisplayMode::Browser,
        );
        let section_guidance =
            codex_core::reading_view_section_follow_up_guidance(
                codex_core::ReadingViewDisplayMode::Browser,
            );

        // Build context similar to the TUI document reader's submit_follow_up.
        // The sentinel `<!-- READER_TOOL_INSTRUCTIONS -->` separates the user-visible
        // portion (header + question) from internal instructions that are stripped
        // by `strip_system_instruction_prefix` when rendering chat history.
        // Section content and tool instructions go AFTER the sentinel so they
        // never leak into the chat history display.
        let context = if let Some(ref sel) = selected_text {
            // The user highlighted specific text — tell the agent to patch
            // the answer in right after the selection (matching TUI behavior).
            format!(
                "[The user is reading \"{title}\" in the browser reading view, \
                 currently viewing the section titled \"{section_heading}\"]\n\n\
                 The user selected specific text from the section (shown below) and is asking about it.\n\
                 [Selected text:]\n{sel}\n\n\
                 {text}\n\n\
                 <!-- READER_TOOL_INSTRUCTIONS -->\n\
                 Tool target for this turn: document_id=\"{doc_id}\", section_index={section_idx_str}.\n\n\
                 {selection_guidance}",
            )
        } else {
            // No selection — use the existing context format.
            format!(
                "[The user is reading \"{title}\" in the browser reading view, \
                 currently viewing the section titled \"{section_heading}\"]\n\n\
                 {text}\n\n\
                 <!-- READER_TOOL_INSTRUCTIONS -->\n\
                 Tool target for this turn: document_id=\"{doc_id}\", section_index={section_idx_str}.\n\n\
                 Section content (for context):\n{content_preview}\n\n\
                 {section_guidance}",
            )
        };

        self.last_turn_was_local_submit = true;
        self.app_event_tx.send(AppEvent::CodexOp(Op::UserInput {
            items: vec![UserInput::Text {
                text: context,
                text_elements: vec![],
            }],
            final_output_json_schema: None,
        }));

        // Show "thinking" indicator in the browser.
        let ws_msg = serde_json::json!({
            "type": "followUpThinking",
            "question": text,
        });
        self.forward_to_reading_view_server(&ws_msg.to_string());
    }

    fn browser_read_aloud_text_for_section(&self, section_index: usize) -> Option<String> {
        let content = self.reading_view_browser_raw_sections.get(section_index)?;
        let text = browser_read_aloud_markup(content);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn prefetch_browser_section_read_aloud(&mut self, section_index: usize) {
        let Some(text) = self.browser_read_aloud_text_for_section(section_index) else {
            append_browser_reading_view_debug_log(&format!(
                "prefetch_read_aloud skipped_empty index={section_index}"
            ));
            return;
        };

        append_browser_reading_view_debug_log(&format!(
            "prefetch_read_aloud index={section_index} spoken_len={} preview={}",
            text.len(),
            browser_log_preview(&text)
        ));
        self.app_event_tx.send(AppEvent::VoiceModePrefetchSection {
            document_id: self.reading_view_browser_doc_id.clone(),
            section_index,
            text,
        });
    }

    #[cfg(target_os = "linux")]
    fn prefetch_browser_section_read_aloud(&mut self, _section_index: usize) {}

    fn handle_browser_visible_section_changed(&mut self, section_index: usize) {
        append_browser_reading_view_debug_log(&format!(
            "visible_section_changed index={section_index}"
        ));
        self.prefetch_browser_section_read_aloud(section_index);
        if section_index + 1 < self.reading_view_browser_raw_sections.len() {
            self.prefetch_browser_section_read_aloud(section_index + 1);
        }
    }

    /// Handle a read-aloud request from the browser reading view.
    ///
    /// Triggers TTS narration of the specified section if voice/TTS
    /// infrastructure is available.
    fn handle_browser_request_read_aloud(&mut self, section_index: usize) {
        let Some(content) = self.reading_view_browser_raw_sections.get(section_index) else {
            tracing::warn!(
                "Browser requestReadAloud for section {section_index} but section not found"
            );
            append_browser_reading_view_debug_log(&format!(
                "request_read_aloud missing_section index={section_index}"
            ));
            // Clear hourglass — the browser already set _ttsState='starting'.
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "stopped",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            return;
        };

        let Some(text) = self.browser_read_aloud_text_for_section(section_index) else {
            append_browser_reading_view_debug_log(&format!(
                "request_read_aloud skipped_empty index={section_index}"
            ));
            // Clear hourglass — the browser already set _ttsState='starting'.
            let ws_msg = serde_json::json!({
                "type": "ttsStateChanged",
                "state": "stopped",
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
            return;
        };
        append_browser_reading_view_debug_log(&format!(
            "request_read_aloud index={section_index} raw_len={} spoken_len={} preview={}",
            content.len(),
            text.len(),
            browser_log_preview(&text)
        ));

        let ws_msg = serde_json::json!({ "type": "stopKaraoke" });
        self.forward_to_reading_view_server(&ws_msg.to_string());
        let ws_msg = serde_json::json!({
            "type": "ttsStateChanged",
            "state": "starting",
            "sectionIndex": section_index,
        });
        self.forward_to_reading_view_server(&ws_msg.to_string());

        // Send only the body content (not the heading) to TTS.
        //
        // The browser's karaoke system (`_walkAndWrap`) wraps only the
        // visible words inside `.section-content`. Structural markdown
        // markers like headings, blockquotes, task checkboxes, and list
        // numbers/bullets are not rendered as `.kw` spans in the DOM.
        // If we left them in the spoken stream, the alignment timeline and
        // progress bar would drift as soon as the section hit a list.
        // Emit a VoiceModeNarrateSection event so the existing TTS pipeline
        // handles it (with caching, alignment, etc.).
        #[cfg(not(target_os = "linux"))]
        self.app_event_tx.send(AppEvent::VoiceModeNarrateSection {
            document_id: self.reading_view_browser_doc_id.clone(),
            section_index,
            text,
            selection_word_offset: None,
            manual: true,
        });
        if section_index + 1 < self.reading_view_browser_raw_sections.len() {
            self.prefetch_browser_section_read_aloud(section_index + 1);
        }
    }

    /// Handle a read-aloud request for the currently selected text in the
    /// browser reading view.
    fn handle_browser_request_selection_read_aloud(
        &mut self,
        section_index: usize,
        selected_text: String,
        selection_start_word: Option<usize>,
    ) {
        let Some(section) = self.reading_view_browser_sections.get(section_index) else {
            tracing::warn!(
                "Browser requestSelectionReadAloud for section {section_index} but section not found"
            );
            return;
        };

        let selection_word_offset = selection_start_word
            .or_else(|| browser_selection_word_offset(&section.1, &selected_text));
        let text = browser_read_aloud_markup(&selected_text);
        append_browser_reading_view_debug_log(&format!(
            "request_selection_read_aloud index={section_index} selected_len={} spoken_len={} offset={selection_word_offset:?} preview={}",
            selected_text.len(),
            text.len(),
            browser_log_preview(&text)
        ));
        if text.trim().is_empty() {
            append_browser_reading_view_debug_log(&format!(
                "request_selection_read_aloud skipped_empty index={section_index}"
            ));
            return;
        }

        let ws_msg = serde_json::json!({ "type": "stopKaraoke" });
        self.forward_to_reading_view_server(&ws_msg.to_string());
        let ws_msg = serde_json::json!({
            "type": "ttsStateChanged",
            "state": "starting",
            "sectionIndex": section_index,
        });
        self.forward_to_reading_view_server(&ws_msg.to_string());

        #[cfg(not(target_os = "linux"))]
        self.app_event_tx.send(AppEvent::VoiceModeNarrateSection {
            document_id: self.reading_view_browser_doc_id.clone(),
            section_index,
            text,
            selection_word_offset,
            manual: true,
        });
    }

    fn on_present_document(
        &mut self,
        ev: codex_protocol::document_reader::PresentDocumentEvent,
        from_replay: bool,
        is_resume_replay: bool,
    ) {
        if !self.is_reading_view_enabled() {
            // Reading view disabled — render the document as inline chat markdown.
            self.flush_active_cell();
            let markdown = format!("# {}\n\n{}", ev.title, ev.content);
            self.handle_streaming_delta(markdown);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }

        // On resume, just show a summary in chat — don't open browser or TUI.
        if is_resume_replay {
            let section_headings: Vec<String> = ev
                .content
                .lines()
                .filter_map(|line| line.strip_prefix("## ").map(|h| h.trim().to_string()))
                .collect();
            let cell = crate::history_cell::new_document_cell(
                ev.title,
                section_headings,
                ev.content,
            );
            self.add_boxed_history(Box::new(cell));
            self.request_redraw();
            return;
        }

        // Browser mode: send outline first, then fill sections progressively.
        if self.is_reading_view_browser_mode() {
            // Stop any active karaoke before replacing the document.
            #[cfg(not(target_os = "linux"))]
            self.on_voice_interrupt_tts();
            self.flush_active_cell();

            let raw_sections = Self::reading_view_sections_parsed(&ev.content);
            let sections: Vec<(String, String)> = raw_sections
                .iter()
                .map(|(h, c)| {
                    (
                        crate::text_formatting::strip_voice_tags(h),
                        crate::text_formatting::strip_voice_tags(c),
                    )
                })
                .collect();

            append_browser_reading_view_debug_log(&format!(
                "present_document title={} sections={} body_lengths={:?}",
                browser_log_preview(&ev.title),
                sections.len(),
                raw_sections
                    .iter()
                    .map(|(_, content)| content.len())
                    .collect::<Vec<_>>()
            ));

            // Cache the full section bodies for TTS/alignment. The browser UI
            // keeps the read-aloud controls disabled until a section has been
            // rendered, so the cached source text can exist before the DOM is
            // visibly filled without exposing not-yet-visible narration.
            self.reading_view_browser_title = ev.title.clone();
            self.reading_view_browser_doc_id = ev.document_id.clone();
            self.reading_view_browser_sections = sections.clone();
            self.reading_view_browser_raw_sections = raw_sections
                .into_iter()
                .map(|(_, content)| content)
                .collect();

            // Send outline with empty content (shows shimmer placeholders).
            let outline_sections: Vec<serde_json::Value> = sections
                .iter()
                .map(|(heading, _)| {
                    serde_json::json!({"heading": heading, "content": ""})
                })
                .collect();
            let ws_msg = serde_json::json!({
                "type": "presentDocument",
                "title": ev.title,
                "sections": outline_sections,
                "resumeCommand": browser_resume_command(
                    self.thread_name.as_deref(),
                    self.thread_id,
                ),
            });
            self.ensure_reading_view_server();
            self.forward_to_reading_view_server(&ws_msg.to_string());

            // Mark section 0 as currently generating.
            if !sections.is_empty() {
                let stream_msg = serde_json::json!({
                    "type": "updateStreamingState",
                    "nextIndex": 0,
                });
                self.forward_to_reading_view_server(&stream_msg.to_string());
            }

            // Fill each section with its content (triggers fade-in animation).
            // Only advance the streaming state after actually filling a section,
            // so that the generating indicator stays on the first unfilled section.
            //
            // These events go through `queue_section_update_to_reading_view_server`
            // so that when the server is still starting up, they are held in the
            // `reading_view_pending_section_updates` queue.  `set_reading_view_server`
            // then streams them out with per-section delays so the browser renders
            // each section progressively rather than receiving the full document at once.
            for (i, (_, content)) in sections.iter().enumerate() {
                if !content.is_empty() {
                    let update_msg = serde_json::json!({
                        "type": "updateSection",
                        "index": i,
                        "content": content,
                    });
                    self.queue_section_update_to_reading_view_server(&update_msg.to_string());

                    // Advance streaming state to the next section, or -1 if done.
                    let next_index: i64 = if i + 1 < sections.len() {
                        (i + 1) as i64
                    } else {
                        -1
                    };
                    let stream_msg = serde_json::json!({
                        "type": "updateStreamingState",
                        "nextIndex": next_index,
                    });
                    self.queue_section_update_to_reading_view_server(&stream_msg.to_string());
                }
            }

            if let Some(ref server) = self.reading_view_server {
                self.add_info_message(
                    format!("Reading view opened in browser: {} — {}", ev.title, server.url()),
                    None,
                );
            } else {
                self.reading_view_pending_browser_info = true;
            }
            self.request_redraw();
            return;
        }

        self.flush_active_cell();

        // (is_resume_replay already handled above)
        self.bottom_pane.show_document_reader(ev, from_replay);
    }

    fn on_update_document_section(
        &mut self,
        ev: codex_protocol::document_reader::UpdateDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            // Render section update as inline chat content.
            self.flush_active_cell();
            self.handle_streaming_delta(ev.content.clone());
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }

        // Forward to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            let clean_content = crate::text_formatting::strip_voice_tags(&ev.content);
            append_browser_reading_view_debug_log(&format!(
                "update_section index={} content_len={} preview={}",
                ev.section_index,
                ev.content.len(),
                browser_log_preview(&clean_content)
            ));

            // Keep browser section cache in sync.
            if let Some(sec) = self.reading_view_browser_sections.get_mut(ev.section_index) {
                sec.1 = clean_content.clone();
            }
            if let Some(raw) = self
                .reading_view_browser_raw_sections
                .get_mut(ev.section_index)
            {
                *raw = ev.content.clone();
            }

            let ws_msg = serde_json::json!({
                "type": "updateSection",
                "index": ev.section_index,
                "content": clean_content,
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());

            // Advance streaming state to the next section.
            // The browser knows its own section count and will treat an
            // out-of-range nextIndex as "all done" (no more shimmers).
            let stream_msg = serde_json::json!({
                "type": "updateStreamingState",
                "nextIndex": ev.section_index + 1,
            });
            self.forward_to_reading_view_server(&stream_msg.to_string());
        }

        self.bottom_pane.update_document_section(&ev);
    }

    fn on_append_document_section(
        &mut self,
        ev: codex_protocol::document_reader::AppendDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            self.flush_active_cell();
            self.handle_streaming_delta(ev.content.clone());
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }

        // Forward to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            let clean_content = crate::text_formatting::strip_voice_tags(&ev.content);
            append_browser_reading_view_debug_log(&format!(
                "append_section index={} content_len={} preview={}",
                ev.section_index,
                ev.content.len(),
                browser_log_preview(&clean_content)
            ));

            // Keep browser section cache in sync (append content).
            if let Some(sec) = self.reading_view_browser_sections.get_mut(ev.section_index) {
                if !sec.1.is_empty() {
                    sec.1.push_str("\n\n");
                }
                sec.1.push_str(&clean_content);
            }
            if let Some(raw) = self
                .reading_view_browser_raw_sections
                .get_mut(ev.section_index)
            {
                if !raw.is_empty() {
                    raw.push_str("\n\n");
                }
                raw.push_str(&ev.content);
            }

            let ws_msg = serde_json::json!({
                "type": "appendToSection",
                "index": ev.section_index,
                "content": clean_content,
                "foldable": ev.foldable,
                "summary": ev.summary,
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }

        self.bottom_pane.append_document_section(&ev);
    }

    fn on_add_document_section(
        &mut self,
        ev: codex_protocol::document_reader::AddDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            self.flush_active_cell();
            let markdown = format!("## {}\n\n{}", ev.heading, ev.content);
            self.handle_streaming_delta(markdown);
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }

        // Forward to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            let clean_heading = crate::text_formatting::strip_voice_tags(&ev.heading);
            let clean_content = crate::text_formatting::strip_voice_tags(&ev.content);

            // Keep browser section cache in sync (insert after).
            let new_section = (clean_heading.clone(), clean_content.clone());
            let insert_at = if ev.after_section_index < 0 {
                0usize
            } else {
                (ev.after_section_index as usize) + 1
            };
            if insert_at <= self.reading_view_browser_sections.len() {
                self.reading_view_browser_sections.insert(insert_at, new_section);
            } else {
                self.reading_view_browser_sections.push(new_section);
            }
            if insert_at <= self.reading_view_browser_raw_sections.len() {
                self.reading_view_browser_raw_sections
                    .insert(insert_at, ev.content.clone());
            } else {
                self.reading_view_browser_raw_sections.push(ev.content.clone());
            }

            let ws_msg = serde_json::json!({
                "type": "addSection",
                "afterIndex": ev.after_section_index,
                "heading": clean_heading,
                "content": clean_content,
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }

        self.bottom_pane.add_document_section(&ev);
    }

    fn on_patch_document_section(
        &mut self,
        ev: codex_protocol::document_reader::PatchDocumentSectionEvent,
    ) {
        if !self.is_reading_view_enabled() {
            // For patches, show the new text inline.
            self.flush_active_cell();
            self.handle_streaming_delta(ev.new_text.clone());
            self.flush_answer_stream_with_separator();
            self.handle_stream_finished();
            self.request_redraw();
            return;
        }

        // Forward to browser if in browser mode.
        if self.is_reading_view_browser_mode() {
            let clean_old = crate::text_formatting::strip_voice_tags(&ev.old_text);
            let clean_new = crate::text_formatting::strip_voice_tags(&ev.new_text);

            // Keep browser section cache in sync (apply patch).
            if let Some(sec) = self.reading_view_browser_sections.get_mut(ev.section_index) {
                sec.1 = sec.1.replacen(&clean_old, &clean_new, 1);
            }
            if let Some(raw) = self
                .reading_view_browser_raw_sections
                .get_mut(ev.section_index)
            {
                *raw = raw.replacen(&ev.old_text, &ev.new_text, 1);
            }

            let ws_msg = serde_json::json!({
                "type": "patchSection",
                "index": ev.section_index,
                "find": clean_old,
                "replace": clean_new,
            });
            self.forward_to_reading_view_server(&ws_msg.to_string());
        }

        self.bottom_pane.patch_document_section(&ev);
    }

    /// Handle a karaoke seek request from the browser (jump to a specific word).
    ///
    /// Updates the TTS highlight word index and seeks the audio player to
    /// the target word's start time so audio and visual highlight stay in sync.
    #[cfg(not(target_os = "linux"))]
    fn handle_browser_karaoke_seek(&mut self, word_index: usize) {
        let mut section_index = None;
        if let Some(ref mut state) = self.voice_mode_state {
            let offset = state.selection_word_offset.unwrap_or(0);

            // With alignment-driven karaoke, data-wi IS the spoken index.
            // With heuristic wrapping, data-wi is the visible index and
            // needs visible→spoken conversion.
            let spoken = if state.alignment_driven_karaoke {
                word_index.saturating_sub(offset)
            } else {
                let visible = word_index.saturating_sub(offset);
                let mut eq_offset = 0usize;
                for &(_, start, end) in &state.equation_word_spans {
                    let eq_len = end - start;
                    let eq_visible_start = start.saturating_sub(eq_offset);
                    if visible < eq_visible_start {
                        break;
                    }
                    eq_offset += eq_len;
                }
                visible + eq_offset
            };

            state.tts_highlight_word_idx = Some(spoken);
            section_index = state
                .narrating_section
                .as_ref()
                .map(|(_, section_index, _)| *section_index);

            // Seek the audio to the target word's start time.
            if let Some(entry) = state.tts_alignment_timeline.get(spoken) {
                let target_ms = entry.start_ms;
                if let Some(ref player) = state.audio_player {
                    player.seek_to_ms(target_ms);
                }
            }
        }
        // Forward the updated word position back to the browser.
        let ws_msg = serde_json::json!({
            "type": "karaokeWord",
            "sectionIndex": section_index,
            "wordIndex": word_index,
        });
        self.forward_to_reading_view_server(&ws_msg.to_string());
        self.request_redraw();
    }

    /// Handle a progress-fraction seek from the browser (e.g. progress bar click).
    ///
    /// Converts the fraction [0.0, 1.0] to a word index based on the total
    /// words in the currently narrated text.
    #[cfg(not(target_os = "linux"))]
    fn handle_browser_seek_to_progress(&mut self, fraction: f64) {
        let fraction = fraction.clamp(0.0, 1.0);
        let total_words = self
            .voice_mode_state
            .as_ref()
            .and_then(|s| s.narrating_cleaned_text.as_ref())
            .map(|t| t.split_whitespace().count())
            .unwrap_or(0);
        if total_words == 0 {
            return;
        }
        let target_word = ((fraction * total_words as f64).round() as usize).min(total_words.saturating_sub(1));
        let offset = self
            .voice_mode_state
            .as_ref()
            .and_then(|s| s.selection_word_offset)
            .unwrap_or(0);
        self.handle_browser_karaoke_seek(target_word + offset);
    }
}
