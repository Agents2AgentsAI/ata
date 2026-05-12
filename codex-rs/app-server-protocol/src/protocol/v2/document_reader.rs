//! ATA reading-view notifications.
//!
//! These wrap `codex_protocol::document_reader::*Event` payloads in the v2
//! ServerNotification framing so the TUI can route document-reader tool
//! results to its overlay.

use codex_protocol::document_reader::AddDocumentSectionEvent;
use codex_protocol::document_reader::AppendDocumentSectionEvent;
use codex_protocol::document_reader::PatchDocumentSectionEvent;
use codex_protocol::document_reader::PresentDocumentEvent;
use codex_protocol::document_reader::UpdateDocumentSectionEvent;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PresentDocumentNotification {
    pub thread_id: String,
    pub event: PresentDocumentEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct UpdateDocumentSectionNotification {
    pub thread_id: String,
    pub event: UpdateDocumentSectionEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AppendDocumentSectionNotification {
    pub thread_id: String,
    pub event: AppendDocumentSectionEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct AddDocumentSectionNotification {
    pub thread_id: String,
    pub event: AddDocumentSectionEvent,
}

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct PatchDocumentSectionNotification {
    pub thread_id: String,
    pub event: PatchDocumentSectionEvent,
}
