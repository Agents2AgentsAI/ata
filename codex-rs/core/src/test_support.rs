//! Test-only helpers exposed for cross-crate integration tests.
//!
//! Production code should not depend on this module.
//! We prefer this to using a crate feature to avoid building multiple
//! permutations of the crate.

use std::path::PathBuf;
use std::sync::Arc;

use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;

use crate::AuthManager;
use crate::CodexAuth;
use crate::ModelProviderInfo;
use crate::ThreadManager;
use crate::config::Config;
use crate::models_manager::collaboration_mode_presets;
use crate::models_manager::manager::ModelsManager;
use crate::models_manager::model_presets;
use crate::thread_manager;
use crate::unified_exec;

pub fn set_thread_manager_test_mode(enabled: bool) {
    thread_manager::set_thread_manager_test_mode_for_tests(enabled);
}

pub fn set_deterministic_process_ids(enabled: bool) {
    unified_exec::set_deterministic_process_ids_for_tests(enabled);
}

pub fn auth_manager_from_auth(auth: CodexAuth) -> Arc<AuthManager> {
    AuthManager::from_auth_for_testing(auth)
}

pub fn auth_manager_from_auth_with_home(auth: CodexAuth, codex_home: PathBuf) -> Arc<AuthManager> {
    AuthManager::from_auth_for_testing_with_home(auth, codex_home)
}

pub fn thread_manager_with_models_provider(
    auth: CodexAuth,
    provider: ModelProviderInfo,
) -> ThreadManager {
    ThreadManager::with_models_provider_for_tests(auth, provider)
}

pub fn thread_manager_with_models_provider_and_home(
    auth: CodexAuth,
    provider: ModelProviderInfo,
    codex_home: PathBuf,
) -> ThreadManager {
    ThreadManager::with_models_provider_and_home_for_tests(auth, provider, codex_home)
}

pub fn models_manager_with_provider(
    codex_home: PathBuf,
    auth_manager: Arc<AuthManager>,
    provider: ModelProviderInfo,
) -> ModelsManager {
    ModelsManager::with_provider_for_tests(codex_home, auth_manager, provider)
}

pub fn get_model_offline(model: Option<&str>) -> String {
    ModelsManager::get_model_offline_for_tests(model)
}

pub fn construct_model_info_offline(model: &str, config: &Config) -> ModelInfo {
    ModelsManager::construct_model_info_offline_for_tests(model, config)
}

pub fn all_model_presets() -> &'static Vec<ModelPreset> {
    &model_presets::PRESETS
}

pub fn builtin_collaboration_mode_presets() -> Vec<CollaborationModeMask> {
    collaboration_mode_presets::builtin_collaboration_mode_presets()
}

/// Pre-populate the URL file download cache so that the `attach_url_files`
/// tool handler finds a cached entry for `url` without making a network
/// request.  Intended for integration tests that need `url_file` content
/// blocks to appear in API request bodies.
pub async fn prepopulate_url_file_cache(codex_home: &std::path::Path, url: &str, content: &[u8]) {
    use crate::tools::url_downloader::cache_entry_dir;
    use crate::tools::url_validation::normalize_url_for_cache;

    let parsed = url::Url::parse(url).expect("valid test URL");
    let normalized_cache_key = normalize_url_for_cache(&parsed);
    let filename = crate::tools::url_validation::derive_pdf_filename(&parsed, None);
    let dir = cache_entry_dir(codex_home, &normalized_cache_key);
    tokio::fs::create_dir_all(&dir)
        .await
        .expect("create cache dir for test");
    let path = dir.join(filename);
    tokio::fs::write(&path, content)
        .await
        .expect("write test cache file");
}
