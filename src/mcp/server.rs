//! MCP server handler — implements `ServerHandler` for CogZ.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{ServerHandler, ServiceExt, model::*, tool_handler, transport::stdio};

use crate::config::Config;
use crate::embed::{ModelType, OnnxEmbeddingModel};
use crate::storage::Storage;

/// The MCP server. Holds shared state accessible to all tool handlers.
/// The `query_model` is a persistent ONNX model used for query embedding
/// — it loads lazily on first use and is reused across calls to avoid
/// reloading the model from disk on every search.
pub struct CogzServer {
    pub storage: Arc<Storage>,
    pub config: Config,
    pub cogz_dir: PathBuf,
    pub query_model: Arc<OnnxEmbeddingModel>,
}

impl CogzServer {
    pub fn new(storage: Arc<Storage>, config: Config, cogz_dir: PathBuf) -> Self {
        let models_dir = crate::embed::models_dir();
        let query_model = Arc::new(OnnxEmbeddingModel::new(
            ModelType::Knowledge,
            &models_dir,
            config.embedding.dimension,
        ));
        Self {
            storage,
            config,
            cogz_dir,
            query_model,
        }
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
            .with_instructions(
                "CogZ — local-first engineering cognition runtime. \
                 Tools: record_observation, query_observations, create_rule, \
                 query_rules, create_knowledge, update_knowledge, query_knowledge, \
                 search, get_context, get_status, list_entities."
                    .to_string(),
            )
    }
}
