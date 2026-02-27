use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

/// A complete job definition as parsed from a TOML file in `~/.ata/jobs/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub task: TaskSpec,
    pub schedule: ScheduleSpec,
    #[serde(default)]
    pub execution: ExecutionPolicy,
}

/// What the job executes — either a skill reference or an inline prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Skill name (references a SKILL.md in `~/.ata/skills/`).
    pub skill: Option<String>,
    /// Inline prompt text (mutually exclusive with `skill`).
    pub prompt: Option<String>,
    /// Additional context prepended to the prompt or skill.
    pub context: Option<String>,
    /// Working directory for the job execution.
    pub cwd: Option<PathBuf>,
    /// Model and sandbox overrides.
    #[serde(default)]
    pub config: TaskConfigOverrides,
}

/// Overrides for model, sandbox, and other exec settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskConfigOverrides {
    pub model: Option<String>,
    pub sandbox_mode: Option<String>,
}

/// When the job should run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleSpec {
    /// Cron expression (e.g. `"0 7 * * *"`).
    pub cron: Option<String>,
    /// Fixed interval in minutes.
    pub interval_minutes: Option<u64>,
    /// IANA timezone name (e.g. `"America/Los_Angeles"`). Defaults to local.
    pub timezone: Option<String>,
    /// Event-based trigger (alternative to cron/interval).
    pub trigger: Option<TriggerSpec>,
}

/// Event-based trigger that the daemon monitors.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerSpec {
    /// Watch a filesystem path for changes.
    FileWatch {
        path: PathBuf,
        /// Optional glob pattern to filter changed files.
        pattern: Option<String>,
        /// Seconds to wait after last change before firing (default 5).
        #[serde(default = "default_debounce")]
        debounce_seconds: u64,
    },
    /// Poll an HTTP endpoint for changes.
    HttpPoll {
        url: String,
        /// Polling interval in seconds.
        interval_seconds: u64,
        /// Optional HTTP headers (e.g. auth tokens).
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
        /// JSON path to extract a value for change detection.
        change_path: Option<String>,
    },
    /// Listen for incoming POST requests on a local HTTP server path.
    Webhook {
        /// URL path (e.g. `"/hooks/deploy-done"`).
        path: String,
        /// Secret name (from `ata secrets`) for validating requests.
        secret_name: Option<String>,
    },
}

/// Controls job execution behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    /// Maximum run time in minutes before the job is killed.
    #[serde(default = "default_timeout")]
    pub timeout_minutes: u64,
    /// Number of retries on failure.
    #[serde(default)]
    pub max_retries: u32,
    /// Seconds to wait between retries.
    #[serde(default = "default_retry_delay")]
    pub retry_delay_seconds: u64,
    /// Skip this run if the job is already running.
    #[serde(default)]
    pub skip_if_running: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            timeout_minutes: default_timeout(),
            max_retries: 0,
            retry_delay_seconds: default_retry_delay(),
            skip_if_running: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_timeout() -> u64 {
    30
}

fn default_retry_delay() -> u64 {
    60
}

fn default_debounce() -> u64 {
    5
}
