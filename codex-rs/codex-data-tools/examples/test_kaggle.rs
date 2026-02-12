use codex_data_tools::DataToolkit;
use codex_data_tools::config::DataConfig;
use codex_data_tools::load_dotenv;
use codex_data_tools::types::DatasetDownloadParams;
use codex_data_tools::types::DatasetFilesParams;
use codex_data_tools::types::DatasetSearchParams;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("codex_data_tools=info")
        .init();

    load_dotenv(Some(".env"));
    let config = DataConfig::from_env();

    println!("Kaggle configured: {}", config.is_kaggle_configured());

    let client = reqwest::Client::new();
    let toolkit = DataToolkit::new(client, config);

    println!("\n=== Searching Kaggle for 'titanic' ===");
    let result = toolkit
        .dataset_search(DatasetSearchParams {
            query: "titanic".to_string(),
            source: Some("kaggle".to_string()),
            limit: Some(3),
            ..Default::default()
        })
        .await?;

    println!("Found {} datasets", result.datasets.len());
    for ds in &result.datasets {
        println!("  - {} ({})", ds.id, ds.name);
    }

    // Pick the first dataset and list its files
    if let Some(first) = result.datasets.first() {
        let dataset_id = format!("kaggle:{}", first.id);
        println!("\n=== Listing files in {} ===", dataset_id);

        let files = toolkit
            .dataset_list_files(DatasetFilesParams {
                dataset_id: dataset_id.clone(),
                path: None,
                revision: None,
            })
            .await?;

        println!("Total files: {}", files.files.len());
        for file in &files.files {
            println!("  - {} ({:?} bytes)", file.path, file.size_bytes);
        }

        // Download to /tmp
        println!(
            "\n=== Downloading {} to /tmp/kaggle_titanic ===",
            dataset_id
        );
        let download = toolkit
            .dataset_download(DatasetDownloadParams {
                dataset_id: dataset_id.clone(),
                output_path: "/tmp/kaggle_titanic".to_string(),
                files: None,
                revision: None,
            })
            .await?;

        println!(
            "Downloaded {} files ({} bytes) in {:.2}s",
            download.files_downloaded, download.total_bytes, download.duration_secs
        );
    }

    Ok(())
}
