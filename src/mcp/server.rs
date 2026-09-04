//! MCP server handler — implements `ServerHandler` for CogZ.
//!
//! The server holds a cache of `RepoState` entries keyed by canonical
//! repo path. Every tool call must provide a `repo` parameter — there
//! are no fallbacks. Models are shared across repos: if two repos use
//! the same model, they share one ONNX session instance.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, model::*, tool_handler, transport::stdio,
};

use crate::embed::{ModelType, OnnxEmbeddingModel, OnnxNliModel};
use crate::mcp::errors::mcp_invalid_parameter;
use crate::mcp::repo_cache::{RepoCache, RepoState, make_repo_state, open_repo};

/// Cache key for embedding models: (model_id, dimension, model_type).
type EmbeddingModelKey = (String, usize, ModelType);

/// Cache key for NLI models: model_id only (no dimension or type).
type NliModelKey = String;

/// The MCP server. Holds a cache of repo states and a cache of model
/// instances shared across repos. Every tool call must specify which
/// repo it targets via the required `repo` parameter.
pub struct CogzServer {
    repo_cache: RepoCache,
    models_dir: PathBuf,
    embedding_models: Mutex<HashMap<EmbeddingModelKey, Arc<OnnxEmbeddingModel>>>,
    nli_models: Mutex<HashMap<NliModelKey, Arc<OnnxNliModel>>>,
}

impl CogzServer {
    /// Create a server with no pre-loaded repos. Tool calls must
    /// provide `repo` explicitly. Used by `mcp-stdio` in production.
    pub fn with_models_dir_only(models_dir: &Path) -> Self {
        Self {
            repo_cache: RepoCache::new(),
            models_dir: models_dir.to_path_buf(),
            embedding_models: Mutex::new(HashMap::new()),
            nli_models: Mutex::new(HashMap::new()),
        }
    }

