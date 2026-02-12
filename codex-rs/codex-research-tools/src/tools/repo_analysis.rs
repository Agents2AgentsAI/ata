use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::SystemTime;

use dashmap::DashMap;
use regex::Regex;
use serde::Deserialize;
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::timeout;
use walkdir::WalkDir;

use crate::ResearchToolkit;
use crate::cache::CacheKey;
use crate::cache::FetchOutput;
use crate::clients::github;
use crate::clients::github::GitHubConfig;
use crate::error::ResearchError;
use crate::error::Result;
use crate::rate_limiter::ResearchApi;
use crate::text_utils::truncate_chars;
use crate::tools::cache_helpers::get_or_fetch_typed;
use crate::tools::cache_helpers::hash_cache_payload;
use crate::types::ConfigParam;
use crate::types::ConfigSchema;
use crate::types::ModelDefinition;
use crate::types::RepoEntrypoint;
use crate::types::RepoExportPath;
use crate::types::RepoHealth;
use crate::types::RepoIoShape;
use crate::types::RepoRequirements;
use crate::types::RepoSummary;
use crate::types::RequirementsDiff;

const SOURCE_CACHE_DIR: &str = "codex-research-repos";
const CLONE_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const REPO_MAX_SIZE_BYTES: u64 = 500 * 1024 * 1024;
const REPO_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SCAN_FILE_SIZE_BYTES: u64 = 1_000_000;
const MAX_SCAN_FILE_COUNT: usize = 10_000;
const MAX_SCAN_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 200;
const MAX_TREE_DEPTH: usize = 3;
const MAX_MODELS_RETURNED: usize = 300;
const MAX_ENTRYPOINTS_RETURNED: usize = 150;
const MAX_IO_SHAPES_RETURNED: usize = 250;
const MAX_EXPORT_PATHS_RETURNED: usize = 300;
const MAX_CONFIG_SCHEMAS_RETURNED: usize = 200;
const MAX_KEY_PARAMS_PER_SCHEMA: usize = 200;
const MAX_REQUIREMENTS_RETURNED: usize = 2_000;
const GITHUB_HOST: &str = "github.com";
const GITLAB_HOST: &str = "gitlab.com";
const ACCESS_MARKER_FILE: &str = ".codex_last_access";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RepoRef {
    host: String,
    owner: String,
    repo: String,
    branch: Option<String>,
}

impl RepoRef {
    fn normalized_id(&self) -> String {
        format!(
            "{}/{}/{}",
            self.host.to_ascii_lowercase(),
            self.owner.to_ascii_lowercase(),
            self.repo.to_ascii_lowercase(),
        )
    }

    fn cache_key(&self) -> String {
        let branch = self
            .branch
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        format!("{}@{}", self.normalized_id(), branch.to_ascii_lowercase())
    }

    fn clone_url(&self) -> String {
        format!("https://{}/{}/{}.git", self.host, self.owner, self.repo)
    }

    fn fs_dir_name(&self) -> String {
        let branch = self
            .branch
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("default");
        let stem = format!("{}_{}_{}", self.host, self.owner, self.repo)
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let mut hasher = DefaultHasher::new();
        self.cache_key().hash(&mut hasher);
        format!(
            "{}-{}-{:016x}",
            stem,
            branch
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect::<String>(),
            hasher.finish()
        )
    }
}

#[derive(Debug, Clone)]
struct RepoCheckout {
    path: PathBuf,
    commit_sha: Option<String>,
}

#[derive(Debug, Clone)]
struct TextFile {
    relative_path: String,
    content: String,
}

#[derive(Debug, Clone)]
struct RepoCacheEntry {
    path: PathBuf,
    last_accessed: SystemTime,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
struct RepoHealthCacheKeyPayload {
    repo_id: String,
    auth_tier: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RepoHealthCacheEntry {
    Hit { health: RepoHealth },
    Miss { status: u16, message: String },
}

#[derive(Debug, Clone, Serialize)]
struct ModelSearchConfig<'a> {
    framework: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
struct EntrypointSearchConfig<'a> {
    task_hint: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
struct IoShapeSearchConfig<'a> {
    model_class: Option<&'a str>,
}

fn repo_locks() -> &'static DashMap<String, Arc<Mutex<()>>> {
    static LOCKS: OnceLock<DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();
    LOCKS.get_or_init(DashMap::new)
}

fn repo_cache_dir() -> PathBuf {
    std::env::temp_dir().join(SOURCE_CACHE_DIR)
}

pub(crate) async fn repo_clone_and_summarize(
    toolkit: &ResearchToolkit,
    repo_url: &str,
    branch: Option<&str>,
) -> Result<RepoSummary> {
    let repo_ref = parse_repo_ref(repo_url, branch)?;
    let cache_key = repo_tool_cache_key(
        "repo_clone_and_summarize",
        &repo_ref.normalized_id(),
        &repo_ref.branch,
    )?;
    let requested_repo_url = repo_url.to_string();
    let summary_repo_url = requested_repo_url.clone();

    let mut summary = get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || {
                build_repo_summary(summary_repo_url, checkout.path, checkout.commit_sha)
            })
            .await
        },
    )
    .await?;

    summary.url = requested_repo_url;
    Ok(summary)
}

pub(crate) async fn repo_find_models(
    toolkit: &ResearchToolkit,
    repo_url: &str,
    framework: Option<&str>,
) -> Result<Vec<ModelDefinition>> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let framework = normalize_framework(framework)?;
    let cache_key = repo_tool_cache_key(
        "repo_find_models",
        &repo_ref.normalized_id(),
        &ModelSearchConfig {
            framework: framework.as_deref(),
        },
    )?;

    get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || {
                find_model_definitions(checkout.path.as_path(), framework.as_deref())
            })
            .await
        },
    )
    .await
}

pub(crate) async fn repo_extract_requirements(
    toolkit: &ResearchToolkit,
    repo_url: &str,
) -> Result<RepoRequirements> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let cache_key =
        repo_tool_cache_key("repo_extract_requirements", &repo_ref.normalized_id(), &())?;
    let requested_repo_url = repo_url.to_string();
    let requirements_repo_url = requested_repo_url.clone();

    let mut requirements = get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || {
                let dependencies = extract_requirements_map(checkout.path.as_path())?;
                let mut rendered_dependencies = dependencies
                    .iter()
                    .map(|(name, constraint)| format_dependency(name, constraint))
                    .collect::<Vec<_>>();
                rendered_dependencies.sort();
                rendered_dependencies.truncate(MAX_REQUIREMENTS_RETURNED);

                let mut source_files = collect_requirement_source_files(checkout.path.as_path())?;
                source_files.sort();

                Ok(RepoRequirements {
                    repo_url: requirements_repo_url,
                    dependencies: rendered_dependencies,
                    source_files,
                })
            })
            .await
        },
    )
    .await?;

    requirements.repo_url = requested_repo_url;
    Ok(requirements)
}

pub(crate) async fn repo_find_entrypoints(
    toolkit: &ResearchToolkit,
    repo_url: &str,
    task_hint: Option<&str>,
) -> Result<Vec<RepoEntrypoint>> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let hint = normalize_task_hint(task_hint)?;
    let cache_key = repo_tool_cache_key(
        "repo_find_entrypoints",
        &repo_ref.normalized_id(),
        &EntrypointSearchConfig {
            task_hint: hint.as_deref(),
        },
    )?;

    get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || find_entrypoints(checkout.path.as_path(), hint.as_deref())).await
        },
    )
    .await
}

pub(crate) async fn repo_extract_io_shapes(
    toolkit: &ResearchToolkit,
    repo_url: &str,
    model_class: Option<&str>,
) -> Result<Vec<RepoIoShape>> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let model_class = model_class
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let cache_key = repo_tool_cache_key(
        "repo_extract_io_shapes",
        &repo_ref.normalized_id(),
        &IoShapeSearchConfig {
            model_class: model_class.as_deref(),
        },
    )?;

    get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || find_io_shapes(checkout.path.as_path(), model_class.as_deref()))
                .await
        },
    )
    .await
}

