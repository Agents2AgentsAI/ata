//! In-session scheduling primitives for ata: Cron, Monitor, and Loop.
//!
//! Phase 1 scope: data types only. Higher-level concerns — agent tools,
//! TUI surface, structured logs, resume semantics — land in later phases.
//! All call sites are gated by `codex_features::Feature::Scheduling`.

pub mod cron_job;
pub mod loop_registry;
pub mod loop_task;
pub mod monitor;
pub mod monitor_registry;
pub mod os_cron;
pub mod persist;
pub mod registry;
pub mod task;

pub use cron_job::CRON_FIRE_HISTORY_CAPACITY;
pub use cron_job::CronError;
pub use cron_job::CronFireRecord;
pub use cron_job::CronJob;
pub use loop_registry::LoopRegistry;
pub use loop_task::LOOP_ITERATION_HISTORY_CAPACITY;
pub use loop_task::LoopIterationRecord;
pub use loop_task::LoopTask;
pub use monitor::MonitorTask;
pub use monitor_registry::MonitorRegistry;
pub use persist::SchedulingSnapshot;
pub use persist::load as load_scheduling_state;
pub use persist::save as save_scheduling_state;
pub use persist::scheduling_state_path;
pub use registry::CronRegistry;
pub use task::TaskId;
pub use task::TaskKind;
pub use task::TaskStatus;
