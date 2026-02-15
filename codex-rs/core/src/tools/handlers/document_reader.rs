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
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

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
                       reports, or any long-form content (roughly 500+ words) with ## headings. \
                       Do NOT use this for short answers, confirmations, or conversational \
                       replies. After calling this tool, end your response and wait for user \
                       interaction. To re-display a previously presented document (with all \
                       section updates intact), pass only the document_id."
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
        description: "Update a specific section of a document currently being read by the user. \
                       Use this when the user asks a follow-up question about a section."
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
                let (title, doc_content) = {
                    let mut cache = doc_cache.lock();
                    match (args.title, args.content) {
                        (Some(t), Some(c)) => {
                            // New document or full replacement — cache it.
                            let sections = parse_sections(&c);
                            cache.insert(
                                args.document_id.clone(),
                                CachedDocument {
                                    title: t.clone(),
                                    sections,
                                },
                            );
                            (t, c)
                        }
                        _ => {
                            // Re-display from cache.
                            if let Some(cached) = cache.get(&args.document_id) {
                                (cached.title.clone(), cached.to_markdown())
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

                session
                    .send_event(
                        turn.as_ref(),
                        EventMsg::PresentDocument(PresentDocumentEvent {
                            call_id,
                            turn_id: turn.sub_id.clone(),
                            document_id: args.document_id,
                            title,
                            content: doc_content,
                        }),
                    )
                    .await;
                "Document displayed in reading mode. The user can now navigate sections \
                 and ask follow-up questions. When the user asks about a section, use \
                 `append_to_section` to add your answer below the existing content (preferred). \
                 Use `update_document_section` only if the user asks you to rewrite a section. \
                 Do NOT output plain text responses \u{2014} only tool calls are visible to the user."
                    .to_string()
            }
            "update_document_section" => {
                let args: UpdateDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse update_document_section arguments: {e}"
                        ))
                    })?;
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the update in the cache.
                {
                    let mut cache = doc_cache.lock();
                    if let Some(doc) = cache.get_mut(&args.document_id)
                        && let Some(section) = doc.sections.get_mut(args.section_index)
                    {
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
                }

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
                "Section updated. The user can see the change immediately. \
                 Do NOT call present_reading_view again."
                    .to_string()
            }
            "append_to_section" => {
                let args: AppendToSectionArgs = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse append_to_section arguments: {e}"
                    ))
                })?;
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the append in the cache.
                {
                    let mut cache = doc_cache.lock();
                    if let Some(doc) = cache.get_mut(&args.document_id)
                        && let Some(section) = doc.sections.get_mut(args.section_index)
                    {
                        if !section.content.is_empty() && !section.content.ends_with('\n') {
                            section.content.push('\n');
                        }
                        section.content.push_str(&args.content);
                    }
                }

                session
                    .send_event(
                        turn.as_ref(),
                        EventMsg::AppendDocumentSection(AppendDocumentSectionEvent {
                            call_id,
                            turn_id: turn.sub_id.clone(),
                            document_id: args.document_id,
                            section_index: args.section_index,
                            content: args.content,
                        }),
                    )
                    .await;
                "Content appended to section. The user can see the change immediately. \
                 Do NOT call present_reading_view again."
                    .to_string()
            }
            "patch_document_section" => {
                let args: PatchDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse patch_document_section arguments: {e}"
                        ))
                    })?;
                if !doc_cache.contains(&args.document_id) {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "No document with id \"{}\" is currently being viewed. \
                         Call present_reading_view first to display a document.",
                        args.document_id
                    )));
                }

                // Mirror the patch in the cache.
                {
                    let mut cache = doc_cache.lock();
                    if let Some(doc) = cache.get_mut(&args.document_id)
                        && let Some(section) = doc.sections.get_mut(args.section_index)
                        && section.content.contains(&args.old_text)
                    {
                        section.content =
                            section.content.replacen(&args.old_text, &args.new_text, 1);
                    }
                }

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
                        }),
                    )
                    .await;
                "Section patched. The user can see the change immediately. \
                 Do NOT call present_reading_view again."
                    .to_string()
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