pub(crate) async fn repo_get_health(
    toolkit: &ResearchToolkit,
    repo_url: &str,
) -> Result<RepoHealth> {
    let repo_ref = github::normalize_repo_url(repo_url)?;
    let cache_key = repo_health_cache_key(toolkit, &repo_ref.normalized_id())?;

    let cached = toolkit
        .cache()
        .get_or_fetch_with_meta_ttls(
            cache_key,
            toolkit.config().cache_ttls.repo_health,
            toolkit.config().cache_ttls.negative,
            || async move {
                match github::get_repo_health(
                    toolkit.http(),
                    GitHubConfig {
                        api_base_url: &toolkit.config().github_api_base_url,
                        token: toolkit.config().github_token.as_deref(),
                    },
                    &repo_ref,
                )
                .await
                {
                    Ok(health) => {
                        serialize_cache_entry(RepoHealthCacheEntry::Hit { health }, false)
                    }
                    Err(error) if should_negative_cache(&error) => serialize_negative_error(error),
                    Err(error) => Err(error),
                }
            },
        )
        .await?;

    deserialize_cache_entry(cached)
}

pub(crate) async fn repo_find_export_paths(
    toolkit: &ResearchToolkit,
    repo_url: &str,
) -> Result<Vec<RepoExportPath>> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let cache_key = repo_tool_cache_key("repo_find_export_paths", &repo_ref.normalized_id(), &())?;

    get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || find_export_paths(checkout.path.as_path())).await
        },
    )
    .await
}

pub(crate) async fn repo_extract_config_schema(
    toolkit: &ResearchToolkit,
    repo_url: &str,
) -> Result<Vec<ConfigSchema>> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let cache_key =
        repo_tool_cache_key("repo_extract_config_schema", &repo_ref.normalized_id(), &())?;

    get_or_fetch_typed(
        toolkit,
        cache_key,
        toolkit.config().cache_ttls.repo_analysis,
        || async move {
            let checkout = ensure_cloned(&repo_ref).await?;
            run_blocking(move || extract_config_schemas(checkout.path.as_path())).await
        },
    )
    .await
}

pub(crate) async fn repo_diff_requirements(
    _toolkit: &ResearchToolkit,
    repo_url: &str,
    local_requirements_path: &str,
) -> Result<RequirementsDiff> {
    let repo_ref = parse_repo_ref(repo_url, None)?;
    let local_requirements_path = local_requirements_path.trim();
    if local_requirements_path.is_empty() {
        return Err(ResearchError::InvalidInput(
            "local_requirements_path must not be empty".to_string(),
        ));
    }

    let local_requirements_file = PathBuf::from(local_requirements_path);
    if !local_requirements_file.exists() {
        return Err(ResearchError::InvalidInput(format!(
            "local requirements file not found: {local_requirements_path}"
        )));
    }

    let checkout = ensure_cloned(&repo_ref).await?;
    let repo_requirements =
        run_blocking(move || extract_requirements_map(checkout.path.as_path())).await?;

    let local_content = tokio::fs::read_to_string(&local_requirements_file)
        .await
        .map_err(|err| {
            ResearchError::InvalidInput(format!(
                "failed to read local requirements file '{local_requirements_path}': {err}"
            ))
        })?;

    let local_requirements =
        parse_requirements_from_text(local_requirements_file.as_path(), local_content.as_str())?;

    let mut conflicts = Vec::new();
    let mut missing_locally = Vec::new();
    let mut compatible = Vec::new();

    for (name, repo_constraint) in &repo_requirements {
        match local_requirements.get(name) {
            Some(local_constraint) => {
                if versions_compatible(repo_constraint, local_constraint) {
                    compatible.push(name.clone());
                } else {
                    conflicts.push(format!(
                        "{}: repo requires '{}', local has '{}'",
                        name,
                        printable_constraint(repo_constraint),
                        printable_constraint(local_constraint),
                    ));
                }
            }
            None => missing_locally.push(format_dependency(name, repo_constraint)),
        }
    }

    conflicts.sort();
    missing_locally.sort();
    compatible.sort();

    Ok(RequirementsDiff {
        repo_url: repo_url.to_string(),
        conflicts,
        missing_locally,
        compatible,
        repo_total_deps: repo_requirements.len(),
        local_total_deps: local_requirements.len(),
    })
}

fn repo_health_cache_key(toolkit: &ResearchToolkit, repo_id: &str) -> Result<CacheKey> {
    let auth_tier = if toolkit.config().github_token.is_some() {
        "authenticated"
    } else {
        "unauthenticated"
    };
    let payload = RepoHealthCacheKeyPayload {
        repo_id: repo_id.to_string(),
        auth_tier,
    };

    Ok(CacheKey {
        tool_name: "repo_get_health",
        params_hash: hash_cache_payload(&payload)?,
    })
}

fn repo_tool_cache_key<T: Serialize>(
    tool_name: &'static str,
    repo_id: &str,
    payload: &T,
) -> Result<CacheKey> {
    Ok(CacheKey {
        tool_name,
        params_hash: hash_cache_payload(&(repo_id, payload))?,
    })
}

fn should_negative_cache(error: &ResearchError) -> bool {
    matches!(
        error,
        ResearchError::Upstream {
            api: ResearchApi::GitHub,
            status,
            ..
        } if *status == reqwest::StatusCode::NOT_FOUND
    )
}

fn serialize_negative_error(error: ResearchError) -> Result<FetchOutput> {
    if let ResearchError::Upstream {
        status, message, ..
    } = error
    {
        return serialize_cache_entry(
            RepoHealthCacheEntry::Miss {
                status: status.as_u16(),
                message,
            },
            true,
        );
    }

    Err(ResearchError::Internal(
        "attempted to negative-cache a non-upstream repo health error".to_string(),
    ))
}

fn serialize_cache_entry(entry: RepoHealthCacheEntry, is_negative: bool) -> Result<FetchOutput> {
    let data = serde_json::to_value(entry).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to serialize repo health cache entry: {err}"
        ))
    })?;

    Ok(if is_negative {
        FetchOutput::negative(data)
    } else {
        FetchOutput::positive(data)
    })
}

fn deserialize_cache_entry(output: FetchOutput) -> Result<RepoHealth> {
    let entry: RepoHealthCacheEntry = serde_json::from_value(output.data).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to deserialize repo health cache entry: {err}"
        ))
    })?;

    match entry {
        RepoHealthCacheEntry::Hit { health } => Ok(health),
        RepoHealthCacheEntry::Miss { status, message } => {
            let status =
                reqwest::StatusCode::from_u16(status).unwrap_or(reqwest::StatusCode::NOT_FOUND);
            Err(ResearchError::Upstream {
                api: ResearchApi::GitHub,
                status,
                message,
            })
        }
    }
}

