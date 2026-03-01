use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;

use super::JobDefinition;
use super::validate_job;

/// Return the directory where job TOML files live (`~/.ata/jobs/`).
pub fn jobs_dir() -> anyhow::Result<PathBuf> {
    let home = codex_utils_home_dir::find_codex_home().map_err(|e| anyhow::anyhow!(e))?;
    Ok(home.join("jobs"))
}

/// Load a single job from a TOML file. The file stem is used as the job ID.
pub fn load_single_job(path: &Path) -> anyhow::Result<(String, JobDefinition)> {
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(String::from)
        .context("invalid job filename")?;
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let def: JobDefinition =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_job(&id, &def)?;
    Ok((id, def))
}

/// Scan `~/.ata/jobs/` and load all `.toml` files.
pub fn load_jobs() -> anyhow::Result<Vec<(String, JobDefinition)>> {
    let dir = jobs_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut jobs = Vec::new();
    let entries =
        std::fs::read_dir(&dir).with_context(|| format!("cannot read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            match load_single_job(&path) {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    tracing::warn!("skipping {}: {e:#}", path.display());
                }
            }
        }
    }
    Ok(jobs)
}
