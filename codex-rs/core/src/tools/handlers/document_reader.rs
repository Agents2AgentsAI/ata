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
use std::sync::LazyLock;

pub struct DocumentReaderHandler;

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
        name: "present_document".to_string(),
        description: "Present a long document in sectioned reading mode. The user can navigate \
                       sections and ask follow-up questions. After calling this tool, end your \
                       response and wait for user interaction."
            .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec![
                "document_id".to_string(),
                "title".to_string(),
                "content".to_string(),
            ]),
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
                "The document to update (must match a previous present_document call)".to_string(),
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
                "The document to update (must match a previous present_document call)".to_string(),
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
                "The document to update (must match a previous present_document call)".to_string(),
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

        let content = match tool_name.as_str() {
            "present_document" => {
                let args: PresentDocumentArgs = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse present_document arguments: {e}"
                    ))
                })?;
                session
                    .send_event(
                        turn.as_ref(),
                        EventMsg::PresentDocument(PresentDocumentEvent {
                            call_id,
                            turn_id: turn.sub_id.clone(),
                            document_id: args.document_id,
                            title: args.title,
                            content: args.content,
                        }),
                    )
                    .await;
                "Document displayed in reading mode".to_string()
            }
            "update_document_section" => {
                let args: UpdateDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse update_document_section arguments: {e}"
                        ))
                    })?;
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
                "Section updated".to_string()
            }
            "append_to_section" => {
                let args: AppendToSectionArgs = serde_json::from_str(&arguments).map_err(|e| {
                    FunctionCallError::RespondToModel(format!(
                        "failed to parse append_to_section arguments: {e}"
                    ))
                })?;
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
                "Content appended to section".to_string()
            }
            "patch_document_section" => {
                let args: PatchDocumentSectionArgs =
                    serde_json::from_str(&arguments).map_err(|e| {
                        FunctionCallError::RespondToModel(format!(
                            "failed to parse patch_document_section arguments: {e}"
                        ))
                    })?;
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
                "Section patched".to_string()
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