async fn ensure_cloned(repo_ref: &RepoRef) -> Result<RepoCheckout> {
    let cache_root = repo_cache_dir();
    tokio::fs::create_dir_all(&cache_root)
        .await
        .map_err(|err| {
            ResearchError::Internal(format!(
                "failed to create repo cache directory '{}': {err}",
                cache_root.display()
            ))
        })?;

    let lock = repo_locks()
        .entry(repo_ref.cache_key())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    let checkout_dir = cache_root.join(repo_ref.fs_dir_name());
    if checkout_dir.exists() {
        touch_access_marker(checkout_dir.as_path())?;
        let commit_sha = git_rev_parse_head(checkout_dir.as_path()).await;
        return Ok(RepoCheckout {
            path: checkout_dir,
            commit_sha,
        });
    }

    let temp_dir = cache_root.join(format!(
        "{}.tmp-{}",
        repo_ref.fs_dir_name(),
        rand::random::<u64>()
    ));
    if temp_dir.exists() {
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    let mut cmd = Command::new("git");
    cmd.kill_on_drop(true);
    cmd.arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--single-branch")
        .arg("--no-tags")
        .arg("--filter=blob:limit=1m");

    if let Some(branch) = repo_ref.branch.as_deref() {
        cmd.arg("--branch").arg(branch);
    }

    cmd.arg(repo_ref.clone_url()).arg(&temp_dir);

    let output = timeout(CLONE_TIMEOUT, cmd.output())
        .await
        .map_err(|_| {
            ResearchError::Internal(format!(
                "git clone timed out after {}s for {}",
                CLONE_TIMEOUT.as_secs(),
                repo_ref.clone_url(),
            ))
        })?
        .map_err(|err| {
            ResearchError::Internal(format!(
                "failed to launch git clone for {}: {err}",
                repo_ref.clone_url(),
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(output.stderr.as_slice())
            .trim()
            .to_string();
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        return Err(ResearchError::Internal(format!(
            "git clone failed for {}: {}",
            repo_ref.clone_url(),
            if stderr.is_empty() {
                "unknown git error"
            } else {
                stderr.as_str()
            }
        )));
    }

    tokio::fs::rename(&temp_dir, &checkout_dir)
        .await
        .map_err(|err| {
            ResearchError::Internal(format!(
                "failed to move cloned repository into cache '{}': {err}",
                checkout_dir.display()
            ))
        })?;

    let checkout_size = run_blocking({
        let checkout_dir = checkout_dir.clone();
        move || dir_size_bytes(checkout_dir.as_path())
    })
    .await?;

    if checkout_size > REPO_MAX_SIZE_BYTES {
        let _ = tokio::fs::remove_dir_all(&checkout_dir).await;
        return Err(ResearchError::InvalidInput(format!(
            "repository checkout exceeds {} MB limit ({} MB): {}",
            REPO_MAX_SIZE_BYTES / (1024 * 1024),
            checkout_size / (1024 * 1024),
            repo_ref.clone_url(),
        )));
    }

    touch_access_marker(checkout_dir.as_path())?;
    enforce_repo_cache_cap(cache_root.as_path(), Some(checkout_dir.as_path())).await?;

    let commit_sha = git_rev_parse_head(checkout_dir.as_path()).await;
    Ok(RepoCheckout {
        path: checkout_dir,
        commit_sha,
    })
}

async fn enforce_repo_cache_cap(cache_root: &Path, preserve: Option<&Path>) -> Result<()> {
    let cache_root = cache_root.to_path_buf();
    let preserve = preserve.map(Path::to_path_buf);

    run_blocking(move || {
        let mut entries = collect_cache_entries(cache_root.as_path())?;
        let mut total_bytes: u64 = entries.iter().map(|entry| entry.size_bytes).sum();
        if total_bytes <= REPO_CACHE_MAX_BYTES {
            return Ok(());
        }

        entries.sort_by_key(|entry| entry.last_accessed);
        for entry in entries {
            if let Some(ref preserve_path) = preserve
                && entry.path == *preserve_path
            {
                continue;
            }
            if total_bytes <= REPO_CACHE_MAX_BYTES {
                break;
            }
            if fs::remove_dir_all(&entry.path).is_ok() {
                total_bytes = total_bytes.saturating_sub(entry.size_bytes);
            }
        }

        Ok(())
    })
    .await
}

fn collect_cache_entries(cache_root: &Path) -> Result<Vec<RepoCacheEntry>> {
    let mut entries = Vec::new();
    let dir_entries = fs::read_dir(cache_root).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to list repo cache directory '{}': {err}",
            cache_root.display()
        ))
    })?;

    for entry in dir_entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable repo cache entry");
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let size_bytes = dir_size_bytes(path.as_path())?;
        let marker = path.join(ACCESS_MARKER_FILE);
        let last_accessed = fs::metadata(&marker)
            .and_then(|metadata| metadata.modified())
            .or_else(|_| fs::metadata(path.as_path()).and_then(|metadata| metadata.modified()))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        entries.push(RepoCacheEntry {
            path,
            last_accessed,
            size_bytes,
        });
    }

    Ok(entries)
}

fn touch_access_marker(repo_path: &Path) -> Result<()> {
    let marker = repo_path.join(ACCESS_MARKER_FILE);
    fs::write(&marker, format!("{:?}", SystemTime::now())).map_err(|err| {
        ResearchError::Internal(format!(
            "failed to update repo cache access marker '{}': {err}",
            marker.display()
        ))
    })
}

async fn git_rev_parse_head(repo_path: &Path) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.kill_on_drop(true);
    cmd.arg("-C").arg(repo_path).arg("rev-parse").arg("HEAD");

    let output = timeout(GIT_METADATA_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let sha = String::from_utf8_lossy(output.stdout.as_slice())
        .trim()
        .to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

fn build_repo_summary(
    repo_url: String,
    repo_path: PathBuf,
    commit_sha: Option<String>,
) -> Result<RepoSummary> {
    let directory_tree = render_directory_tree(repo_path.as_path())?;
    let readme_snippet = read_readme_snippet(repo_path.as_path())?;
    let mut key_files = find_key_files(repo_path.as_path())?;
    key_files.sort();
    let total_files = count_files(repo_path.as_path())?;

    Ok(RepoSummary {
        url: repo_url,
        commit_sha,
        directory_tree,
        readme_snippet,
        key_files,
        total_files,
    })
}

fn render_directory_tree(repo_path: &Path) -> Result<String> {
    let mut lines = Vec::new();

    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .min_depth(1)
        .max_depth(MAX_TREE_DEPTH)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable entry while building repo tree");
                continue;
            }
        };

        if lines.len() >= MAX_TREE_ENTRIES {
            lines.push("...".to_string());
            break;
        }

        let file_name = entry.file_name().to_string_lossy();
        if file_name.is_empty() {
            continue;
        }

        let depth = entry.depth().saturating_sub(1);
        let indent = "  ".repeat(depth);
        if entry.file_type().is_dir() {
            lines.push(format!("{indent}{file_name}/"));
        } else {
            lines.push(format!("{indent}{file_name}"));
        }
    }

    if lines.is_empty() {
        Ok("(empty repository checkout)".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

fn read_readme_snippet(repo_path: &Path) -> Result<Option<String>> {
    let candidates = [
        "README.md",
        "README.rst",
        "README.txt",
        "readme.md",
        "Readme.md",
    ];

    for candidate in candidates {
        let file_path = repo_path.join(candidate);
        if file_path.exists() {
            let content = fs::read_to_string(&file_path).map_err(|err| {
                ResearchError::Internal(format!(
                    "failed reading README '{}' from repository checkout: {err}",
                    file_path.display()
                ))
            })?;
            return Ok(Some(truncate_chars(content.as_str(), 2_000)));
        }
    }

    Ok(None)
}

fn find_key_files(repo_path: &Path) -> Result<Vec<String>> {
    let key_names = [
        "README.md",
        "README.rst",
        "requirements.txt",
        "requirements-dev.txt",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "Pipfile",
        "Dockerfile",
        "train.py",
        "evaluate.py",
        "eval.py",
        "inference.py",
        "infer.py",
        "export.py",
    ];

    let mut found = BTreeSet::new();
    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable key-file candidate");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy();
        if key_names
            .iter()
            .any(|name| file_name.eq_ignore_ascii_case(name))
            && let Ok(relative) = entry.path().strip_prefix(repo_path)
        {
            found.insert(relative.to_string_lossy().replace('\\', "/"));
            if found.len() >= 100 {
                break;
            }
        }
    }

    Ok(found.into_iter().collect())
}

fn count_files(repo_path: &Path) -> Result<usize> {
    let mut count = 0usize;
    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable file while counting repo files");
                continue;
            }
        };
        if entry.file_type().is_file() {
            count = count.saturating_add(1);
        }
    }

    Ok(count)
}

fn find_model_definitions(
    repo_path: &Path,
    framework: Option<&str>,
) -> Result<Vec<ModelDefinition>> {
    let files = load_text_files(repo_path, &["py"])?;

    let pytorch_re = Regex::new(r"class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*nn\.Module[^)]*\)")
        .map_err(|err| {
            ResearchError::Internal(format!("failed to compile pytorch regex: {err}"))
        })?;
    let tensorflow_re = Regex::new(
        r"class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*(?:tf\.keras\.Model|keras\.Model)[^)]*\)",
    )
    .map_err(|err| ResearchError::Internal(format!("failed to compile tensorflow regex: {err}")))?;
    let jax_re = Regex::new(
        r"class\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^)]*(?:flax\.linen\.Module|flax\.nnx\.Module|equinox\.Module|eqx\.Module)[^)]*\)",
    )
    .map_err(|err| ResearchError::Internal(format!("failed to compile jax regex: {err}")))?;

    let mut definitions = Vec::new();
    for file in files {
        let lines = file.content.lines().collect::<Vec<_>>();
        for (idx, line) in lines.iter().enumerate() {
            let captures = match framework {
                Some("pytorch") => pytorch_re.captures(line),
                Some("tensorflow") => tensorflow_re.captures(line),
                Some("jax") => jax_re.captures(line),
                None => pytorch_re
                    .captures(line)
                    .or_else(|| tensorflow_re.captures(line))
                    .or_else(|| jax_re.captures(line)),
                Some(other) => {
                    return Err(ResearchError::InvalidInput(format!(
                        "unsupported framework '{other}'; expected one of pytorch|tensorflow|jax"
                    )));
                }
            };

            if let Some(captures) = captures {
                let class_name = captures
                    .get(1)
                    .map(|capture| capture.as_str().to_string())
                    .unwrap_or_else(|| "UnknownClass".to_string());
                definitions.push(ModelDefinition {
                    file_path: file.relative_path.clone(),
                    class_name,
                    line_number: idx + 1,
                    context: build_line_context(lines.as_slice(), idx, 2),
                });
                if definitions.len() >= MAX_MODELS_RETURNED {
                    break;
                }
            }
        }
        if definitions.len() >= MAX_MODELS_RETURNED {
            break;
        }
    }

    definitions.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.line_number.cmp(&b.line_number))
            .then(a.class_name.cmp(&b.class_name))
    });
    definitions.dedup_by(|left, right| {
        left.file_path == right.file_path
            && left.line_number == right.line_number
            && left.class_name == right.class_name
    });

    Ok(definitions)
}

