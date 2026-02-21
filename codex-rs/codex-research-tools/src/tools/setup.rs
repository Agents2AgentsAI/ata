use std::path::PathBuf;

/// Status of a single dependency after a probe or install attempt.
pub enum DependencyStatus {
    /// Already present on the system.
    Found(PathBuf),
    /// Just installed by the setup process.
    Installed(PathBuf),
    /// Just downloaded (e.g. the pdffigures2 JAR).
    Downloaded(PathBuf),
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
/// research tools: pdflatex, tlmgr, java, poppler-utils, pdffigures2.jar.
pub async fn run_setup(options: SetupOptions) -> SetupResult {
    let mut dependencies = Vec::new();

    // 1. pdflatex
    dependencies.push(check_pdflatex(&options).await);

    // 2. tlmgr (probe only, non-fatal)
    dependencies.push(check_tlmgr().await);

    // 3. java
    dependencies.push(check_java(&options).await);

    // 4. poppler-utils (pdftocairo)
    dependencies.push(check_pdftocairo(&options).await);

    // 5. pdffigures2.jar
    dependencies.push(check_pdffigures2_jar(&options).await);

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

async fn check_java(options: &SetupOptions) -> DependencyResult {
    if let Some(path) = super::pdf_images::probe_java().await {
        return DependencyResult {
            name: "java",
            description: "JRE for pdffigures2",
            status: DependencyStatus::Found(path),
        };
    }

    if options.check_only {
        return DependencyResult {
            name: "java",
            description: "JRE for pdffigures2",
            status: DependencyStatus::Failed("not found".into()),
        };
    }

    match super::pdf_images::find_java().await {
        Ok(path) => DependencyResult {
            name: "java",
            description: "JRE for pdffigures2",
            status: DependencyStatus::Installed(path),
        },
        Err(e) => DependencyResult {
            name: "java",
            description: "JRE for pdffigures2",
            status: DependencyStatus::Failed(e),
        },
    }
}

async fn check_pdftocairo(options: &SetupOptions) -> DependencyResult {
    if super::pdf_images::probe_pdftocairo().await {
        return DependencyResult {
            name: "poppler-utils",
            description: "PDF rendering (pdftocairo/pdfimages)",
            status: DependencyStatus::Found("pdftocairo".into()),
        };
    }

    if options.check_only {
        return DependencyResult {
            name: "poppler-utils",
            description: "PDF rendering (pdftocairo/pdfimages)",
            status: DependencyStatus::Failed("not found".into()),
        };
    }

    match super::pdf_images::ensure_pdftocairo().await {
        Ok(()) => DependencyResult {
            name: "poppler-utils",
            description: "PDF rendering (pdftocairo/pdfimages)",
            status: DependencyStatus::Installed("pdftocairo".into()),
        },
        Err(e) => DependencyResult {
            name: "poppler-utils",
            description: "PDF rendering (pdftocairo/pdfimages)",
            status: DependencyStatus::Failed(e),
        },
    }
}

async fn check_pdffigures2_jar(options: &SetupOptions) -> DependencyResult {
    if let Some(path) = super::pdf_images::probe_pdffigures2_jar() {
        return DependencyResult {
            name: "pdffigures2.jar",
            description: "Figure extraction JAR",
            status: DependencyStatus::Found(path),
        };
    }

    if options.check_only {
        return DependencyResult {
            name: "pdffigures2.jar",
            description: "Figure extraction JAR",
            status: DependencyStatus::Failed("not found".into()),
        };
    }

    match super::pdf_images::ensure_pdffigures2_jar().await {
        Ok(path) => DependencyResult {
            name: "pdffigures2.jar",
            description: "Figure extraction JAR",
            status: DependencyStatus::Downloaded(path),
        },
        Err(e) => DependencyResult {
            name: "pdffigures2.jar",
            description: "Figure extraction JAR",
            status: DependencyStatus::Failed(e),
        },
    }
}
