//! OS-level cron persistence for ata.
//!
//! Replaces the in-memory `CronRegistry` for cron scheduling. Each ata-managed
//! schedule lives as a two-line block in the user's system crontab:
//!
//! ```text
//! # ata-cron:<task_id> | created=<rfc3339>
//! <5-field-schedule> <ata-binary> exec - < <prompt-file> >> <log-file> 2>&1
//! ```
//!
//! - Source of truth: the user's crontab (`crontab -l`).
//! - Side-state: `~/.ata/cron/<task_id>.prompt` (the prompt) and
//!   `~/.ata/cron/<task_id>.log` (output captured by cron).
//!
//! The prompt is stored in a file rather than embedded in the cron command
//! line so we don't have to shell-escape it. The cron command reads it via
//! `ata exec - < <prompt-file>` (stdin), which is bulletproof.
//!
//! This module is platform-specific: macOS and Linux only. Windows has no
//! `crontab` and is not supported.
//!
//! Note: OS cron has 1-minute minimum granularity. Sub-minute schedules
//! (e.g. `*/30 * * * * *`) are rejected with `OsCronError::SubMinuteUnsupported`.

use chrono::DateTime;
use chrono::Utc;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::str::FromStr;
use thiserror::Error;

use crate::cron_job::CronJob;
use crate::task::TaskId;

/// Errors that can come out of `crontab` interaction.
#[derive(Debug, Error)]
pub enum OsCronError {
    #[error("crontab command failed: {0}")]
    CrontabFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid crontab line: {0}")]
    Parse(String),
    #[error(
        "cron expression has sub-minute granularity; OS cron can only schedule at 1-minute resolution"
    )]
    SubMinuteUnsupported,
    #[error("cron expression invalid: {0}")]
    InvalidExpression(String),
    #[error("ata binary not found on PATH (need absolute path for crontab)")]
    BinaryNotFound,
}

/// Tag that identifies our entries inside an arbitrary user crontab. Anything
/// not prefixed with this is ignored by our list/delete operations.
const TAG_PREFIX: &str = "# ata-cron:";

