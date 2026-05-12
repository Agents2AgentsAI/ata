#[cfg(debug_assertions)]
use anyhow::Result;
#[cfg(debug_assertions)]
use predicates::str::contains;
#[cfg(debug_assertions)]
use std::path::Path;
#[cfg(debug_assertions)]
use tempfile::TempDir;

#[cfg(debug_assertions)]
fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("ata")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn update_does_not_start_interactive_prompt() -> Result<()> {
    let codex_home = TempDir::new()?;

    codex_command(codex_home.path())?
        .arg("update")
        .assert()
        .failure()
        .stderr(contains("`codex update` is not available in debug builds"));

    Ok(())
}
