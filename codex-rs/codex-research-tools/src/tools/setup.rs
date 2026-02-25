use std::path::PathBuf;

/// Status of a single dependency after a probe or install attempt.
pub enum DependencyStatus {
    /// Already present on the system.
    Found(PathBuf),
    /// Just installed by the setup process.
    Installed(PathBuf),
    /// Could not be found or installed.
    Failed(String),
}

/// Result for a single dependency check/install.
pub struct DependencyResult {
    /// Short identifier (e.g. "pdflatex", "java").
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Outcome of the probe/install.
    pub status: DependencyStatus,
}

/// Options controlling setup behavior.
pub struct SetupOptions {
    /// When true, only probe for dependencies without installing anything.
    pub check_only: bool,
}

/// Aggregate result of the setup process.
pub struct SetupResult {
    /// Per-dependency outcomes, in check order.
    pub dependencies: Vec<DependencyResult>,
}

impl SetupResult {
    /// Returns `true` if no dependency failed.
    pub fn all_ok(&self) -> bool {
        self.dependencies
            .iter()
            .all(|d| !matches!(d.status, DependencyStatus::Failed(_)))
    }
}

/// Run the full research dependency setup.
///
/// Checks (and optionally installs) all dependencies required by the
/// research tools: pdflatex, tlmgr.
pub async fn run_setup(options: SetupOptions) -> SetupResult {
    let mut dependencies = Vec::new();

    // 1. pdflatex
    dependencies.push(check_pdflatex(&options).await);

    // 2. tlmgr (probe only, non-fatal)
    dependencies.push(check_tlmgr().await);

    SetupResult { dependencies }
}

async fn check_pdflatex(options: &SetupOptions) -> DependencyResult {
    if let Some(path) = super::latex::probe_engine().await {
        return DependencyResult {
            name: "pdflatex",
            description: "LaTeX compiler",
            status: DependencyStatus::Found(path),
        };
    }

    if options.check_only {
        return DependencyResult {
            name: "pdflatex",
            description: "LaTeX compiler",
            status: DependencyStatus::Failed("not found".into()),
        };
    }

    match super::latex::find_engine().await {
        Ok(path) => DependencyResult {
            name: "pdflatex",
            description: "LaTeX compiler",
            status: DependencyStatus::Installed(path),
        },
        Err(e) => DependencyResult {
            name: "pdflatex",
            description: "LaTeX compiler",
            status: DependencyStatus::Failed(e.to_string()),
        },
    }
}

async fn check_tlmgr() -> DependencyResult {
    match super::latex::find_tlmgr().await {
        Some(path) => DependencyResult {
            name: "tlmgr",
            description: "LaTeX package manager",
            status: DependencyStatus::Found(path),
        },
        None => DependencyResult {
            name: "tlmgr",
            description: "LaTeX package manager",
            status: DependencyStatus::Failed("not found (non-fatal)".into()),
        },
    }
}
