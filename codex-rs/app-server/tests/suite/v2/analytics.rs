use anyhow::Result;
use codex_core::config::ConfigBuilder;
use codex_core::config::types::OtelExporterKind;
use codex_core::config::types::OtelHttpProtocol;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use tempfile::TempDir;

const SERVICE_VERSION: &str = "0.0.0-test";

fn set_metrics_exporter(config: &mut codex_core::config::Config) {
    config.otel.metrics_exporter = OtelExporterKind::OtlpHttp {
        endpoint: "http://localhost:4318".to_string(),
        headers: HashMap::new(),
        protocol: OtelHttpProtocol::Json,
        tls: None,
    };
}

#[tokio::test]
async fn app_server_default_analytics_disabled_when_unset() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await?;
    set_metrics_exporter(&mut config);
    config.analytics_enabled = None;

    let provider =
        codex_core::otel_init::build_provider(&config, SERVICE_VERSION, Some("codex-app-server"))
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    // With analytics unset in the config, metrics are disabled.
    // A provider may still exist for non-metrics telemetry, so check metrics specifically.
    let has_metrics = provider.as_ref().and_then(|otel| otel.metrics()).is_some();
    assert_eq!(has_metrics, false);
    Ok(())
}

#[tokio::test]
async fn app_server_opted_in_analytics_enables_metrics() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await?;
    set_metrics_exporter(&mut config);
    config.analytics_enabled = Some(true);

    let provider =
        codex_core::otel_init::build_provider(&config, SERVICE_VERSION, Some("codex-app-server"))
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let has_metrics = provider.as_ref().and_then(|otel| otel.metrics()).is_some();
    assert_eq!(has_metrics, true);
    Ok(())
}
