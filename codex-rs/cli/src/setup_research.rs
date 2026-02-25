use clap::Parser;
use codex_core::config::edit::ConfigEditsBuilder;
use codex_core::config::find_codex_home;
use codex_research_tools::setup::DependencyStatus;
use codex_research_tools::setup::SetupOptions;
use codex_research_tools::setup::run_setup;
use owo_colors::OwoColorize;

#[derive(Debug, Parser, Clone)]
pub(super) struct SetupResearchArgs {
    /// Only check dependencies without installing anything.
    #[arg(long)]
    check_only: bool,

    /// Don't enable the `research` feature in config.toml after setup.
    #[arg(long)]
    no_enable_feature: bool,
}

pub(super) async fn run(args: SetupResearchArgs) -> anyhow::Result<()> {
    let check_only = args.check_only;

    if check_only {
        println!("Checking research dependencies...");
    } else {
        println!("Setting up research dependencies...");
    }
    println!();

    let result = run_setup(SetupOptions { check_only }).await;

    for dep in &result.dependencies {
        let (icon, annotation) = match &dep.status {
            DependencyStatus::Found(path) => (
                "\u{2714}".green().to_string(),
                format!(
                    "{} {}",
                    "(already installed)".dimmed(),
                    path.display().dimmed()
                ),
            ),
            DependencyStatus::Installed(path) => (
                "\u{2714}".green().to_string(),
                format!(
                    "{} {}",
                    "(just installed)".dimmed(),
                    path.display().dimmed()
                ),
            ),
            DependencyStatus::Failed(msg) => ("\u{2718}".red().to_string(), msg.red().to_string()),
        };

        println!(
            "  {icon} {name} - {desc}  {annotation}",
            name = dep.name,
            desc = dep.description.dimmed(),
        );
    }

    println!();

    let all_ok = result.all_ok();
    // tlmgr is non-fatal — count real failures excluding it.
    let fatal_failures = result
        .dependencies
        .iter()
        .filter(|d| matches!(d.status, DependencyStatus::Failed(_)) && d.name != "tlmgr")
        .count();

    if fatal_failures == 0 {
        println!("{}", "All research dependencies are ready.".green());

        if !check_only && !args.no_enable_feature {
            if let Err(e) = enable_research_feature().await {
                eprintln!("Warning: could not enable research feature in config.toml: {e}");
            } else {
                println!("{}", "Enabled `research` feature in config.toml.".dimmed());
            }
        }
    } else {
        let msg = format!("{fatal_failures} dependency(ies) could not be resolved.");
        eprintln!("{}", msg.red());
    }

    if !all_ok && fatal_failures > 0 {
        std::process::exit(1);
    }

    Ok(())
}

async fn enable_research_feature() -> anyhow::Result<()> {
    let codex_home = find_codex_home()?;
    ConfigEditsBuilder::new(&codex_home)
        .set_feature_enabled("research", true)
        .apply()
        .await?;
    Ok(())
}
