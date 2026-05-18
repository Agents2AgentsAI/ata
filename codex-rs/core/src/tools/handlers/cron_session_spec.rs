//! Responses API tool definitions for IN-SESSION cron scheduling.
//!
//! These specs sit alongside the OS-cron specs in `cron_spec.rs`. The agent
//! must pick between them based on whether the user wants the schedule to
//! survive ata closing. Each tool's description leans on a clear "use when"
//! / "don't use when" contrast with the OS-cron equivalent.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const CRON_SESSION_CREATE_TOOL_NAME: &str = "cron_create_session";
pub const CRON_SESSION_LIST_TOOL_NAME: &str = "cron_list_session";
pub const CRON_SESSION_DELETE_TOOL_NAME: &str = "cron_delete_session";

// @agent-facing
pub fn create_cron_session_create_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "cron_expr".to_string(),
            JsonSchema::string(Some(
                "Required. 6-field cron expression `sec min hour day-of-month month day-of-week`. Examples: `0 0 * * * *` (every hour on the hour), `0 0 9 * * 1-5` (weekdays at 09:00:00), `0 */5 * * * *` (every 5 minutes). Sub-minute granularity is allowed here (unlike `cron_create`, which is bound by OS-cron's 1-minute floor)."
                    .to_string(),
            )),
        ),
        (
            "prompt".to_string(),
            JsonSchema::string(Some(
                "Required. The text injected as a new user message into THIS chat session each time the schedule fires. The agent receives it as a normal turn — it can reason, call tools, and reference the conversation up to that point."
                    .to_string(),
            )),
        ),
        (
            "background".to_string(),
            JsonSchema::boolean(Some(
                "Optional. Default `true`. Controls whether each firing's natural-language reply is visible in chat.\n\n\
                Pass `false` when the user wants to SEE the result of every firing — \"say X every minute\", \"tell me X each hour\", \"report back\".\n\n\
                Pass `true` (or omit) for alert-style polls where you only want to surface conditions — \"alert me if\", \"only tell me when\". With `background=true`, tool outputs still render but the agent's reply text is hidden so periodic crons don't flood the chat."
                    .to_string(),
            )),
        ),
        (
            "max_firings".to_string(),
            JsonSchema::integer(Some(
                "Optional. Stop after this many total firings. `1` = one-shot reminder. Omit (default) = run until session ends or until manual delete."
                    .to_string(),
            )),
        ),
        (
            "until".to_string(),
            JsonSchema::string(Some(
                "Optional. Stop firing after this RFC3339 timestamp. Useful for finite-duration schedules. If both `max_firings` and `until` are set, whichever triggers first ends the job."
                    .to_string(),
            )),
        ),
        (
            "timezone".to_string(),
            JsonSchema::string(Some(
                "Optional. UTC offset like `+07:00`, `-05:00`, `Z`. Interprets the cron expression as wall-clock time at that offset. Omit = UTC. Fixed offset — does NOT auto-adjust for DST."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: CRON_SESSION_CREATE_TOOL_NAME.to_string(),
        description: r#"Schedule a prompt to fire on a recurring **clock-aligned** schedule, **inside the current ata session**.

This is the IN-SESSION counterpart to `cron_create`. They share clock-aligned semantics but differ on persistence and what each firing looks like:

| | `cron_create_session` (this tool) | `cron_create` (OS cron) |
|---|---|---|
| Persistence | Dies when ata closes (resumes if you re-open the session) | Survives ata closing — OS owns the schedule |
| Each firing | Injects as a new user turn in THIS chat — agent has full conversation memory | Spawns a fresh `ata exec` subprocess with no memory of any chat |
| Visibility | Renders in chat (modulated by `background` flag) | Output goes to a log file; nothing in chat |
| Minimum interval | Sub-minute OK (e.g. `*/30 * * * * *` for every 30 sec) | 1 minute floor (OS cron limit) |
| Extras | Supports `max_firings`, `until`, `timezone` (UTC offset) | None of these in OS cron |

USE THIS TOOL when:
- The schedule should die when the user closes ata ("while I'm working today, every 5 min check X")
- Each firing should reference the current conversation ("every hour, summarize what we've discussed so far")
- The user wants to SEE firings live in chat
- Sub-minute granularity is needed
- The user explicitly says "in this session" / "while I'm here" / "until I close ata"

DO NOT USE THIS TOOL when:
- The schedule must keep firing after ata closes ("every morning at 9am send me X" — use `cron_create`)
- The user wants a persistent automation that runs in the background of their machine (use `cron_create`)
- The user wants interval-from-now timing ("every 30 seconds starting now" — use `loop_start`)

Returns a task_id usable with `cron_delete_session`."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["cron_expr".to_string(), "prompt".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_cron_session_list_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: CRON_SESSION_LIST_TOOL_NAME.to_string(),
        description: r#"List in-session cron jobs scheduled in THIS chat session.

Use when:
- The user asks about session-scoped cron jobs.
- You need a task_id before calling `cron_delete_session`.

Returns each job's task_id, cron expression, prompt, status, next_fire_at, last_fired_at, and fire_count.

To list OS-level (persistent) cron jobs instead, use `cron_list`."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_cron_session_delete_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "task_id".to_string(),
        JsonSchema::string(Some(
            "Required. The task_id returned from `cron_create_session` or shown in `cron_list_session`."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: CRON_SESSION_DELETE_TOOL_NAME.to_string(),
        description: r#"Cancel an in-session cron job by its task_id. No further firings happen.

Use when the user wants to stop a session-scoped cron. If the task_id isn't in this session, returns `deleted: false` rather than erroring.

For OS-level (persistent) cron jobs, use `cron_delete` instead. Call `cron_list_session` first if you don't have the task_id."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["task_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_tool_requires_cron_expr_and_prompt() {
        let ToolSpec::Function(tool) = create_cron_session_create_tool() else {
            panic!("cron_create_session should be a function tool");
        };
        let required = tool.parameters.required.as_ref().expect("required");
        assert!(required.contains(&"cron_expr".to_string()));
        assert!(required.contains(&"prompt".to_string()));
    }

    #[test]
    fn list_tool_has_no_required_args() {
        let ToolSpec::Function(tool) = create_cron_session_list_tool() else {
            panic!("cron_list_session should be a function tool");
        };
        assert!(matches!(
            tool.parameters.required.as_ref().map(|r| r.is_empty()),
            Some(true)
        ));
    }

    #[test]
    fn delete_tool_requires_task_id() {
        let ToolSpec::Function(tool) = create_cron_session_delete_tool() else {
            panic!("cron_delete_session should be a function tool");
        };
        assert_eq!(tool.parameters.required.as_ref().expect("required").len(), 1);
    }
}
