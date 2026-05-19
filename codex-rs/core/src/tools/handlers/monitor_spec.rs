//! Responses API tool definitions for in-session Monitor watchers.
//!
//! Registered only when [`Feature::Scheduling`] is enabled. Descriptions
//! include "use when…" / "don't use when…" guidance so the model picks
//! Monitor (stream-driven) over Cron (fixed schedule) or Loop (model-paced
//! retry-until) when appropriate.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub const MONITOR_START_TOOL_NAME: &str = "monitor_start";
pub const MONITOR_LIST_TOOL_NAME: &str = "monitor_list";
pub const MONITOR_STOP_TOOL_NAME: &str = "monitor_stop";
pub const MONITOR_WAIT_TOOL_NAME: &str = "monitor_wait";
pub const MONITOR_WATCH_FOR_TOOL_NAME: &str = "monitor_watch_for";

// @agent-facing
pub fn create_monitor_start_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "command".to_string(),
            JsonSchema::string(Some(
                "Required. Shell command to run in the background. Examples: `tail -F build.log`, `ping -c 30 example.com`, `cargo test 2>&1`."
                    .to_string(),
            )),
        ),
        (
            "background".to_string(),
            JsonSchema::boolean(Some(
                "Optional. Default `true`. Controls per-line visibility in chat.\n\n\
                Pass `false` when the user wants to watch streaming output live — phrases like \"show me each ping\", \"tail and display\", \"stream the log\", \"I want to see every line as it comes\". With `background=false`, every stdout/stderr line renders as a chat cell as it happens.\n\n\
                Pass `true` (or omit) when the user just wants to know when it finishes (or alert on the result), not see every line — phrases like \"wait for X to finish then…\", \"download and summarize\", \"run build and tell me if it fails\". With `background=true`, per-line output is suppressed in chat; the `/scheduling` panel's `lines N` counter still climbs and the terminate summary still fires."
                    .to_string(),
            )),
        ),
        (
            "restart_on_resume".to_string(),
            JsonSchema::boolean(Some(
                "Optional. Default `false`. When `true`, if ata quits or restarts while this monitor is running, the same command is automatically re-spawned on session resume (same task_id, fresh tail buffer).\n\n\
                ONLY set this for safely-restartable, long-running commands:\n\
                - `tail -F /path/to/log` — re-tailing is idempotent\n\
                - `watch -n 5 'kubectl get pods'` — periodic snapshot, fine to restart\n\
                - `python3 -m http.server 8000` — long-lived dev server\n\
                - `ping example.com` — continuous reachability check\n\n\
                NEVER set this for:\n\
                - One-shot builds/tests (`cargo build`, `npm test`, `pytest`) — re-running silently is surprising\n\
                - Destructive commands (`git push`, `rm`, `make deploy`) — never auto-replay\n\
                - Batch processors that emit per-item output — restart would re-process items\n\n\
                Rule of thumb: if the command would behave identically when run a second time and the user expects it to stay alive across sessions, set `true`. Otherwise leave it `false` and let the monitor stay `Interrupted` on resume."
                    .to_string(),
            )),
        ),
        (
            "name".to_string(),
            JsonSchema::string(Some(
                "Optional but strongly recommended. A short human-readable label (2-5 words, e.g. \"Disk monitor\", \"Build tail\") shown alongside the monitor in the user's `/scheduling` panel. Falls back to the short task id when omitted."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: MONITOR_START_TOOL_NAME.to_string(),
        description: r#"Spawn a background shell command. Each output line is streamed to the user's view in real time; on termination you receive a single summary message with status and the tail of output.

Returns immediately with a task_id. To wait for the command to finish and get the final output in the same turn, call `monitor_wait` with that task_id.

Use when:
- The user wants to run a long-running command (build, test, ping, log tail) and react to the result.
- There is no fixed schedule — work happens whenever output appears.

Don't use when:
- The user wants something to happen on a fixed schedule (every minute, daily at 9am) — use cron_create.
- The user wants model-paced retries until a condition is met — use the loop tool.

The command runs until it exits naturally or you call `monitor_stop`.

USEFUL PATTERNS (compose monitor_start with the right command for the trigger you want):

**File-watch** — fire when a file or directory changes (works while ata is open):
- macOS: `monitor_start command="fswatch /path/to/file"` then `monitor_watch_for pattern=".*"`. Needs `brew install fswatch`.
- Linux: `monitor_start command="inotifywait -m -e modify /path/to/file"` then `monitor_watch_for pattern="MODIFY"`. Needs `apt install inotify-tools`.
Each file change emits one line; `monitor_watch_for` wakes the agent on the match.

**Receive a webhook / one-way HTTP signal** — listen on a localhost port (works while ata is open):
- `monitor_start command="ncat -l 8765 --keep-open"` then `monitor_watch_for pattern="^POST"`. Needs `ncat` (from nmap).
- Caveats: no HTTP response is sent back to the caller (connection just closes); no path routing; no header parsing. Fine for simple "ping me when X happens" hooks, not for production integrations.

NOTE on persistence: monitor (and these patterns) only fire while ata is open. For schedules that must survive ata closing, the only available tool is `cron_create` (OS cron). There is no persistent file-watch or webhook tool today."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["command".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_monitor_list_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: MONITOR_LIST_TOOL_NAME.to_string(),
        description: r#"List all background monitors active in this session.

Use when:
- The user asks "what's being watched?", "what monitors are running?", or similar.
- You need to look up a task_id before calling monitor_stop.

Returns each monitor's task_id, command, status (Pending / Running / Completed / Failed / Killed), started_at, stopped_at, and lines_emitted count."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_monitor_watch_for_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "task_id".to_string(),
            JsonSchema::string(Some(
                "Required. The task_id returned from monitor_start.".to_string(),
            )),
        ),
        (
            "pattern".to_string(),
            JsonSchema::string(Some(
                "Required. Literal substring to look for in each output line (case-sensitive). The call returns the moment a matching line appears on stdout or stderr. The tail buffer is also scanned first so matches that happened before this call started are not missed."
                    .to_string(),
            )),
        ),
        (
            "timeout_seconds".to_string(),
            JsonSchema::number(Some(
                "Optional. Maximum seconds to wait before returning even if no line has matched yet. Omit to wait indefinitely — the call stays alive until either (a) a matching line appears, or (b) the subprocess terminates without ever matching."
                    .to_string(),
            )),
        ),
        (
            "all_matches".to_string(),
            JsonSchema::boolean(Some(
                "Optional. Default `false`. When `false`, the call returns on the FIRST matching line — use this when reacting to a one-off event (server ready, first error, panic line, etc).\n\nWhen `true`, the call keeps collecting EVERY matching line until the subprocess terminates, an explicit `timeout_seconds` fires, or `max_matches` is reached. Used for batch workflows: \"as each of 50 papers downloads, give me the line\" / \"every test that emits 'FAILED:' in this run.\" The response carries the full list under `matches`."
                    .to_string(),
            )),
        ),
        (
            "max_matches".to_string(),
            JsonSchema::number(Some(
                "Optional. Only meaningful with `all_matches=true`. Stop collecting after this many matches even if the subprocess keeps running. Omit to collect every match for the lifetime of the subprocess."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: MONITOR_WATCH_FOR_TOOL_NAME.to_string(),
        description: r#"Block until a running monitor emits a line containing `pattern`, or until the subprocess terminates without ever matching, or until an explicit `timeout_seconds` fires. Lighter than `monitor_wait` when the caller only cares about a specific event (a phase boundary, an error keyword, a "ready" signal) rather than the final result.

By default this waits indefinitely and returns on the FIRST matching line. Matching is a literal substring on the raw line payload (not the `[stdout]` / `[stderr]` tag).

Batch mode: pass `all_matches=true` to collect EVERY matching line over the lifetime of the subprocess (or until `max_matches` / `timeout_seconds`). Used for "react to each item in a batch" workflows — every paper processed, every test that fails, every download that completes.

Returns: {
  matched: bool,
  matching_line: string | null,        # first match (single-match mode, or convenience first-match in batch mode)
  stream: "stdout" | "stderr" | null,  # only set when at least one match
  matches: [{ matching_line, stream }] # populated only when all_matches=true, contains every match in order
  terminated_without_match: bool,
  timed_out: bool
}.

Use when:
- The user asked you to react to a *specific* phrase in the output ("tell me when the build prints 'warning:'", "alert me when the server says 'listening on port'", "wait for 'PANIC' to appear").
- You want to act mid-stream without waiting for the full subprocess to finish.
- The user is processing a batch and wants per-item feedback ("tell me as each paper downloads", "as each test fails, list it") — pass `all_matches=true`.

Don't use when:
- The user wants the *final result* of the command, including exit status — use `monitor_wait` instead.
- The user wants to keep watching forever — just call `monitor_start` with `background=false` and let the per-line stream render in chat."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["task_id".to_string(), "pattern".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

// @agent-facing
pub fn create_monitor_wait_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "task_id".to_string(),
            JsonSchema::string(Some(
                "Required. The task_id returned from monitor_start.".to_string(),
            )),
        ),
        (
            "timeout_seconds".to_string(),
            JsonSchema::number(Some(
                "Optional. Maximum seconds to wait before returning even if the monitor is still running. Omit to wait indefinitely — the call stays alive for as long as the subprocess takes (minutes, hours, days). Set a value only when the caller explicitly wants to give up after some bound (e.g. \"check back in 30 seconds\")."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: MONITOR_WAIT_TOOL_NAME.to_string(),
        description: r#"Block until a running monitor terminates and return its final status plus the tail of its output. Use this right after monitor_start when you want the result in the same turn instead of polling.

By default this call waits indefinitely — there is no internal time cap. The function sits in a cheap polling loop against an in-memory registry, costs no LLM tokens while waiting, and returns the instant the subprocess actually exits. Set `timeout_seconds` only when the caller explicitly wants a bounded wait.

Returns: { status: "Completed" | "Failed" | "Killed" | "Running" (only if an explicit timeout fires), tail: [string] }.

Use when:
- You started a monitor and want to use its output in the same response (e.g. report the result, summarize errors).
- The user asked for a result that depends on a command finishing.

Don't use when:
- The user wants the monitor to keep running in the background — just call monitor_start and move on."#
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

// @agent-facing
pub fn create_monitor_stop_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "task_id".to_string(),
        JsonSchema::string(Some(
            "Required. The task_id returned from monitor_start or shown in monitor_list."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: MONITOR_STOP_TOOL_NAME.to_string(),
        description: r#"Stop a running background monitor by its task_id. The underlying process is killed and no further output is injected.

Use when:
- The user asks to "stop watching", "kill the monitor", or "cancel" a watch task.

If the task_id is not found, returns a not-found result rather than an error. Use monitor_list first if you don't already have the task_id."#
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
    fn start_tool_requires_command() {
        let ToolSpec::Function(tool) = create_monitor_start_tool() else {
            panic!("monitor_start should be a function tool");
        };
        assert_eq!(
            tool.parameters.required.as_ref().expect("required").len(),
            1
        );
    }

    #[test]
    fn stop_tool_requires_task_id() {
        let ToolSpec::Function(tool) = create_monitor_stop_tool() else {
            panic!("monitor_stop should be a function tool");
        };
        assert_eq!(
            tool.parameters.required.as_ref().expect("required").len(),
            1
        );
    }

    #[test]
    fn list_tool_has_no_required_args() {
        let ToolSpec::Function(tool) = create_monitor_list_tool() else {
            panic!("monitor_list should be a function tool");
        };
        assert!(matches!(
            tool.parameters.required.as_ref().map(Vec::is_empty),
            Some(true)
        ));
    }
}
