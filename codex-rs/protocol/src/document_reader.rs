use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

/// Arguments for the `present_reading_view` tool call.
///
/// When `title` and `content` are provided, the document is cached and displayed.
/// When only `document_id` is provided, the most recent cached version (including
/// any section updates) is re-displayed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PresentDocumentArgs {
    /// Unique slug identifying this document for targeted updates.
    pub document_id: String,
    /// Display title for the document. Optional when re-displaying a cached document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Full markdown content; sections are split on `## ` headings.
    /// Optional when re-displaying a cached document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Arguments for the `update_document_section` tool call (full replacement).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateDocumentSectionArgs {
    /// The document to update (must match a previous `present_reading_view` call).
    pub document_id: String,
    /// 0-based section index to replace.
    pub section_index: usize,
    /// New markdown content for the section.
    pub content: String,
}

/// Arguments for the `append_to_section` tool call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct AppendToSectionArgs {
    /// The document to update (must match a previous `present_reading_view` call).
    pub document_id: String,
    /// 0-based section index to append to.
    pub section_index: usize,
    /// Markdown content to append at the end of the section.
    pub content: String,
    /// When true, the appended content appears in a collapsible region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foldable: Option<bool>,
    /// Short summary shown when the fold is collapsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Arguments for the `patch_document_section` tool call (find-and-replace).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct PatchDocumentSectionArgs {
    /// The document to update (must match a previous `present_reading_view` call).
    pub document_id: String,
    /// 0-based section index to patch.
    pub section_index: usize,
    /// Exact text to find within the section.
    pub old_text: String,
    /// Replacement text.
    pub new_text: String,
    /// When true, the patched content appears in a collapsible region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foldable: Option<bool>,
    /// Short summary shown when the fold is collapsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Event emitted when the agent calls `present_reading_view`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct PresentDocumentEvent {
    pub call_id: String,
    pub turn_id: String,
    pub document_id: String,
    pub title: String,
    pub content: String,
}

/// Event emitted when the agent calls `update_document_section`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct UpdateDocumentSectionEvent {
    pub call_id: String,
    pub turn_id: String,
    pub document_id: String,
    pub section_index: usize,
    pub content: String,
}

/// Event emitted when the agent calls `append_to_section`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct AppendDocumentSectionEvent {
    pub call_id: String,
    pub turn_id: String,
    pub document_id: String,
    pub section_index: usize,
    pub content: String,
    /// Whether the appended content should be collapsible.
    #[serde(default)]
    pub foldable: bool,
    /// Short summary shown when the fold is collapsed.
    #[serde(default)]
    pub summary: Option<String>,
}

/// Event emitted when the agent calls `patch_document_section`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
pub struct PatchDocumentSectionEvent {
    pub call_id: String,
    pub turn_id: String,
    pub document_id: String,
    pub section_index: usize,
    pub old_text: String,
    pub new_text: String,
    /// Whether the patched content should be collapsible.
    #[serde(default)]
    pub foldable: bool,
    /// Short summary shown when the fold is collapsed.
    #[serde(default)]
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn present_document_args_round_trip() {
        let args = PresentDocumentArgs {
            document_id: "report-1".to_string(),
            title: Some("My Report".to_string()),
            content: Some("## Intro\nHello\n## Body\nWorld".to_string()),
        };
        let json = serde_json::to_string(&args).expect("serialize");
        let decoded: PresentDocumentArgs = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(args.document_id, decoded.document_id);
        assert_eq!(args.title, decoded.title);
        assert_eq!(args.content, decoded.content);
    }

    #[test]
    fn present_document_args_id_only() {
        let json = r#"{"document_id":"report-1"}"#;
        let decoded: PresentDocumentArgs = serde_json::from_str(json).expect("deserialize");
        assert_eq!(decoded.document_id, "report-1");
        assert_eq!(decoded.title, None);
        assert_eq!(decoded.content, None);
    }

    #[test]
    fn update_document_section_args_round_trip() {
        let args = UpdateDocumentSectionArgs {
            document_id: "report-1".to_string(),
            section_index: 2,
            content: "Updated section content".to_string(),
        };
        let json = serde_json::to_string(&args).expect("serialize");
        let decoded: UpdateDocumentSectionArgs = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(args.document_id, decoded.document_id);
        assert_eq!(args.section_index, decoded.section_index);
        assert_eq!(args.content, decoded.content);
    }

    #[test]
    fn append_to_section_args_round_trip() {
        let args = AppendToSectionArgs {
            document_id: "report-1".to_string(),
            section_index: 1,
            content: "Additional paragraphs.".to_string(),
            foldable: Some(true),
            summary: Some("Extra detail".to_string()),
        };
        let json = serde_json::to_string(&args).expect("serialize");
        let decoded: AppendToSectionArgs = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(args.document_id, decoded.document_id);
        assert_eq!(args.section_index, decoded.section_index);
        assert_eq!(args.content, decoded.content);
        assert_eq!(args.foldable, decoded.foldable);
        assert_eq!(args.summary, decoded.summary);
    }

    #[test]
    fn append_to_section_args_backwards_compat() {
        // JSON without foldable/summary fields should deserialize with None defaults.
        let json = r#"{"document_id":"report-1","section_index":1,"content":"text"}"#;
        let decoded: AppendToSectionArgs = serde_json::from_str(json).expect("deserialize");
        assert_eq!(decoded.foldable, None);
        assert_eq!(decoded.summary, None);
    }

    #[test]
    fn patch_document_section_args_round_trip() {
        let args = PatchDocumentSectionArgs {
            document_id: "report-1".to_string(),
            section_index: 0,
            old_text: "old phrase".to_string(),
            new_text: "new phrase".to_string(),
            foldable: Some(true),
            summary: Some("Clarification".to_string()),
        };
        let json = serde_json::to_string(&args).expect("serialize");
        let decoded: PatchDocumentSectionArgs = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(args.document_id, decoded.document_id);
        assert_eq!(args.section_index, decoded.section_index);
        assert_eq!(args.old_text, decoded.old_text);
        assert_eq!(args.new_text, decoded.new_text);
        assert_eq!(args.foldable, decoded.foldable);
        assert_eq!(args.summary, decoded.summary);
    }

    #[test]
    fn patch_document_section_args_backwards_compat() {
        let json = r#"{"document_id":"report-1","section_index":0,"old_text":"a","new_text":"b"}"#;
        let decoded: PatchDocumentSectionArgs = serde_json::from_str(json).expect("deserialize");
        assert_eq!(decoded.foldable, None);
        assert_eq!(decoded.summary, None);
    }

    #[test]
    fn append_event_backwards_compat() {
        // Events without foldable/summary fields should deserialize with defaults.
        let json =
            r#"{"call_id":"c","turn_id":"t","document_id":"d","section_index":0,"content":"x"}"#;
        let decoded: AppendDocumentSectionEvent = serde_json::from_str(json).expect("deserialize");
        assert!(!decoded.foldable);
        assert_eq!(decoded.summary, None);
    }

    #[test]
    fn patch_event_backwards_compat() {
        let json = r#"{"call_id":"c","turn_id":"t","document_id":"d","section_index":0,"old_text":"a","new_text":"b"}"#;
        let decoded: PatchDocumentSectionEvent = serde_json::from_str(json).expect("deserialize");
        assert!(!decoded.foldable);
        assert_eq!(decoded.summary, None);
    }
}
