//! Repo cache with thundering herd protection, LRU eviction, and
//! config staleness detection.
//!
//! Extracted from `server.rs` to keep that file under 400 lines.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use rmcp::ErrorData as McpError;

use crate::mcp::errors::{mcp_internal_error, mcp_invalid_parameter};

/// Maximum number of repos cached before LRU eviction kicks in.
const MAX_CACHED_REPOS: usize = 8;

/// Per-repo state: DB connection, config, file root.
/// Model instances are shared via the server's model cache.
pub struct RepoState {
    pub storage: Arc<crate::storage::Storage>,
    pub config: crate::config::Config,
    pub cogz_dir: PathBuf,
    pub query_model: Arc<crate::embed::OnnxEmbeddingModel>,
    pub code_model: Arc<crate::embed::OnnxEmbeddingModel>,
    pub nli_model: Arc<crate::embed::OnnxNliModel>,
    /// mtime of config.toml when this entry was created. Used to
    /// detect config changes and trigger a cache reload.
    config_mtime: SystemTime,
}

/// Cache entry: either being opened (thundering herd guard) or ready.
enum RepoCacheEntry {
    /// A caller is opening this repo. Other callers wait on the mutex.
    Opening(Arc<Mutex<()>>),
    /// Repo is open and ready to serve tool calls.
    Ready(Arc<RepoState>),
}

/// Repo cache with LRU tracking and thundering herd protection.
///
/// Lock ordering: always acquire `repos` before `lru_order`. This
/// ordering is consistent across all methods. Violating it risks
/// deadlock.
pub struct RepoCache {
    repos: Mutex<HashMap<PathBuf, RepoCacheEntry>>,
    lru_order: Mutex<Vec<PathBuf>>,
}

impl Default for RepoCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RepoCache {
    pub fn new() -> Self {
        Self {
            repos: Mutex::new(HashMap::new()),
            lru_order: Mutex::new(Vec::new()),
        }
    }

    /// Insert a pre-built RepoState (used by tests and the
    /// `with_models_dir` constructor).
    pub fn insert_ready(&self, path: PathBuf, state: Arc<RepoState>) {
        let mut repos = self.lock_repos();
        repos.insert(path.clone(), RepoCacheEntry::Ready(state));
        drop(repos);
        let mut order = self.lock_lru();
        order.retain(|p| p != &path);
        order.push(path);
    }

