//! Prompt snapshot logger — writes ground-truth context breakdown to a JSONL file
//! for every API call. This captures exactly what the model sees, unlike the
//! rollout which records items as they're added to history.
//!
//! Output: `~/.ata/sessions/YYYY/MM/DD/prompt-snapshot-<session-id>.jsonl`

use std::path::Path;
use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::warn;

use crate::client_common::Prompt;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

/// Rough token estimate: ~4 chars per token for English/code.
fn est_tokens(chars: usize) -> usize {
    chars / 4
}

/// Per-API-call snapshot of what was sent to the model.
#[derive(Serialize)]
struct PromptSnapshot {
    timestamp: String,
    model: String,
    turn_id: String,

    base_instructions_chars: usize,
    base_instructions_tokens: usize,

    tools_count: usize,
    tools_chars: usize,
    tools_tokens: usize,

    input_items_count: usize,
    breakdown: Vec<CategoryEntry>,

    total_input_chars: usize,
    total_input_tokens: usize,

    /// Estimated grand total: base_instructions + tools + input.
    total_prompt_chars: usize,
    total_prompt_tokens: usize,
}

#[derive(Serialize)]
struct CategoryEntry {
    category: String,
    count: usize,
    chars: usize,
    tokens: usize,
}

/// Compute and log a prompt snapshot to the session's prompt-snapshot JSONL file.
pub(crate) async fn log_prompt_snapshot(
    prompt: &Prompt,
    model: &str,
    turn_id: &str,
    session_dir: &Path,
    session_id: &str,
) {
    let snapshot = build_snapshot(prompt, model, turn_id);

    let path = snapshot_path(session_dir, session_id);
    if let Err(e) = write_snapshot(&path, &snapshot).await {
        warn!("Failed to write prompt snapshot: {e:#}");
    }
}

fn snapshot_path(session_dir: &Path, session_id: &str) -> PathBuf {
    session_dir.join(format!("prompt-snapshot-{session_id}.jsonl"))
}

fn content_item_chars(item: &ContentItem) -> usize {
    match item {
        ContentItem::InputText { text, .. } => text.len(),
        ContentItem::OutputText { text, .. } => text.len(),
        _ => 0,
    }
}

fn build_snapshot(prompt: &Prompt, model: &str, turn_id: &str) -> PromptSnapshot {
    let base_instructions_chars = prompt.base_instructions.text.len();

    let tools_chars: usize = prompt
        .tools
        .iter()
        .filter_map(|t| serde_json::to_string(t).ok())
        .map(|s| s.len())
        .sum();

    let mut developer_chars: usize = 0;
    let mut developer_count: usize = 0;
    let mut user_chars: usize = 0;
    let mut user_count: usize = 0;
    let mut assistant_chars: usize = 0;
    let mut assistant_count: usize = 0;
    let mut tool_call_chars: usize = 0;
    let mut tool_call_count: usize = 0;
    let mut tool_output_chars: usize = 0;
    let mut tool_output_count: usize = 0;
    let mut reasoning_chars: usize = 0;
    let mut reasoning_count: usize = 0;
    let mut other_chars: usize = 0;
    let mut other_count: usize = 0;

    for item in &prompt.input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let chars: usize = content.iter().map(content_item_chars).sum();
                match role.as_str() {
                    "developer" | "system" => {
                        developer_chars += chars;
                        developer_count += 1;
                    }
                    "user" => {
                        user_chars += chars;
                        user_count += 1;
                    }
                    "assistant" => {
                        assistant_chars += chars;
                        assistant_count += 1;
                    }
                    _ => {
                        other_chars += chars;
                        other_count += 1;
                    }
                }
            }
            ResponseItem::FunctionCall { arguments, .. } => {
                tool_call_chars += arguments.len();
                tool_call_count += 1;
            }
            ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } => {
                let chars = output.body.to_text().map_or(0, |t| t.len());
                tool_output_chars += chars;
                tool_output_count += 1;
            }
            ResponseItem::Reasoning { summary, .. } => {
                let chars: usize = summary
                    .iter()
                    .filter_map(|part| serde_json::to_string(part).ok())
                    .map(|s| s.len())
                    .sum();
                reasoning_chars += chars;
                reasoning_count += 1;
            }
            _ => {
                other_chars += serde_json::to_string(item).map_or(0, |s| s.len());
                other_count += 1;
            }
        }
    }

    let mut breakdown = Vec::new();
    let cats = [
        ("developer", developer_count, developer_chars),
        ("user", user_count, user_chars),
        ("assistant", assistant_count, assistant_chars),
        ("tool_call", tool_call_count, tool_call_chars),
        ("tool_output", tool_output_count, tool_output_chars),
        ("reasoning", reasoning_count, reasoning_chars),
        ("other", other_count, other_chars),
    ];
    for (category, count, chars) in cats {
        if count > 0 {
            breakdown.push(CategoryEntry {
                category: category.to_string(),
                count,
                chars,
                tokens: est_tokens(chars),
            });
        }
    }

    let total_input_chars = developer_chars
        + user_chars
        + assistant_chars
        + tool_call_chars
        + tool_output_chars
        + reasoning_chars
        + other_chars;

    let total_prompt_chars = base_instructions_chars + tools_chars + total_input_chars;

    PromptSnapshot {
        timestamp: Utc::now().to_rfc3339(),
        model: model.to_string(),
        turn_id: turn_id.to_string(),
        base_instructions_chars,
        base_instructions_tokens: est_tokens(base_instructions_chars),
        tools_count: prompt.tools.len(),
        tools_chars,
        tools_tokens: est_tokens(tools_chars),
        input_items_count: prompt.input.len(),
        breakdown,
        total_input_chars,
        total_input_tokens: est_tokens(total_input_chars),
        total_prompt_chars,
        total_prompt_tokens: est_tokens(total_prompt_chars),
    }
}

async fn write_snapshot(path: &Path, snapshot: &PromptSnapshot) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let mut line = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await?;
    Ok(())
}