fn find_entrypoints(repo_path: &Path, task_hint: Option<&str>) -> Result<Vec<RepoEntrypoint>> {
    let files = load_text_files(repo_path, &["py"])?;
    let task_hint = task_hint
        .map(str::to_ascii_lowercase)
        .filter(|value| !value.is_empty());

    let mut entrypoints = Vec::new();
    for file in files {
        let lower_path = file.relative_path.to_ascii_lowercase();
        let lower_content = file.content.to_ascii_lowercase();

        for kind in ["train", "eval", "infer", "export"] {
            if let Some(ref hint) = task_hint
                && !kind.contains(hint.as_str())
                && !hint.contains(kind)
            {
                continue;
            }

            let by_name = matches_entrypoint_name(kind, lower_path.as_str());
            let by_content = matches_entrypoint_content(kind, lower_content.as_str());
            if !by_name && !by_content {
                continue;
            }

            entrypoints.push(RepoEntrypoint {
                file_path: file.relative_path.clone(),
                kind: kind.to_string(),
                cli_args_summary: summarize_cli_args(file.content.as_str()),
                config_file: find_associated_config(repo_path, file.relative_path.as_str()),
            });

            if entrypoints.len() >= MAX_ENTRYPOINTS_RETURNED {
                break;
            }
        }

        if entrypoints.len() >= MAX_ENTRYPOINTS_RETURNED {
            break;
        }
    }

    entrypoints.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.file_path.cmp(&b.file_path)));
    entrypoints
        .dedup_by(|left, right| left.kind == right.kind && left.file_path == right.file_path);
    Ok(entrypoints)
}

fn find_io_shapes(repo_path: &Path, model_class: Option<&str>) -> Result<Vec<RepoIoShape>> {
    let files = load_text_files(repo_path, &["py", "yaml", "yml", "json", "toml"])?;
    let model_class = model_class
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let input_re = Regex::new(
        r"(?i)(?:input_size|image_size|img_size|input_shape)\s*[:=]\s*([A-Za-z0-9_\[\]\(\),\.\s-]+)",
    )
    .map_err(|err| ResearchError::Internal(format!("failed to compile input shape regex: {err}")))?;
    let output_re = Regex::new(
        r"(?i)(?:output_size|output_shape|num_classes|out_channels)\s*[:=]\s*([A-Za-z0-9_\[\]\(\),\.\s-]+)",
    )
    .map_err(|err| ResearchError::Internal(format!("failed to compile output shape regex: {err}")))?;
    let tensor_ctor_re = Regex::new(r"(?:torch\.(?:randn|zeros)|np\.zeros)\(([^\)]{1,200})\)")
        .map_err(|err| {
            ResearchError::Internal(format!("failed to compile tensor ctor regex: {err}"))
        })?;
    let dtype_re = Regex::new(r"(?i)dtype\s*[:=]\s*([A-Za-z0-9_\.]+)")
        .map_err(|err| ResearchError::Internal(format!("failed to compile dtype regex: {err}")))?;

    let mut shapes = Vec::new();
    for file in files {
        if let Some(ref model_class) = model_class
            && !file.content.contains(model_class)
        {
            continue;
        }

        for (idx, line) in file.content.lines().enumerate() {
            let mut input_shapes = input_re.captures(line).and_then(|captures| {
                captures
                    .get(1)
                    .map(|capture| capture.as_str().trim().to_string())
            });
            if input_shapes.is_none() {
                input_shapes = tensor_ctor_re.captures(line).and_then(|captures| {
                    captures
                        .get(1)
                        .map(|capture| capture.as_str().trim().to_string())
                });
            }

            let output_shapes = output_re.captures(line).and_then(|captures| {
                captures
                    .get(1)
                    .map(|capture| capture.as_str().trim().to_string())
            });

            let dtype = dtype_re.captures(line).and_then(|captures| {
                captures
                    .get(1)
                    .map(|capture| capture.as_str().trim().to_string())
            });

            if input_shapes.is_none() && output_shapes.is_none() && dtype.is_none() {
                continue;
            }

            shapes.push(RepoIoShape {
                input_shapes,
                output_shapes,
                dtype,
                source_file: file.relative_path.clone(),
                source_line: idx + 1,
            });

            if shapes.len() >= MAX_IO_SHAPES_RETURNED {
                break;
            }
        }
        if shapes.len() >= MAX_IO_SHAPES_RETURNED {
            break;
        }
    }

    shapes.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then(a.source_line.cmp(&b.source_line))
    });
    shapes.dedup_by(|left, right| {
        left.source_file == right.source_file && left.source_line == right.source_line
    });

    Ok(shapes)
}

fn find_export_paths(repo_path: &Path) -> Result<Vec<RepoExportPath>> {
    let files = load_text_files(repo_path, &["py"])?;
    let requirements = extract_requirements_map(repo_path)?;
    let framework_version = infer_framework_version(&requirements);

    let patterns = [
        (
            Regex::new(r"torch\.onnx\.export|onnx\.save").map_err(|err| {
                ResearchError::Internal(format!("failed to compile ONNX regex: {err}"))
            })?,
            "onnx",
        ),
        (
            Regex::new(r"(?i)tensorrt|trt\.Builder|torch2trt").map_err(|err| {
                ResearchError::Internal(format!("failed to compile TensorRT regex: {err}"))
            })?,
            "tensorrt",
        ),
        (
            Regex::new(r"(?i)tf\.lite\.TFLiteConverter|tflite").map_err(|err| {
                ResearchError::Internal(format!("failed to compile TFLite regex: {err}"))
            })?,
            "tflite",
        ),
        (
            Regex::new(r"(?i)coremltools|ct\.convert").map_err(|err| {
                ResearchError::Internal(format!("failed to compile CoreML regex: {err}"))
            })?,
            "coreml",
        ),
        (
            Regex::new(r"(?i)openvino|mo\.convert").map_err(|err| {
                ResearchError::Internal(format!("failed to compile OpenVINO regex: {err}"))
            })?,
            "openvino",
        ),
        (
            Regex::new(r"torch\.jit\.(?:script|trace)").map_err(|err| {
                ResearchError::Internal(format!("failed to compile TorchScript regex: {err}"))
            })?,
            "torchscript",
        ),
    ];

    let mut exports = Vec::new();
    for file in files {
        for (regex, target_format) in &patterns {
            if regex.is_match(file.content.as_str()) {
                exports.push(RepoExportPath {
                    file_path: file.relative_path.clone(),
                    target_format: (*target_format).to_string(),
                    framework_version: framework_version.clone(),
                });
            }
        }

        if exports.len() >= MAX_EXPORT_PATHS_RETURNED {
            break;
        }
    }

    exports.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.target_format.cmp(&b.target_format))
    });
    exports.dedup_by(|left, right| {
        left.file_path == right.file_path && left.target_format == right.target_format
    });
    Ok(exports)
}

