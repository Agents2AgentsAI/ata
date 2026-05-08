use crate::AuthManager;
use crate::CodexAuth;
use crate::ModelProviderInfo;
use crate::OPENAI_PROVIDER_ID;
use crate::agent::AgentControl;
use crate::codex::Codex;
use crate::codex::CodexSpawnArgs;
use crate::codex::CodexSpawnOk;
use crate::codex::INITIAL_SUBMIT_ID;
use crate::codex_thread::CodexThread;
use crate::config::Config;
#[cfg(feature = "data")]
use crate::data::SharedDataToolkit;
use crate::default_client::build_reqwest_client_with_timeouts;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::features::Feature;
use crate::file_watcher::FileWatcher;
use crate::mcp::McpManager;
use crate::models_manager::collaboration_mode_presets::CollaborationModesConfig;
use crate::models_manager::manager::ModelsManager;
use crate::plugins::PluginsManager;
use crate::protocol::Event;
use crate::protocol::EventMsg;
use crate::protocol::SessionConfiguredEvent;
use crate::research::SharedResearchToolkit;
use crate::rollout::RolloutRecorder;
use crate::rollout::truncation;
use crate::session::Codex;
use crate::session::CodexSpawnArgs;
use crate::session::CodexSpawnOk;
use crate::session::INITIAL_SUBMIT_ID;
use crate::shell_snapshot::ShellSnapshot;
use crate::skills::SkillsManager;
#[cfg(feature = "data")]
use crate::tools::handlers::data::build_data_config;
use crate::tools::handlers::research::build_research_config;
use codex_protocol::ThreadId;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
#[cfg(test)]
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::W3cTraceContext;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::sync::OnceCell;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tracing::warn;

const THREAD_CREATED_CHANNEL_CAPACITY: usize = 1024;
/// Test-only override for enabling thread-manager behaviors used by integration
/// tests.
///
/// In production builds this value should remain at its default (`false`) and
/// must not be toggled.
static FORCE_TEST_THREAD_MANAGER_BEHAVIOR: AtomicBool = AtomicBool::new(false);

type CapturedOps = Vec<(ThreadId, Op)>;
type SharedCapturedOps = Arc<std::sync::Mutex<CapturedOps>>;

pub(crate) fn set_thread_manager_test_mode_for_tests(enabled: bool) {
    FORCE_TEST_THREAD_MANAGER_BEHAVIOR.store(enabled, Ordering::Relaxed);
}

fn should_use_test_thread_manager_behavior() -> bool {
    FORCE_TEST_THREAD_MANAGER_BEHAVIOR.load(Ordering::Relaxed)
}

struct TempCodexHomeGuard {
    path: PathBuf,
}

impl Drop for TempCodexHomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn build_skills_watcher(skills_manager: Arc<SkillsManager>) -> Arc<SkillsWatcher> {
    if should_use_test_thread_manager_behavior()
        && let Ok(handle) = Handle::try_current()
        && handle.runtime_flavor() == RuntimeFlavor::CurrentThread
    {
        // The real watcher spins background tasks that can starve the
        // current-thread test runtime and cause event waits to time out.
        warn!("using noop skills watcher under current-thread test runtime");
        return Arc::new(SkillsWatcher::noop());
    }

    let file_watcher = match FileWatcher::new() {
        Ok(file_watcher) => Arc::new(file_watcher),
        Err(err) => {
            warn!("failed to initialize file watcher: {err}");
            Arc::new(FileWatcher::noop())
        }
    };
    let skills_watcher = Arc::new(SkillsWatcher::new(&file_watcher));

    let mut rx = skills_watcher.subscribe();
    let skills_manager = Arc::clone(&skills_manager);
    if let Ok(handle) = Handle::try_current() {
        handle.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(SkillsWatcherEvent::SkillsChanged { .. }) => {
                        skills_manager.clear_cache();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });
    } else {
        warn!("skills watcher listener skipped: no Tokio runtime available");
    }

    skills_watcher
}

