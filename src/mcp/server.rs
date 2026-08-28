//! MCP server handler — implements `ServerHandler` for CogZ.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::storage::Storage;

/// The MCP server. Holds shared state accessible to all tool handlers.
pub struct CogzServer {
    pub storage: Arc<Storage>,
    pub config: Config,
    pub cogz_dir: PathBuf,
}

impl CogzServer {
    pub fn new(storage: Arc<Storage>, config: Config, cogz_dir: PathBuf) -> Self {
        Self {
            storage,
            config,
            cogz_dir,
        }
    }
}
