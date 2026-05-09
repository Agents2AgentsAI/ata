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

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let bin = ata_bin();
    let mut cmd = assert_cmd::Command::new(bin);
    cmd.env("CODEX_HOME", codex_home);
    for variable in [
        "ZOTERO_API_KEY",
        "ZOTERO_BASE_URL",
        "ZOTERO_LIBRARY_TYPE",
        "ZOTERO_GROUP_ID",
        "ZOTERO_USER_ID",
    ] {
        cmd.env_remove(variable);
    }
    Ok(cmd)
}

#[test]
fn zotero_search_commands_returns_ranked_matches() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd
        .args(["zotero", "search-commands", "cite", "paper"])
        .output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Matches:"));
    assert!(stdout.contains("1. item citation — Generate a citation for an item."));
    assert!(stdout.contains("Best match manual:"));
    assert!(stdout.contains("Command: item citation"));
    assert!(stdout.contains("Generate a citation for an item"));
    assert_eq!(stdout.matches("Generate a citation for an item").count(), 2);
    assert!(stdout.contains("Usage: ata zotero item citation --item-key <ITEM_KEY>"));
    assert_eq!(stdout.matches("Usage: ata zotero").count(), 1);
    assert!(stdout.contains("--item-key <ITEM_KEY>"));
    assert!(!stdout.contains("--config"));
    assert!(!stdout.contains("--library-type"));
    assert!(!stdout.contains("--format"));

    Ok(())
}

#[test]
fn zotero_search_commands_routes_repo_queries_to_find_repos() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd
        .args([
            "zotero",
            "search-commands",
            "find",
            "github",
            "repo",
            "urls",
        ])
        .output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains(
        "1. find-repos — Find repository URLs in Zotero items, collections, or linked records."
    ));
    assert!(stdout.contains("Command: find-repos"));
    assert!(stdout.contains("Usage: ata zotero find-repos"));

    Ok(())
}

#[test]
fn zotero_status_reports_effective_mode() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd.args(["zotero", "status"]).output()?;
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Effective mode:"));
    assert!(stdout.contains("Base URL:"));
    assert!(stdout.contains("Library scope:"));

    Ok(())
}
