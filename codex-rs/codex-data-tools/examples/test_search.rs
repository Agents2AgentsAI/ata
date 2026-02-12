//! Quick test for HuggingFace and Kaggle integration

use codex_data_tools::DataToolkit;
use codex_data_tools::config::DataConfig;
use codex_data_tools::load_dotenv;
use codex_data_tools::types::DatasetDownloadParams;
use codex_data_tools::types::DatasetFilesParams;
use codex_data_tools::types::DatasetGetParams;
use codex_data_tools::types::DatasetSearchParams;
use codex_data_tools::types::KaggleCompetitionsParams;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("codex_data_tools=info".parse()?),
        )
        .init();

    // Load .env from repo root
    load_dotenv(Some(".env"));

    let config = DataConfig::from_env();

    println!("=== Configuration ===");
    println!(
        "HuggingFace configured: {}",
        config.is_huggingface_configured()
    );
    println!("Kaggle configured: {}", config.is_kaggle_configured());
    println!();

    let client = reqwest::Client::new();
    let toolkit = DataToolkit::new(client, config);

    // Test search
    println!("=== Searching for 'robotics' datasets ===");
    let result = toolkit
        .dataset_search(DatasetSearchParams {
            query: "robotics".to_string(),
            limit: Some(5),
            ..Default::default()
        })
        .await?;

    println!("\nPer-source status:");
    for status in &result.per_source_status {
        println!("  {}: {:?}", status.source, status.status);
    }

    println!("\nFound {} datasets:", result.datasets.len());
    for dataset in &result.datasets {
        println!("  - {} ({:?})", dataset.id, dataset.source);
        if let Some(desc) = &dataset.description {
            let short_desc: String = desc.chars().take(80).collect();
            println!(
                "    {}{}",
                short_desc,
                if desc.len() > 80 { "..." } else { "" }
            );
        }
        println!("    Downloads: {:?}", dataset.downloads);
    }

    // Test get dataset detail (HuggingFace)
    println!("\n=== Getting HuggingFace dataset detail ===");
    let hf_detail = toolkit
        .dataset_get(DatasetGetParams {
            dataset_id: "squad".to_string(),
            include_files: Some(true),
            include_readme: Some(true),
        })
        .await?;
    println!("Dataset: {}", hf_detail.dataset.id);
    println!("License: {:?}", hf_detail.dataset.license);
    println!("Downloads: {:?}", hf_detail.dataset.downloads);
    println!("Files: {}", hf_detail.files.len());

    // Test list files
    println!("\n=== Listing files in 'squad' ===");
    let files = toolkit
        .dataset_list_files(DatasetFilesParams {
            dataset_id: "squad".to_string(),
            path: None,
            revision: None,
        })
        .await?;
    println!("Total files: {}", files.files.len());
    for file in files.files.iter().take(5) {
        println!("  - {} ({:?} bytes)", file.path, file.size_bytes);
    }

    if !result.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in &result.warnings {
            println!("  ! {}", warning);
        }
    }

    // Test download (small file only)
    println!("\n=== Testing download (README only) ===");
    let temp_dir = std::env::temp_dir().join("codex_data_test");
    let download_result = toolkit
        .dataset_download(DatasetDownloadParams {
            dataset_id: "squad".to_string(),
            output_path: temp_dir.to_string_lossy().to_string(),
            files: Some(vec!["README.md".to_string()]), // Only download README
            revision: None,
        })
        .await;
    match download_result {
        Ok(result) => {
            println!(
                "Downloaded {} files ({} bytes) in {:.2}s",
                result.files_downloaded, result.total_bytes, result.duration_secs
            );
            println!("Output path: {}", result.output_path);
        }
        Err(e) => {
            println!(
                "Download failed (this may be expected for some datasets): {}",
                e
            );
        }
    }

    // Test Kaggle competitions (if configured)
    if toolkit.config().is_kaggle_configured() {
        println!("\n=== Listing Kaggle competitions ===");
        match toolkit
            .kaggle_competitions(KaggleCompetitionsParams {
                limit: Some(5),
                ..Default::default()
            })
            .await
        {
            Ok(competitions) => {
                println!("Found {} competitions:", competitions.competitions.len());
                for comp in competitions.competitions.iter().take(5) {
                    println!("  - {} ({})", comp.title, comp.id);
                    if let Some(reward) = &comp.reward {
                        println!("    Reward: {}", reward);
                    }
                }
            }
            Err(e) => {
                println!("Failed to list competitions: {}", e);
                println!("(This may require additional Kaggle permissions)");
            }
        }
    }

    println!("\nAll tests passed!");
    Ok(())
}