fn extract_config_schemas(repo_path: &Path) -> Result<Vec<ConfigSchema>> {
    let config_files = load_text_files(repo_path, &["yaml", "yml", "json", "toml", "cfg"])?;
    let mut schemas = Vec::new();

    for file in config_files {
        let config_file = file.relative_path;
        let format = detect_config_format(config_file.as_str()).to_string();
        let mut key_params = extract_config_key_params(file.content.as_str(), format.as_str())?;
        key_params.sort_by(|a, b| a.name.cmp(&b.name));
        key_params.dedup_by(|left, right| left.name == right.name);
        key_params.truncate(MAX_KEY_PARAMS_PER_SCHEMA);

        if !key_params.is_empty() {
            schemas.push(ConfigSchema {
                config_file,
                format,
                key_params,
            });
        }

        if schemas.len() >= MAX_CONFIG_SCHEMAS_RETURNED {
            break;
        }
    }

    let python_files = load_text_files(repo_path, &["py"])?;
    for file in python_files {
        let key_params = extract_argparse_params(file.content.as_str())?;
        if key_params.is_empty() {
            continue;
        }
        schemas.push(ConfigSchema {
            config_file: file.relative_path,
            format: "argparse".to_string(),
            key_params,
        });
        if schemas.len() >= MAX_CONFIG_SCHEMAS_RETURNED {
            break;
        }
    }

    schemas.sort_by(|a, b| a.config_file.cmp(&b.config_file));
    schemas.dedup_by(|left, right| {
        left.config_file == right.config_file && left.format == right.format
    });
    Ok(schemas)
}

fn extract_config_key_params(content: &str, format: &str) -> Result<Vec<ConfigParam>> {
    match format {
        "json" => {
            let parsed = serde_json::from_str::<serde_json::Value>(content).map_err(|err| {
                ResearchError::Internal(format!("failed to parse JSON config content: {err}"))
            })?;
            let mut params = Vec::new();
            if let Some(root) = parsed.as_object() {
                for (name, value) in root {
                    let default = if value.is_string() || value.is_number() || value.is_boolean() {
                        Some(value.to_string().trim_matches('"').to_string())
                    } else {
                        None
                    };
                    params.push(ConfigParam {
                        name: name.to_string(),
                        default,
                        description: None,
                    });
                }
            }
            Ok(params)
        }
        "yaml" | "toml" | "cfg" => {
            let key_re = Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_\.-]*)\s*[:=]\s*([^#\n]+)?")
                .map_err(|err| {
                    ResearchError::Internal(format!("failed to compile config key regex: {err}"))
                })?;
            let mut params = Vec::new();
            for line in content.lines() {
                if let Some(captures) = key_re.captures(line)
                    && let Some(name_match) = captures.get(1)
                {
                    params.push(ConfigParam {
                        name: name_match.as_str().to_string(),
                        default: captures
                            .get(2)
                            .map(|capture| capture.as_str().trim().trim_matches('"').to_string())
                            .filter(|value| !value.is_empty()),
                        description: None,
                    });
                }
            }
            Ok(params)
        }
        _ => Ok(Vec::new()),
    }
}

fn extract_argparse_params(content: &str) -> Result<Vec<ConfigParam>> {
    let add_argument_re = Regex::new(
        r#"add_argument\(\s*[\"'](--?[A-Za-z0-9_\-]+)[\"'](?:[^\)]*default\s*=\s*([^,\)]+))?"#,
    )
    .map_err(|err| ResearchError::Internal(format!("failed to compile argparse regex: {err}")))?;
    let click_option_re = Regex::new(r#"@click\.option\(\s*[\"'](--?[A-Za-z0-9_\-]+)[\"']"#)
        .map_err(|err| {
            ResearchError::Internal(format!("failed to compile click option regex: {err}"))
        })?;
    let typer_option_re =
        Regex::new(r#"typer\.Option\([^\)]*--([A-Za-z0-9_\-]+)"#).map_err(|err| {
            ResearchError::Internal(format!("failed to compile typer option regex: {err}"))
        })?;

    let mut params = Vec::new();

    for captures in add_argument_re.captures_iter(content) {
        let Some(name) = captures.get(1).map(|capture| capture.as_str()) else {
            continue;
        };
        params.push(ConfigParam {
            name: name.trim_start_matches('-').replace('-', "_"),
            default: captures.get(2).map(|capture| {
                capture
                    .as_str()
                    .trim()
                    .trim_matches('\"')
                    .trim_matches('\'')
                    .to_string()
            }),
            description: None,
        });
    }

    for captures in click_option_re.captures_iter(content) {
        if let Some(name) = captures.get(1).map(|capture| capture.as_str()) {
            params.push(ConfigParam {
                name: name.trim_start_matches('-').replace('-', "_"),
                default: None,
                description: None,
            });
        }
    }

    for captures in typer_option_re.captures_iter(content) {
        if let Some(name) = captures.get(1).map(|capture| capture.as_str()) {
            params.push(ConfigParam {
                name: name.replace('-', "_"),
                default: None,
                description: None,
            });
        }
    }

    params.sort_by(|a, b| a.name.cmp(&b.name));
    params.dedup_by(|left, right| left.name == right.name);
    params.truncate(MAX_KEY_PARAMS_PER_SCHEMA);
    Ok(params)
}

fn detect_config_format(path: &str) -> &str {
    if path.ends_with(".yaml") || path.ends_with(".yml") {
        return "yaml";
    }
    if path.ends_with(".json") {
        return "json";
    }
    if path.ends_with(".toml") {
        return "toml";
    }
    if path.ends_with(".cfg") {
        return "cfg";
    }

    "yaml"
}

fn summarize_cli_args(content: &str) -> Option<String> {
    let add_argument_re = Regex::new(r#"add_argument\(\s*[\"'](--?[A-Za-z0-9_\-]+)[\"']"#).ok()?;
    let click_option_re =
        Regex::new(r#"@click\.option\(\s*[\"'](--?[A-Za-z0-9_\-]+)[\"']"#).ok()?;

    let mut args = BTreeSet::new();
    for captures in add_argument_re.captures_iter(content) {
        if let Some(arg) = captures.get(1).map(|capture| capture.as_str().to_string()) {
            args.insert(arg);
        }
        if args.len() >= 8 {
            break;
        }
    }

    if args.len() < 8 {
        for captures in click_option_re.captures_iter(content) {
            if let Some(arg) = captures.get(1).map(|capture| capture.as_str().to_string()) {
                args.insert(arg);
            }
            if args.len() >= 8 {
                break;
            }
        }
    }

    if args.is_empty() {
        None
    } else {
        Some(args.into_iter().collect::<Vec<_>>().join(", "))
    }
}

fn find_associated_config(repo_path: &Path, file_path: &str) -> Option<String> {
    let full_path = repo_path.join(file_path);
    let parent = full_path.parent()?;
    let stem = full_path
        .file_stem()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);

    let candidates = fs::read_dir(parent).ok()?;
    for candidate in candidates {
        let candidate = candidate.ok()?;
        let candidate_path = candidate.path();
        if !candidate_path.is_file() {
            continue;
        }

        let extension = candidate_path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        let Some(extension) = extension else {
            continue;
        };
        if !["yaml", "yml", "json", "toml", "cfg"].contains(&extension.as_str()) {
            continue;
        }

        let filename = candidate_path
            .file_name()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)?;
        if filename.contains("config")
            || stem.as_deref().is_some_and(|stem| filename.contains(stem))
        {
            let relative = candidate_path.strip_prefix(repo_path).ok()?;
            return Some(relative.to_string_lossy().replace('\\', "/"));
        }
    }

    None
}

fn matches_entrypoint_name(kind: &str, lower_path: &str) -> bool {
    let filename = lower_path.rsplit('/').next().unwrap_or(lower_path);
    match kind {
        "train" => filename.contains("train") || filename.contains("fit") || filename == "main.py",
        "eval" => filename.contains("eval") || filename.contains("test"),
        "infer" => {
            filename.contains("infer") || filename.contains("predict") || filename.contains("demo")
        }
        "export" => {
            filename.contains("export") || filename.contains("convert") || filename.contains("onnx")
        }
        _ => false,
    }
}

