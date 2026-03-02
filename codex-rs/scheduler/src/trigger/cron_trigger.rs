use std::str::FromStr;

use chrono::Utc;

/// Parse a cron expression and return the next occurrence as a Unix timestamp.
/// Accepts both 5-field (standard Unix) and 6/7-field (cron crate) formats.
pub fn next_occurrence(cron_expr: &str) -> anyhow::Result<i64> {
    let normalized = crate::job::normalize_cron(cron_expr);
    let schedule = cron::Schedule::from_str(&normalized)?;
    schedule
        .upcoming(Utc)
        .next()
        .map(|dt| dt.timestamp())
        .ok_or_else(|| anyhow::anyhow!("cron expression '{cron_expr}' has no upcoming occurrence"))
}

/// Check if a cron expression is valid (accepts 5, 6, or 7-field formats).
pub fn is_valid(cron_expr: &str) -> bool {
    let normalized = crate::job::normalize_cron(cron_expr);
    cron::Schedule::from_str(&normalized).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_cron_expressions() {
        // Standard 5-field Unix cron (auto-normalized to 6-field).
        assert!(is_valid("0 * * * *"));
        assert!(is_valid("0 7 * * *"));
        assert!(is_valid("*/5 * * * *"));
        assert!(is_valid("0 0 * * 1-5"));
        // Also accepts native 6-field format.
        assert!(is_valid("0 0 7 * * *"));
    }

    #[test]
    fn invalid_cron_expressions() {
        assert!(!is_valid("not a cron"));
        assert!(!is_valid(""));
    }

    #[test]
    fn next_occurrence_returns_future_timestamp() {
        let ts = next_occurrence("* * * * * *").unwrap();
        assert!(ts > Utc::now().timestamp() - 2);
    }
}
