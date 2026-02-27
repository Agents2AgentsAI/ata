use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use codex_core::config::CONFIG_TOML_FILE;
use tokio::select;
use tokio::time::timeout;
use toml::Value as TomlValue;

#[tokio::test]
async fn slash_logout_resets_provider_selection_in_config() -> anyhow::Result<()> {
    if cfg!(windows) {
        return Ok(());
    }

    let codex_home = tempfile::tempdir()?;
    let cwd = std::env::current_dir()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"
model = "claude-sonnet-4-20250514"
model_provider = "anthropic"

[projects]
"{cwd}" = {{ trust_level = "trusted" }}
"#,
            cwd = cwd.display()
        ),
    )?;

    let before_raw = std::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE))?;
    let before: TomlValue = toml::from_str(&before_raw)?;
    let before_table = before.as_table().expect("config table");
    assert_eq!(
        before_table.get("model_provider"),
        Some(&"anthropic".into())
    );

    let ata_bin = codex_utils_cargo_bin::cargo_bin("ata")?;
    let mut env = HashMap::new();
    env.insert(
        "CODEX_HOME".to_string(),
        codex_home.path().display().to_string(),
    );
    env.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-env".to_string());

    let args = vec!["-c".to_string(), "analytics.enabled=false".to_string()];
    let spawned =
        codex_utils_pty::spawn_pty_process(&ata_bin.to_string_lossy(), &args, &cwd, &env, &None)
            .await?;
    let mut output_rx = spawned.output_rx;
    let mut exit_rx = spawned.exit_rx;
    let writer_tx = spawned.session.writer_sender();
    let captured_output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured_output_for_loop = Arc::clone(&captured_output);

    let exit_code_result = timeout(Duration::from_secs(20), async {
        let ready_marker = b"for shortcuts";
        let mut sent_logout = false;
        let mut logout_retries = 0_u8;
        let mut chunk_count = 0_u32;
        loop {
            select! {
                result = output_rx.recv() => match result {
                    Ok(chunk) => {
                        chunk_count += 1;
                        let should_send_logout = {
                            let mut output = captured_output_for_loop
                                .lock()
                                .expect("captured output lock");
                            output.extend_from_slice(&chunk);
                            !sent_logout
                                && (output
                                    .windows(ready_marker.len())
                                    .any(|window| window == ready_marker)
                                    || chunk_count > 20)
                        };
                        if should_send_logout {
                            sent_logout = true;
                        }
                        if chunk.windows(4).any(|window| window == b"\x1b[6n") {
                            let _ = writer_tx.send(b"\x1b[1;1R".to_vec()).await;
                        }
                        if sent_logout && logout_retries < 5 {
                            let _ = writer_tx.send(b"/logout\r".to_vec()).await;
                            logout_retries += 1;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break exit_rx.await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                },
                result = &mut exit_rx => break result,
            }
        }
    })
    .await;

    let exit_code = match exit_code_result {
        Ok(Ok(code)) => code,
        Ok(Err(err)) => return Err(err.into()),
        Err(_) => {
            spawned.session.terminate();
            let output = captured_output.lock().expect("captured output lock");
            let output_text = String::from_utf8_lossy(&output);
            anyhow::bail!(
                "timed out waiting for ata to exit after /logout; captured output: {output_text}"
            );
        }
    };
    assert_eq!(exit_code, 0, "expected ata to exit cleanly after /logout");

    let after = match std::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)) {
        Ok(raw) if !raw.trim().is_empty() => toml::from_str::<TomlValue>(&raw)?,
        Ok(_) => TomlValue::Table(Default::default()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            TomlValue::Table(Default::default())
        }
        Err(err) => return Err(err.into()),
    };
    let after_table = after.as_table().expect("config table");
    assert!(
        !after_table.contains_key("model_provider"),
        "model_provider should be cleared by /logout"
    );
    assert!(
        !after_table.contains_key("model"),
        "model should be cleared by /logout"
    );
    assert!(
        !after_table.contains_key("model_reasoning_effort"),
        "model_reasoning_effort should be cleared by /logout"
    );

    Ok(())
}
