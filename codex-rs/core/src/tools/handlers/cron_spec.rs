//! Responses API tool definitions for OS-level Cron scheduling.
//!
//! These specs are registered only when [`Feature::Scheduling`] is enabled.
//! Each description includes "use when…" / "don't use when…" guidance so
//! the model picks the right tool from this family (vs. Monitor or Loop).
//!
//! Persistence: schedules created here are written to the user's system
//! crontab. They survive ata exit and reboot. Each firing launches a
//! short-lived `ata exec` subprocess; output is captured to a log file.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const CRON_CREATE_TOOL_NAME: &str = "cron_create";
pub const CRON_LIST_TOOL_NAME: &str = "cron_list";
pub const CRON_DELETE_TOOL_NAME: &str = "cron_delete";

// @agent-facing
pub fn create_cron_create_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "cron_expr".to_string(),
            JsonSchema::string(Some(
                "Required. 5-field cron expression in the OS-cron format: `min hour day-of-month month day-of-week`. Examples: `*/2 * * * *` (every 2 minutes), `0 9 * * 1-5` (weekdays at 09:00), `0 */6 * * *` (every 6 hours), `30 14 1 * *` (14:30 on the 1st of every month). The minimum granularity is 1 minute — sub-minute schedules are NOT supported."
                    .to_string(),
            )),
        ),
        (
            "prompt".to_string(),
            JsonSchema::string(Some(
                "Required. The prompt to send to a fresh `ata exec` subprocess each time the schedule fires. Each firing is a brand-new ata process with NO memory of the current chat session — write the prompt as a self-contained instruction. Output (the agent's reply + any tool output) is captured to a log file; nothing appears in chat."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: CRON_CREATE_TOOL_NAME.to_string(),
        description: r#"Schedule a prompt to run on a recurring **clock-aligned** schedule, persisted at the OS level via the user's system crontab.

KEY FACTS the model must understand before calling this:

1. **Persistence**: schedules survive ata closing, reboot, and re-login. They live in the user's crontab (`crontab -l`), not in any ata session. Closing ata does NOT stop the schedule.

2. **Fresh process per firing**: each firing launches `ata exec "<prompt>"` as a brand-new ata process. The new process has NO memory of the current chat session, no conversation history, no local variables. Write the prompt as fully self-contained.

3. **Output goes to a log file**, not chat. Each job logs to `~/.ata/cron/<task_id>.log`. The user must `tail -f` the file (or ask you to read it) to see firings.

4. **1-minute minimum**: OS cron's smallest interval is 1 minute. `*/30 * * * * *` (every 30 seconds) is impossible.

5. **5-field, not 6-field**: standard POSIX cron has NO seconds column. Format is `min hour day-of-month month day-of-week`.

USE THIS TOOL when the user wants:
- "every Monday at 9am, do X"
- "at the top of every hour, do X"
- "daily at 09:00, do X"
- "every 2 minutes, do X" (2-min is fine; sub-minute is not)
- Any persistent recurring task that should outlive a single ata session

DO NOT USE THIS TOOL when:
- The user wants the schedule to die when ata closes ("every 5 min while I'm working today") — use `cron_create_session`.
- The firing needs the current chat's context ("every hour, summarize what we've discussed") — use `cron_create_session` (in-chat firings have conversation memory) or `loop_start`.
- The user wants sub-minute granularity — OS cron can't do it. Use `cron_create_session` (allows sub-minute) or `loop_start`.
- The user wants to react to streaming output (logs, build progress) — use `monitor_start`.
- The user wants the agent to keep checking until a condition is met, then stop — use `loop_start`.

Rule of thumb: clock-aligned + persistent + can-be-self-contained → cron. Anything else → loop or monitor.

The tool returns `task_id`, `next_fire_at` (UTC RFC3339), and `log_path`. Surface `log_path` to the user so they know where to look.

Examples:
- "Every 2 minutes, create a new file in /tmp" → cron_expr="*/2 * * * *", prompt="run `touch /tmp/scheduled-$(date +%s).file` and exit."
- "Every weekday at 9am, summarize my GitHub notifications" → cron_expr="0 9 * * 1-5", prompt="fetch my GitHub notifications via gh, write a 3-bullet summary to ~/notes/gh-digest-$(date +%F).md, and exit."
- "Daily at 6am, sync new Zotero items" → cron_expr="0 6 * * *", prompt="run the zotero-sync skill to pull new items into my knowledge base, then exit."
"#
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
pub fn create_cron_list_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: CRON_LIST_TOOL_NAME.to_string(),
        description: r#"List all ata-managed cron jobs currently scheduled in the user's system crontab.

Use when:
- The user asks "what's scheduled?", "what cron jobs are running?", or similar.
- You need to look up a task_id before calling cron_delete.

Returns the task_id, 5-field cron expression, the prompt, next_fire_at (UTC), log_path, and created_at for each ata-tagged entry. Entries from other tools or the user's manually-written crontab lines are ignored — only ata's own jobs are returned.

Note: cron jobs are NOT session-scoped. This list reflects all jobs across all ata sessions for the current user. Created in one session, visible from any."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_cron_delete_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "task_id".to_string(),
        JsonSchema::string(Some(
            "Required. The task_id returned from cron_create or shown in cron_list."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: CRON_DELETE_TOOL_NAME.to_string(),
        description: r#"Cancel a scheduled cron job by its task_id. Removes the entry from the user's system crontab. After deletion, the job will not fire again.

Use when:
- The user asks to "cancel", "stop", or "remove" a scheduled task.
- The user no longer needs a recurring schedule.

If the task_id is not found, returns `deleted: false` rather than an error.

Use cron_list first if you don't already have the task_id."#
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
        let ToolSpec::Function(tool) = create_cron_create_tool() else {
            panic!("cron_create should be a function tool");
        };
        let required = tool
            .parameters
            .required
            .as_ref()
            .expect("required fields list");
        assert!(required.contains(&"cron_expr".to_string()));
        assert!(required.contains(&"prompt".to_string()));
    }

    #[test]
    fn list_tool_has_no_required_args() {
        let ToolSpec::Function(tool) = create_cron_list_tool() else {
            panic!("cron_list should be a function tool");
        };
        assert!(matches!(tool.parameters.required.as_ref().map(|r| r.is_empty()), Some(true)));
    }

    #[test]
    fn delete_tool_requires_task_id() {
        let ToolSpec::Function(tool) = create_cron_delete_tool() else {
            panic!("cron_delete should be a function tool");
        };
        assert_eq!(
            tool.parameters.required.as_ref().expect("required").len(),
            1
        );
    }
}
