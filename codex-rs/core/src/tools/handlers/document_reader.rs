use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;
use async_trait::async_trait;
use codex_protocol::document_reader::AppendDocumentSectionEvent;
use codex_protocol::document_reader::AppendToSectionArgs;
use codex_protocol::document_reader::PatchDocumentSectionArgs;
use codex_protocol::document_reader::PatchDocumentSectionEvent;
use codex_protocol::document_reader::PresentDocumentArgs;
use codex_protocol::document_reader::PresentDocumentEvent;
use codex_protocol::document_reader::UpdateDocumentSectionArgs;
use codex_protocol::document_reader::UpdateDocumentSectionEvent;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SessionSource;
use regex_lite::Regex;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

/// Strip web-browsing citation markers like `citeturn5view0`,
/// `citeturn1view0turn2view3`, or `citeturn5search0` that the model
/// sometimes injects into reading-view content. These are internal
/// artifacts that appear as garbage to the user.
fn strip_citation_markers(text: &str) -> String {
    static RE: LazyLock<Regex> =
        LazyLock::new(
            || match Regex::new(r"\s*cite(?:turn\d+(?:view|search)\d+)+") {
                Ok(re) => re,
                Err(err) => panic!("invalid citation regex: {err}"),
            },
        );
    RE.replace_all(text, "").into_owned()
}

// ---------------------------------------------------------------------------
// Cached document state
// ---------------------------------------------------------------------------

struct CachedSection {
    heading: String,
    content: String,
}

struct CachedDocument {
    title: String,
    sections: Vec<CachedSection>,
    /// When streaming, tracks the next section index to fill.
    /// `Some(n)` means sections `n..` still need content.
    streaming_next: Option<usize>,
}

impl CachedDocument {
    /// Reconstruct full markdown from cached sections.
    fn to_markdown(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if !section.heading.is_empty() {
                out.push_str("## ");
                out.push_str(&section.heading);
                out.push('\n');
            }
            out.push_str(&section.content);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }
}

/// Parse markdown content into sections split on `## ` headings.
fn parse_sections(content: &str) -> Vec<CachedSection> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_content = String::new();

    for line in content.lines() {
        if let Some(heading_text) = line.strip_prefix("## ") {
            sections.push(CachedSection {
                heading: current_heading,
                content: current_content,
            });
            current_heading = heading_text.trim().to_string();
            current_content = String::new();
        } else {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            current_content.push_str(line);
        }
    }

    sections.push(CachedSection {
        heading: current_heading,
        content: current_content,
    });

    // Drop the empty preamble section when the document starts with `## `.
    if sections.len() > 1 && sections[0].heading.is_empty() && sections[0].content.trim().is_empty()
    {
        sections.remove(0);
    }

    sections
}

