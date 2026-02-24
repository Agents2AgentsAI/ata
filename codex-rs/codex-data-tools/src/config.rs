use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use crate::rate_limiter::ApiRateLimit;
use crate::rate_limiter::DataApi;

/// Load environment variables from a .env file.
///
/// Searches for .env files in the following order:
/// 1. The path specified (if provided)
/// 2. Current directory
/// 3. Parent directories (up to root)
///
/// This function should be called before `DataConfig::from_env()`.
///
/// # Example
/// ```ignore
/// use codex_data_tools::config::{load_dotenv, DataConfig};
///
/// // Load from default locations
/// load_dotenv(None);
///
/// // Or from a specific path
/// load_dotenv(Some("/path/to/.env"));
///
/// let config = DataConfig::from_env();
/// ```
pub fn load_dotenv(path: Option<&str>) {
    if let Some(p) = path {
        if let Err(e) = dotenvy::from_path(Path::new(p)) {
            tracing::debug!("Could not load .env from {}: {}", p, e);
        }
    } else {
        // Try to load from current directory or parent directories
        if let Err(e) = dotenvy::dotenv() {
            tracing::debug!("No .env file found: {}", e);
        }
    }
}

#[derive(Clone)]
pub struct DataConfig {
    // HuggingFace
    pub huggingface_token: Option<String>,
    pub huggingface_base_url: String,

    // Kaggle
    pub kaggle_username: Option<String>,
    pub kaggle_key: Option<String>,
    pub kaggle_api_token: Option<String>,
    pub kaggle_base_url: String,

    // Isaac Sim (for future use)
    pub isaac_sim_url: Option<String>,
    pub nucleus_server: Option<String>,

    // Cosmos (for future use)
    pub cosmos_api_key: Option<String>,
    pub cosmos_base_url: Option<String>,

    // Storage
    pub data_cache_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub max_cache_size_gb: u32,

    // Timeouts
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub tool_timeout: Duration,
    pub download_timeout: Duration,

    // Download settings
    pub max_concurrent_downloads: u32,
    pub chunk_size: usize,

    // Cache settings
    pub cache_max_entries: usize,
    pub cache_ttls: CacheTtls,

    // Retry settings
    pub retry: RetryConfig,

    // Rate limit overrides
    pub rate_limit_overrides: RateLimitOverrides,
}