/// Represents a newly created Codex thread (formerly called a conversation), including the first event
/// (which is [`EventMsg::SessionConfigured`]).
pub struct NewThread {
    pub thread_id: ThreadId,
    pub thread: Arc<CodexThread>,
    pub session_configured: SessionConfiguredEvent,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThreadShutdownReport {
    pub completed: Vec<ThreadId>,
    pub submit_failed: Vec<ThreadId>,
    pub timed_out: Vec<ThreadId>,
}

enum ShutdownOutcome {
    Complete,
    SubmitFailed,
    TimedOut,
}

/// [`ThreadManager`] is responsible for creating threads and maintaining
/// them in memory.
pub struct ThreadManager {
    state: Arc<ThreadManagerState>,
    _test_codex_home_guard: Option<TempCodexHomeGuard>,
}

pub struct StartThreadOptions {
    pub config: Config,
    pub initial_history: InitialHistory,
    pub session_source: Option<SessionSource>,
    pub thread_source: Option<ThreadSource>,
    pub dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
    pub persist_extended_history: bool,
    pub metrics_service_name: Option<String>,
    pub parent_trace: Option<W3cTraceContext>,
    pub environments: Vec<TurnEnvironmentSelection>,
}

pub(crate) struct ResumeThreadWithHistoryOptions {
    pub(crate) config: Config,
    pub(crate) initial_history: InitialHistory,
    pub(crate) agent_control: AgentControl,
    pub(crate) session_source: SessionSource,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<crate::exec_policy::ExecPolicyManager>>,
}

/// Shared, `Arc`-owned state for [`ThreadManager`]. This `Arc` is required to have a single
/// `Arc` reference that can be downgraded to by `AgentControl` while preventing every single
/// function to require an `Arc<&Self>`.
pub(crate) struct ThreadManagerState {
    threads: Arc<RwLock<HashMap<ThreadId, Arc<CodexThread>>>>,
    thread_created_tx: broadcast::Sender<ThreadId>,
    auth_manager: Arc<AuthManager>,
    models_manager: Arc<ModelsManager>,
    research_toolkit: OnceCell<Arc<SharedResearchToolkit>>,
    #[cfg(feature = "data")]
    data_toolkit: OnceCell<Arc<SharedDataToolkit>>,
    skills_manager: Arc<SkillsManager>,
    plugins_manager: Arc<PluginsManager>,
    mcp_manager: Arc<McpManager>,
    skills_watcher: Arc<SkillsWatcher>,
    thread_store: Arc<dyn ThreadStore>,
    session_source: SessionSource,
    installation_id: String,
    analytics_events_client: Option<AnalyticsEventsClient>,
    state_db: Option<StateDbHandle>,
    // Captures submitted ops for testing purpose when test mode is enabled.
    ops_log: Option<SharedCapturedOps>,
}

pub fn build_models_manager(
    config: &Config,
    auth_manager: Arc<AuthManager>,
) -> SharedModelsManager {
    let provider = create_model_provider(config.model_provider.clone(), Some(auth_manager));
    provider.models_manager(
        config.codex_home.to_path_buf(),
        config.model_catalog.clone(),
    )
}

pub fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(LocalThreadStore::new(
            LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::Remote { endpoint } => Arc::new(RemoteThreadStore::new(endpoint)),
        ThreadStoreConfig::InMemory { id } => InMemoryThreadStore::for_id(id),
    }
}

impl ThreadManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &Config,
        auth_manager: Arc<AuthManager>,
        session_source: SessionSource,
        collaboration_modes_config: CollaborationModesConfig,
    ) -> Self {
        let codex_home = config.codex_home.clone();
        let openai_models_provider = config
            .model_providers
            .get(OPENAI_PROVIDER_ID)
            .cloned()
            .unwrap_or_else(|| ModelProviderInfo::create_openai_provider(/* base_url */ None));
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let plugins_manager = Arc::new(PluginsManager::new(codex_home.clone()));
        let mcp_manager = Arc::new(McpManager::new(Arc::clone(&plugins_manager)));
        let skills_manager = Arc::new(SkillsManager::new(
            codex_home.clone(),
            Arc::clone(&plugins_manager),
            config.bundled_skills_enabled(),
        ));
        let mcp_manager = Arc::new(McpManager::new(Arc::clone(&plugins_manager)));
        let skills_manager = Arc::new(SkillsManager::new_with_restriction_product(
            codex_home,
            config.bundled_skills_enabled(),
            restriction_product,
        ));
        let skills_watcher = build_skills_watcher(Arc::clone(&skills_manager));
        Self {
            state: Arc::new(ThreadManagerState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: Arc::new(ModelsManager::new_with_provider(
                    codex_home,
                    auth_manager.clone(),
                    config.model_catalog.clone(),
                    collaboration_modes_config,
                    openai_models_provider,
                )),
                research_toolkit: OnceCell::new(),
                #[cfg(feature = "data")]
                data_toolkit: OnceCell::new(),
                skills_manager,
                plugins_manager,
                mcp_manager,
                skills_watcher,
                thread_store,
                auth_manager,
                session_source,
                installation_id,
                analytics_events_client,
                state_db,
                ops_log: should_use_test_thread_manager_behavior()
                    .then(|| Arc::new(std::sync::Mutex::new(Vec::new()))),
            }),
            _test_codex_home_guard: None,
        }
    }

    /// Construct with a dummy AuthManager containing the provided CodexAuth.
    /// Used for integration tests: should not be used by ordinary business logic.
    pub(crate) fn with_models_provider_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
    ) -> Self {
        set_thread_manager_test_mode_for_tests(/*enabled*/ true);
        let codex_home = std::env::temp_dir().join(format!(
            "codex-thread-manager-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&codex_home)
            .unwrap_or_else(|err| panic!("temp codex home dir create failed: {err}"));
        let mut manager = Self::with_models_provider_and_home_for_tests(
            auth,
            provider,
            codex_home.clone(),
            Arc::new(EnvironmentManager::default_for_tests()),
        );
        manager._test_codex_home_guard = Some(TempCodexHomeGuard { path: codex_home });
        manager
    }

    /// Construct with a dummy AuthManager containing the provided CodexAuth and codex home.
    /// Used for integration tests: should not be used by ordinary business logic.
    pub(crate) fn with_models_provider_and_home_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        codex_home: PathBuf,
        environment_manager: Arc<EnvironmentManager>,
    ) -> Self {
        Self::with_models_provider_home_and_state_for_tests(
            auth,
            provider,
            codex_home,
            environment_manager,
            /*state_db*/ None,
        )
    }

    pub(crate) fn with_models_provider_home_and_state_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        codex_home: PathBuf,
        environment_manager: Arc<EnvironmentManager>,
        state_db: Option<StateDbHandle>,
    ) -> Self {
        set_thread_manager_test_mode_for_tests(/*enabled*/ true);
        let auth_manager = AuthManager::from_auth_for_testing(auth);
        let installation_id = uuid::Uuid::new_v4().to_string();
        let skills_codex_home = match AbsolutePathBuf::from_absolute_path_checked(&codex_home) {
            Ok(codex_home) => codex_home,
            Err(err) => panic!("test codex_home should be absolute: {err}"),
        };
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let restriction_product = SessionSource::Exec.restriction_product();
        let plugins_manager = Arc::new(PluginsManager::new_with_restriction_product(
            codex_home.clone(),
            Arc::clone(&plugins_manager),
            true,
        ));
        Self {
            state: Arc::new(ThreadManagerState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: Arc::new(ModelsManager::with_provider_for_tests(
                    codex_home,
                    auth_manager.clone(),
                    provider,
                )),
                research_toolkit: OnceCell::new(),
                #[cfg(feature = "data")]
                data_toolkit: OnceCell::new(),
                skills_manager,
                plugins_manager,
                mcp_manager,
                skills_watcher,
                thread_store,
                auth_manager,
                session_source: SessionSource::Exec,
                installation_id,
                analytics_events_client: None,
                state_db,
                ops_log: should_use_test_thread_manager_behavior()
                    .then(|| Arc::new(std::sync::Mutex::new(Vec::new()))),
            }),
            _test_codex_home_guard: None,
        }
    }

    pub fn session_source(&self) -> SessionSource {
        self.state.session_source.clone()
    }

    pub fn auth_manager(&self) -> Arc<AuthManager> {
        self.state.auth_manager.clone()
    }

    pub fn skills_manager(&self) -> Arc<SkillsManager> {
        self.state.skills_manager.clone()
    }

    pub fn plugins_manager(&self) -> Arc<PluginsManager> {
        self.state.plugins_manager.clone()
    }

    pub fn mcp_manager(&self) -> Arc<McpManager> {
        self.state.mcp_manager.clone()
    }

    pub fn environment_manager(&self) -> Arc<EnvironmentManager> {
        self.state.environment_manager.clone()
    }

    pub fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection> {
        default_thread_environment_selections(self.state.environment_manager.as_ref(), cwd)
    }

    pub fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()> {
        resolve_environment_selections(self.state.environment_manager.as_ref(), environments)
            .map(|_| ())
    }

    pub fn get_models_manager(&self) -> SharedModelsManager {
        self.state.models_manager.clone()
    }

    pub async fn list_models(&self, refresh_strategy: RefreshStrategy) -> Vec<ModelPreset> {
        self.state
            .models_manager
            .list_models(refresh_strategy)
            .await
    }

    pub fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        self.state.models_manager.list_collaboration_modes()
    }

    pub async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.state.list_thread_ids().await
    }

    pub fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadId> {
        self.state.thread_created_tx.subscribe()
    }

    pub async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        self.state.get_thread(thread_id).await
    }

    /// List `thread_id` plus all known descendants in its spawn subtree.
    pub async fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        let thread = self.state.get_thread(thread_id).await?;

        let mut subtree_thread_ids = Vec::new();
        let mut seen_thread_ids = HashSet::new();
        subtree_thread_ids.push(thread_id);
        seen_thread_ids.insert(thread_id);

        if let Some(state_db_ctx) = thread.state_db() {
            for status in [
                DirectionalThreadSpawnEdgeStatus::Open,
                DirectionalThreadSpawnEdgeStatus::Closed,
            ] {
                for descendant_id in state_db_ctx
                    .list_thread_spawn_descendants_with_status(thread_id, status)
                    .await
                    .map_err(|err| {
                        CodexErr::Fatal(format!("failed to load thread-spawn descendants: {err}"))
                    })?
                {
                    if seen_thread_ids.insert(descendant_id) {
                        subtree_thread_ids.push(descendant_id);
                    }
                }
            }
        }

        for descendant_id in thread
            .codex
            .session
            .services
            .agent_control
            .list_live_agent_subtree_thread_ids(thread_id)
            .await?
        {
            if seen_thread_ids.insert(descendant_id) {
                subtree_thread_ids.push(descendant_id);
            }
        }

        Ok(subtree_thread_ids)
    }

    pub async fn start_thread(&self, config: Config) -> CodexResult<NewThread> {
        // Box delegated thread-spawn futures so these convenience wrappers do
        // not inline the full spawn path into every caller's async state.
        Box::pin(self.start_thread_with_tools(
            config,
            Vec::new(),
            /*persist_extended_history*/ false,
        ))
        .await
    }

    pub async fn start_thread_with_tools(
        &self,
        config: Config,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        Box::pin(self.start_thread_with_options(StartThreadOptions {
            config,
            initial_history: InitialHistory::New,
            session_source: None,
            thread_source: None,
            dynamic_tools,
            persist_extended_history,
            None,
            None,
        ))
        .await
    }

    pub async fn start_thread_with_options(
        &self,
        config: Config,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let session_source = options
            .session_source
            .unwrap_or_else(|| self.state.session_source.clone());
        let thread_source = options
            .thread_source
            .or_else(|| options.initial_history.get_resumed_thread_source());
        Box::pin(self.state.spawn_thread_with_source(
            options.config,
            options.initial_history,
            Arc::clone(&self.state.auth_manager),
            self.agent_control(),
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            parent_trace,
        ))
        .await
    }

    pub async fn resume_thread_from_rollout(
        &self,
        config: Config,
        rollout_path: PathBuf,
        auth_manager: Arc<AuthManager>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let initial_history = RolloutRecorder::get_rollout_history(&rollout_path).await?;
        Box::pin(self.resume_thread_with_history(
            config,
            initial_history,
            auth_manager,
            false,
            parent_trace,
        ))
        .await
    }

    pub async fn resume_thread_with_history(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.state.spawn_thread(
            config,
            initial_history,
            auth_manager,
            self.agent_control(),
            thread_source,
            Vec::new(),
            persist_extended_history,
            None,
            parent_trace,
        ))
        .await
    }

    /// Removes the thread from the manager's internal map, though the thread is stored
    /// as `Arc<CodexThread>`, it is possible that other references to it exist elsewhere.
    /// Returns the thread if the thread was found and removed.
    pub async fn remove_thread(&self, thread_id: &ThreadId) -> Option<Arc<CodexThread>> {
        self.state.threads.write().await.remove(thread_id)
    }

    /// Tries to shut down all tracked threads concurrently within the provided timeout.
    /// Threads that complete shutdown are removed from the manager; incomplete shutdowns
    /// remain tracked so callers can retry or inspect them later.
    pub async fn shutdown_all_threads_bounded(&self, timeout: Duration) -> ThreadShutdownReport {
        let threads = {
            let threads = self.state.threads.read().await;
            threads
                .iter()
                .map(|(thread_id, thread)| (*thread_id, Arc::clone(thread)))
                .collect::<Vec<_>>()
        };

        let mut shutdowns = threads
            .into_iter()
            .map(|(thread_id, thread)| async move {
                let outcome = match tokio::time::timeout(timeout, thread.shutdown_and_wait()).await
                {
                    Ok(Ok(())) => ShutdownOutcome::Complete,
                    Ok(Err(_)) => ShutdownOutcome::SubmitFailed,
                    Err(_) => ShutdownOutcome::TimedOut,
                };
                (thread_id, outcome)
            })
            .collect::<FuturesUnordered<_>>();
        let mut report = ThreadShutdownReport::default();

        while let Some((thread_id, outcome)) = shutdowns.next().await {
            match outcome {
                ShutdownOutcome::Complete => report.completed.push(thread_id),
                ShutdownOutcome::SubmitFailed => report.submit_failed.push(thread_id),
                ShutdownOutcome::TimedOut => report.timed_out.push(thread_id),
            }
        }

        let mut tracked_threads = self.state.threads.write().await;
        for thread_id in &report.completed {
            tracked_threads.remove(thread_id);
        }

        report
            .completed
            .sort_by_key(std::string::ToString::to_string);
        report
            .submit_failed
            .sort_by_key(std::string::ToString::to_string);
        report
            .timed_out
            .sort_by_key(std::string::ToString::to_string);
        report
    }

    /// Fork an existing thread by snapshotting rollout history according to
    /// `snapshot` and starting a new thread with identical configuration
    /// (unless overridden by the caller's `config`). The new thread will have
    /// a fresh id.
    pub async fn fork_thread<S>(
        &self,
        snapshot: S,
        config: Config,
        path: PathBuf,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let history = RolloutRecorder::get_rollout_history(&path).await?;
        self.fork_thread_from_history(
            snapshot,
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    /// Fork an existing thread from already-loaded store history.
    pub async fn fork_thread_from_history<S>(
        &self,
        snapshot: S,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        self.fork_thread_with_initial_history(
            snapshot.into(),
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    async fn fork_thread_with_initial_history(
        &self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let interrupted_marker = InterruptedTurnHistoryMarker::from_config(&config);
        let history = fork_history_from_snapshot(snapshot, history, interrupted_marker);
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        Box::pin(self.state.spawn_thread(
            config,
            history,
            Arc::clone(&self.state.auth_manager),
            self.agent_control(),
            thread_source,
            Vec::new(),
            persist_extended_history,
            None,
            parent_trace,
        ))
        .await
    }

    pub(crate) fn agent_control(&self) -> AgentControl {
        AgentControl::new(Arc::downgrade(&self.state))
    }

    #[cfg(test)]
    pub(crate) fn captured_ops(&self) -> Vec<(ThreadId, Op)> {
        self.state
            .ops_log
            .as_ref()
            .and_then(|ops_log| ops_log.lock().ok().map(|log| log.clone()))
            .unwrap_or_default()
    }
}

impl ThreadManagerState {
    pub(crate) fn state_db(&self) -> Option<StateDbHandle> {
        self.state_db.clone()
    }

    pub(crate) async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.threads
            .read()
            .await
            .iter()
            .filter_map(|(thread_id, thread)| {
                (!thread.session_source.is_internal()).then_some(*thread_id)
            })
            .collect()
    }

    /// Fetch a thread by ID or return ThreadNotFound.
    pub(crate) async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        let threads = self.threads.read().await;
        match threads.get(&thread_id) {
            Some(thread) if !thread.session_source.is_internal() => Ok(thread.clone()),
            Some(_) | None => Err(CodexErr::ThreadNotFound(thread_id)),
        }
    }

    pub(crate) async fn read_stored_thread(
        &self,
        params: ReadThreadParams,
    ) -> CodexResult<StoredThread> {
        let thread_id = params.thread_id;
        self.thread_store
            .read_thread(params)
            .await
            .map_err(|err| match err {
                ThreadStoreError::ThreadNotFound { thread_id } => {
                    CodexErr::ThreadNotFound(thread_id)
                }
                ThreadStoreError::InvalidRequest { message } => {
                    if message.starts_with("no rollout found for thread id ") {
                        CodexErr::ThreadNotFound(thread_id)
                    } else {
                        CodexErr::Fatal(format!(
                            "failed to read stored thread {thread_id}: invalid thread-store request: {message}"
                        ))
                    }
                }
                err => CodexErr::Fatal(format!("failed to read stored thread {thread_id}: {err}")),
            })
    }

    /// Send an operation to a thread by ID.
    pub(crate) async fn send_op(&self, thread_id: ThreadId, op: Op) -> CodexResult<String> {
        let thread = self.get_thread(thread_id).await?;
        if let Some(ops_log) = &self.ops_log
            && let Ok(mut log) = ops_log.lock()
        {
            log.push((thread_id, op.clone()));
        }
        thread.submit(op).await
    }

    #[cfg(test)]
    /// Append a prebuilt message to a thread by ID outside the normal user-input path.
    pub(crate) async fn append_message(
        &self,
        thread_id: ThreadId,
        message: ResponseItem,
    ) -> CodexResult<String> {
        let thread = self.get_thread(thread_id).await?;
        thread.append_message(message).await
    }

    /// Remove a thread from the manager by ID, returning it when present.
    pub(crate) async fn remove_thread(&self, thread_id: &ThreadId) -> Option<Arc<CodexThread>> {
        self.threads.write().await.remove(thread_id)
    }

    /// Spawn a new thread with no history using a provided config.
    pub(crate) async fn spawn_new_thread(
        &self,
        config: Config,
        agent_control: AgentControl,
    ) -> CodexResult<NewThread> {
        Box::pin(self.spawn_new_thread_with_source(
            config,
            agent_control,
            self.session_source.clone(),
            /*thread_source*/ None,
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            /*inherited_shell_snapshot*/ None,
            /*inherited_exec_policy*/ None,
            /*environments*/ None,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_new_thread_with_source(
        &self,
        config: Config,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<crate::exec_policy::ExecPolicyManager>>,
        environments: Option<Vec<TurnEnvironmentSelection>>,
    ) -> CodexResult<NewThread> {
        let environments = environments.unwrap_or_else(|| {
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd)
        });
        Box::pin(self.spawn_thread_with_source(
            config,
            InitialHistory::New,
            Arc::clone(&self.auth_manager),
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            None,
        ))
        .await
    }

    pub(crate) async fn resume_thread_with_history_with_source(
        &self,
        options: ResumeThreadWithHistoryOptions,
    ) -> CodexResult<NewThread> {
        let ResumeThreadWithHistoryOptions {
            config,
            initial_history,
            agent_control,
            session_source,
            inherited_shell_snapshot,
            inherited_exec_policy,
        } = options;
        let environments =
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd);
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            Arc::clone(&self.auth_manager),
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            inherited_shell_snapshot,
            None,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fork_thread_with_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<crate::exec_policy::ExecPolicyManager>>,
        environments: Option<Vec<TurnEnvironmentSelection>>,
    ) -> CodexResult<NewThread> {
        let environments = environments.unwrap_or_else(|| {
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd)
        });
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            Arc::clone(&self.auth_manager),
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            persist_extended_history,
            /*metrics_service_name*/ None,
            inherited_shell_snapshot,
            None,
        ))
        .await
    }

    /// Spawn a new thread with optional history and register it with the manager.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        agent_control: AgentControl,
        thread_source: Option<ThreadSource>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            auth_manager,
            agent_control,
            self.session_source.clone(),
            thread_source,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            None,
            parent_trace,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread_with_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        auth_manager: Arc<AuthManager>,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let watch_registration = self
            .file_watcher
            .register_config(&config, self.skills_manager.as_ref());
        let research_toolkit = Some(
            self.research_toolkit
                .get_or_init(|| async {
                    let research_config = build_research_config(
                        config.research.as_ref(),
                        config.codex_home.as_path(),
                        config.cwd.as_path(),
                    );
                    Arc::new(codex_research_tools::ResearchToolkit::new(
                        build_reqwest_client_with_timeouts(
                            Some(research_config.connect_timeout),
                            Some(research_config.request_timeout),
                        ),
                        research_config,
                    ))
                })
                .await
                .clone(),
        );
        let data_toolkit = if config.features.enabled(Feature::Data) {
            #[cfg(feature = "data")]
            {
                Some(
                    self.data_toolkit
                        .get_or_init(|| async {
                            let data_config = build_data_config(
                                config.data.as_ref(),
                                config.codex_home.as_path(),
                                config.cwd.as_path(),
                            );
                            Arc::new(codex_data_tools::DataToolkit::new(
                                build_reqwest_client_with_timeouts(
                                    Some(data_config.connect_timeout),
                                    Some(data_config.request_timeout),
                                ),
                                data_config,
                            ))
                        })
                        .await
                        .clone(),
                )
            }
            #[cfg(not(feature = "data"))]
            {
                warn!(
                    "data feature flag is enabled in config, but codex-core was built without `--features data`"
                );
                None
            }
        } else {
            None
        };
        let CodexSpawnOk {
            codex, thread_id, ..
        } = Codex::spawn(CodexSpawnArgs {
            config,
            installation_id: self.installation_id.clone(),
            auth_manager,
            models_manager: Arc::clone(&self.models_manager),
            skills_manager: Arc::clone(&self.skills_manager),
            plugins_manager: Arc::clone(&self.plugins_manager),
            mcp_manager: Arc::clone(&self.mcp_manager),
            file_watcher: Arc::clone(&self.file_watcher),
            conversation_history: initial_history,
            session_source,
            thread_source,
            agent_control,
            dynamic_tools,
            research_toolkit,
            data_toolkit,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            parent_trace,
        })
        .await?;
        let new_thread = self
            .finalize_thread_spawn(codex, thread_id, tracked_session_source, watch_registration)
            .await?;
        if is_resumed_thread
            && let Err(err) = new_thread.thread.apply_goal_resume_runtime_effects().await
        {
            warn!("failed to apply goal resume runtime effects: {err}");
        }
        Ok(new_thread)
    }

    async fn finalize_thread_spawn(
        &self,
        codex: Codex,
        thread_id: ThreadId,
        session_source: SessionSource,
        watch_registration: crate::file_watcher::WatchRegistration,
    ) -> CodexResult<NewThread> {
        let event = codex.next_event().await?;
        let session_configured = match event {
            Event {
                id,
                msg: EventMsg::SessionConfigured(session_configured),
            } if id == INITIAL_SUBMIT_ID => session_configured,
            _ => {
                return Err(CodexErr::SessionConfiguredNotFirstEvent);
            }
        };

        {
            let mut threads = self.threads.write().await;
            if let std::collections::hash_map::Entry::Vacant(e) = threads.entry(thread_id) {
                let thread = Arc::new(CodexThread::new(
                    codex,
                    session_configured.clone(),
                    session_configured.rollout_path.clone(),
                    session_source,
                    watch_registration,
                ));
                e.insert(thread.clone());
                return Ok(NewThread {
                    thread_id,
                    thread,
                    session_configured,
                });
            }
        }

        if let Err(err) = codex.shutdown_and_wait().await {
            warn!("failed to shut down duplicate thread {thread_id}: {err}");
        }
        Err(CodexErr::InvalidRequest(format!(
            "thread {thread_id} is already running"
        )))
    }

    pub(crate) fn notify_thread_created(&self, thread_id: ThreadId) {
        let _ = self.thread_created_tx.send(thread_id);
    }

    async fn parent_rollout_thread_trace_for_source(
        &self,
        session_source: &SessionSource,
        initial_history: &InitialHistory,
    ) -> codex_rollout_trace::ThreadTraceContext {
        // A fresh v2 child belongs to the same rollout tree as its parent, so
        // session startup derives its child trace from the parent's thread
        // context. Resumed children already have a prior `ThreadStarted` event
        // for this thread id; deriving a child trace during resume would write
        // that start event again and make the bundle unreplayable.
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) = session_source
        else {
            return codex_rollout_trace::ThreadTraceContext::disabled();
        };
        if matches!(initial_history, InitialHistory::Resumed(_)) {
            return codex_rollout_trace::ThreadTraceContext::disabled();
        }
        // Parent lookup can fail if the parent was closed or released between
        // spawn preparation and session construction. Tracing is diagnostic, so
        // that race should not block child creation; the child simply starts
        // without a parent rollout trace.
        self.get_thread(*parent_thread_id)
            .await
            .ok()
            .map(|thread| thread.codex.session.services.rollout_thread_trace.clone())
            .unwrap_or_else(codex_rollout_trace::ThreadTraceContext::disabled)
    }
}

/// Return a fork snapshot cut strictly before the nth user message (0-based).
///
/// Out-of-range values keep the full committed history at a turn boundary, but
/// when the source thread is currently mid-turn they fall back to cutting
/// before the active turn's opening boundary so the fork omits the unfinished
/// suffix entirely.
fn truncate_before_nth_user_message(
    history: InitialHistory,
    n: usize,
    snapshot_state: &SnapshotTurnState,
) -> InitialHistory {
    let items: Vec<RolloutItem> = history.get_rollout_items();
    let user_positions = truncation::user_message_positions_in_rollout(&items);
    let rolled = if snapshot_state.ends_mid_turn && n >= user_positions.len() {
        if let Some(cut_idx) = snapshot_state
            .active_turn_start_index
            .or_else(|| user_positions.last().copied())
        {
            items[..cut_idx].to_vec()
        } else {
            items
        }
    } else {
        truncation::truncate_rollout_before_nth_user_message_from_start(&items, n)
    };

    if rolled.is_empty() {
        InitialHistory::New
    } else {
        InitialHistory::Forked(rolled)
    }
}

#[cfg(test)]
#[path = "thread_manager_tests.rs"]
mod tests;