/// If the document has unfilled streaming sections, return a reminder string
/// for the agent to continue filling them.
fn streaming_unfilled_reminder(doc: &CachedDocument) -> String {
    if let Some(next) = doc.streaming_next {
        let unfilled: Vec<String> = (next..doc.sections.len()).map(|i| i.to_string()).collect();
        if !unfilled.is_empty() {
            return format!(
                " Note: sections {} still need content. \
                 Continue filling them with update_document_section after answering.",
                unfilled.join(", ")
            );
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Session-scoped cache
// ---------------------------------------------------------------------------

/// Opaque cache stored on `Session` so documents survive across `ToolRouter`
/// rebuilds within the same session.
///
/// The `ToolRouter` (and therefore the `DocumentReaderHandler`) is recreated on
/// every sampling-request roundtrip.  A per-instance cache would lose
/// previously presented documents, causing `append_to_section` /
/// `update_document_section` to fail with "No document with id …".
///
/// Storing the cache on `Session` mirrors how other cross-turn state (e.g.
/// `JsReplHandle`, `FileReferenceCache`) is managed.
pub struct DocumentCache {
    docs: Mutex<HashMap<String, CachedDocument>>,
}

impl DocumentCache {
    pub fn new() -> Self {
        Self {
            docs: Mutex::new(HashMap::new()),
        }
    }

    fn contains(&self, document_id: &str) -> bool {
        self.docs
            .lock()
            .map(|d| d.contains_key(document_id))
            .unwrap_or(false)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CachedDocument>> {
        self.docs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub struct DocumentReaderHandler;

// ---------------------------------------------------------------------------
// Tool specs
// ---------------------------------------------------------------------------

pub static PRESENT_DOCUMENT_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::String {
            description: Some(
                "Unique slug identifying this document for targeted updates".to_string(),
            ),
        },
    );
    properties.insert(
        "title".to_string(),
        JsonSchema::String {
            description: Some("Display title for the document".to_string()),
        },
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::String {
            description: Some(
                "Full markdown content. Use ## headings to define sections.".to_string(),
            ),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "present_reading_view".to_string(),
        description: "Present structured content in a sectioned reading view that the user can \
                       navigate and ask follow-up questions about. Use this instead of inline \
                       text whenever your response is a structured explanation with multiple \
                       sections — paper walkthroughs, deep dives, research briefings, organized \
                       reports, or any multi-section content with ## headings. \
                       FOLLOW-UPS: When a reading view was previously presented and the user \
                       asks ANY follow-up about the same topic (re-explain, simplify, go deeper, \
                       different angle, etc.), ALWAYS use the reading view tools — either \
                       update/append to the existing document or present a fresh one. Never \
                       fall back to plain text for follow-ups on a topic that has a reading view. \
                       Do NOT use this for short answers, confirmations, or conversational \
                       replies unrelated to an active document. \
                       IMPORTANT: Never mention 'KB', 'knowledge base', 'card', or \
                       'card ID' in the title or content — the user cares about the subject \
                       matter, not internal storage. Use the paper/topic name as the title \
                       (e.g. 'Cosmos Policy Walkthrough', not 'KB Walkthrough: paper-cosmos-policy'). \
                       Write as if you understand the material directly, not as if you are \
                       reading from a database entry. FIGURES: The reading view is text-only — \
                       images and figures cannot be displayed. Never include sections like \
                       'Figure Pointers' or 'How to view figures' that tell the user to look \
                       at specific figures. Instead, describe what each important figure shows \
                       inline in the narrative (e.g. 'The architecture diagram shows three \
                       stages connected by…'). After calling this tool, end your response \
                       and wait for user interaction. To re-display a previously presented \
                       document (with all section updates intact), pass only the document_id."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["document_id".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
});

pub static UPDATE_DOCUMENT_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::String {
            description: Some(
                "The document to update (must match a previous present_reading_view call)"
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::Number {
            description: Some("0-based section index to replace".to_string()),
        },
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::String {
            description: Some("New markdown content for the section".to_string()),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "update_document_section".to_string(),
        description: "Replace the entire content of a section in a document being read by the user. \
                       Use this to fill an empty section, or when the user explicitly asks to \
                       rewrite/restructure/simplify the whole section. Prefer patch_document_section \
                       for targeted edits. \
                       Content style: write straight prose that continues the section\u{2019}s \
                       voice. Do NOT prefix with bold/italic topic lines like \
                       '**On the efficiency gains:**' or '*Regarding caching:*' \u{2014} \
                       just write the content directly."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "content".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
});

pub static APPEND_TO_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::String {
            description: Some(
                "The document to update (must match a previous present_reading_view call)"
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::Number {
            description: Some("0-based section index to append to".to_string()),
        },
    );
    properties.insert(
        "content".to_string(),
        JsonSchema::String {
            description: Some("Markdown content to append at the end of the section".to_string()),
        },
    );
    properties.insert(
        "foldable".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "When true, content appears in a collapsible region. Use for supplementary \
                 content (explanations, examples). Default: false."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "summary".to_string(),
        JsonSchema::String {
            description: Some(
                "Short descriptive label for this content (5-10 words). Always provide this. Used as fold title when collapsed."
                    .to_string(),
            ),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "append_to_section".to_string(),
        description: "Append content to the end of a section in a document currently being read. \
                       Use this when adding information to a section without rewriting it entirely."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "content".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
});

pub static PATCH_DOCUMENT_SECTION_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut properties = BTreeMap::new();
    properties.insert(
        "document_id".to_string(),
        JsonSchema::String {
            description: Some(
                "The document to update (must match a previous present_reading_view call)"
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "section_index".to_string(),
        JsonSchema::Number {
            description: Some("0-based section index to patch".to_string()),
        },
    );
    properties.insert(
        "old_text".to_string(),
        JsonSchema::String {
            description: Some("Exact text to find within the section content".to_string()),
        },
    );
    properties.insert(
        "new_text".to_string(),
        JsonSchema::String {
            description: Some("Replacement text".to_string()),
        },
    );
    properties.insert(
        "foldable".to_string(),
        JsonSchema::Boolean {
            description: Some(
                "When true, content appears in a collapsible region. Use for supplementary \
                 content (explanations, examples). Default: false."
                    .to_string(),
            ),
        },
    );
    properties.insert(
        "summary".to_string(),
        JsonSchema::String {
            description: Some(
                "Short descriptive label for this content (5-10 words). Always provide this. Used as fold title when collapsed."
                    .to_string(),
            ),
        },
    );

    ToolSpec::Function(ResponsesApiTool {
        name: "patch_document_section".to_string(),
        description: "Find and replace specific text within a section of a document currently \
                       being read. Use this for targeted edits like fixing a sentence or updating \
                       a specific paragraph without rewriting the entire section."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "document_id".to_string(),
                "section_index".to_string(),
                "old_text".to_string(),
                "new_text".to_string(),
            ]),
            additional_properties: Some(false.into()),
        },
    })
});

// ---------------------------------------------------------------------------
// ToolHandler impl
// ---------------------------------------------------------------------------

#[async_trait]
impl ToolHandler for DocumentReaderHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            tool_name,
            ..
        } = invocation;

        let is_subagent = matches!(turn.session_source, SessionSource::SubAgent(_));

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "document_reader handler received unsupported payload".to_string(),
                ));
            }
        };

        let doc_cache = &session.document_cache;

        let content = match tool_name.as_str() {
            "present_reading_view" => {
                let args: PresentDocumentArgs = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse present_reading_view arguments: {e}"
                    ))
                })?;

                // Resolve title and content: use provided values, or fall back to cache.
                let (title, doc_content, _is_outline_only, _section_count) = {
                    let mut cache = doc_cache.lock();
                    match (args.title, args.content) {
                        (Some(t), Some(c)) => {
                            // New document or full replacement — cache it.
                            let c = strip_citation_markers(&c);
                            let sections = parse_sections(&c);
                            let sec_count = sections.len();
                            // Detect outline-only: all headed sections have empty
                            // content and there are at least 2 sections.
                            let outline = sec_count > 1
                                && sections
                                    .iter()
                                    .all(|s| s.heading.is_empty() || s.content.trim().is_empty());
                            cache.insert(
                                args.document_id.clone(),
                                CachedDocument {
                                    title: t.clone(),
                                    sections,
                                    streaming_next: if outline { Some(0) } else { None },
                                },
                            );
                            (t, c, outline, sec_count)
                        }
                        _ => {
                            // Re-display from cache.
                            if let Some(cached) = cache.get(&args.document_id) {
                                (cached.title.clone(), cached.to_markdown(), false, 0)
                            } else {
                                return Err(FunctionCallError::RespondToModel(format!(
                                    "No cached document with id \"{}\". \
                                     Provide title and content when presenting a document \
                                     for the first time.",
                                    args.document_id
                                )));
                            }
                        }
                    }
                };

                let _doc_id = args.document_id;
                if is_subagent {
                    // In a subagent context the TUI never receives the event
                    // (it only processes events from the active thread).
                    // Return the full document content inline so the parent
                    // agent receives it through the normal wait() path.
                    format!("# {title}\n\n{doc_content}")
                } else {
                    "Document displayed in reading mode. The user can now navigate sections \
                     and ask follow-up questions. For ANY follow-up about this topic \u{2014} \
                     whether about a specific section or a broad request like 'explain more \
                     intuitively' or 'simplify this' \u{2014} use the reading view tools:\n\
                     \n\
                     Placement rule: before inserting content, determine its SCOPE. If the \
                     content spans or references multiple items in a list/sequence (e.g. a \
                     walkthrough of steps 1\u{2013}6), place it AFTER the entire list, not \
                     after the first item it mentions. Match placement to the scope of the \
                     content, not to the first keyword match.\n\
                     \n\
                     Tool choice for follow-ups:\n\
                     - `append_to_section` with `foldable=true` \u{2014} preferred for \
                     elaborations, examples, and walkthroughs. Adds a collapsible block at \
                     the end of the section. Use a clear `fold_summary`.\n\
                     - `update_document_section` \u{2014} for rewriting or restructuring a \
                     section. When inserting new content, respect the placement rule above: \
                     put elaborations after the full structure they reference, never in the \
                     middle of a numbered list or multi-step sequence.\n\
                     - `patch_document_section` \u{2014} for small targeted fixes like \
                     correcting a sentence or updating a specific paragraph.\n\
                     \n\
                     Do NOT output plain text responses \u{2014} always \
                     use reading view tools for follow-ups on this topic. \
                     Content style: write straight prose that continues the section\u{2019}s \
                     voice. Never prefix with bold/italic topic lines like \
                     '**On the efficiency gains:**' \u{2014} just write the content directly."
                        .to_string()
                }
            }
            "update_document_section" => {
                let mut args: UpdateDocumentSectionArgs = serde_json::from_str(&arguments)
                    .map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse update_document_section arguments: {e}"
                        ))
                    })?;
                args.content = strip_citation_markers(&args.content);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the update in the cache and advance streaming state.
                let (streaming_msg, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let mut msg: Option<String> = None;
                    if let Some(doc) = cache.get_mut(&args.document_id) {
                        if let Some(section) = doc.sections.get_mut(args.section_index) {
                            if let Some(rest) = args.content.strip_prefix("## ") {
                                if let Some(nl) = rest.find('\n') {
                                    section.heading = rest[..nl].trim().to_string();
                                    section.content = rest[nl + 1..].to_string();
                                } else {
                                    section.heading = rest.trim().to_string();
                                    section.content = String::new();
                                }
                            } else {
                                section.content = args.content.clone();
                            }
                        }
                        // Advance streaming_next.
                        if let Some(next) = doc.streaming_next {
                            let new_next = next.max(args.section_index) + 1;
                            if new_next < doc.sections.len() {
                                let next_heading = doc.sections[new_next].heading.clone();
                                doc.streaming_next = Some(new_next);
                                msg = Some(format!(
                                    "Section {idx} updated. NOW call \
                                     update_document_section with \
                                     section_index={new_next} ('{next_heading}'). \
                                     Do not output text \u{2014} make the tool call \
                                     immediately.",
                                    idx = args.section_index,
                                ));
                            } else {
                                doc.streaming_next = None;
                                msg = Some(
                                    "All sections filled. Wait for user interaction.".to_string(),
                                );
                            }
                        }
                    }
                    // When not streaming, capture payload to re-open the
                    // reading view in case the user closed it.
                    let reopen = if msg.is_none() {
                        cache
                            .get(&args.document_id)
                            .map(|doc| (doc.title.clone(), doc.to_markdown()))
                    } else {
                        None
                    };
                    (msg, reopen)
                };

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::UpdateDocumentSection(UpdateDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                content: args.content,
                            }),
                        )
                        .await;
                }
                streaming_msg.unwrap_or_else(|| {
                    "Section updated. The user can see the change immediately.".to_string()
                })
            }
            "append_to_section" => {
                let mut args: AppendToSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse append_to_section arguments: {e}"
                        ))
                    })?;
                args.content = strip_citation_markers(&args.content);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the append in the cache.
                let (streaming_reminder, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let reminder = if let Some(doc) = cache.get_mut(&args.document_id) {
                        if let Some(section) = doc.sections.get_mut(args.section_index) {
                            if !section.content.is_empty() && !section.content.ends_with('\n') {
                                section.content.push('\n');
                            }
                            section.content.push_str(&args.content);
                        }
                        streaming_unfilled_reminder(doc)
                    } else {
                        String::new()
                    };
                    // When not streaming, capture payload to re-open the
                    // reading view in case the user closed it.
                    let reopen = if reminder.is_empty() {
                        cache
                            .get(&args.document_id)
                            .map(|doc| (doc.title.clone(), doc.to_markdown()))
                    } else {
                        None
                    };
                    (reminder, reopen)
                };

                let foldable = args.foldable.unwrap_or(false);
                let summary = args.summary;

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::AppendDocumentSection(AppendDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                content: args.content,
                                foldable,
                                summary,
                            }),
                        )
                        .await;
                }
                format!(
                    "Content appended to section. The user can see the change immediately.\
                     {streaming_reminder}"
                )
            }
            "patch_document_section" => {
                let mut args: PatchDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse patch_document_section arguments: {e}"
                        ))
                    })?;
                args.new_text = strip_citation_markers(&args.new_text);
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the patch in the cache.
                let (streaming_reminder, reopen_payload) = {
                    let mut cache = doc_cache.lock();
                    let reminder = if let Some(doc) = cache.get_mut(&args.document_id) {
                        if let Some(section) = doc.sections.get_mut(args.section_index)
                            && section.content.contains(&args.old_text)
                        {
                            section.content =
                                section.content.replacen(&args.old_text, &args.new_text, 1);
                        }
                        streaming_unfilled_reminder(doc)
                    } else {
                        String::new()
                    };
                    // When not streaming, capture payload to re-open the
                    // reading view in case the user closed it.
                    let reopen = if reminder.is_empty() {
                        cache
                            .get(&args.document_id)
                            .map(|doc| (doc.title.clone(), doc.to_markdown()))
                    } else {
                        None
                    };
                    (reminder, reopen)
                };

                let foldable = args.foldable.unwrap_or(false);
                let summary = args.summary;

                // Re-open the reading view if the user closed it (non-streaming).
                if let Some((title, full_content)) = reopen_payload {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PresentDocument(PresentDocumentEvent {
                                call_id: call_id.clone(),
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id.clone(),
                                title,
                                content: full_content,
                            }),
                        )
                        .await;
                }

                if !is_subagent {
                    session
                        .send_event(
                            turn.as_ref(),
                            EventMsg::PatchDocumentSection(PatchDocumentSectionEvent {
                                call_id,
                                turn_id: turn.sub_id.clone(),
                                document_id: args.document_id,
                                section_index: args.section_index,
                                old_text: args.old_text,
                                new_text: args.new_text,
                                foldable,
                                summary,
                            }),
                        )
                        .await;
                }
                format!(
                    "Section patched. The user can see the change immediately.\
                     {streaming_reminder}"
                )
            }
            other => {
                return Err(FunctionCallError::RespondToModel(format!(
                    "document_reader handler received unknown tool: {other}"
                )));
            }
        };

        Ok(ToolOutput::Function {
            body: FunctionCallOutputBody::Text(content),
            success: Some(true),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_citation_markers() {
        assert_eq!(strip_citation_markers("hello citeturn0view0"), "hello");
        assert_eq!(
            strip_citation_markers("quality. citeturn1view10turn7view0"),
            "quality."
        );
        assert_eq!(
            strip_citation_markers("text citeturn5search0 more"),
            "text more"
        );
        assert_eq!(
            strip_citation_markers("mixed citeturn7view0turn5search0turn1view2"),
            "mixed"
        );
        assert_eq!(strip_citation_markers("no markers"), "no markers");
    }
}