    /// Try to get a ready repo from the cache. Returns:
    /// - `Ok(Some(state))` if cached and ready (also touches LRU)
    /// - `Ok(None)` if not cached or being opened
    /// - `Err(path)` if cached but config changed (caller should evict)
    pub fn get(&self, path: &PathBuf) -> Result<Option<Arc<RepoState>>, PathBuf> {
        let repos = self.lock_repos();
        match repos.get(path) {
            Some(RepoCacheEntry::Ready(state)) => {
                // Config staleness check.
                let config_path = state.cogz_dir.join("config.toml");
                let current_mtime = std::fs::metadata(&config_path)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                if current_mtime != state.config_mtime {
                    tracing::info!("Config changed for {}, reloading", path.display());
                    return Err(path.clone());
                }
                let state = state.clone();
                drop(repos);
                self.touch_lru(path);
                Ok(Some(state))
            }
            Some(RepoCacheEntry::Opening(lock)) => {
                let lock = lock.clone();
                drop(repos);
                let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Insert an Opening marker so concurrent callers wait. Returns
    /// the lock to hold during I/O. If another caller already inserted
    /// an entry (Ready or Opening), returns None — the caller should
    /// retry `get`.
    pub fn begin_open(&self, path: &PathBuf) -> Option<Arc<Mutex<()>>> {
        let mut repos = self.lock_repos();
        match repos.get(path) {
            Some(_) => None, // Entry exists — retry get.
            None => {
                let lock = Arc::new(Mutex::new(()));
                repos.insert(path.clone(), RepoCacheEntry::Opening(lock.clone()));
                Some(lock)
            }
        }
    }

    /// Finalize an open: replace Opening with Ready.
    pub fn finish_open(&self, path: PathBuf, state: Arc<RepoState>) {
        let mut repos = self.lock_repos();
        repos.insert(path.clone(), RepoCacheEntry::Ready(state));
        drop(repos);
        self.touch_lru(&path);
        self.evict_if_over_capacity();
    }

    /// Remove an entry (used on open failure or config staleness).
    pub fn remove(&self, path: &PathBuf) {
        let mut repos = self.lock_repos();
        repos.remove(path);
        drop(repos);
        let mut order = self.lock_lru();
        order.retain(|p| p != path);
    }

    /// Move a path to the back of the LRU queue (most recently used).
    fn touch_lru(&self, path: &PathBuf) {
        let mut order = self.lock_lru();
        order.retain(|p| p != path);
        order.push(path.clone());
    }

    /// If the cache exceeds MAX_CACHED_REPOS, evict the least recently
    /// used Ready entry (front of the LRU queue).
    fn evict_if_over_capacity(&self) {
        loop {
            // Lock repos first (consistent ordering), then lru_order.
            let to_evict = {
                let repos = self.lock_repos();
                let order = self.lock_lru();
                if order.len() <= MAX_CACHED_REPOS {
                    None
                } else {
                    order
                        .iter()
                        .find(|p| matches!(repos.get(*p), Some(RepoCacheEntry::Ready(_))))
                        .cloned()
                }
            };

            match to_evict {
                Some(path) => {
                    tracing::debug!("Evicting repo from cache: {}", path.display());
                    self.remove(&path);
                }
                None => break,
            }
        }
    }

    /// Lock the repos mutex, recovering from poisoning.
    fn lock_repos(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, RepoCacheEntry>> {
        self.repos.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lock the LRU mutex, recovering from poisoning.
    fn lock_lru(&self) -> std::sync::MutexGuard<'_, Vec<PathBuf>> {
        self.lru_order.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Open a repo: load config, open DB, get shared models.
/// Called by the server when a cache miss occurs.
pub fn open_repo(
    canonical: &std::path::Path,
    get_embedding_model: impl Fn(
        crate::embed::ModelType,
        &str,
        usize,
        u64,
        u64,
    ) -> Result<Arc<crate::embed::OnnxEmbeddingModel>, McpError>,
    get_nli_model: impl Fn(&str, u64, u64) -> Result<Arc<crate::embed::OnnxNliModel>, McpError>,
) -> Result<Arc<RepoState>, McpError> {
    let cogz_dir = canonical.join(".cogz");
    let config_path = cogz_dir.join("config.toml");
    if !config_path.exists() {
        return Err(mcp_invalid_parameter(&format!(
            "No .cogz/ directory found in {}. Run `cogz init` first.",
            canonical.display()
        )));
    }

    let config = crate::config::load(&config_path)
        .map_err(|e| mcp_internal_error("config", &e.to_string()))?;

    let db_path = canonical.join(&config.storage.db_path);
    if !db_path.exists() {
        return Err(mcp_invalid_parameter(&format!(
            "Database not found at {}. Run `cogz index` first.",
            db_path.display()
        )));
    }

    let storage = Arc::new(
        crate::storage::Storage::open(&db_path, config.embedding.dimension)
            .map_err(|e| mcp_internal_error("storage", &e.to_string()))?,
    );

    let query_model = get_embedding_model(
        crate::embed::ModelType::Knowledge,
        &config.embedding.knowledge_model,
        config.embedding.dimension,
        config.embedding.model_idle_ttl,
        config.embedding.model_min_free_mb,
    )?;
    let code_model = get_embedding_model(
        crate::embed::ModelType::Code,
        &config.embedding.code_model,
        config.embedding.dimension,
        config.embedding.model_idle_ttl,
        config.embedding.model_min_free_mb,
    )?;
    let nli_model = get_nli_model(
        &config.embedding.nli_model,
        config.embedding.model_idle_ttl,
        config.embedding.model_min_free_mb,
    )?;

    let config_mtime = std::fs::metadata(&config_path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    Ok(Arc::new(RepoState {
        storage,
        config,
        cogz_dir,
        query_model,
        code_model,
        nli_model,
        config_mtime,
    }))
}

/// Create a RepoState from pre-opened components (used by tests).
pub fn make_repo_state(
    storage: Arc<crate::storage::Storage>,
    config: crate::config::Config,
    cogz_dir: PathBuf,
    query_model: Arc<crate::embed::OnnxEmbeddingModel>,
    code_model: Arc<crate::embed::OnnxEmbeddingModel>,
    nli_model: Arc<crate::embed::OnnxNliModel>,
) -> Arc<RepoState> {
    let config_mtime = std::fs::metadata(cogz_dir.join("config.toml"))
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    Arc::new(RepoState {
        storage,
        config,
        cogz_dir,
        query_model,
        code_model,
        nli_model,
        config_mtime,
    })
}
