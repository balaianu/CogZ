//! MCP server handler — implements `ServerHandler` for CogZ.
//!
//! The server holds a cache of `RepoState` entries keyed by canonical
//! repo path. Tool calls specify which repo they target via an optional
//! `repo` parameter. If omitted, the server falls back to a default
//! repo (provided via `--repo` at startup) or cwd.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, model::*, tool_handler, transport::stdio,
};

use crate::config::Config;
use crate::embed::{ModelType, OnnxEmbeddingModel, OnnxNliModel};
use crate::mcp::errors::{mcp_internal_error, mcp_invalid_parameter};
use crate::storage::Storage;

/// Per-repo state: DB connection, config, file root, and model instances.
/// One of these exists for each repo the server has opened.
pub struct RepoState {
    pub storage: Arc<Storage>,
    pub config: Config,
    pub cogz_dir: PathBuf,
    pub query_model: Arc<OnnxEmbeddingModel>,
    pub code_model: Arc<OnnxEmbeddingModel>,
    pub nli_model: Arc<OnnxNliModel>,
}

/// The MCP server. Holds a cache of repo states so tool calls can
/// target any repo with a `.cogz/` directory. Models are lazy-loaded
/// per repo — opening a repo doesn't load ONNX models until first use.
pub struct CogzServer {
    repos: Mutex<HashMap<PathBuf, Arc<RepoState>>>,
    models_dir: PathBuf,
    default_repo: Option<PathBuf>,
}

impl CogzServer {
    /// Create a server with a pre-loaded default repo. Used when
    /// `--repo` is passed to `mcp-stdio` and by tests.
    pub fn with_models_dir(
        storage: Arc<Storage>,
        config: Config,
        cogz_dir: PathBuf,
        models_dir: &Path,
    ) -> Self {
        let repo_root = cogz_dir.parent().unwrap_or(&cogz_dir).to_path_buf();

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

        let state = Arc::new(RepoState {
            storage,
            config,
            cogz_dir: cogz_dir.clone(),
            query_model,
            code_model,
            nli_model,
        });

        let mut repos = HashMap::new();
        repos.insert(repo_root.clone(), state);

        Self {
            repos: Mutex::new(repos),
            models_dir: models_dir.to_path_buf(),
            default_repo: Some(repo_root),
        }
    }

    /// Create an empty server with no pre-loaded repos. Tool calls
    /// must specify `repo` or the server tries cwd.
    pub fn empty(models_dir: &Path) -> Self {
        Self {
            repos: Mutex::new(HashMap::new()),
            models_dir: models_dir.to_path_buf(),
            default_repo: None,
        }
    }

    /// Resolve a repo from the cache or open it on demand.
    /// If `repo` is None, falls back to `default_repo` or cwd.
    pub fn resolve_repo(&self, repo: Option<&str>) -> Result<Arc<RepoState>, McpError> {
        let path = match repo {
            Some(r) => PathBuf::from(r),
            None => match &self.default_repo {
                Some(d) => d.clone(),
                None => std::env::current_dir()
                    .map_err(|e| mcp_internal_error("resolve_repo", &e.to_string()))?,
            },
        };

        let canonical = path.canonicalize().unwrap_or(path);

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

        let query_model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            ModelType::Knowledge,
            &self.models_dir,
            config.embedding.dimension,
            &config.embedding.knowledge_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));
        let code_model = Arc::new(OnnxEmbeddingModel::with_resource_config(
            ModelType::Code,
            &self.models_dir,
            config.embedding.dimension,
            &config.embedding.code_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));
        let nli_model = Arc::new(OnnxNliModel::with_resource_config(
            &self.models_dir,
            &config.embedding.nli_model,
            config.embedding.model_idle_ttl,
            config.embedding.model_min_free_mb,
        ));

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
                 All tools accept an optional `repo` parameter (absolute path \
                 to the project root containing .cogz/). If omitted, the \
                 server default or cwd is used."
                    .to_string(),
            )
    }
}
