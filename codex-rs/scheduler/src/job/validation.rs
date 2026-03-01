use anyhow::bail;

use super::JobDefinition;

/// Normalize a cron expression: the `cron` crate expects 6 or 7 fields
/// (sec min hour dom month dow [year]), but users typically write standard
/// 5-field Unix cron (min hour dom month dow). This prepends `"0 "` for
/// the seconds field when a 5-field expression is detected.
pub fn normalize_cron(expr: &str) -> String {
    let fields = expr.split_whitespace().count();
    if fields == 5 {
        format!("0 {expr}")
    } else {
        expr.to_string()
    }
}

/// Validate a parsed job definition for logical consistency.
pub fn validate_job(id: &str, def: &JobDefinition) -> anyhow::Result<()> {
    if id.is_empty() {
        bail!("job ID cannot be empty");
    }

    // Task: must have exactly one of skill or prompt.
    match (&def.task.skill, &def.task.prompt) {
        (Some(_), Some(_)) => {
            bail!("job '{id}': `task.skill` and `task.prompt` are mutually exclusive")
        }
        (None, None) => bail!("job '{id}': must specify either `task.skill` or `task.prompt`"),
        _ => {}
    }

    // Schedule: must have exactly one of cron, interval_minutes, or trigger.
    let schedule_count = def.schedule.cron.is_some() as u8
        + def.schedule.interval_minutes.is_some() as u8
        + def.schedule.trigger.is_some() as u8;
    if schedule_count == 0 {
        bail!(
            "job '{id}': must specify one of `schedule.cron`, `schedule.interval_minutes`, or `schedule.trigger`"
        );
    }
    if schedule_count > 1 {
        bail!(
            "job '{id}': `schedule.cron`, `schedule.interval_minutes`, and `schedule.trigger` are mutually exclusive"
        );
    }

    // Validate cron expression if provided (normalize 5-field to 6-field).
    if let Some(cron_expr) = &def.schedule.cron {
        let normalized = normalize_cron(cron_expr);
        normalized.parse::<cron::Schedule>().map_err(|e| {
            anyhow::anyhow!("job '{id}': invalid cron expression '{cron_expr}': {e}")
        })?;
    }

    // Validate interval is positive.
    if let Some(interval) = def.schedule.interval_minutes
        && interval == 0
    {
        bail!("job '{id}': `schedule.interval_minutes` must be > 0");
    }

    // Validate execution policy.
    if def.execution.timeout_minutes == 0 {
        bail!("job '{id}': `execution.timeout_minutes` must be > 0");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::definition::ExecutionPolicy;
    use crate::job::definition::ScheduleSpec;
    use crate::job::definition::TaskConfigOverrides;
    use crate::job::definition::TaskSpec;

    use pretty_assertions::assert_eq;

    fn minimal_job(skill: Option<&str>, prompt: Option<&str>) -> JobDefinition {
        JobDefinition {
            name: "test".into(),
            description: String::new(),
            enabled: true,
            task: TaskSpec {
                skill: skill.map(String::from),
                prompt: prompt.map(String::from),
                context: None,
                cwd: None,
                config: TaskConfigOverrides::default(),
            },
            schedule: ScheduleSpec {
                cron: Some("0 * * * *".into()),
                interval_minutes: None,
                timezone: None,
                trigger: None,
            },
            execution: ExecutionPolicy::default(),
        }
    }

    #[test]
    fn valid_job_with_skill() {
        assert!(validate_job("my-job", &minimal_job(Some("my-skill"), None)).is_ok());
    }

    #[test]
    fn valid_job_with_prompt() {
        assert!(validate_job("my-job", &minimal_job(None, Some("do something"))).is_ok());
    }

    #[test]
    fn rejects_both_skill_and_prompt() {
        let result = validate_job("bad", &minimal_job(Some("s"), Some("p")));
        assert!(result.is_err());
        assert_eq!(
            result
                .unwrap_err()
                .to_string()
                .contains("mutually exclusive"),
            true,
        );
    }

    #[test]
    fn rejects_neither_skill_nor_prompt() {
        let result = validate_job("bad", &minimal_job(None, None));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_empty_id() {
        let result = validate_job("", &minimal_job(Some("s"), None));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_cron() {
        let mut job = minimal_job(Some("s"), None);
        job.schedule.cron = Some("not a cron".into());
        let result = validate_job("bad-cron", &job);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string().contains("invalid cron"),
            true,
        );
    }

    #[test]
    fn rejects_zero_interval() {
        let mut job = minimal_job(Some("s"), None);
        job.schedule.cron = None;
        job.schedule.interval_minutes = Some(0);
        assert!(validate_job("zero", &job).is_err());
    }
}