#[derive(Debug, Clone, Copy)]
pub struct CacheTtls {
    pub dataset_search: Duration,
    pub dataset_info: Duration,
    pub dataset_files: Duration,
    pub negative: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct RateLimitOverrides {
    pub huggingface: Option<ApiRateLimit>,
    pub kaggle: Option<ApiRateLimit>,
    pub isaac_sim: Option<ApiRateLimit>,
    pub cosmos: Option<ApiRateLimit>,
}

impl Default for CacheTtls {
    fn default() -> Self {
        Self {
            dataset_search: Duration::from_secs(5 * 60),
            dataset_info: Duration::from_secs(10 * 60),
            dataset_files: Duration::from_secs(30 * 60),
            negative: Duration::from_secs(30),
        }
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl Default for DataConfig {
    fn default() -> Self {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("codex-data");

        let temp_dir = std::env::temp_dir().join("codex-data-tmp");

        Self {
            huggingface_token: None,
            huggingface_base_url: "https://huggingface.co".to_string(),

            kaggle_username: None,
            kaggle_key: None,
            kaggle_api_token: None,
            kaggle_base_url: "https://www.kaggle.com/api/v1".to_string(),

            isaac_sim_url: None,
            nucleus_server: None,

            cosmos_api_key: None,
            cosmos_base_url: None,

            data_cache_dir: cache_dir,
            temp_dir,
            max_cache_size_gb: 50,

            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(120),
            download_timeout: Duration::from_secs(3600), // 1 hour for large datasets

            max_concurrent_downloads: 4,
            chunk_size: 8 * 1024 * 1024, // 8MB chunks

            cache_max_entries: 10_000,
            cache_ttls: CacheTtls::default(),
            retry: RetryConfig::default(),
            rate_limit_overrides: RateLimitOverrides::default(),
        }
    }
}

impl DataConfig {
    /// Load configuration from environment variables.
    /// Call `load_dotenv()` first if you want to load from a .env file.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // HuggingFace
        if let Ok(token) = std::env::var("HF_TOKEN") {
            config.huggingface_token = Some(token);
        } else if let Ok(token) = std::env::var("HUGGING_FACE_HUB_TOKEN") {
            config.huggingface_token = Some(token);
        }
        if let Ok(url) = std::env::var("HF_ENDPOINT") {
            config.huggingface_base_url = url;
        }

        // Kaggle
        config.kaggle_username = std::env::var("KAGGLE_USERNAME").ok();
        config.kaggle_key = std::env::var("KAGGLE_KEY").ok();
        config.kaggle_api_token = std::env::var("KAGGLE_API_TOKEN").ok();
        if let Ok(url) = std::env::var("KAGGLE_API_URL") {
            config.kaggle_base_url = url;
        }

        // Isaac Sim
        config.isaac_sim_url = std::env::var("ISAAC_SIM_URL").ok();
        config.nucleus_server = std::env::var("OMNIVERSE_NUCLEUS").ok();

        // Cosmos
        config.cosmos_api_key = std::env::var("NVIDIA_API_KEY").ok();
        config.cosmos_base_url = std::env::var("COSMOS_BASE_URL").ok();

        // Storage
        if let Ok(dir) = std::env::var("DATA_CACHE_DIR") {
            config.data_cache_dir = PathBuf::from(dir);
        }
        if let Ok(size) = std::env::var("DATA_CACHE_MAX_GB")
            && let Ok(parsed) = size.parse::<u32>()
        {
            config.max_cache_size_gb = parsed;
        }

        // Cache entries
        if let Ok(raw) = std::env::var("DATA_CACHE_MAX_ENTRIES")
            && let Ok(parsed) = raw.parse::<usize>()
        {
            config.cache_max_entries = parsed.max(1);
        }

        config
    }

    #[must_use]
    pub fn rate_limits(&self) -> HashMap<DataApi, ApiRateLimit> {
        let mut limits = HashMap::from([
            (
                DataApi::HuggingFace,
                if self.huggingface_token.is_some() {
                    // Authenticated: higher limits
                    ApiRateLimit::new(100, Duration::from_secs(60), 10)
                } else {
                    // Anonymous: conservative limits
                    ApiRateLimit::new(30, Duration::from_secs(60), 3)
                },
            ),
            (
                DataApi::Kaggle,
                // Kaggle requires auth, so always use auth limits
                ApiRateLimit::new(60, Duration::from_secs(60), 5),
            ),
            (
                DataApi::IsaacSim,
                // Local instance, can be more aggressive
                ApiRateLimit::new(100, Duration::from_secs(1), 10),
            ),
            (
                DataApi::Cosmos,
                // API limits TBD
                ApiRateLimit::new(10, Duration::from_secs(60), 3),
            ),
        ]);

        // Apply overrides
        if let Some(limit) = self.rate_limit_overrides.huggingface {
            limits.insert(DataApi::HuggingFace, limit);
        }
        if let Some(limit) = self.rate_limit_overrides.kaggle {
            limits.insert(DataApi::Kaggle, limit);
        }
        if let Some(limit) = self.rate_limit_overrides.isaac_sim {
            limits.insert(DataApi::IsaacSim, limit);
        }
        if let Some(limit) = self.rate_limit_overrides.cosmos {
            limits.insert(DataApi::Cosmos, limit);
        }

        limits
    }

    #[must_use]
    pub fn is_huggingface_configured(&self) -> bool {
        // HuggingFace works without auth for public datasets
        true
    }

    #[must_use]
    pub fn is_kaggle_configured(&self) -> bool {
        self.kaggle_api_token.is_some()
            || self.kaggle_username.is_some() && self.kaggle_key.is_some()
    }

    #[must_use]
    pub fn is_isaac_sim_configured(&self) -> bool {
        self.isaac_sim_url.is_some()
    }

    #[must_use]
    pub fn is_cosmos_configured(&self) -> bool {
        self.cosmos_api_key.is_some()
    }
}

impl fmt::Debug for DataConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataConfig")
            .field("huggingface_token", &redact(&self.huggingface_token))
            .field("huggingface_base_url", &self.huggingface_base_url)
            .field("kaggle_username", &self.kaggle_username)
            .field("kaggle_key", &redact(&self.kaggle_key))
            .field("kaggle_api_token", &redact(&self.kaggle_api_token))
            .field("kaggle_base_url", &self.kaggle_base_url)
            .field("isaac_sim_url", &self.isaac_sim_url)
            .field("nucleus_server", &self.nucleus_server)
            .field("cosmos_api_key", &redact(&self.cosmos_api_key))
            .field("cosmos_base_url", &self.cosmos_base_url)
            .field("data_cache_dir", &self.data_cache_dir)
            .field("temp_dir", &self.temp_dir)
            .field("max_cache_size_gb", &self.max_cache_size_gb)
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("tool_timeout", &self.tool_timeout)
            .field("cache_max_entries", &self.cache_max_entries)
            .field("cache_ttls", &self.cache_ttls)
            .field("retry", &self.retry)
            .finish()
    }
}

fn redact(value: &Option<String>) -> &'static str {
    if value.is_some() {
        "<redacted>"
    } else {
        "<unset>"
    }
}
