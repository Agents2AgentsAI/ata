use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use tempfile::TempDir;

static ATA_BIN: OnceLock<PathBuf> = OnceLock::new();

fn ata_bin() -> &'static PathBuf {
    ATA_BIN.get_or_init(|| match codex_utils_cargo_bin::cargo_bin("ata") {
        Ok(path) => path,
        Err(error) => panic!("failed to locate codex binary: {error}"),
    })
}

fn ata_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(ata_bin());
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

#[test]
fn jobs_search_commands_prints_simplified_manual() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = ata_command(codex_home.path())?;
    let output = cmd
        .args(["jobs", "search-commands", "run", "job", "now"])
        .output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Matches:"));
    assert!(stdout.contains("1. run — Trigger an immediate run of a job."));
    assert!(stdout.contains("Best match manual:"));
    assert!(stdout.contains("Command: run"));
    assert!(stdout.contains("Trigger an immediate run of a job."));
    assert_eq!(
        stdout.matches("Trigger an immediate run of a job").count(),
        2
    );
    assert!(stdout.contains("Usage: ata jobs run <NAME>"));
    assert_eq!(stdout.matches("Usage: ata jobs").count(), 1);
    assert!(stdout.contains("<NAME>"));
    assert!(!stdout.contains("--config"));
    assert!(!stdout.contains("--limit"));

    Ok(())
}

#[test]
fn scheduler_search_commands_prints_simplified_manual() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = ata_command(codex_home.path())?;
    let output = cmd
        .args(["scheduler", "search-commands", "background", "daemon"])
        .output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Matches:"));
    assert!(stdout.contains("1. start — Start the scheduler daemon."));
    assert!(stdout.contains("Best match manual:"));
    assert!(stdout.contains("Command: start"));
    assert!(stdout.contains("Start the scheduler daemon."));
    assert_eq!(stdout.matches("Start the scheduler daemon").count(), 2);
    assert!(stdout.contains("Usage: ata scheduler start"));
    assert_eq!(stdout.matches("Usage: ata scheduler").count(), 1);
    assert!(stdout.contains("--daemon"));
    assert!(!stdout.contains("--config"));

    Ok(())
}
