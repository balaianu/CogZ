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
use crate::mcp::errors::{mcp_internal_error, mcp_invalid_parameter};
use crate::storage::Storage;

/// Per-repo state: DB connection, config, file root.
/// Model instances are shared via the server's model cache.
pub struct RepoState {
    pub storage: Arc<Storage>,
    pub config: crate::config::Config,
    pub cogz_dir: PathBuf,
    pub query_model: Arc<OnnxEmbeddingModel>,
    pub code_model: Arc<OnnxEmbeddingModel>,
    pub nli_model: Arc<OnnxNliModel>,
}

/// Cache key for embedding models: (model_id, dimension, model_type).
/// Two repos with the same model config share one ONNX session.
type EmbeddingModelKey = (String, usize, ModelType);

/// Cache key for NLI models: model_id only (no dimension or type).
type NliModelKey = String;

/// The MCP server. Holds a cache of repo states and a cache of model
/// instances shared across repos. Every tool call must specify which
/// repo it targets via the required `repo` parameter.
pub struct CogzServer {
    repos: Mutex<HashMap<PathBuf, Arc<RepoState>>>,
    models_dir: PathBuf,
    embedding_models: Mutex<HashMap<EmbeddingModelKey, Arc<OnnxEmbeddingModel>>>,
    nli_models: Mutex<HashMap<NliModelKey, Arc<OnnxNliModel>>>,
}

impl CogzServer {
    /// Create a server with no pre-loaded repos. Tool calls must
    /// provide `repo` explicitly. Used by `mcp-stdio` in production.
    pub fn with_models_dir_only(models_dir: &Path) -> Self {
        Self {
            repos: Mutex::new(HashMap::new()),
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
        storage: Arc<Storage>,
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

        let state = Arc::new(RepoState {
            storage,
            config,
            cogz_dir,
            query_model: query_model.clone(),
            code_model: code_model.clone(),
            nli_model: nli_model.clone(),
        });

        let mut repos = HashMap::new();
        repos.insert(repo_root, state);

        let mut embedding_models = HashMap::new();
        embedding_models.insert(
            (knowledge_model_id, dimension, ModelType::Knowledge),
            query_model,
        );
        embedding_models.insert((code_model_id, dimension, ModelType::Code), code_model);

        let mut nli_models = HashMap::new();
        nli_models.insert(nli_model_id, nli_model);

        Self {
            repos: Mutex::new(repos),
            models_dir: models_dir.to_path_buf(),
            embedding_models: Mutex::new(embedding_models),
            nli_models: Mutex::new(nli_models),
        }
    }

    /// Resolve a repo from the cache or open it on demand.
    /// `repo` is the absolute path to the project root.
    pub fn resolve_repo(&self, repo: &str) -> Result<Arc<RepoState>, McpError> {
        let path = PathBuf::from(repo);
        let canonical = path.canonicalize().map_err(|e| {
            mcp_invalid_parameter(&format!(
                "Cannot resolve repo path '{}': {}. Provide an absolute path to the project root.",
                repo, e
            ))
        })?;

        // Fast path: cache hit (brief lock, no I/O).
        {
            let repos = self.repos.lock().unwrap();
            if let Some(state) = repos.get(&canonical) {
                return Ok(state.clone());
            }
        }

        // Slow path: open a new repo.
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
            Storage::open(&db_path, config.embedding.dimension)
                .map_err(|e| mcp_internal_error("storage", &e.to_string()))?,
        );

        // Get or create shared model instances.
        let query_model = self.get_or_create_embedding_model(
            ModelType::Knowledge,
            &config.embedding.knowledge_model,
            config.embedding.dimension,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        )?;
        let code_model = self.get_or_create_embedding_model(
            ModelType::Code,
            &config.embedding.code_model,
            config.embedding.dimension,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        )?;
        let nli_model = self.get_or_create_nli_model(
            &config.embedding.nli_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        )?;

        let state = Arc::new(RepoState {
            storage,
            config,
            cogz_dir,
            query_model,
            code_model,
            nli_model,
        });

        let mut repos = self.repos.lock().unwrap();
        repos.insert(canonical, state.clone());
        Ok(state)
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

        // Fast path: cache hit.
        {
            let cache = self.embedding_models.lock().unwrap();
            if let Some(model) = cache.get(&key) {
                return Ok(model.clone());
            }
        }

        // Slow path: create and cache.
        let model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            model_type,
            &self.models_dir,
            dimension,
            model_id,
            idle_ttl,
            min_free_mb,
        ));

        let mut cache = self.embedding_models.lock().unwrap();
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
            let cache = self.nli_models.lock().unwrap();
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

        let mut cache = self.nli_models.lock().unwrap();
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