/// Returns `~/.ata/cron/`, creating it if needed. All per-job files
/// (prompts, logs) live inside.
pub fn data_dir() -> Result<PathBuf, OsCronError> {
    let home = std::env::var("HOME")
        .map_err(|_| OsCronError::Io(std::io::Error::other("HOME not set")))?;
    let dir = PathBuf::from(home).join(".ata").join("cron");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn prompt_path(task_id: &TaskId) -> Result<PathBuf, OsCronError> {
    Ok(data_dir()?.join(format!("{task_id}.prompt")))
}

fn log_path(task_id: &TaskId) -> Result<PathBuf, OsCronError> {
    Ok(data_dir()?.join(format!("{task_id}.log")))
}

/// Read the user's current crontab. Returns an empty string if the user
/// has no crontab (the `no crontab for X` case, which `crontab -l` reports
/// via exit code 1 + stderr).
pub fn read_crontab() -> Result<String, OsCronError> {
    let output = Command::new("crontab").arg("-l").output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no crontab") {
            return Ok(String::new());
        }
        return Err(OsCronError::CrontabFailed(stderr.into_owned()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Replace the user's crontab with `content`. Note: this overwrites the
/// entire crontab atomically — read first, modify, write back.
pub fn write_crontab(content: &str) -> Result<(), OsCronError> {
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(content.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(OsCronError::CrontabFailed(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    Ok(())
}

/// Locate the absolute path to the `ata` binary. Cron strips PATH, so we
/// must embed an absolute path in the crontab command. Resolution order:
/// 1. `ATA_BIN` env var (test/dev override).
/// 2. `std::env::current_exe()` — the binary that's currently running. This
///    is what we want: whichever ata creates the cron is the one that runs
///    it, regardless of what `which ata` would find first on PATH.
/// 3. `which ata` as a last resort (e.g. if current_exe is unavailable).
pub fn resolve_ata_binary() -> Result<PathBuf, OsCronError> {
    if let Ok(p) = std::env::var("ATA_BIN") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(p) = std::env::current_exe() {
        return Ok(p);
    }
    let output = Command::new("which").arg("ata").output()?;
    if !output.status.success() {
        return Err(OsCronError::BinaryNotFound);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err(OsCronError::BinaryNotFound);
    }
    Ok(PathBuf::from(path))
}

/// Compute the next firing time (UTC) of a 6-field cron expression. Returns
/// `None` if the expression is invalid or has no future matches.
pub fn next_fire_after_now(cron_expr_six_field: &str) -> Option<DateTime<Utc>> {
    cron::Schedule::from_str(cron_expr_six_field)
        .ok()?
        .upcoming(Utc)
        .next()
}

/// Like `next_fire_after_now` but accepts the 5-field form (no seconds
/// column) by prepending a `0` seconds field.
pub fn next_fire_after_now_five_field(cron_expr_five_field: &str) -> Option<DateTime<Utc>> {
    next_fire_after_now(&format!("0 {cron_expr_five_field}"))
}

/// Convert a 6-field cron expression (which the rest of ata uses) to the
/// 5-field form required by OS cron. Returns `SubMinuteUnsupported` if the
/// seconds field demands sub-minute resolution.
pub fn six_field_to_five(cron_expr: &str) -> Result<String, OsCronError> {
    // Validate first.
    cron::Schedule::from_str(cron_expr)
        .map_err(|e| OsCronError::InvalidExpression(e.to_string()))?;
    let parts: Vec<&str> = cron_expr.split_whitespace().collect();
    if parts.len() != 6 {
        return Err(OsCronError::InvalidExpression(format!(
            "expected 6 fields (sec min hour dom mon dow), got {}",
            parts.len()
        )));
    }
    let seconds = parts[0];
    if seconds != "0" && seconds != "*" {
        return Err(OsCronError::SubMinuteUnsupported);
    }
    // Drop the seconds field for OS cron's 5-field form.
    Ok(parts[1..].join(" "))
}

/// Build the two crontab lines for a job. Returned as a single string with
/// a trailing newline — ready to append to the crontab.
pub fn format_entry(
    job: &CronJob,
    ata_binary: &Path,
    prompt_file: &Path,
    log_file: &Path,
) -> Result<String, OsCronError> {
    let five_field = six_field_to_five(&job.cron_expr)?;
    let comment = format!(
        "{prefix}{id} | created={created}",
        prefix = TAG_PREFIX,
        id = job.id,
        created = job.created_at.to_rfc3339(),
    );
    // Cron runs with a minimal PATH (typically `/usr/bin:/bin`). If `ata`
    // is a wrapper script (the npm distribution is a node shebang), the
    // interpreter it shells out to (e.g. `node`) must be discoverable.
    // Prepend the binary's parent dir to PATH so wrapper scripts find their
    // interpreter without the user having to edit their crontab manually.
    let bin_dir = ata_binary
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path_prefix = if bin_dir.is_empty() {
        String::new()
    } else {
        format!(
            "PATH={}:/usr/bin:/bin ",
            shell_quote_path(Path::new(&bin_dir))
        )
    };
    // `--skip-git-repo-check` is required because cron starts in `$HOME`,
    // which isn't a trusted git repo. The scheduled prompt is the user's
    // own intent so the git-trust gate isn't useful here anyway.
    let command = format!(
        "{schedule} {path}{bin} exec --skip-git-repo-check - < {prompt} >> {log} 2>&1",
        schedule = five_field,
        path = path_prefix,
        bin = shell_quote_path(ata_binary),
        prompt = shell_quote_path(prompt_file),
        log = shell_quote_path(log_file),
    );
    Ok(format!("{comment}\n{command}\n"))
}

/// Minimal shell quoting for paths: wrap in single quotes if the path
/// contains anything beyond safe characters. Single quotes inside are
/// escaped via the `'\''` idiom.
fn shell_quote_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Best-effort runtime stats for a single OS-cron entry, derived from its
/// captured log file. OS cron itself doesn't track per-job firing history, so
/// we infer it from the log: every `ata exec` invocation prints a session
/// header line (`session id: <uuid>`), so counting those gives `fire_count`.
/// `last_fired_at` is the log file's modification time.
#[derive(Debug, Clone, Default)]
pub struct FireStats {
    pub fire_count: u64,
    pub last_fired_at: Option<DateTime<Utc>>,
}

/// Read firing statistics for `task_id` by inspecting its log file. Returns
/// `FireStats::default()` (zero fires, no last-fired) when the log doesn't
/// exist or can't be read — i.e. the job has been scheduled but not fired yet.
pub fn fire_stats(task_id: &TaskId) -> FireStats {
    let Ok(log) = log_path(task_id) else {
        return FireStats::default();
    };
    let last_fired_at = std::fs::metadata(&log)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(DateTime::<Utc>::from);
    let content = std::fs::read_to_string(&log).unwrap_or_default();
    let fire_count = content
        .lines()
        .filter(|l| l.trim_start().starts_with("session id:"))
        .count() as u64;
    FireStats {
        fire_count,
        last_fired_at,
    }
}

/// Insert a job into the user's crontab. Writes the prompt to its sidecar
/// file, then appends the two crontab lines.
pub fn insert(job: &CronJob) -> Result<(), OsCronError> {
    let ata = resolve_ata_binary()?;
    let prompt_file = prompt_path(&job.id)?;
    let log_file = log_path(&job.id)?;

    // Write prompt sidecar atomically (best-effort: write to .tmp, rename).
    let tmp = prompt_file.with_extension("prompt.tmp");
    std::fs::write(&tmp, job.prompt.as_bytes())?;
    std::fs::rename(&tmp, &prompt_file)?;

    let block = format_entry(job, &ata, &prompt_file, &log_file)?;
    let current = read_crontab()?;
    let mut new = current;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(&block);
    write_crontab(&new)?;
    Ok(())
}

/// Remove the two-line block for `task_id` from the crontab. Returns
/// `true` if an entry was removed, `false` if none was found. Also deletes
/// the prompt sidecar (best-effort; log is preserved for inspection).
pub fn delete(task_id: &TaskId) -> Result<bool, OsCronError> {
    let current = read_crontab()?;
    let tag = format!("{TAG_PREFIX}{task_id}");

    let mut out = String::with_capacity(current.len());
    let mut skip_next = false;
    let mut removed = false;
    for line in current.lines() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if line.starts_with(&tag) {
            skip_next = true;
            removed = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    if removed {
        write_crontab(&out)?;
        // Clean up prompt sidecar. Ignore errors — log might still be useful.
        let _ = std::fs::remove_file(prompt_path(task_id)?);
    }
    Ok(removed)
}

/// Parsed view of one ata-cron entry. The schedule line is parsed only for
/// its 5-field expression; the rest of the line (command path, redirects)
/// is reconstructed deterministically by `format_entry`, so we don't try
/// to round-trip it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtaCronEntry {
    pub task_id: TaskId,
    /// 5-field cron expression as it appears in crontab.
    pub cron_expr_five_field: String,
    pub prompt: String,
    pub created_at: Option<DateTime<Utc>>,
    pub log_path: PathBuf,
}

/// List all ata-managed cron entries currently in the user's crontab.
/// Non-ata entries (other tools, user's own crontab lines) are ignored.
pub fn list() -> Result<Vec<AtaCronEntry>, OsCronError> {
    parse_crontab(&read_crontab()?)
}

fn parse_crontab(content: &str) -> Result<Vec<AtaCronEntry>, OsCronError> {
    let mut entries = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line.strip_prefix(TAG_PREFIX) {
            // Tag line: <task_id> | created=... | ...
            let (task_id_str, metadata) = match rest.split_once(" | ") {
                Some((id, rest)) => (id.trim(), rest),
                None => (rest.trim(), ""),
            };
            let task_id = TaskId::from(task_id_str.to_string());

            let created_at = metadata
                .split(" | ")
                .find_map(|kv| kv.strip_prefix("created="))
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            // The next non-blank line is the schedule line.
            let schedule_idx = (i + 1..lines.len()).find(|&j| !lines[j].trim().is_empty());
            let Some(j) = schedule_idx else {
                return Err(OsCronError::Parse(format!(
                    "tag line for {task_id_str} not followed by a schedule line"
                )));
            };
            let schedule_line = lines[j];

            // 5-field expression = first five whitespace-separated tokens.
            let mut parts = schedule_line.split_whitespace();
            let mut five = Vec::with_capacity(5);
            for _ in 0..5 {
                let Some(p) = parts.next() else {
                    return Err(OsCronError::Parse(format!(
                        "schedule line for {task_id_str} has fewer than 5 fields"
                    )));
                };
                five.push(p);
            }
            let cron_expr_five = five.join(" ");

            // Read the prompt sidecar. If missing, prompt is empty (entry
            // partially deleted — still surface it so the agent can clean up).
            let prompt = prompt_path(&task_id)
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            let log = log_path(&task_id).unwrap_or_default();

            entries.push(AtaCronEntry {
                task_id,
                cron_expr_five_field: cron_expr_five,
                prompt,
                created_at,
                log_path: log,
            });
            i = j + 1;
        } else {
            i += 1;
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_field_strips_seconds_when_zero() {
        let five = six_field_to_five("0 */5 * * * *").unwrap();
        assert_eq!(five, "*/5 * * * *");
    }

    #[test]
    fn six_field_strips_seconds_when_star() {
        let five = six_field_to_five("* */5 * * * *").unwrap();
        assert_eq!(five, "*/5 * * * *");
    }

    #[test]
    fn six_field_rejects_sub_minute() {
        let err = six_field_to_five("30 * * * * *").unwrap_err();
        assert!(matches!(err, OsCronError::SubMinuteUnsupported));
    }

    #[test]
    fn six_field_rejects_garbage() {
        let err = six_field_to_five("not a cron").unwrap_err();
        assert!(matches!(err, OsCronError::InvalidExpression(_)));
    }

    #[test]
    fn shell_quote_leaves_safe_paths_unquoted() {
        let p = PathBuf::from("/usr/local/bin/ata");
        assert_eq!(shell_quote_path(&p), "/usr/local/bin/ata");
    }

    #[test]
    fn shell_quote_wraps_paths_with_spaces() {
        let p = PathBuf::from("/Users/tim with space/bin/ata");
        assert_eq!(shell_quote_path(&p), "'/Users/tim with space/bin/ata'");
    }

    #[test]
    fn shell_quote_escapes_embedded_quote() {
        let p = PathBuf::from("/weird'path/ata");
        assert_eq!(shell_quote_path(&p), "'/weird'\\''path/ata'");
    }

    #[test]
    fn parse_finds_one_entry_and_skips_user_lines() {
        let content = "\
# user's own cron
0 9 * * * /Users/me/morning.sh
# ata-cron:abc-123 | created=2026-05-16T10:00:00+00:00
*/2 * * * * /usr/local/bin/ata exec - < /home/me/.ata/cron/abc-123.prompt >> /home/me/.ata/cron/abc-123.log 2>&1
# another user line
30 14 * * 1 /Users/me/weekly.sh
";
        let entries = parse_crontab(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].task_id.as_str(), "abc-123");
        assert_eq!(entries[0].cron_expr_five_field, "*/2 * * * *");
    }

    #[test]
    fn parse_handles_empty_crontab() {
        assert!(parse_crontab("").unwrap().is_empty());
    }

    #[test]
    fn parse_extracts_created_timestamp() {
        let content = "\
# ata-cron:xyz | created=2026-05-16T10:00:00+00:00
0 9 * * * /bin/true
";
        let entries = parse_crontab(content).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].created_at.is_some());
    }

    #[test]
    fn parse_errors_on_dangling_tag() {
        let content = "# ata-cron:dangling\n";
        let err = parse_crontab(content).unwrap_err();
        assert!(matches!(err, OsCronError::Parse(_)));
    }

    #[test]
    fn format_entry_round_trips_through_parse() {
        let job = CronJob::new("0 */5 * * * *".to_string(), "say hi".to_string()).unwrap();
        let bin = PathBuf::from("/usr/local/bin/ata");
        let prompt = PathBuf::from("/tmp/p.prompt");
        let log = PathBuf::from("/tmp/p.log");
        let block = format_entry(&job, &bin, &prompt, &log).unwrap();
        let entries = parse_crontab(&block).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].task_id, job.id);
        assert_eq!(entries[0].cron_expr_five_field, "*/5 * * * *");
    }
}
