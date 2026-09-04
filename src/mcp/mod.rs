//! MCP server — exposes CogZ tools over the Model Context Protocol.
//!
//! The server holds an `Arc<Storage>` and `Config`. Tool handlers are
//! async methods on `CogzServer`. DB operations go through
//! `tokio::task::spawn_blocking` to avoid blocking the tokio runtime.

pub mod dedup;
pub mod errors;
pub mod helpers;
pub mod params;
pub mod responses;
pub mod server;
pub mod status;
pub mod tools;
pub mod tools_query;
pub mod tools_search;
pub mod tools_system;
pub mod tools_write;
pub mod update_knowledge;

pub use server::{CogzServer, RepoState};
