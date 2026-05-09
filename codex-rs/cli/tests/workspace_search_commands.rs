use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use tempfile::TempDir;

static ATA_BIN: OnceLock<PathBuf> = OnceLock::new();

fn ata_bin() -> &'static PathBuf {
    ATA_BIN.get_or_init(|| match codex_utils_cargo_bin::cargo_bin("codex") {
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
fn workspace_search_commands_prints_simplified_manual() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = ata_command(codex_home.path())?;
    let output = cmd
        .args(["workspace", "search-commands", "clone", "repo"])
        .output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Matches:"));
    assert!(stdout.contains("1. repo-clone — Validate, clone, register, and audit a repo."));
    assert!(stdout.contains("Best match manual:"));
    assert!(stdout.contains("Command: repo-clone"));
    assert!(stdout.contains("Validate, clone, register, and audit a repo."));
    assert_eq!(
        stdout
            .matches("Validate, clone, register, and audit a repo")
            .count(),
        2
    );
    assert!(stdout.contains("Usage: ata workspace repo-clone <URL> <ALIAS>"));
    assert_eq!(stdout.matches("Usage: ata workspace").count(), 1);
    assert!(stdout.contains("<URL>"));
    assert!(stdout.contains("<ALIAS>"));
    assert!(!stdout.contains("--workspace"));
    assert!(!stdout.contains("--full"));

    Ok(())
}
