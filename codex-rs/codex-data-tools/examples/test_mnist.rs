//! Test mnist dataset download specifically

use codex_data_tools::DataToolkit;
use codex_data_tools::config::DataConfig;
use codex_data_tools::load_dotenv;
use codex_data_tools::types::DatasetDownloadParams;
use codex_data_tools::types::DatasetFilesParams;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("codex_data_tools=info".parse()?),
        )
        .init();

    load_dotenv(Some(".env"));
    let config = DataConfig::from_env();
    let client = reqwest::Client::new();
    let toolkit = DataToolkit::new(client, config);

    println!("\n=== Listing files in ylecun/mnist ===");
    let files = toolkit
        .dataset_list_files(DatasetFilesParams {
            dataset_id: "ylecun/mnist".to_string(),
            path: None,
            revision: None,
        })
        .await?;

    println!("Total files: {}", files.files.len());
    for file in &files.files {
        println!("  - {} ({:?} bytes)", file.path, file.size_bytes);
    }

    println!("\n=== Downloading ylecun/mnist to /tmp/mnist ===");
    let result = toolkit
        .dataset_download(DatasetDownloadParams {
            dataset_id: "ylecun/mnist".to_string(),
            output_path: "/tmp/mnist".to_string(),
            files: None, // Download all files
            revision: None,
        })
        .await?;

    println!(
        "\nDownloaded {} files ({} bytes) in {:.2}s",
        result.files_downloaded, result.total_bytes, result.duration_secs
    );

    // List downloaded files recursively
    println!("\n=== Downloaded files ===");
    fn list_dir(path: &std::path::Path, indent: usize) -> std::io::Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let prefix = " ".repeat(indent);
            if metadata.is_dir() {
                println!("{}📁 {:?}", prefix, entry.file_name());
                list_dir(&entry.path(), indent + 2)?;
            } else {
                println!(
                    "{}📄 {:?} ({} bytes)",
                    prefix,
                    entry.file_name(),
                    metadata.len()
                );
            }
        }
        Ok(())
    }
    list_dir(std::path::Path::new("/tmp/mnist"), 2)?;

    println!("\nAll done!");
    Ok(())
}