    /// Create a server with a pre-loaded repo. Used by tests to set
    /// up a server with an in-memory Storage without opening a real
    /// DB file. The repo is inserted into the cache so tool calls
    /// with the matching path find it without disk I/O.
    pub fn with_models_dir(
        storage: Arc<crate::storage::Storage>,
        config: crate::config::Config,
        cogz_dir: PathBuf,
        models_dir: &Path,
    ) -> Self {
        let repo_root = cogz_dir
            .parent()
            .unwrap_or(&cogz_dir)
            .canonicalize()
            .unwrap_or_else(|_| cogz_dir.parent().unwrap_or(&cogz_dir).to_path_buf());

        let query_model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            ModelType::Knowledge,
            models_dir,
            config.embedding.dimension,
            &config.embedding.knowledge_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));
        let code_model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            ModelType::Code,
            models_dir,
            config.embedding.dimension,
            &config.embedding.code_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));
        let nli_model = Arc::new(OnnxNliModel::with_resource_config(
            models_dir,
            &config.embedding.nli_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));

        let knowledge_model_id = config.embedding.knowledge_model.clone();
        let code_model_id = config.embedding.code_model.clone();
        let nli_model_id = config.embedding.nli_model.clone();
        let dimension = config.embedding.dimension;

        let state = make_repo_state(
            storage,
            config,
            cogz_dir,
            query_model.clone(),
            code_model.clone(),
            nli_model.clone(),
        );

        let repo_cache = RepoCache::new();
        repo_cache.insert_ready(repo_root, state);

        let mut embedding_models = HashMap::new();
        embedding_models.insert(
            (knowledge_model_id, dimension, ModelType::Knowledge),
            query_model,
        );
        embedding_models.insert((code_model_id, dimension, ModelType::Code), code_model);

        let mut nli_models = HashMap::new();
        nli_models.insert(nli_model_id, nli_model);

        Self {
            repo_cache,
            models_dir: models_dir.to_path_buf(),
            embedding_models: Mutex::new(embedding_models),
            nli_models: Mutex::new(nli_models),
        }
    }

    /// Resolve a repo from the cache or open it on demand.
    /// `repo` is the absolute path to the project root.
    ///
    /// Thundering herd protection: if two concurrent calls target the
    /// same new repo, only one opens it. The other waits on a per-key
    /// mutex, then uses the cached result.
    ///
    /// Config staleness: on cache hit, checks if config.toml was
    /// modified since the entry was created. If so, evicts and reopens.
    pub fn resolve_repo(&self, repo: &str) -> Result<Arc<RepoState>, McpError> {
        let path = PathBuf::from(repo);
        let canonical = path.canonicalize().map_err(|e| {
            mcp_invalid_parameter(&format!(
                "Cannot resolve repo path '{}': {}. Provide an absolute path to the project root.",
                repo, e
            ))
        })?;

        loop {
            // Fast path: cache hit, config fresh.
            match self.repo_cache.get(&canonical) {
                Ok(Some(state)) => return Ok(state),
                Ok(None) => {} // Not ready — fall through to open.
                Err(path) => {
                    // Config changed — evict and retry.
                    self.repo_cache.remove(&path);
                    continue;
                }
            }

            // Slow path: claim the Opening slot.
            let opening_lock = match self.repo_cache.begin_open(&canonical) {
                Some(lock) => lock,
                None => continue, // Another caller inserted — retry.
            };

            // Hold the per-key lock during I/O so concurrent callers wait.
            let _opening_guard = opening_lock.lock().unwrap_or_else(|e| e.into_inner());

            match open_repo(
                &canonical,
                |mt, id, dim, ttl, mb| self.get_or_create_embedding_model(mt, id, dim, ttl, mb),
                |id, ttl, mb| self.get_or_create_nli_model(id, ttl, mb),
            ) {
                Ok(state) => {
                    self.repo_cache.finish_open(canonical, state.clone());
                    return Ok(state);
                }
                Err(e) => {
                    self.repo_cache.remove(&canonical);
                    return Err(e);
                }
            }
        }
    }

    /// Get a shared embedding model from the cache, or create and
    /// cache a new one. Two repos with the same model_id and dimension
    /// share the same ONNX session.
    fn get_or_create_embedding_model(
        &self,
        model_type: ModelType,
        model_id: &str,
        dimension: usize,
        idle_ttl: u64,
        min_free_mb: u64,
    ) -> Result<Arc<OnnxEmbeddingModel>, McpError> {
        let key = (model_id.to_string(), dimension, model_type);

        {
            let cache = self
                .embedding_models
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(model) = cache.get(&key) {
                return Ok(model.clone());
            }
        }

        let model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            model_type,
            &self.models_dir,
            dimension,
            model_id,
            idle_ttl,
            min_free_mb,
        ));

        let mut cache = self
            .embedding_models
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = cache.get(&key) {
            return Ok(existing.clone());
        }
        cache.insert(key, model.clone());
        Ok(model)
    }

    /// Get a shared NLI model from the cache, or create and cache
    /// a new one.
    fn get_or_create_nli_model(
        &self,
        model_id: &str,
        idle_ttl: u64,
        min_free_mb: u64,
    ) -> Result<Arc<OnnxNliModel>, McpError> {
        {
            let cache = self.nli_models.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(model) = cache.get(model_id) {
                return Ok(model.clone());
            }
        }

        let model = Arc::new(OnnxNliModel::with_resource_config(
            &self.models_dir,
            model_id,
            idle_ttl,
            min_free_mb,
        ));

        let mut cache = self.nli_models.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = cache.get(model_id) {
            return Ok(existing.clone());
        }
        cache.insert(model_id.to_string(), model.clone());
        Ok(model)
    }
}

/// Start the MCP server over stdio. Blocks until the client disconnects.
pub async fn run_stdio(server: CogzServer) -> anyhow::Result<()> {
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP serve error: {:?}", e))?;
    service.waiting().await?;
    Ok(())
}

#[tool_handler]
impl ServerHandler for CogzServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new("cogz", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "CogZ — local-first engineering cognition runtime. \
                 Tools: record_observation, query_observations, create_rule, \
                 query_rules, create_knowledge, update_knowledge, query_knowledge, \
                 search, get_context, get_status, list_entities, consolidate, \
                 capture_event. \
                 Every tool requires a `repo` parameter — the absolute path \
                 to the project root containing .cogz/."
                    .to_string(),
            )
    }
}
