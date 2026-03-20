// Document reader integration for ChatWidget.
//
// This file is included into `chatwidget.rs` via `include!` to keep
// fork-specific document reader code in a separate file, reducing merge
// conflicts when syncing with upstream.

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
    pub(crate) fn handle_reading_view_browser_message(&mut self, json: &str) {
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(json) else {
            tracing::warn!("Failed to parse browser message: {json}");
            return;
        };

        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
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

        // Formatting guidance shared by both selection and no-selection paths.
        let formatting_guidance = "\
            Write your answer as straight prose that continues the section's voice. \
            Do NOT use a Q:/A: format. If the answer would be unclear without context, \
            a short italic lead-in is fine (e.g. *On dropout:* ...), but skip it when \
            the meaning is obvious from placement. Don't overuse it.\n\n\
            SUMMARY (required): Always set the `summary` parameter to a short descriptive \
            label of your answer (5-10 words), e.g. summary=\"Role of attention heads in GPT\". \
            This is used as a section label regardless of foldable.\n\n\
            FOLDABLE CONTENT: For supplementary content (explanations, examples, deep dives), \
            set foldable=true. Direct answers, corrections, \
            and rewrites should NOT be foldable (foldable=false, the default).\n\n\
            DISPLAY FORMAT: The reading view is displayed in a web browser with full \
            HTML rendering support. You can use:\n\
            - LaTeX math: wrap equations in $...$ (inline) or $$...$$ (display)\n\
            - Mermaid diagrams: use ```mermaid code blocks for flowcharts, sequence diagrams, etc.\n\
            - Rich markdown formatting including tables, blockquotes, etc.";

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
                 DEFAULT — insert your answer after the selection:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={section_idx_str}, \
                 old_text=\"<the selected text exactly>\", \
                 new_text=\"<the selected text>\\n\\n<your answer>\")\n\
                 This inserts your answer right after the selected passage. \
                 Reproduce the selected text verbatim as old_text so the patch matches.\n\n\
                 REWRITE — if the user asks to rewrite, simplify, or rephrase the selection:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={section_idx_str}, \
                 old_text=\"<the selected text exactly>\", \
                 new_text=\"<the rewritten version that replaces it>\")\n\
                 The new_text must NOT contain the old_text — it fully replaces it.\n\n\
                 {formatting_guidance}\n\n\
                 Do NOT rewrite the entire section unless the user explicitly asks for a rewrite.\n\
                 Do NOT output plain text; only tool calls are visible to the user.",
            )
        } else {
            // No selection — use the existing context format.
            format!(
                "[The user is reading \"{title}\" in the browser reading view, \
                 currently viewing the section titled \"{section_heading}\"]\n\n\
                 {text}\n\n\
                 <!-- READER_TOOL_INSTRUCTIONS -->\n\
                 Section content (for context):\n{content_preview}\n\n\
                 Respond using document tool calls so your answer appears in the reading view.\n\
                 PREFERRED: Use patch_section to insert your answer inline after the relevant passage:\n\
                 patch_section(document_id=\"{doc_id}\", section_index={section_idx_str}, \
                 old_text=\"<passage>\", new_text=\"<passage>\\n\\n<your answer>\")\n\n\
                 ALTERNATIVE: Use append_to_section to add your answer to the end of this section:\n\
                 append_to_section(document_id=\"{doc_id}\", section_index={section_idx_str}, \
                 content=\"<your answer>\", foldable=true, summary=\"<short label>\")\n\n\
                 NEW SECTION: Use add_document_section when the question introduces a new topic:\n\
                 add_document_section(document_id=\"{doc_id}\", after_section_index={section_idx_str}, \
                 heading=\"<heading>\", content=\"<body>\")\n\n\
                 {formatting_guidance}\n\n\
                 Do NOT rewrite the entire section unless the user explicitly asks for a rewrite.\n\
                 Do NOT output plain text; only tool calls are visible to the user.",
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

    /// Handle a read-aloud request from the browser reading view.
    ///
    /// Triggers TTS narration of the specified section if voice/TTS
    /// infrastructure is available.
    fn handle_browser_request_read_aloud(&mut self, section_index: usize) {
        let section = self.reading_view_browser_sections.get(section_index);
        let Some((_heading, _content)) = section else {
            tracing::warn!(
                "Browser requestReadAloud for section {section_index} but section not found"
            );
            return;
        };

        // Send only the body content (not the heading) to TTS.
        //
        // The browser's karaoke system (`_walkAndWrap`) wraps words only
        // inside `.section-content`, not the heading `<h2>`.  If we
        // included the heading in the TTS text, the heading words would
        // be spoken and counted in the word-alignment timeline but would
        // have no corresponding `.kw` spans in the DOM, causing the
        // karaoke highlight to lag behind the audio by the number of
        // heading words.  The heading is already displayed as a static
        // element, so there is no need to narrate it.
        // Emit a VoiceModeNarrateSection event so the existing TTS pipeline
        // handles it (with caching, alignment, etc.).
        #[cfg(not(target_os = "linux"))]
        self.app_event_tx.send(AppEvent::VoiceModeNarrateSection {
            document_id: self.reading_view_browser_doc_id.clone(),
            section_index,
            text: _content.clone(),
            selection_word_offset: None,
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

        // Browser mode: send outline first, then fill sections progressively.
        if self.is_reading_view_browser_mode() {
            self.flush_active_cell();

            let sections: Vec<(String, String)> = Self::reading_view_sections_parsed(&ev.content)
                .into_iter()
                .map(|(h, c)| {
                    (
                        crate::text_formatting::strip_voice_tags(&h),
                        crate::text_formatting::strip_voice_tags(&c),
                    )
                })
                .collect();

            // Store browser document state for follow-up questions and read-aloud.
            self.reading_view_browser_title = ev.title.clone();
            self.reading_view_browser_doc_id = ev.document_id.clone();
            self.reading_view_browser_sections = sections.clone();

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

            self.add_info_message(
                format!("Reading view opened in browser: {}", ev.title),
                None,
            );
            self.request_redraw();
            return;
        }

        self.flush_active_cell();

        // On session resume, show a collapsed DocumentCell instead of opening
        // the full reading view.  The document cache in codex-core is already
        // pre-populated from the same replayed events, so asking the agent to
        // reopen will be instant.
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

            // Keep browser section cache in sync.
            if let Some(sec) = self.reading_view_browser_sections.get_mut(ev.section_index) {
                sec.1 = clean_content.clone();
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

            // Keep browser section cache in sync (append content).
            if let Some(sec) = self.reading_view_browser_sections.get_mut(ev.section_index) {
                if !sec.1.is_empty() {
                    sec.1.push_str("\n\n");
                }
                sec.1.push_str(&clean_content);
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
        if let Some(ref mut state) = self.voice_mode_state {
            let offset = state.selection_word_offset.unwrap_or(0);
            let adjusted = word_index.saturating_sub(offset);
            state.tts_highlight_word_idx = Some(adjusted);

            // Seek the audio to the target word's start time.
            if let Some(entry) = state.tts_alignment_timeline.get(adjusted) {
                let target_ms = entry.start_ms;
                if let Some(ref player) = state.audio_player {
                    player.seek_to_ms(target_ms);
                }
            }
        }
        // Forward the updated word position back to the browser.
        let ws_msg = serde_json::json!({
            "type": "karaokeWord",
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