fn matches_entrypoint_content(kind: &str, lower_content: &str) -> bool {
    if !lower_content.contains("__name__") || !lower_content.contains("__main__") {
        return false;
    }

    match kind {
        "train" => {
            lower_content.contains("train")
                || lower_content.contains("fit(")
                || lower_content.contains("trainer")
        }
        "eval" => {
            lower_content.contains("evaluate")
                || lower_content.contains("validation")
                || lower_content.contains("test")
        }
        "infer" => {
            lower_content.contains("inference")
                || lower_content.contains("predict")
                || lower_content.contains("infer")
        }
        "export" => {
            lower_content.contains("onnx")
                || lower_content.contains("torch.jit")
                || lower_content.contains("tensorrt")
        }
        _ => false,
    }
}

fn normalize_framework(framework: Option<&str>) -> Result<Option<String>> {
    let normalized = framework
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    if let Some(ref value) = normalized
        && value != "pytorch"
        && value != "tensorflow"
        && value != "jax"
    {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported framework '{value}'; expected one of pytorch|tensorflow|jax"
        )));
    }

    Ok(normalized)
}

fn normalize_task_hint(task_hint: Option<&str>) -> Result<Option<String>> {
    let normalized = task_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    if let Some(ref value) = normalized
        && !["train", "eval", "infer", "export"]
            .iter()
            .any(|known| known.contains(value.as_str()) || value.contains(known))
    {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported task_hint '{value}'; expected one of train|eval|infer|export"
        )));
    }

    Ok(normalized)
}

fn parse_repo_ref(repo_url: &str, branch: Option<&str>) -> Result<RepoRef> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        return Err(ResearchError::InvalidInput(
            "repo_url must not be empty".to_string(),
        ));
    }

    let parsed = reqwest::Url::parse(trimmed).map_err(|err| {
        ResearchError::InvalidInput(format!(
            "invalid repo_url '{trimmed}': {err}. expected https://github.com/{{owner}}/{{repo}} or https://gitlab.com/{{owner}}/{{repo}}"
        ))
    })?;

    if parsed.scheme() != "https" {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported repo_url scheme '{}' (expected https)",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ResearchError::InvalidInput("repo_url is missing a host".to_string()))?;

    if !host.eq_ignore_ascii_case(GITHUB_HOST) && !host.eq_ignore_ascii_case(GITLAB_HOST) {
        return Err(ResearchError::InvalidInput(format!(
            "unsupported repo host '{host}' (expected {GITHUB_HOST} or {GITLAB_HOST})"
        )));
    }

    let segments = parsed
        .path_segments()
        .ok_or_else(|| {
            ResearchError::InvalidInput(format!(
                "repo_url '{trimmed}' does not contain owner/repo path segments"
            ))
        })?
        .collect::<Vec<_>>();

    if segments.len() < 2 {
        return Err(ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' must include owner and repo path segments"
        )));
    }

    let owner = segments[0].trim();
    let repo_segment = segments[1].trim();
    if owner.is_empty() || repo_segment.is_empty() {
        return Err(ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' must include owner and repo segments"
        )));
    }

    let repo = repo_segment
        .strip_suffix(".git")
        .unwrap_or(repo_segment)
        .trim();
    if repo.is_empty() {
        return Err(ResearchError::InvalidInput(format!(
            "repo_url '{trimmed}' contains an empty repository name"
        )));
    }

    let mut resolved_branch = branch
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if resolved_branch.is_none() && segments.len() >= 4 && segments[2] == "tree" {
        let branch_segment = segments[3].trim();
        if !branch_segment.is_empty() {
            resolved_branch = Some(branch_segment.to_string());
        }
    }

    if let Some(ref branch_name) = resolved_branch {
        validate_branch_name(branch_name)?;
    }

    Ok(RepoRef {
        host: host.to_ascii_lowercase(),
        owner: owner.to_string(),
        repo: repo.to_string(),
        branch: resolved_branch,
    })
}

fn validate_branch_name(branch: &str) -> Result<()> {
    if branch.trim().is_empty() {
        return Err(ResearchError::InvalidInput(
            "branch must not be empty".to_string(),
        ));
    }

    if branch.starts_with('-') {
        return Err(ResearchError::InvalidInput(format!(
            "invalid branch '{branch}': branch cannot start with '-'"
        )));
    }

    if branch.chars().any(char::is_whitespace) {
        return Err(ResearchError::InvalidInput(format!(
            "invalid branch '{branch}': branch cannot contain whitespace"
        )));
    }

    if branch.contains("..") || branch.contains('~') || branch.contains('^') || branch.contains(':')
    {
        return Err(ResearchError::InvalidInput(format!(
            "invalid branch '{branch}': contains unsupported git ref characters"
        )));
    }

    Ok(())
}

fn should_descend(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(OsStr::to_str) {
        let lowered = name.to_ascii_lowercase();
        return !matches!(
            lowered.as_str(),
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".pytest_cache"
                | ".mypy_cache"
                | ".tox"
                | ".ruff_cache"
                | "vendor"
        );
    }

    true
}

fn load_text_files(repo_path: &Path, allowed_extensions: &[&str]) -> Result<Vec<TextFile>> {
    let allowed_extensions = allowed_extensions
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();

    let mut files_seen = 0usize;
    let mut bytes_loaded = 0u64;
    let mut files = Vec::new();

    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable file while scanning repo");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        files_seen = files_seen.saturating_add(1);
        if files_seen > MAX_SCAN_FILE_COUNT {
            break;
        }

        let extension = entry
            .path()
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();

        if !allowed_extensions.is_empty() && !allowed_extensions.contains(&extension) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::debug!(path = %entry.path().display(), error = %err, "skipping file with unreadable metadata");
                continue;
            }
        };

        let file_size = metadata.len();
        if file_size > MAX_SCAN_FILE_SIZE_BYTES {
            continue;
        }

        if bytes_loaded.saturating_add(file_size) > MAX_SCAN_TOTAL_BYTES {
            break;
        }

        let bytes = match fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::debug!(path = %entry.path().display(), error = %err, "failed to read file while scanning repo");
                continue;
            }
        };

        if is_binary(bytes.as_slice()) {
            continue;
        }

        let relative_path = match entry.path().strip_prefix(repo_path) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        bytes_loaded = bytes_loaded.saturating_add(file_size);
        files.push(TextFile {
            relative_path,
            content: String::from_utf8_lossy(bytes.as_slice()).to_string(),
        });
    }

    Ok(files)
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(4_096).any(|byte| *byte == 0)
}

fn dir_size_bytes(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    let walker = WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable file while computing directory size");
                continue;
            }
        };

        if entry.file_type().is_file() {
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    tracing::debug!(path = %entry.path().display(), error = %err, "unable to read file metadata while computing directory size");
                    continue;
                }
            };
            total = total.saturating_add(metadata.len());
        }
    }

    Ok(total)
}

fn collect_requirement_source_files(repo_path: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable requirement source file entry");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if is_requirement_file(entry.path())
            && let Ok(relative) = entry.path().strip_prefix(repo_path)
        {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }

    files.sort();
    files.dedup();
    Ok(files)
}

fn extract_requirements_map(repo_path: &Path) -> Result<BTreeMap<String, String>> {
    let mut requirements = BTreeMap::new();
    let walker = WalkDir::new(repo_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry.path()));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::debug!(error = %err, "skipping unreadable dependency source file");
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_requirement_file(entry.path()) {
            continue;
        }

        let content = match fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(err) => {
                tracing::debug!(path = %entry.path().display(), error = %err, "failed reading dependency source file");
                continue;
            }
        };

        let parsed = parse_requirements_from_text(entry.path(), content.as_str())?;
        for (name, constraint) in parsed {
            requirements.insert(name, constraint);
        }

        if requirements.len() >= MAX_REQUIREMENTS_RETURNED {
            break;
        }
    }

    Ok(requirements)
}

fn is_requirement_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let lower = file_name.to_ascii_lowercase();
    if lower.starts_with("requirements") && lower.ends_with(".txt") {
        return true;
    }

    matches!(
        lower.as_str(),
        "pyproject.toml"
            | "setup.py"
            | "setup.cfg"
            | "pipfile"
            | "environment.yml"
            | "environment.yaml"
    )
}

fn parse_requirements_from_text(path: &Path, content: &str) -> Result<BTreeMap<String, String>> {
    let lower_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if lower_name.starts_with("requirements") && lower_name.ends_with(".txt") {
        return parse_requirements_txt(content);
    }
    if lower_name == "pyproject.toml" {
        return parse_pyproject_toml(content);
    }
    if lower_name == "setup.py" {
        return parse_setup_py(content);
    }
    if lower_name == "setup.cfg" {
        return parse_setup_cfg(content);
    }
    if lower_name == "pipfile" {
        return parse_pipfile(content);
    }
    if lower_name == "environment.yml" || lower_name == "environment.yaml" {
        return parse_environment_yaml(content);
    }

    Ok(BTreeMap::new())
}

