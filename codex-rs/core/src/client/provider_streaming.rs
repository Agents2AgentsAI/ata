use codex_api::common::Reasoning;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde_json::Value;

use crate::error::CodexErr;
use crate::error::Result;

/// Serializes input items with proper error handling.
///
/// Unlike `filter_map(...ok())`, this returns an error if any item fails to serialize,
/// preventing incomplete prompts from being sent silently.
pub(super) fn serialize_input_items(input: &[ResponseItem]) -> Result<Vec<Value>> {
    input
        .iter()
        .map(|item| {
            serde_json::to_value(item)
                .map_err(|e| CodexErr::Api(format!("Failed to serialize input item: {e}")))
        })
        .collect()
}

/// Builds provider reasoning payload for non-Responses streaming adapters.
pub(super) fn build_reasoning_value(
    model_info: &ModelInfo,
    effort: Option<ReasoningEffortConfig>,
    summary: ReasoningSummaryConfig,
) -> Option<Value> {
    if !model_info.supports_reasoning_summaries {
        return None;
    }

    let reasoning = Reasoning {
        effort: effort.or(model_info.default_reasoning_level),
        summary: if summary == ReasoningSummaryConfig::None {
            None
        } else {
            Some(summary)
        },
    };

    serde_json::to_value(reasoning).ok()
}