fn parse_requirements_txt(content: &str) -> Result<BTreeMap<String, String>> {
    let requirement_re = Regex::new(r"^([A-Za-z0-9_.-]+)\s*([<>=!~].*)?$").map_err(|err| {
        ResearchError::Internal(format!("failed to compile requirements regex: {err}"))
    })?;

    let mut requirements = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("-r")
            || line.starts_with("--")
            || line.starts_with("git+")
        {
            continue;
        }

        let no_comment = line.split('#').next().map(str::trim).unwrap_or("");
        if no_comment.is_empty() {
            continue;
        }

        if let Some(captures) = requirement_re.captures(no_comment)
            && let Some(name_capture) = captures.get(1)
        {
            let name = normalize_requirement_name(name_capture.as_str());
            let constraint = captures
                .get(2)
                .map(|capture| normalize_constraint(capture.as_str()))
                .unwrap_or_default();
            requirements.insert(name, constraint);
        }
    }

    Ok(requirements)
}

fn parse_pyproject_toml(content: &str) -> Result<BTreeMap<String, String>> {
    let requirement_re = Regex::new(
        r#"[\"']([A-Za-z0-9_.-]+(?:\[[^\]]+\])?(?:\s*[<>=!~].*?)?)[\"']"#,
    )
    .map_err(|err| {
        ResearchError::Internal(format!(
            "failed to compile pyproject requirement regex: {err}"
        ))
    })?;

    let mut requirements = BTreeMap::new();
    for captures in requirement_re.captures_iter(content) {
        let Some(raw) = captures.get(1).map(|capture| capture.as_str().trim()) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }

        let (name, constraint) = split_requirement(raw);
        requirements.insert(name, constraint);
    }

    Ok(requirements)
}

fn parse_setup_py(content: &str) -> Result<BTreeMap<String, String>> {
    let requirement_re = Regex::new(
        r#"[\"']([A-Za-z0-9_.-]+(?:\[[^\]]+\])?(?:\s*[<>=!~].*?)?)[\"']"#,
    )
    .map_err(|err| {
        ResearchError::Internal(format!(
            "failed to compile setup.py requirement regex: {err}"
        ))
    })?;

    let mut requirements = BTreeMap::new();
    for captures in requirement_re.captures_iter(content) {
        let Some(raw) = captures.get(1).map(|capture| capture.as_str().trim()) else {
            continue;
        };

        if raw.contains("python") || raw.contains("setuptools") {
            continue;
        }

        let (name, constraint) = split_requirement(raw);
        requirements.insert(name, constraint);
    }

    Ok(requirements)
}

fn parse_setup_cfg(content: &str) -> Result<BTreeMap<String, String>> {
    parse_requirements_txt(content)
}

fn parse_pipfile(content: &str) -> Result<BTreeMap<String, String>> {
    let package_re =
        Regex::new(r#"^\s*([A-Za-z0-9_.-]+)\s*=\s*["']?([^"'\n]+)"#).map_err(|err| {
            ResearchError::Internal(format!("failed to compile Pipfile regex: {err}"))
        })?;

    let mut requirements = BTreeMap::new();
    for line in content.lines() {
        if let Some(captures) = package_re.captures(line)
            && let Some(name_capture) = captures.get(1)
        {
            let name = normalize_requirement_name(name_capture.as_str());
            let constraint = captures
                .get(2)
                .map(|capture| normalize_constraint(capture.as_str()))
                .unwrap_or_default();
            requirements.insert(name, constraint);
        }
    }

    Ok(requirements)
}

fn parse_environment_yaml(content: &str) -> Result<BTreeMap<String, String>> {
    let requirement_re = Regex::new(r"^-\s*([A-Za-z0-9_.-]+)\s*([<>=!~].*)?$").map_err(|err| {
        ResearchError::Internal(format!("failed to compile environment.yaml regex: {err}"))
    })?;

    let mut requirements = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }

        let value = trimmed.trim_start_matches('-').trim();
        if value.starts_with("python=") {
            continue;
        }

        if let Some(captures) = requirement_re.captures(trimmed)
            && let Some(name_capture) = captures.get(1)
        {
            let name = normalize_requirement_name(name_capture.as_str());
            let constraint = captures
                .get(2)
                .map(|capture| normalize_constraint(capture.as_str()))
                .unwrap_or_default();
            requirements.insert(name, constraint);
        }
    }

    Ok(requirements)
}

fn split_requirement(raw: &str) -> (String, String) {
    let normalized = raw.trim().replace(' ', "");
    for marker in ["==", ">=", "<=", "~=", "!=", ">", "<"] {
        if let Some((name, constraint)) = normalized.split_once(marker) {
            return (
                normalize_requirement_name(name),
                normalize_constraint(format!("{marker}{constraint}").as_str()),
            );
        }
    }

    (
        normalize_requirement_name(normalized.as_str()),
        String::new(),
    )
}

fn normalize_requirement_name(name: &str) -> String {
    name.trim()
        .trim_matches('\"')
        .trim_matches('\'')
        .split('[')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

fn normalize_constraint(constraint: &str) -> String {
    constraint
        .trim()
        .trim_matches('\"')
        .trim_matches('\'')
        .replace(' ', "")
}

fn format_dependency(name: &str, constraint: &str) -> String {
    if constraint.is_empty() {
        name.to_string()
    } else {
        format!("{name}{constraint}")
    }
}

fn printable_constraint(constraint: &str) -> String {
    if constraint.is_empty() {
        "(unspecified)".to_string()
    } else {
        constraint.to_string()
    }
}

fn infer_framework_version(requirements: &BTreeMap<String, String>) -> Option<String> {
    for package in ["torch", "tensorflow", "jax", "flax"] {
        if let Some(constraint) = requirements.get(package) {
            return Some(format_dependency(package, constraint));
        }
    }

    None
}

fn versions_compatible(repo_constraint: &str, local_constraint: &str) -> bool {
    let repo_constraint = normalize_constraint(repo_constraint);
    let local_constraint = normalize_constraint(local_constraint);

    if repo_constraint.is_empty() || local_constraint.is_empty() {
        return true;
    }

    if repo_constraint == local_constraint {
        return true;
    }

    if let (Some(repo_exact), Some(local_exact)) = (
        repo_constraint.strip_prefix("=="),
        local_constraint.strip_prefix("=="),
    ) {
        return repo_exact == local_exact;
    }

    if let (Some(min_required), Some(local_exact)) = (
        repo_constraint.strip_prefix(">="),
        local_constraint.strip_prefix("=="),
    ) {
        return compare_version_tuple(local_exact, min_required)
            .is_some_and(|ordering| ordering != std::cmp::Ordering::Less);
    }

    if let (Some(min_required), Some(local_min)) = (
        repo_constraint.strip_prefix(">="),
        local_constraint.strip_prefix(">="),
    ) {
        return compare_version_tuple(local_min, min_required)
            .is_some_and(|ordering| ordering != std::cmp::Ordering::Less);
    }

    false
}

fn compare_version_tuple(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left_parts = left
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let right_parts = right
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;

    let max = left_parts.len().max(right_parts.len());
    for idx in 0..max {
        let l = *left_parts.get(idx).unwrap_or(&0);
        let r = *right_parts.get(idx).unwrap_or(&0);
        if l < r {
            return Some(std::cmp::Ordering::Less);
        }
        if l > r {
            return Some(std::cmp::Ordering::Greater);
        }
    }

    Some(std::cmp::Ordering::Equal)
}

fn build_line_context(lines: &[&str], center_line: usize, radius: usize) -> String {
    let start = center_line.saturating_sub(radius);
    let end = (center_line + radius + 1).min(lines.len());
    lines[start..end].join("\n")
}

async fn run_blocking<T, F>(work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| ResearchError::Internal(format!("blocking task join error: {err}")))?
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::header;
    use wiremock::matchers::method;
    use wiremock::matchers::path;
    use wiremock::matchers::query_param;

    use crate::ResearchToolkit;
    use crate::config::CacheTtls;
    use crate::config::ResearchConfig;
    use crate::error::ResearchError;
    use crate::tools::test_helpers::build_test_toolkit_with_config;

    use super::find_model_definitions;
    use super::parse_repo_ref;
    use super::parse_requirements_txt;
    use super::versions_compatible;

    #[test]
    fn parse_repo_ref_accepts_supported_hosts_and_variants() {
        let parsed = parse_repo_ref("https://github.com/openai/codex.git/tree/main", None)
            .expect("github URL should parse");
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.owner, "openai");
        assert_eq!(parsed.repo, "codex");
        assert_eq!(parsed.branch.as_deref(), Some("main"));

        let parsed = parse_repo_ref("https://gitlab.com/group/project", Some("dev"))
            .expect("gitlab URL should parse");
        assert_eq!(parsed.host, "gitlab.com");
        assert_eq!(parsed.owner, "group");
        assert_eq!(parsed.repo, "project");
        assert_eq!(parsed.branch.as_deref(), Some("dev"));
    }

    #[test]
    fn parse_repo_ref_rejects_invalid_values() {
        let cases = [
            "",
            "github.com/openai/codex",
            "http://github.com/openai/codex",
            "https://bitbucket.org/openai/codex",
            "https://github.com/openai",
        ];

        for value in cases {
            let error = parse_repo_ref(value, None).expect_err("value should fail");
            assert!(
                matches!(error, ResearchError::InvalidInput(_)),
                "unexpected error for {value}: {error}"
            );
        }
    }

    #[test]
    fn parse_requirements_txt_extracts_constraints() {
        let parsed = parse_requirements_txt(
            "\
                torch>=2.2\n\
                pydantic==2.10.6\n\
                # comment\n\
                numpy\n",
        )
        .expect("requirements should parse");

        assert_eq!(parsed.get("torch").cloned(), Some(">=2.2".to_string()));
        assert_eq!(
            parsed.get("pydantic").cloned(),
            Some("==2.10.6".to_string())
        );
        assert_eq!(parsed.get("numpy").cloned(), Some(String::new()));
    }

    #[test]
    fn versions_compatible_handles_common_cases() {
        assert!(versions_compatible("", "==1.0.0"));
        assert!(versions_compatible("==1.2.3", "==1.2.3"));
        assert!(!versions_compatible("==1.2.3", "==1.2.4"));
        assert!(versions_compatible(">=1.2.3", "==1.5.0"));
        assert!(!versions_compatible(">=2.0.0", "==1.9.9"));
    }

    #[test]
    fn find_model_definitions_supports_jax_framework() {
        let repo_dir = std::env::temp_dir().join(format!(
            "codex-research-tools-jax-{}",
            rand::random::<u64>()
        ));
        fs::create_dir_all(&repo_dir).expect("create repo dir");
        fs::write(
            repo_dir.join("jax_model.py"),
            "import flax\n\nclass FlaxModel(flax.linen.Module):\n    pass\n",
        )
        .expect("write jax model file");
        fs::write(
            repo_dir.join("torch_model.py"),
            "import torch.nn as nn\n\nclass TorchModel(nn.Module):\n    pass\n",
        )
        .expect("write torch model file");

        let models = find_model_definitions(repo_dir.as_path(), Some("jax"))
            .expect("jax model detection should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].class_name, "FlaxModel".to_string());

        let _ = fs::remove_dir_all(&repo_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_returns_expected_shape() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": { "spdx_id": "MIT" },
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 321,
                "open_issues_count": 14,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .append_header(
                        "link",
                        "<https://api.github.com/repos/openai/codex/releases?per_page=1&page=4>; rel=\"last\"",
                    )
                    .set_body_json(serde_json::json!([{ "id": 1 }])),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "check_runs": [
                    { "status": "completed", "conclusion": "success" }
                ]
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), Some("test-token"));

        let health = toolkit
            .repo_get_health("https://github.com/openai/codex.git/tree/main")
            .await
            .expect("repo_get_health should succeed");

        assert_eq!(health.license, Some("MIT".to_string()));
        assert_eq!(
            health.last_commit_date,
            Some("2026-02-01T01:02:03Z".to_string())
        );
        assert_eq!(health.stars, 321);
        assert_eq!(health.open_issues, 14);
        assert_eq!(health.releases_count, 4);
        assert_eq!(health.ci_passing, Some(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_handles_missing_optional_signals() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": null,
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 100,
                "open_issues_count": 9,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let health = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect("repo_get_health should succeed");

        assert_eq!(health.license, None);
        assert_eq!(health.releases_count, 0);
        assert_eq!(health.ci_passing, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_surfaces_forbidden_check_runs() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "license": null,
                "pushed_at": "2026-02-01T01:02:03Z",
                "stargazers_count": 100,
                "open_issues_count": 9,
                "default_branch": "main"
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/releases"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex/commits/main/check-runs"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "message": "Resource not accessible by integration"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let error = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect_err("forbidden check-runs should fail");

        assert!(
            matches!(
                error,
                ResearchError::Upstream {
                    api: crate::rate_limiter::ResearchApi::GitHub,
                    status,
                    ..
                } if status == reqwest::StatusCode::FORBIDDEN
            ),
            "unexpected error: {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_rejects_non_github_urls() {
        let toolkit = build_test_toolkit("https://api.github.com".to_string(), None);
        let error = toolkit
            .repo_get_health("https://gitlab.com/openai/codex")
            .await
            .expect_err("non-github URL should fail");

        assert!(
            matches!(error, ResearchError::InvalidInput(_)),
            "expected invalid input, got {error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_negative_caches_not_found() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Not Found"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);

        for _ in 0..2 {
            let error = toolkit
                .repo_get_health("https://github.com/openai/missing")
                .await
                .expect_err("missing repo should fail");
            assert!(
                matches!(
                    error,
                    ResearchError::Upstream {
                        api: crate::rate_limiter::ResearchApi::GitHub,
                        status,
                        ..
                    } if status == reqwest::StatusCode::NOT_FOUND
                ),
                "unexpected error: {error}"
            );
        }

        let requests = server
            .received_requests()
            .await
            .expect("request history should be available");
        let metadata_calls = requests
            .iter()
            .filter(|request| request.url.path() == "/repos/openai/missing")
            .count();
        assert_eq!(metadata_calls, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_negative_cache_respects_negative_ttl() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/missing"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "message": "Not Found"
            })))
            .mount(&server)
            .await;

        let mut config = ResearchConfig {
            github_api_base_url: server.uri(),
            ..ResearchConfig::default()
        };
        config.cache_ttls = CacheTtls {
            repo_health: Duration::from_secs(60),
            negative: Duration::from_millis(20),
            ..config.cache_ttls
        };

        let toolkit = build_test_toolkit_with_config(config);

        let first = toolkit
            .repo_get_health("https://github.com/openai/missing")
            .await
            .expect_err("first missing repo request should fail");
        assert!(
            matches!(first, ResearchError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND),
            "unexpected error: {first}"
        );

        tokio::time::sleep(Duration::from_millis(35)).await;

        let second = toolkit
            .repo_get_health("https://github.com/openai/missing")
            .await
            .expect_err("second missing repo request should fail");
        assert!(
            matches!(second, ResearchError::Upstream { status, .. } if status == reqwest::StatusCode::NOT_FOUND),
            "unexpected error: {second}"
        );

        let requests = server
            .received_requests()
            .await
            .expect("request history should be available");
        let metadata_calls = requests
            .iter()
            .filter(|request| request.url.path() == "/repos/openai/missing")
            .count();
        assert_eq!(metadata_calls, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repo_get_health_rate_limit_error_is_retryable() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/openai/codex"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "message": "API rate limit exceeded"
            })))
            .mount(&server)
            .await;

        let toolkit = build_test_toolkit(server.uri(), None);
        let error = toolkit
            .repo_get_health("https://github.com/openai/codex")
            .await
            .expect_err("rate-limited request should fail");

        assert!(error.is_retryable(), "error should be retryable: {error}");
    }

    fn build_test_toolkit(
        github_api_base_url: String,
        github_token: Option<&str>,
    ) -> ResearchToolkit {
        build_test_toolkit_with_config(ResearchConfig {
            github_api_base_url,
            github_token: github_token.map(ToString::to_string),
            ..ResearchConfig::default()
        })
    }
}
